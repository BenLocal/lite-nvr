# 媒体管线与录像踩坑

> media-pipe / ZLM sink、ffmpeg-bus 输出、nvr-recorder 重连，以及默认账号相关的非显然问题；设计见 `design-media-pipe.md`、`design-recorder.md`、`design-auth.md`。

- 2026-06-30：设备开了音频但源无音轨，ZLM 上流始终不上线 → `ZlmTrackCoordinator` 期望 2 条轨只注册了 1 条，`init_complete` 不触发 → 输出被拒时 Pipe 必须调 `sink.on_rejected()`，自定义协调型 sink 也要实现；尽量用 `zlm_outputs` 构造输出（证据：crates/media-pipe-core/src/pipe.rs:141-143、crates/media-pipe-zlm/src/lib.rs:52-59）
- 2026-07-15：stream-copy 只转封装，开段却报 "encoder not found for codec_id" → `AvOutput::add_stream` 用 `encoder::find(codec_id)` 建输出流，FFmpeg 构建无对应编码器就失败 → 冷门编码先确认 FFmpeg 构建注册了编码器，否则开录前当配置错误处理（证据：crates/ffmpeg-bus/src/output.rs:88-89）
- 2026-07-15：`nvr-recorder` 设了有限 `max_retries`，长跑后不再重连、退避停在最大值 → `attempt` 整个生命周期只增不减，成功录段不归零 → 视为「累计失败上限」；要按连续失败计数需改代码在会话成功后归零（证据：crates/nvr-recorder/src/recorder.rs:100,119）
- 2026-07-13：删掉 admin 后重启，又能用 admin/admin 登录 → `ensure_default_admin_user` 每次启动执行，admin 不存在就以密码 admin 重建 → 收紧默认口令应保留 admin 并改密码，不要删（证据：nvr-db/src/migrations.rs:43-59、nvr/src/main.rs:56）
- 2026-10-01：本机 QSV 能打开 h264 解码器，但首包报 "MFX session" 失败，降级软解后一帧都解不出 → 降级时丢掉了失败的那个包，而它往往是流里第一个关键帧 → 硬解运行期降级必须把失败包重放给软解（证据：crates/ffmpeg-bus/src/decoder.rs `Decoder::send_packet`）
- 2026-10-01：同一个文件源上的第二个 File 输出永远不写 moov → 第一个输出就启动了 input，短文件瞬间读完（< 4096 包缓冲），后加的输出订阅 input 时已错过 EOF，复用器一直等 → 全转码的 mux 只等编码器 EOF，不订阅 input；2026-10-02 起 Pipe 改用 `Bus::new_deferred` + `start()`，先注册完所有输出再开始读。`start()` 之后再加的输出 / 订阅（ASR、检测）仍是中途加入，文件源会丢开头（证据：crates/ffmpeg-bus/src/bus.rs `spawn_multi_stream_mux`、`Bus::new_deferred`）
- 2026-10-01：`scripts/test.mp4` 本身是 320x240 / 10fps / 50 帧，只有开头一个关键帧 → 要求 320x240 的 EncodeConfig 会被自适应判成 copy，不会真转码 → 写转码测试要选不同分辨率；测试产物统一写 `crates/ffmpeg-bus/.test_media/`（已 gitignore）
- 2026-10-01：所有视频转码输出时长塌缩（5s 源转出来 0.15s、约快 100 倍），只数帧数的测试一直发现不了 → 解码帧 / rawvideo 帧的 pts 用的是输入流时间基，编码器时间基是 1/1_000_000，送编码器前没换算 → `Encoder::send_frame` 先把 pts 从输入流时间基换算到编码器时间基；转码测试必须断言时长（证据：crates/ffmpeg-bus/src/encoder.rs `send_frame`）
- 2026-10-01：Net 输出（RTSP 推流）连一个没人监听的地址，`add_output` 照样返回 Ok，只在后台日志报错 → RTSP 等网络输出直到写 header 才连接，原来要等写第一个包时才写 header → File/Net mux 在 `add_output` 里就先写 header，连接错误直接返回给调用方（证据：crates/ffmpeg-bus/src/output.rs `AvOutput::write_header`）
- 2026-10-01：lavfi `testsrc` 出来的是 WRAPPED_AVFRAME，不是 RAWVIDEO，走的是解码路径；而且单独跑会报 "input format not found: lavfi" → 要先 `crate::init()` 注册 device（别的测试先注册了，才掩盖了这个顺序依赖）；真正的 RAWVIDEO 输入要用 `rawvideo` demuxer 打开裸 yuv（见 bus_test `write_raw_yuv`）
- 2026-10-01：RTSP 直播 e2e 不需要外部服务器 → 接收端 Bus 带 `rtsp_flags=listen` 打开 URL 充当 RTSP 服务端，另一个 Bus 用 Net rtsp 推流过去；用 lavfi `testsrc,realtime` 模拟实时摄像头（见 crates/ffmpeg-bus/src/bus_rtsp_test.rs）。后加入运行中流的有损输出收到的数据量随负载浮动，只断言「有数据且收到 EOF」
- 2026-10-01：一个解码器挂多个编码器，CPU / 内存带宽随编码器数线性涨 → 解码帧总有多份引用（每个订阅者一份、broadcast 环形缓冲一份），`RawVideoFrame::get_mut()`（`Arc::make_mut`）每次都会深拷贝整帧像素（ffmpeg-next 的 Video::clone 用的是 av_frame_copy）→ 只改 pts 等属性时用 `props_mut()`（av_frame_ref 浅拷贝），只读时用 `as_video()`（证据：crates/ffmpeg-bus/src/frame.rs）
- 2026-10-01：没人用的共享解码器 / 编码器只对 Bus 管理的任务自动停（`new_auto_stop`）；音频混音、合成器自己的 `DecoderTask::new()` 不开，因为它们的订阅者会来来去去，开了以后下一个订阅者会拿到已关闭的 receiver。解码 / 编码结束后再订阅会直接收到 Closed，不会一直挂着（证据：crates/ffmpeg-bus/src/decoder.rs `FrameSubscribers`）
- 2026-10-02：RTSP 推流设的 `rtsp_transport=tcp` 从来没生效（日志里有 `udp bind failed`）→ `output_rtsp_alloc_only` 只分配上下文、把 options 丢了，而 RTSP muxer 是在 `avformat_write_header` 时连接、读私有参数的 → `AvOutput` 记住 RTSP 参数，`write_header` 用 `write_header_with` 传入；新增的 RTSP 参数（如 `timeout`）也必须走这条路（证据：crates/ffmpeg-bus/src/output.rs `header_options`）
- 2026-10-02：打开输入、打开输出写 header、File/Net 写包都是阻塞调用，已经移到 `spawn_blocking` / 专用写线程；RTSP 输入和推流默认 10s 超时（`DEFAULT_RTSP_TIMEOUT_US`，listen 模式不加）。验证方法：在 `worker_threads = 1` 的 runtime 上，一个 Bus 连一个只接受连接、从不回应的 TCP 服务，另一个 Bus 必须照常跑完（见 bus_test `test_hanging_rtsp_*`）
- 2026-10-02：直播源被慢的 File/Net 转码拖住的问题，在几秒的测试里根本复现不出来 → 输入广播能存 4096 个包、编码器队列能存 128 帧，25fps 下要积压大约 2.7 分钟才会卡住输入读取 → 这类策略不要用短的端到端测试来证明，而是直接检查 Bus 内部状态里的策略接线（见 bus_test `file_transcode_loss_policy`）；改完要做变异检查，确认测试在旧行为下会失败
- 2026-10-02：H.265 摄像头和带 G.711 音频的摄像头接入 ZLM 后播放异常 → `media-pipe-zlm` 注册轨道和送帧时把编码写死成 H264 / AAC → 改为按 `av.parameters().id()` 映射；rszlm 的 `CodecId` 没有 derive 任何 trait，每次使用都要重新映射（证据：crates/media-pipe-zlm/src/lib.rs `zlm_codec_id`）
- 2026-10-02：直播有损模式下，音频转码在丢帧后会和视频对不上 → 音频重采样器只在第一帧取一次源时间，之后按采样数累加，丢帧不会留下时间空档 → 输入时间戳往后跳超过 50ms 就补静音（不超过 2s；更大的跳变只把时间轴往后挪）（证据：crates/ffmpeg-bus/src/encoder.rs `AudioResampler::bridge_gap`）
- 2026-10-02：音频 Mux 成 opus 直接报 `Invalid argument` → libopus 只接受 48k/24k/16k/12k/8k 采样率，编码器原来照搬输入的 44100 → `Encoder::new_audio` 按编码器支持的采样率列表选最接近的（`pick_sample_rate`），由重采样器负责转换（证据：crates/ffmpeg-bus/src/encoder.rs）
- 2026-10-02：内存 Mux 输出（`AvOutputStream`）的写回调在通道满时会丢掉已经封装好的字节，码流随之损坏 → 回调改用 `blocking_send`，写循环（`run_mux_stream_writer`）因此必须跑在阻塞线程上；在异步上下文里直接写 `AvOutputStream` 会 panic
- 2026-10-02：硬件编解码器运行期失败后会被记进进程级黑名单（`hw::mark_runtime_failure`），之后挑候选时跳过。副作用：同一个测试进程里，只有第一个解码测试会走 QSV 失败、降级的路径
- 2026-10-02：摄像头掉线后会话一直显示在运行，但没有数据，也不重连；`remove_input` / 进程退出都会卡住 → ffmpeg-next 的 `packets()` 迭代器遇到 EOF 以外的读错误，会在 `next()` 里无限重试；而且阻塞中的读不检查取消 → 改用 `AvInput::read`（`Packet::read`：EAGAIN 重试，其它错误按流结束处理），并通过 `AvInput::open` 给 FFmpeg 设置中断回调，取消输入或 `Bus::stop` 时能打断阻塞中的读和打开（证据：crates/ffmpeg-bus/src/input.rs）
- 2026-10-02：模拟「读到一半卡住」的测试一开始测错了地方：打开输入时 `avformat_find_stream_info` 会读满默认 5 秒的分析时长，只发一半数据的假服务器让「打开」先卡住了 → 测试输入要设很小的 `analyzeduration` / `probesize`，让打开先完成，后面的读才会卡住
