# 设备解码帧按需共享订阅 spec

> 每个设备的视频最多只解码一次，按订阅方的实际需求自动选择“不解码 / 只解关键帧 / 全量解码”，解码结果封装成一个流，供检测、截图、转码等多个业务同时订阅。

状态：第一阶段已实现（进程内订阅、检测、JPEG 截图、后台显示），已在 RK 主机 10.6.171.20 实测，2026-10-15。

## 1. 目标和已经明确的约定

用户确认的分级规则：

1. 设备已经在解码，新业务直接拿已解码的帧，不再开第二个解码器。
2. 设备还没有解码，并且当前只有“图片”类业务（截图、缩略图、按关键帧采样分析），只解码关键帧。
3. 有业务需要解码后重新编码、改变视频参数（分辨率、码率、编码格式等），必须全量解码。

由此推出：

- 业务只声明自己是哪一类（图片 / 转码），不设置“是否只解关键帧”。解码器是全量还是只解关键帧（`key_frames_only`），完全由当前订阅方自动推导：
  - 有任何转码订阅方 → 全量解码；
  - 否则有任何图片订阅方 → 只解关键帧；
  - 都没有 → 不解码。最后一个订阅方离开时停止解码，这与现有行为一致。
- 设备已在全量解码时，图片订阅方直接拿全部已解码帧（规则 1），不再只挑关键帧；它们自己按采样间隔取用。
- 视频直接转发（stream-copy）的输出不算解码需求。
- 订阅方之间互不阻塞：一个慢订阅方只会丢掉自己的帧，不能拖慢解码器或其他订阅方。文件、网络转码的无损订阅方沿用现有背压语义。

## 2. 当前实现与缺口

| 当前事实 | 代码证据 | 对新需求的影响 |
| --- | --- | --- |
| 每路输入流已经只有一个共享解码器，转码编码器、`Raw` 输出、`subscribe_video()` 都复用它 | `BusState::decoder_tasks`、`Bus::start_decoder_task()`、`Bus::subscribe_decoder()`（`crates/ffmpeg-bus/src/bus.rs`） | 规则 1 在进程内已基本满足，不需要重做 |
| 解码器在最后一个订阅方离开时自动停止 | `DecoderTask::new_auto_stop()`、`FrameSubscribers::publish()`（`crates/ffmpeg-bus/src/decoder.rs`） | 可沿用，作为“无订阅 = 不解码”的基础 |
| 每个订阅方有独立的 16 帧环形缓冲，分无损和有损两种 | `FRAME_CHAN_CAP`、`send_frame_backpressure()` | 图片订阅方一律按有损处理 |
| 解码帧以 `Arc` 共享，多个订阅方之间不拷贝像素 | `RawVideoFrame { frame: Arc<..> }`（`crates/ffmpeg-bus/src/frame.rs`） | 共享成本低；只有转成 RGB、JPEG 或跨进程传输时才拷贝 |
| RKMPP 解码结果被要求拷回 CPU 内存 | `rockchip_software_format()`（`decoder.rs`） | 订阅方拿到的是普通像素帧，不是 DRM/AFBC 句柄；零拷贝跨进程需要另行设计 |
| 解码器把收到的所有包都送去解码，没有“只解关键帧”的模式 | `DecoderTask::start()` 中转任务、`decoder_loop()` | **缺口**：规则 2 无法实现 |
| 订阅接口不带业务类别，`subscribe_video()` 固定为全量、有损 | `Bus::subscribe_video_internal()`、`Pipe::subscribe_video()`（`crates/media-pipe-core/src/pipe.rs`） | **缺口**：无法表达“只要图片”，解码器也无从推导模式 |
| 检测 tap 订阅全量解码，再按 `sample_interval_ms` 丢掉多余的帧 | `nvr/src/detect/tap.rs` 中的 `run()` | 采样间隔长时，大部分解码工作被直接丢掉 |
| 订阅只能在 NVR 进程内使用 | `RawFrameReceiver` 是 tokio broadcast | **缺口**：板上其他业务进程用不了，见第 6 节 |

## 3. 业务类别与解码模式的推导

```rust
// crates/ffmpeg-bus：订阅方只声明业务类别，不声明怎么解码
pub enum FrameDemand {
    /// 图片类：截图、缩略图、JPEG、目标检测等，不重新编码
    Image,
    /// 转码类：解码后重新编码、改变视频参数（分辨率、码率、编码格式等）
    Full,
}
```

每个视频流的解码器由当前订阅方自动推导模式，业务代码不能直接设置：

