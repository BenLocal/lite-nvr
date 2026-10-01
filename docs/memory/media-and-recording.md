# 媒体管线与录像踩坑

> media-pipe / ZLM sink、ffmpeg-bus 输出、nvr-recorder 重连，以及默认账号相关的非显然问题；设计见 `design-media-pipe.md`、`design-recorder.md`、`design-auth.md`。

- 2026-06-30：设备开了音频但源无音轨，ZLM 上流始终不上线 → `ZlmTrackCoordinator` 期望 2 条轨只注册了 1 条，`init_complete` 不触发 → 输出被拒时 Pipe 必须调 `sink.on_rejected()`，自定义协调型 sink 也要实现；尽量用 `zlm_outputs` 构造输出（证据：crates/media-pipe-core/src/pipe.rs:141-143、crates/media-pipe-zlm/src/lib.rs:52-59）
- 2026-07-15：stream-copy 只转封装，开段却报 "encoder not found for codec_id" → `AvOutput::add_stream` 用 `encoder::find(codec_id)` 建输出流，FFmpeg 构建无对应编码器就失败 → 冷门编码先确认 FFmpeg 构建注册了编码器，否则开录前当配置错误处理（证据：crates/ffmpeg-bus/src/output.rs:88-89）
- 2026-07-15：`nvr-recorder` 设了有限 `max_retries`，长跑后不再重连、退避停在最大值 → `attempt` 整个生命周期只增不减，成功录段不归零 → 视为「累计失败上限」；要按连续失败计数需改代码在会话成功后归零（证据：crates/nvr-recorder/src/recorder.rs:100,119）
- 2026-07-13：删掉 admin 后重启，又能用 admin/admin 登录 → `ensure_default_admin_user` 每次启动执行，admin 不存在就以密码 admin 重建 → 收紧默认口令应保留 admin 并改密码，不要删（证据：nvr-db/src/migrations.rs:43-59、nvr/src/main.rs:56）
