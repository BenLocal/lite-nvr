# 设备协议踩坑（GB28181 / ONVIF）

> GB28181 拉流、注册、PTZ 与 ONVIF 鉴权、profile 相关的非显然问题；设计见 `design-gb28181.md`、`design-onvif.md`。

## GB28181
- 2026-07-02：真机注册成功、INVITE 回 200，但一直收不到 RTP → `NVR_GB_MEDIA_IP` 默认 127.0.0.1，会写进 SDP `c=`，设备把流推回它自己的回环 → 对接真机必须设成设备可达的本机 IP（证据：nvr/src/gb/config.rs:34-36、crates/gb28181/src/sdp.rs:30-31）
- 2026-07-02：同一流的 `on_media_not_found` 并发触发两次后，流标为「拉流中」却无数据 → `RtpServer` 表以 stream_id 为 key，第二次 `OpenRtp` 顶掉第一个，落败 `ActiveSession` drop 时的 `CloseRtp(stream_id)` 又关掉胜出方；bridge 里「会自愈」的注释已不成立 → 修法：在 `active` 锁内预占 slot（`Slot::Pulling`），或让 CloseRtp 带端口/代号（证据：nvr/src/zlm/cmd.rs:146-160、nvr/src/gb/receiver.rs:63-67、nvr/src/gb/bridge.rs:121-131）
- 2026-07-02：NVR 重启后 dummy-camera / `GbClient` 注册不回来 → registrar 只在内存；平台对未注册设备的 Keepalive 回 404，`GbClient` 只打 warn，不重新 REGISTER、也不到期续约 → 重启 NVR 后一并重启 dummy，或给 client 加「404 / 续约时重新 REGISTER」（证据：crates/gb28181/src/client.rs:151-180、crates/gb28181/src/server.rs:553-569）
- 2026-07-02：`POST /api/gb/play` 设的 TCP transport 变回 UDP → transport 只存内存映射；重启或设备 update 触发 `ensure_device_pipe` 以 `Transport::Udp` 重新登记 → 打开流前重新调 `/gb/play`，且要等上一路拉流 no_reader 拆掉后才生效（证据：nvr/src/init/device.rs:96-104、nvr/src/gb/api.rs:165-180）
- 2026-07-02：PTZ 单测全过，真机方向反了或不动 → 8 字节 PTZCmd 位布局照标准写，测试向量只保证校验和不漂移 → 只能真机逐方向验证；修只改 `encode_ptz_cmd` 一处并同步测试向量（证据：crates/gb28181/src/ptz.rs:1-20）

## ONVIF
- 2026-07-20：真机一直 "authentication rejected"，账号密码都对 → `nvr-onvif` 未设 `ClientBuilder::fix_time_gap`，时钟偏差大时 UsernameToken 的 `Created` 被拒；dummy 不校验 `Created` 测不出 → 先对时，或实现 GetSystemDateAndTime 后设 time gap（证据：crates/nvr-onvif/src/camera.rs client_at）
- 2026-07-20：设备日志里每个操作都先一条 rejected 再成功 → `AuthType::Any` 先试无鉴权 Digest，失败再用 WS-Security UsernameToken → 正常现象，不用排查（证据：onvif-rs soap/client.rs:172-186）
- 2026-07-20：自制 ONVIF 服务端鉴权失败回 HTTP 401，onvif-rs 却报不出 `OnvifError::Auth` → onvif-rs 收到 401 会进 Digest 握手；只有非 2xx 且 SOAP Fault Subcode 含 `NotAuthorized` 才判 Authorization → 模拟器/代理鉴权失败返回 HTTP 400 + `ter:NotAuthorized` fault（证据：onvif-rs 8f1490e soap/client.rs:307-311、examples/dummy-onvif-camera/src/soap.rs）
- 2026-07-20：dashboard 选了某个 profile，PTZ 却作用在另一个上 → `ptz_move`/`stop`/`presets`/`goto_preset` 固定用 `default_profile`（第一个），`profile_token` 只影响 `stream_uri` → 多 profile 摄像头 PTZ 异常先查这里（证据：crates/nvr-onvif/src/camera.rs ptz_move）