| 当前订阅方 | 解码器模式 | 送进解码器的包 | 图片订阅方收到 | 转码订阅方收到 |
| --- | --- | --- | --- | --- |
| 无 | 停止 | 无 | — | — |
| 只有 `Image` | `key_frames_only` | 只送 `packet.is_key()` 的包 | 每个关键帧一张图 | — |
| 有任意 `Full` | 全量 | 全部包 | 全部帧（自己采样） | 全部帧 |

谁算 `Full`：设置了 `encode` 的转码输出（`start_encoder_task()` 内部订阅），以及现有 `OutputDest::Raw` 原始帧输出（保持现有逐帧语义）。stream-copy 的 Mux、File、Net 输出不经过解码器，不计入任何需求。

谁算 `Image`：目标检测、截图、按需 JPEG 以及后续图片类业务。

### 模式切换

同一个解码器实例切换，不重开，以免 RKMPP 反复打开：

- **关键帧 → 全量**（出现第一个 `Full` 订阅方）：中转任务改为放行所有包，但从下一个关键帧开始。否则 P 帧会引用没解码过的参考帧，导致花屏或解码报错。新加入的转码订阅方最多等一个 GOP 才收到第一帧；图片订阅方此后自动收到全部帧。
- **全量 → 关键帧**（最后一个 `Full` 订阅方离开，仍有 `Image`）：中转任务立即只放行关键帧，已经在解码器里的帧照常输出。
- 从一开始就有 `Full` 订阅方、没有跳过过任何包时，所有包照常放行，和改动前完全一致（直播流中途加入、lavfi 等包上没有关键帧标记的源都不受影响）。
- 每帧都是完整画面的编码（`AV_CODEC_PROP_INTRA_ONLY`，以及 `WRAPPED_AVFRAME`、`RAWVIDEO`）不做过滤。
- 只剩关键帧时，“最后一个订阅方离开就停止解码”的检查也在中转任务里做，不再只靠解出新帧时触发，否则要等下一个关键帧才会停。
- 模式由订阅方的增减自动推导，记录在 `FrameSubscribers` 里，由解码中转任务读取（`DemandGate`）；每个订阅方只记自己的类别，`publish()` 不按类别过滤帧。

## 4. 对外接口

| 层 | 新增或修改 | 兼容性 |
| --- | --- | --- |
| `ffmpeg-bus` | `Bus::subscribe_video_with(FrameDemand) -> RawFrameReceiver`；`subscribe_video()` 和 `DecoderTask::subscribe(lossless)` 保留，等价于 `Full` | 现有调用方不改 |
| `media-pipe-core` | `Pipe::subscribe_video_with(FrameDemand)`，原样透传 | 不引入 ZLM 类型 |
| `nvr` | `nvr/src/frames.rs`：`subscribe(device_id, demand)` 通过 `manager::get_pipe()` 找到 Pipe 订阅；`snapshot_jpeg()` 取下一张已解码画面编码为 JPEG | 截图走这个入口；检测保留自己的启停与重连机制，只把订阅改为 `Image` |
| HTTP | `GET /api/device/snapshot/{id}` 返回 `image/jpeg`，最多等 15 秒（覆盖长 GOP） | 走现有会话鉴权，外部进程可用 `?token=` |
| 指标 | 解码器 `CodecMetrics` 增加 `key_frames_only`（视频解码器运行中且没有 `Full` 订阅方时为 true） | 后台拓扑图的解码节点显示“仅关键帧” |

“封装成一个 stream”：订阅返回 tokio broadcast 接收端，帧以 `Arc` 共享。第一阶段没有做“设备重连后自动换 Pipe 重新订阅”的包装：截图是一次性的，检测已有自己的重连机制；等有长期订阅的新业务时再加。

## 5. 业务接入

| 业务 | 类别 | 说明 |
| --- | --- | --- |
| 截图、缩略图、按需 JPEG | `Image` | 设备没有转码时按关键帧刷新，约 1 个 GOP 一张 |
| 目标检测 | `Image` | 不加用户开关。设备有转码时拿全部帧，按 `sample_every_ms` 采样，和现在一样；设备没有转码时只解关键帧，检测结果约 1 个 GOP 刷新一次，`sample_every_ms` 仍在其上限频 |
| 转码输出（改分辨率、码率、格式） | `Full` | 由 Bus 在 `start_encoder_task()` 内部订阅，业务不需要直接调用 |
| 直播、录像的 stream-copy | 不解码 | 不变 |

