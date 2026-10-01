# 目标检测设计要点

> 实时多模型 YOLO 检测（`crates/nvr-detect` + `nvr/src/detect`）与按设备检测配置（device-config Phase 1）的范围、取舍、清单与环境变量语义、API/UI 契约及端到端验证步骤；启动顺序、auto-start 重试、大栈线程、清单缺失不阻塞启动见 `architecture.md`。

## 一、范围

已完成：
- 离线：`nvr-detect` 库（`Detector` trait + usls 后端 + `DetectorSet` 串行对比）与 `detect-compare` 示例。
- 实时：订阅管线解码视频 → 抽帧 → 同一帧并发喂 N 个模型 → 存每个管线的最新结果 → REST 轮询读取。
- 看板叠框：预览里 `DetectionOverlay` 画框，按模型着色，勾选框控制各模型显隐（纯前端，不调 API）。
- device-config Phase 1：设备持久化 `config.detect`，建/改管线时自动启停；设备对话框改成 Tabs，加「检测」tab。

明确延后（各自单独立 spec）：
- 服务端烧录框 + 重编码 + 回推 ZLM（带框流）；推送通道（SSE/WebSocket），目前只能轮询。
- 跟踪、计数、区域/越线规则、告警事件；检测结果持久化。
- 每个模型多种输入尺寸、训练/导出；GPU：`device` 字段会透传给 usls，但 crate 没开任何 CUDA/TensorRT feature，只验证过 CPU。
- device-config 后续阶段（复用 Phase 1 的 Tabs 外壳和 `DeviceConfig` JSON 存储）：
  - Phase 2：ONVIF/GB 码流参数（profile/子码流选择；ONVIF `profile_token` 已接进 `stream_uri()`）。另一项待办：暴露 onvif/stream supervisor 里的内部 pipe，让检测也能挂上去。
  - Phase 3：转发路由，需要 targets CRUD（Settings）、设备到 target 的映射、worker 过滤（worker 目前完全是全局的）。
  - Phase 4：录像，按设备覆盖保留期，和/或录像计划（后端还没有，工作量最大）。

## 二、关键决策与原因

- **后端选 usls（基于 ort/ONNX Runtime），外面包一层 `Detector` trait**：各代 YOLO 输出布局不同（v5/v7 是 `[n,85]`、带 objectness；v8+ 是 `[84,n]`、不带），usls 已经封装了 v5/v8/v9/v11/RT-DETR 的前后处理，做多版本对比几乎不用写胶水代码。trait 输入是 RGB8 裸字节 + 宽高，不是 `image::RgbImage`，这样同时和 usls、`image` crate 解耦，以后能换 ort 直连或 candle 后端。
- **usls 用 `ort-load-dynamic`，不静态链 ORT**：避免和 `nvr-asr`（sherpa-onnx 自带 ONNX Runtime）在同一进程里符号冲突。代价是运行时必须能找到版本匹配的 `libonnxruntime.so`（见第三节）。
- **多模型对比 = 同一帧扇出**：每个模型在自己的大栈线程上并发推理，各自计时。单个模型出错只填它那条 `ModelResult.error`，其他模型照常出结果，tap 也不停。
- **detector 进程内共享**：首次 start 时一次性构建清单里的**全部**模型并缓存，之后各管线按名字筛子集。所以：任意一个模型加载失败，所有 start 都会失败；清单只在启动时读一次，改 `models.json` 要重启；usls 模型包在 `Mutex` 里，多个管线同时检测时，同一个模型的推理会串行。
- **抽帧，不排队**：距上次采样不足间隔的帧直接丢弃，broadcast `Lagged` 也忽略。推理是瓶颈，只处理越过间隔后到达的最新帧。hub 默认间隔 500ms（`main.rs` 传入，约 2fps）。
- **帧来源**：`subscribe_video` 和 ASR 的 `subscribe_audio` 对称。stream-copy 管线本来没有解码器，检测会为它按需另起一个视频解码器。
- **结果存储/服务**：hub 只保留每个管线最新一帧（内存里，不持久化）。stop 后旧结果仍然保留，`GET latest` 能读到旧帧，靠 `ts`（Unix 秒）判断是否过期。前端停止时只清自己本地的状态。
- **坐标**：bbox 是原始帧像素坐标，附带 `frame_w/frame_h`，由消费方自己缩放。叠框按预览 `<video>` 的 `object-fit: contain` 做 letterbox 映射（`utils/detectOverlay.ts` 的 `frameToScreen`：scale 取宽高比例的较小值，居中加偏移）。overlay 测量的是 `.preview-media` 容器的尺寸，不依赖 video 元素本身。模型颜色按它在 `listDetectModels()` 里的序号从固定调色板取，所以某帧报错没出结果，颜色也不会变。
- **按设备配置的形态**：`DeviceInfo.config: DeviceConfig { detect: Option<DetectConfig> }`，存在 KV JSON 里，带 `#[serde(default)]`，**不需要 migration**，旧数据行解出来就是 `detect: None`（关闭）。`DetectConfig { enabled, models(空=全部), sample_every_ms(0=hub 默认), min_confidence(0=保留模型自带阈值) }`。
- **`min_confidence` 是推理后过滤**（已批准的决策 a）：不按设备重建模型，否则每个设备都要重新实例化一个 ONNX session。实际生效的阈值是 `max(manifest conf, min_confidence)`。
- **支持检测的输入类型**：只有 manager 里登记为 `Entry::Pipe` 的才支持：`net/rtsp/rtmp/file/v4l2/x11grab/lavfi`。onvif/stream 登记的是 `Entry::Task`（它们复用的是管线*驱动*，不是*登记方式*），xiaomi 是 `Entry::Worker`，gb28181 没有常驻管线，`get_pipe` 对这几类都返回 `None`。原设计稿说「onvif 算 pipe-backed」，是错的。这份名单由后端通过 capabilities 接口下发，前端不另外写死。
- **保留手动开关**（已批准的决策 b）：设备配置是 auto-start 的唯一依据；overlay 是可视化 + 手动开关，两者走同一个幂等的 hub。
- **模型名校验**：先 trim、去重，最多 32 个名字，每个最多 128 字符；`sample_every_ms` 只能是 0 或 `1..=3_600_000`，`min_confidence` 在 `[0,1]`。新增/修改设备时不合法直接拒绝；库里已有的不合法配置在 reconcile 时按关闭处理并告警。名字未知时忽略并告警，只跑合法子集；全部未知、解析后为空时退回跑全部模型，兼容历史配置。

