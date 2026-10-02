# 媒体管线拆分设计要点

> `media-pipe-core` / `media-pipe-zlm` 为什么这样拆、边界在哪、调用方必须遵守的约束；改管线输出或接新媒体服务器前先看本文。

原设计稿：2026-06-30「split `nvr/src/media` into reusable crates」（已落地，原 `nvr/src/media/` 已删除）。依赖方向硬规则见 `architecture.md`，这里只记取舍与不变量。

## 一、目标与范围

- 目标：让其他项目（如 media-srv / aibox-nvr）能复用媒体管线而**不被迫链接 ZLMediaKit**。因此 `media-pipe-core` 的硬约束是**零 `rszlm` 依赖**（`crates/media-pipe-core/Cargo.toml` 只依赖 `ffmpeg-bus` 与通用异步库）。
- `media-pipe-core`：配置类型（`PipeConfig` / `OutputDest` / `EncodeConfig` …）、`Pipe`、`RawSinkSource`、到 `ffmpeg-bus` 的转换、`DemuxedSink` trait。
- `media-pipe-zlm`：`ZlmSink`（实现 `DemuxedSink`）、`ZlmTrackCoordinator`、rszlm `Track`/`Frame` 转发，以及便捷构造 `zlm_outputs` / `zlm_video_dest`。
- 与原稿不同的现状：原稿计划让 `media-pipe-zlm` 挂在 `nvr` 的 `zlm` feature 后面；**现在 `nvr` 无条件依赖 `media-pipe-zlm` 和 `rszlm`，没有 `zlm` feature**。可选性只体现在 crate 层面（别的项目可以只用 core）。

## 二、关键决策

- **用 trait 边界替代 `OutputDest::Zlm(Arc<Media>)`**：core 只认 `OutputDest::Demuxed { sink: Arc<dyn DemuxedSink> }`，映射到 `ffmpeg-bus` 的 `Demuxed`（每个 item 是一个解复用后的原始包）。ZLM 只是其中一种 sink，接别的媒体服务器时新写一个 sink crate，不改 core。
- **`Arc<dyn …>` 而不是 `Box`**：为了保持 `OutputConfig: Clone`。代价是 clone 出来的配置**共享同一个 sink 实例**（以及它背后的 coordinator）。
- **coordinator 从 Pipe 挪到接线层**：`ZlmTrackCoordinator` 负责把同一个 `Media` 的视频轨和音频轨攒齐后再调 `init_complete()`。它是 ZLM 特有的概念，所以由 `nvr` 通过 `media-pipe-zlm` 构建，并在同一 `Media` 的各个 `ZlmSink` 之间共享，core 的 `Pipe` 对此完全不知情。
- **sink 自己 spawn 转发任务**：`DemuxedSink::start(av, stream)` 返回 `JoinHandle`，由 Pipe 收集。rszlm 的 `Track` 不是 `Send`，必须在同步代码块里注册完并 drop 掉，不能跨 `.await`。

## 三、调用方必须遵守的不变量

- **coordinator 的 `expected` 必须等于实际挂到同一 `Media` 上的轨道数**。少注册会导致 `init_complete()` 永远不触发，转发任务一直卡在 `wait_complete` 上，流始终不上线。优先使用 `zlm_outputs(media, include_audio)` / `zlm_video_dest(media)`，不要手工拼。
- **输出被拒绝时必须回调 `on_rejected()`**（原稿没有，后来补上）：例如请求了音频而输入没有音轨时，Pipe 的 `add_output` 会失败，此时调用 `sink.on_rejected()`，`ZlmSink` 再执行 `expect_one_less()`，让剩下的视频轨能够完成初始化。自己实现的协调型 sink 也必须处理这个回调。
- **coordinator 只能用一次**：`completed` 置位后不会复位。每个新的 `Media` / 每次重建管线都要新建 coordinator 和输出，不要复用旧的 `OutputConfig`。
- **Pipe 的启动顺序**：Pipe 用 `Bus::new_deferred` 创建 bus，先 `add_output` 注册全部输出、启动各自的转发任务，最后才调用 `bus.start()` 开始读输入。这样文件这类读得很快的源也不会跑在后加的输出前面。之后通过 `subscribe_audio` / `subscribe_video` 加入的消费方（ASR、检测）属于中途加入：直播源从当前位置开始，文件源会错过开头。
- **直播源与非直播源的丢帧策略**：Pipe 调用的是 `add_input`，直播与否按输入类型推断：`File` 不是直播，`Network` / `Device` 是直播；需要显式指定时用 `Bus::add_input_with_live`。非直播源上的 File/Net 输出全程无损，输入按最慢的输出的速度读取。直播源从不为 File/Net 输出放慢：慢的输出自己丢包，断档后一直丢到下一个关键帧再继续写，不会把坏的片段写进录像。
- **Pipe 的结束语义**：只要有任意一个 sink / raw 转发任务结束（输入 EOF、读错误、sink 消失），`Pipe::start` 就会认为整个会话已失效，随即拆掉 bus 并返回，以便上层 supervisor 感知到流断开后重启（例如重新解析过期的直播地址）。只有 `Network` 输出（完全在 bus 内部处理）的 Pipe 只在被 cancel 时才返回。

## 四、测试分布

- `pipe_test.rs` 放在 `media-pipe-core`，基于 `RawSinkSource`，不依赖 ZLM，可以脱离 ZLM 单独跑。