注意：检测改为 `Image` 后，没有转码输出的设备，检测刷新频率会从 `sample_every_ms` 变为约 1 个 GOP。这是规则 2 的直接结果，已按“长 GOP 可接受”确认；后台检测状态需要显示当前解码模式，免得被误认为检测变慢是故障。

## 6. 跨进程订阅

主机 10.6.171.20 上，厂商的 AI 服务（`gocr_arm_slim_socket`、`stu_behave_socket`、`stu_concentrate_socket`、`face_recog_socket`、`ai-face`、`image_service.py`）和 `rkllm-server` 都是独立进程。主机 1 天 22 小时运行后 load average 约 6（8 核）。如果这些业务也要订阅 NVR 的解码帧，就必须跨进程。可选方案：

| 方案 | 每帧成本 | 适用场景 |
| --- | --- | --- |
| A. HTTP 或 unix socket 按需返回 JPEG | 每张图编码一次 JPEG | 图片类业务、低频；实现最简单 |
| B. 共享内存环形缓冲 + unix socket 通知 | 每帧拷贝一次到共享内存 | 高频逐帧分析；需要定义格式（NV12/RGB）、读者计数和超时回收 |
| C. 让 RKMPP 输出 DRM_PRIME，通过 `SCM_RIGHTS` 传 dma-buf fd | 零拷贝 | 性能最好，但要改动第 2 节里“拷回 CPU 内存”的约定，影响所有现有订阅方 |

已确认：第一阶段只做进程内接口（第 3、4 节），跨进程用方案 A 覆盖图片类业务；按需返回 JPEG 的接口以 `Image` 订阅，取下一张已解码画面（没有转码时就是下一个关键帧，最多等 1 个 GOP），不常驻缓存最近一帧，免得没人用时也一直解码。等有逐帧跨进程的实际需求，再评估方案 B 或 C。

## 7. 修改边界

| 模块 | 拟修改内容 |
| --- | --- |
| `crates/ffmpeg-bus/src/decoder.rs` | `FrameSubscriber` 记录 `FrameDemand`；`FrameSubscribers` 根据订阅方推导模式；中转任务按模式过滤包，切到全量时从关键帧开始；指标增加送进解码器的包数 |
| `crates/ffmpeg-bus/src/bus.rs` | 新增 `FrameDemand`、`subscribe_video_with`；`subscribe_decoder()` 带上需求；转码编码器和 `Raw` 输出声明为 `Full` |
| `crates/media-pipe-core/src/pipe.rs` | 透传 `subscribe_video_with` |
| `nvr/src/frames.rs`、`nvr/src/handler/device.rs` | 按设备订阅、JPEG 截图及其路由；`nvr` 新增依赖 `image`（仅 jpeg 特性，树中已有） |
| `nvr/src/detect/control.rs` | `start_tap()` 以 `Image` 订阅 |
| dashboard | 设备拓扑图解码节点显示“仅关键帧”，不新增配置项 |
| 不新增 | `DetectConfig` 等数据库字段：`key_frames_only` 是运行时推导状态，不持久化 |
| 不修改 | 编码器、Mux 输出、ZLM sink、RKMPP 输出格式 |

所有新增任务都保留 cancel 和 JoinHandle，沿用 `stop_and_join` 的关停顺序（见 `docs/memory/media-and-recording.md` 2026-10-14 设备切换 busy 的记录）。

## 8. 风险与待验证

- **解码器延迟（已证实并修复）**：只喂关键帧时，带 B 帧重排或帧线程的解码器会扣住关键帧的画面，等下一个包（1 个 GOP 之后）才输出。本地 `test_image_keyframe_is_output_without_next_packet` 复现；RK 主机冷启动截图实测约 5 秒。修复：关键帧模式下每个关键帧送入后立即 `send_eof` 排空，再 `flush` 复位（`DecodeCmd::Packet { drain }`）；全量模式不受影响。修复后冷启动截图 1.9~2.0 秒，画面完整。
- **非 IDR 的 I 帧和 open-GOP**：H.264/H.265 中 `is_key()` 通常对应 IDR/IRAP。如果某些摄像头的关键帧标记不准确，单独喂这些包可能解出花屏，需要用真实设备覆盖。
- **长 GOP 设备**：GOP 为 4～10 秒的摄像头，图片类业务的刷新会很慢。已确认接受约 1 个 GOP 的刷新间隔，不做“临时升级全量解码”。

## 9. 实施步骤与验收

