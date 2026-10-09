# 媒体管线与录像踩坑

> media-pipe / ZLM sink、ffmpeg-bus 输出、nvr-recorder 重连，以及默认账号相关的非显然问题；设计见 `design-media-pipe.md`、`design-recorder.md`、`design-auth.md`。

- 2026-10-03：同设备并发更新遗留源任务 → 等待旧 Entry join 时释放 map 锁，另一更新可先插入再被覆盖，丢弃 JoinHandle 不会停止任务 → 用按 id 的异步操作门串行化替换与删除，关停等待操作完成并阻止新插入；同步注册表锁不跨 await（回归：`manager_test.rs`、`lifecycle_test.rs`）。
- 2026-10-03：同名新增覆盖原设备、无效协议更新返回错误但已写库 → 名称生成固定 id，add 直接 upsert，协议解析发生在保存之后 → add 在设备操作门内拒绝已有 id，保存前校验协议形状，应用失败回滚；离线摄像头仍允许保存（回归：`handler/device_test.rs`）。
- 2026-10-03：录像保留遗漏截止日当天已过期分片 → RFC3339 的 T 分隔符与 SQLite datetime 的空格不能直接按字符串比较 → 使用 julianday 归一化两侧；文件删除失败保留行、跳过释放空间计数，手动删除与自动清理共用逻辑（回归：`record_segment_test.rs`、`cleanup_test.rs`、`handler/playback_test.rs`）。
- 2026-10-03：普通网络设备 EOF 后不重连，首次开流失败还可能一直等待取消 → 普通设备缺 supervisor，Pipe 没有接受输出时仍进入等待 → net/rtsp/rtmp 使用可取消且可 join 的 supervisor，每次重建 ZLM 会话；Pipe 打开失败或所有输出被拒绝时清理并返回。原始 `/api/pipe` 的单次 Pipe 行为保留（回归：`manager_test.rs` 本地 TCP/ZLM 断流与首次打开失败）。

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
- 2026-10-02：网络推流（RTMP/FLV over TCP 等）在对端停止读取后，写线程会永远阻塞，`Bus::stop` 也打断不了 → `ffmpeg-next` 的 `output_as_with` 调用 `avio_open2` 时没传中断回调，也没有超时 → 网络输出改用 `AvOutput::open_network`（在 `avio_open2` 之前设好中断回调，由 `Bus::stop` 触发），非 RTSP 网络输出默认加 `rw_timeout`（10s），连接类错误直接结束这路输出。文件输出故意不加中断：停止时如果正在写 mp4 结尾，被打断会产生坏文件（证据：crates/ffmpeg-bus/src/output.rs、bus.rs `open_mux`）
- 2026-10-02：推流断开后为什么不直接拆掉 Pipe → 同一个 Pipe 里常常还挂着本地 ZLM 输出，而 `manager::add_pipe` 添加的 pipe 结束后不会自动重启，拆掉会话会连本地播放一起停掉 → 改为在 Bus 内原地重连网络输出（`run_mux_writer` / `reconnect_mux`），被中断（`Bus::stop`）或上游已经结束时不再重连
- 2026-10-02：测试集偶发失败（大约 40 次出现 2 次），最后抓到的是 `test_mux_audio_from_encoder_opus`，时长只有 2s → #4 的「非直播源全程无损」只覆盖了 File/Net，Mux / Encoded / Demuxed 输出在文件源上仍然有损，机器一忙就丢帧 → 这几类输出也改为跟随源的直播属性；Raw 输出保持有损。教训：多次运行验证时要保存每次的完整日志，不能只 grep 统计行，否则偶发失败抓不到是哪个测试（回归测试见 bus_test `test_encoded_output_lossless_on_file_source`）
- 2026-10-03：源中途切换分辨率或音频参数（摄像头切换码流配置、直播平台切换清晰度）后，转码输出从此一帧都不出 → 编码器的缩放器和音频重采样器都只按第一帧创建，参数一变，ffmpeg-next 的 scaler 返回 `InputChanged`，swr 也拒绝每一帧 → 参数变化时重建缩放器；音频先把旧 swr 里缓存的样本冲出来再重建，FIFO 和时间戳保留（证据：crates/ffmpeg-bus/src/encoder.rs `scaler_input`、`AudioResampler::reconfigure`）
- 2026-10-03：用 ffmpeg-next 写测试时，音频帧要用 `av_samples_set_silence` 清零，不能用 `data_mut(i).fill(0)` → `Audio::new` 不初始化样本内存，而 `data_mut(i)` 用 `linesize[i]` 作为切片长度，FFmpeg 对音频帧只设置 `linesize[0]`，所以 planar 音频的第 2 个及以后的声道根本没被清零。残留的随机 float（NaN、非规格化数）会让 AAC 编码器时而报 EINVAL、时而算得极慢像卡死、时而正常（见 encoder_test `fill_silence`）
- 2026-10-03：MPEG-TS 音频转码误补静音、1s 输入的输出 PTS 跨度变成 5.568s → 解码音频 PTS 沿用输入流时间基（TS 为 1/90000），不能直接与采样数相减 → 重采样前先换算成 1/input_rate，声道或采样格式变化但采样率不变时保留 expected_in_pts（证据：encoder_test `test_audio_stream_time_base_is_converted_to_samples`、`test_audio_format_change_preserves_gap`）。
- 2026-10-03：补静音后真实音频帧丢失、send_frame 报 EAGAIN → 缺口一次生成多个编码帧，却等全部发送后才排出编码包 → 每送一个音频帧就排出并暂存编码包，正常 receive 按序取回；EOF 冲刷也走同一路径（证据：encoder_test `test_audio_gap_encodes_every_silence_and_source_frame`）。
- 2026-10-03：无损输入恰好填满 4096 包队列时仍收到 Lagged(1)，最后只剩 4095 包 → EOF 也占队列槽位，原先只有 Data 等空位 → Data 和 EOF 统一走背压发送（证据：input_test `test_lossless_eof_does_not_evict_last_full_queue`）。
- 2026-10-03：浅拷贝帧属性后通过 get_mut 修改像素会影响原帧 → 外层 Arc 独占不代表 FFmpeg AVBuffer 独占 → 不公开返回共享缓冲的可变 Video，只公开 set_pts；get_mut 额外检查 av_frame_is_writable，共享时深拷贝（证据：frame_test `test_pixel_write_after_property_copy_keeps_original_unchanged`；替代此前公开 props_mut 的做法）。
- 2026-10-03：混音总线用 8kHz 源作 template 时，三个正常混音帧却生成 14 个 AAC 包 → DynamicMixerTask 输出 PTS 始终按混音采样率（48kHz）计数，源流时间基只适用于源帧 → MixBus 创建编码器时必须提供 1/48000 时间基，不得直接传原始 template；测试同时覆盖 8kHz、44.1kHz 和 90kHz 源时钟（证据：nvr-audio-mixer `bus_test::test_mixed_audio_timing_is_independent_of_template`，修复后均为三个帧加一个 priming 包）。

