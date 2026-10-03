# 系统架构

> 依赖方向硬规则、启动与关停顺序的约束、关键设计决策；动结构、加模块、接外部依赖前先看本文。

代码结构、符号与调用链用 CodeGraph（`codegraph_explore`）直接查，本文只记代码里读不出来的规则与取舍。

## 一、依赖方向（硬规则）

- `nvr` → 能力 crate（`crates/nvr-*`、`gb28181`、`xiaomi`、`nvr-onvif`）/ 管线 crate（`media-pipe-*`）→ `ffmpeg-bus`，单向；下层不得依赖 `nvr`。
- `media-pipe-core` 与 ZLM 无关，ZLM 相关只放 `media-pipe-zlm`；`gb28181` 只做信令，不碰媒体；`nvr-db` 不依赖媒体 crate。
- 新能力优先拆成独立 crate（可脱离 ZLM 单测），在 `nvr/src/<subsystem>/` 写胶水与 `api.rs`。

## 二、启动与关停顺序的约束

- **DetectHub 必须在设备管线初始化前就绪**：否则设备配置里的检测自动启动会与 ZLM 就绪赛跑而被跳过。
- **关停先停生产者再 exit**：`std::process::exit` 会立即跑 ZLM/ffmpeg 的 C 静态析构，媒体线程仍在写 ZLM 就会 use-after-free 段错误。新增往 ZLM 写数据的子系统必须加进 `main.rs` 的 teardown 链（生产者先于 `manager`），带超时兜底。
- 关停不清持久化配置，下次启动全部恢复。

## 三、关键设计决策

- **一个 registry 统一管理设备源**：ffmpeg Pipe、网络 Pipe supervisor（net/rtsp/rtmp 断流后重建媒体会话）、原生线程（小米，绕开 ffmpeg）、异步 supervisor（直播平台每次重连重新解析拉流地址）共用 `manager` 的同一个 map，增删改与状态查询统一处理；替换同 id 源前必须先 `stop` 再 `join`，确保旧 ZLM `Media` 已释放。同 id 的替换和删除通过异步操作门串行化；关停等待操作结束并禁止后续插入。普通网络设备从 2 秒退避重试到最多 60 秒，连续运行 30 秒后重置退避；删除打断当前会话或重试等待。
- **检测 auto-start 要等 bus 就绪**：`Pipe::start` 是 spawn 出去的，RTSP 源要等 demuxer 读到流头后才能订阅；auto-start 有限时重试，过期重试会被取消。
- **ONNX 推理放独立大栈线程**：ONNX Runtime 构建 session / 推理递归很深，会爆 tokio `spawn_blocking` 默认约 2 MiB 栈。
- **检测模型清单缺失不阻塞启动**：`DETECT_MODELS_DIR/models.json` 不存在时正常启动，检测接口报未配置。
- 各子系统的范围、取舍与协议约束见 `design-*.md`（media-pipe、gb28181、onvif、detect、recorder、auth）。