## 三、models.json 与环境变量

- `DETECT_MODELS_DIR`（默认 `third_party/detect-models`，相对于进程 cwd）下放 ONNX 权重和 `models.json`。权重不入库，示例见 `models.json.example`。
- `models.json` 是一个数组，每项：`name`（显示名，也是 API/配置里引用的 key）、`model_file`（相对清单目录，也可以写绝对路径）、`version`（**实际上必填**：不填时 usls 构建模型直接报 "No clear YOLO Version specified"）、`scale`（可选提示 n/s/m/l/x）、`conf`（默认 0.25，按类别设置）、`iou`（默认 0.45，会生效）、`input_size`（默认 640，**只是提示，没有生效**，usls 从模型文件读真实输入尺寸）、`class_names`（空=COCO-80）、`device`（默认 `"cpu"`）。
- 清单 JSON 写错时只告警，按空清单处理（和文件缺失一样）。
- `ORT_DYLIB_PATH=/path/libonnxruntime.so`（或把它的目录加进 `LD_LIBRARY_PATH`）：版本必须和 `ort =2.0.0-rc.10` 对应的 ORT 1.22.0 一致。缺了照样能编译、能启动，`GET /models` 也正常，直到 start 加载模型时才在运行期失败。
- 模型来源：usls 自家的导出最稳（`github.com/jamjamjon/assets` 的 `v8-n-det.onnx`、`v11-n-det.onnx`），Ultralytics 用 `yolo export format=onnx` 导出的也能用。
- 测试用：`DETECT_TEST_MODEL`、`DETECT_TEST_IMAGE`（`crates/nvr-detect/tests/live.rs`，`#[ignore]`）。

## 四、API / UI 契约