- 2026-10-09：cross GNU 构建报 `libavutil/avutil.h: No such file or directory`，库实际已下载 → 本工程含工作区外的 git 依赖，cross 0.2.5 将源码挂到宿主机原路径而非 `/project` → 打包脚本显式将 `target/cross-deps`、`third_party` 挂到固定容器路径，避免 FFmpeg / ZLM / sherpa 路径随挂载模式改变；修复后 `ffmpeg-sys-next` 已编译通过（证据：`scripts/package-linux.sh` 的 `CROSS_CONTAINER_OPTS`）。
- 2026-10-09：设置 `FFMPEG_URL` 在新环境安装 RK FFmpeg 后，ZLM 报平台不支持 → 原脚本把平台写成 `custom`，丢掉了操作系统和架构 → 自定义 FFmpeg URL 仍保留 `detect_platform` 的结果，ZLM 按真实平台下载；已用 Linux arm64 的隔离模拟验证两个下载 URL（证据：`scripts/pre_install_deps.sh`）。

- 2026-10-09：同一 Cargo target 目录从通用 FFmpeg 切换到 RK SDK 时仍链接旧库、旧绑定掩盖 RK NV15/NV20 不兼容 → ffmpeg-sys-next 8.1.0 未跟踪 FFMPEG_DIR 变化 → 本地补丁跟踪该变量；ffmpeg-next 补丁按 SDK 头文件启用 NV15/NV20RK 双向映射，保留通用 NV20 别名（证据：`patches/ffmpeg-rockchip-8.1.0.patch`）。
- 2026-10-09：RK3588 Docker 中 H.264 可编码但 HEVC 报 `Failed to init MPP context: -1` → Docker 默认 maskedPaths 含 `/sys/firmware`，MPP 读不到 `/proc/device-tree` 的真实目标 → 只读挂载设备树并用 `--security-opt systempaths=unconfined` 解除系统路径屏蔽后，HEVC 命令和 Rust 硬编码测试均通过；此参数会解除其他默认路径屏蔽，部署权限见 `docs/rockchip.md`。

- 2026-10-10：Cargo 无法按 feature 条件选择 crates.io patch → Rockchip 构建入口通过 `scripts/with-rockchip-patch.sh` 临时传入路径覆盖，自动准备 `.cache/rockchip-rust/` 并恢复 Cargo.lock；普通构建使用原始 crates.io 依赖，无需 vendor。