1. 在 `ffmpeg-bus` 中先写失败测试：用 lavfi testsrc 生成 GOP 为 25 的 H.264 文件或直播源。
2. 实现 `FrameDemand`、模式推导、包过滤和模式切换。
3. 透传到 `media-pipe-core`，`nvr` 增加 `frames` 模块，检测以 `Image` 迁移到该入口；增加按需返回 JPEG 的接口；后台显示解码模式。
4. RK 主机实测：RKMPP 只喂关键帧时的出图延迟、CPU 和 MPP 负载。

验收：

- 只有图片订阅方时，模式为 `key_frames_only`，送进解码器的包数等于关键帧数。
- 图片订阅方存在时加入转码输出：解码器实例数仍为 1，模式自动变为全量；转码输出从关键帧开始收到连续帧，没有解码报错；图片订阅方之后收到全部帧。
- 移除转码输出后自动回到 `key_frames_only`，送进解码器的包速率下降到关键帧速率。
- 设备已有转码输出时再加图片订阅方：不新建解码器，图片订阅方立即拿到全部已解码帧。
- 只有 stream-copy 输出时，不启动解码器。
- 慢的图片订阅方只丢自己的帧，不影响转码输出。
- 设备断流重连后，`nvr` 层的 stream 自动恢复，业务侧不需要重新订阅。
- 检测：无转码设备上检测结果按关键帧刷新；有转码设备上按 `sample_every_ms` 采样，行为与现在一致；后台显示的解码模式与指标一致。
- 在 RK 主机上，1/2/4 路 1080p 设备分别测“全量解码”和“只解关键帧”，记录 CPU、RSS、MPP 负载和出图延迟。具体降低多少以实测为准，不提前给数字。

### RK 主机实测（2026-10-15，10.6.171.20，RTMP H.264 1080p25、GOP 2 秒，h264_rkmpp 硬解）

检测（yolov8n、CPU 推理）抽帧间隔设为 2000ms，与 GOP 相同，使新旧两版每秒推理次数一致，只比较解码差异。各采样 60 秒；nvr 进程 CPU 按单核百分比计，两路 1080p60 HDMI 采集编码一直在跑。

| 场景 | 解码帧率 | nvr CPU | 整机忙 | rkvdec 每核负载 |
| --- | --- | --- | --- | --- |
| 旧版，未开检测 | 不解码 | 439.7% | 63.6% | 0% |
| 新版，未开检测 | 不解码 | 443.9% | 64.2% | 0% |
| 旧版，开检测（全量解码） | 约 26 帧/秒 | 571.1% | 78.9% | 1.9% |
| 新版，开检测（只解关键帧） | 0.5 帧/秒 | 518.4% / 512.0%（修复 drain 前 / 后） | 72.3% / 71.8% | 0.06% |

检测带来的额外 CPU 从约 131% 降到约 70%，rkvdec 负载从约 1.9% 降到约 0.06%；检测结果数量一致。单次 60 秒采样，误差未评估。

截图：RTMP 设备热状态（检测在跑）1.4~2.0 秒、冷启动（无解码）修复后 1.9~2.0 秒；截图结束后解码器自动停止。没有订阅方时不启动解码器。

未在主机上验证：同一管线运行中从关键帧升级为全量（板上没有可在不重建管线的情况下加转码输出的入口，由本地测试覆盖）；1/2/4 路扩展（主机只有 1 路 H.264 输入）。

**HDMI（V4L2 rawvideo）截图**：rawvideo 输入不经过解码器，原先 `/api/device/snapshot/{id}` 和 `subscribe_video()` 都返回 `decoder task not found`。现在 `Bus::subscribe_raw_video()` 直接把输入包转成帧：`Image` 订阅方只在取走上一帧后才转换下一帧，60 FPS 源不会为截图逐帧拷贝；订阅方离开或输入结束时转换任务退出。板上两路 1080p60 BGR24 HDMI 截图 0.12~0.19 秒返回，无解码器；与编码后直播流同位置像素一致（无红蓝颠倒）。

## 10. 已确认的业务选择

1. 第一阶段只做进程内订阅接口，外加按需返回 JPEG 的接口供跨进程图片业务使用；共享内存与 dma-buf 方案暂不做。
2. 不加用户开关。`full` 还是 `key_frames_only` 由第 1 节三条规则自动决定：已解码就直接拿，只有图片业务只解关键帧，需要重新编码改变视频参数才全量解码。检测属于图片业务。
3. 长 GOP 设备的图片业务接受约 1 个 GOP 的刷新间隔。