`/api/detect/*` 需要登录（Bearer 或 `?token=`），**不用** `{code,message,data}` 信封，所以前端用裸 `fetch`：
- `POST /{pipe}/start`，body `{models?: string[]}`（不传或空=全部）。返回纯文本：`started`（同时带响应头 `x-detection-tap-lease: <epoch>`）/ `already running`（不带 lease）。模型名不合法返回 400；`pipe not found` / `no video: …` / `no models configured …` / 模型加载失败返回 500，正文是错误文本。手动 start 会先取消该设备待执行的 auto-start 重试。
- `POST /{pipe}/stop`，body 可选 `{lease}`。带 lease 时是条件停止：只有 lease 仍是当前那一代 tap 才停，不会误停替换后的新 tap，也不会取消待执行的 auto-start。不带 lease 时无条件停止，同时取消待执行的 auto-start。返回 `stopped` / `not running`，lease 不是数字返回 400。
- `GET /{pipe}/latest` → `FrameResult {ts, frame_w, frame_h, models:[{name, infer_ms, detections:[{class_id,label,bbox:{x1,y1,x2,y2},confidence}], error}]}`；还没有结果时返回 404。
- `GET /models` → `string[]`；`GET /capabilities` → `{models, supported_input_types, max_sample_interval_ms, max_model_count, max_model_name_chars}`。清单缺失时返回成功，`models` 为空。
- 删除设备时调用 `hub.stop`；新增/修改设备走 `ensure_device_pipe` → `reconcile_detection`（先清掉旧 tap，因为它绑在已拆掉的旧 bus 上）。

前端（`DeviceListView` + `DetectionConfigFields` + `DetectionOverlay`）：
- 「检测」tab 只对 capabilities 返回的输入类型显示。capabilities 请求失败有单独的错误/重试状态（不能当成「未配置检测模型」）；能力没就绪时禁止保存已启用的检测。换成不支持检测的类型时，payload 的 `config` 整体置为 `undefined`。表单默认抽帧间隔是 1000ms，填 0 表示用服务端默认值（500）。
- overlay 每 1s 轮询一次 `latest`。设备已持久化启用检测（`persistentEnabled`）时，打开预览就自动显示，**只轮询、不调用 start/stop**。手动模式下只有 start 拿到 `started` + lease 才算「拥有」这个 tap，关闭或卸载时带 lease 停止；`already running` 只观察，不负责停止，所以关预览不会停掉配置驱动的 tap。
- 已知限制：持久化模式下如果 auto-start 30 次都失败，overlay 只能空轮询，没法手动补启。

## 五、测试与端到端验证

- 纯单测（不需要模型/ORT）：`cargo test -p nvr-detect`。nvr 侧需要加载器路径：`LD_LIBRARY_PATH="$PWD/ffmpeg/lib:$PWD/target/debug/deps" cargo test -p nvr detect::`。按子模块过滤要写 `detect::tap`、`detect::control`；写成 `detect::tap_test` 会匹配到 0 个用例。
- 真模型冒烟：`ORT_DYLIB_PATH=… DETECT_TEST_MODEL=third_party/detect-models/yolov8n.onnx DETECT_TEST_IMAGE=third_party/detect-models/bus.jpg cargo test -p nvr-detect --test live -- --ignored --nocapture`（bus.jpg 由 e2e 脚本下载；文件头注释里写的 `crates/nvr-detect/tests/bus.jpg` 并不存在）。
- 离线对比：`cargo run -p nvr-detect --example detect-compare -- --image <jpg> --models <dir>/models.json --models-dir <dir>`。
- 一键端到端：`bash scripts/detect_e2e.sh`。脚本下载 ORT 1.22.0、yolov8n/yolo11n、bus.jpg 到 `third_party/`（git-ignored，重跑时复用），用 bus.mp4 起 dummy RTSP（:9554），以 `ORT_DYLIB_PATH` + `DETECT_MODELS_DIR` 启动 nvr（:18080），添加 rtsp 设备，用两个模型 start，轮询并打印同一帧的对比，退出时自动清理。前提：已执行 `scripts/pre_install_deps.sh`，且 18080/9554 端口空闲；**不要 export `ZLM_DIR`**（会触发 rszlm-sys 重新构建，缺头文件就失败）。
- Phase 1 手工验证：设备带 `config.detect.enabled` 时，不手动 start 也会出结果；`min_confidence` 从 0 调到 0.9，框应明显变少；禁用、重新启用、删除设备，tap 应分别停止/重启/停止；gb28181/onvif/stream/xiaomi 设备不显示「检测」tab。
- 前端：`npm run type-check && npm run lint && npm run test`（Vitest：表单校验、不可变 payload、能力错误态、overlay 归属）；`npm run test:e2e`（Playwright 会自己在 :4173 起 dev server，覆盖 capabilities 失败后重试的流程）。
