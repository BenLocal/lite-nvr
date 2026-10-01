# ONVIF 设计要点

> NVR 只做 ONVIF 客户端：负责发现摄像头、解析 RTSP URI、下发 PTZ，媒体照旧走 RTSP→ZLM 管道；本地测试用 dummy-onvif-camera 模拟 ONVIF 服务。

## 范围

- 已实现：WS-Discovery 局域网发现（结果只用来预填添加表单），probe（凭据校验 + 列出 profile），按 profile 解析 RTSP URI 后拉流入 ZLM `device/<id>`，PTZ（ContinuousMove / Stop / GetPresets / GotoPreset）。
- 有意不做（v1）：事件订阅（PullPoint / motion）、Imaging、Media2、Profile G 机内录像、音频回传；每个设备只用一个 profile；不做 ONVIF 服务端；发现结果不自动建设备；不支持 TLS。
- `onvif` 设备不支持目标检测：它是 supervisor Task，不是 `Entry::Pipe`，没有可订阅的管道（`DETECT_SUPPORTED_INPUT_TYPES` 里没有它）。

## 关键决策与理由

- **底层库选 `lumeohq/onvif-rs`（git 依赖，Cargo.lock 锁在 `8f1490e`）**。它是最完整的原生 Rust ONVIF 客户端，类型从官方 WSDL/XSD 生成，纯 Rust，不会和 ffmpeg 的 `links=` 冲突。代价有两个：没有 crates.io 版本；生成的 schema 编译慢，所以把它隔离在叶子 crate 里。要同时依赖 `onvif` 和 `schema` 两个包。升级 commit 时 API 会漂移，只调内部调用，`nvr-onvif` 的公开签名保持不变。
- **叶子 crate 叫 `nvr-onvif`，不叫 `onvif`**：一个包不能和它自己的依赖同名。拆分方式照搬 `crates/gb28181` + `nvr/src/gb`：crate 里不出现 nvr/ffmpeg/zlm 类型；`nvr/src/onvif` 负责 REST、registry 和 ingest。
- **ONVIF 只是"解析 + 控制"层，不新增媒体路径**。ingest 和 yt-dlp 的 `stream` 类型同构，复用同一个 `livestream::run_session`（把 URI 包成 `ResolvedStream`，从而沿用 rtsp-over-tcp 的 demux 策略）。禁止另写 RTSP 拉流器，也不要改 `ffmpeg-bus`。
- **设备里存的是 ONVIF 连接配置，不是 RTSP URL**：`input_value` = `OnvifConfig` JSON（host/port/username/password/profile_token）。supervisor 每次（重）连都重新 `connect → GetStreamUri`，退避 2s→60s，单次会话持续 ≥30s 算健康，退避重置为 2s。这样摄像头换 IP、改配置或重启后，下次重连会自己恢复。解析失败只退避重试，不拆除设备。
- **凭据处理**：`GetStreamUri` 返回的 URI 不带凭据，但多数摄像头的 RTSP 也要鉴权，所以用 `inject_credentials` 把同一组用户名和密码（经过百分号编码）注入 URI。URI 已带 userinfo、不是 rtsp scheme 或用户名为空时，保持原样。密码明文存在 DB 的 `input_value` 里，设备列表 API 原样返回；dashboard 只在表格显示层做了脱敏（显示 `host:port (profile)`）。
- **registry**：内存里的 `device_id → OnvifConfig`，设备新增或启动恢复时登记。PTZ 和 presets 从这里读配置，不查 DB。设备被删除、或 `input_type` 从 onvif 改成别的类型时，必须同时 `onvif::remove`。
- **PTZ 动词契约与 `/api/gb/ptz` 一致**：`up/down/left/right/zoom_in/zoom_out/stop/preset_call`，`speed` 取 0..=255（默认 128），映射为速度 `speed/255` 并钳到 -1..1。和 GB28181 的区别：ONVIF 预置位是字符串 token，所以 `preset_call` 必须带 `preset_token`（从 `GET /api/onvif/presets/{id}` 获取），否则按非法动词报错。前端用按住移动、松开发 `stop` 的交互；移动命令丢了，松开时的 stop 也能兜住。
- **每次 PTZ / presets 请求都会重新 `OnvifCamera::connect`**（GetCapabilities + GetProfiles），不缓存客户端。实现简单，代价是每次 PTZ 多两次 SOAP 往返。
- **PTZ 总是用摄像头的第一个 profile**（`default_profile`），不用设备配置里选定的 `profile_token`；这个 token 只影响 `stream_uri`。
- REST 只用 GET/POST，统一挂在 `/api/onvif` 下，复用 `/api` 的 session 鉴权。`probe` 临时直连摄像头，不走 registry，这样表单保存前就能校验凭据。`discover` 超时被钳到 500–10000ms，默认 3000ms。

## 协议 / 互通要点

- **鉴权方式 `AuthType::Any`**：onvif-rs 先发 HTTP Digest 请求（第一次没有 digest 状态，相当于不带鉴权），收到 Authorization 错误后改用 WS-Security UsernameToken（PasswordDigest = `Base64(SHA1(nonce ++ created ++ password))`）重试。所以每个操作在设备端都会先看到一次被拒，这是正常现象。
- **onvif-rs 判定 `Authorization`（映射为 `OnvifError::Auth`，显示为 "authentication rejected"）的条件**：HTTP 401 会走 digest 握手；或者非 2xx 响应体是 SOAP Fault，且其 `Subcode/Value` 含子串 `NotAuthorized`。要让客户端判成鉴权失败，服务端应返回 **HTTP 400 + `ter:NotAuthorized` fault**，而不是 401（证据：onvif-rs `onvif/src/soap/client.rs` 的非 2xx 分支）。
- **错误映射**：连接拒绝、超时等传输错误都归为 `OnvifError::Protocol`；`Connect` 只表示 URL 解析失败。`NoProfile` 目前没有代码会产生它。
- **时钟偏差没有处理**：onvif-rs 提供 `ClientBuilder::fix_time_gap`，但 `nvr-onvif` 没有设置。如果摄像头会校验 UsernameToken 的 `Created`，而它和 NVR 的时钟差太多，就可能一直报 Auth 失败。排查时先对时，或者改为先 GetSystemDateAndTime 再设置 time gap（目前未实现）。
- **服务地址**：入口固定为 `http://host:port/onvif/device_service`。media / PTZ 的地址取自 `GetCapabilities` 返回的 `XAddr`；摄像头没有 PTZ 服务时返回 `NoPtzService`。ONVIF 常用端口是 80 或 8000，live test 默认用 80。
- **stream URI** 用 `RtpUnicast` + `TransportProtocol::Rtsp` 请求。`Profile.fps` 固定为 0（未解析），codec 取 encoding 的 Debug 字符串。
- **schema 的 yaserde 解析对命名空间很严格**。自己构造响应（例如在 dummy 里）时，必须用标准命名空间：SOAP 1.2 `env`，`tds`/`trt` 用 ver10，`tptz` 用 ver20，`tt` 用 ver10/schema，`ter` 用 ver10/error。只靠 XML 结构单测看不出解析问题，真正的检验是用真实客户端跑 live test。
- **WS-Discovery** 依赖 IPv4 组播 `239.255.255.250:3702`。部分容器或沙箱不转发组播，此时发现结果为空，可以改用直连路径（填 host/port 后点「探测」，或跑 live test）。`Discovered.addr` 取第一个 XAddr 的 `host:port`；name 和 hardware 来自 scopes 里的 `onvif://www.onvif.org/name/…`、`…/hardware/…`。

## dummy-onvif-camera 模拟了什么

- 原因：Rust 没有 ONVIF 服务端库（onvif-rs 只有客户端）；外部模拟器要么得编译 gsoap/C++，要么依赖 pip 且质量参差。而客户端实际调用的面很小，只有 8 个 SOAP 操作加一个 UDP 回复，所以在 example 里手写更可控。视频不重新实现，复用 `dummy-rtsp-camera`（oddity）。
- 只有一个 HTTP 端点 `/onvif/device_service`。`GetCapabilities` 返回的 media 和 PTZ `XAddr` 都指向这同一个 URL（ONVIF 允许这么做）。按请求 body 里元素的 local-name 识别 8 个操作：GetCapabilities、GetDeviceInformation、GetProfiles、GetStreamUri、ContinuousMove、Stop、GetPresets、GotoPreset。
- 每个操作都校验 UsernameToken（用户名 + digest）。鉴权失败返回 HTTP 400 + `ter:NotAuthorized`。不认识的操作或 body 无法解析，一律返回 HTTP 500 + `ter:ActionNotSupported`（不返回 400）。
- 局限：只校验 digest，不检查 `Created` 的时效，也不防 nonce 重放，所以测不出时钟偏差问题；也不支持 HTTP Digest。只有一个 profile（`Profile_1`，H264 1920×1080）。预置位固定两个（`Preset_1`、`Preset_2`）。PTZ 只记日志并回空 ACK，不跟踪位置。没有 events / imaging / Media2，没有 TLS。
- `GetStreamUri` 返回 `--rtsp-url`，默认 `rtsp://127.0.0.1:9554/live/test1`，和 dummy-rtsp-camera 的默认值一致。加 `--launch-rtsp` 时，它会用 `cargo run -p dummy-rtsp-camera` 拉起子进程。`--no-discovery` 关闭组播应答。

## 测试流程（精简）

```bash
cargo test -p nvr-onvif -p dummy-onvif-camera          # 纯 Rust，不需要 LD_LIBRARY_PATH
LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr onvif  # nvr 侧需要 ffmpeg 库

# 端到端：终端 1 启动 dummy（--launch-rtsp 会顺带拉起 RTSP 源）
RUST_LOG=info cargo run -q -p dummy-onvif-camera -- --host 127.0.0.1 --port 8000 \
  --username admin --password admin --launch-rtsp
# 终端 2：live test（被 #[ignore]，没设 ONVIF_TEST_HOST 时直接跳过；PORT 默认 80）
ONVIF_TEST_HOST=127.0.0.1 ONVIF_TEST_PORT=8000 ONVIF_TEST_USER=admin ONVIF_TEST_PASS=admin \
  cargo test -p nvr-onvif --test live -- --ignored --nocapture
```

- 预期输出 `device: lite-nvr dummy-onvif-camera fw 0.1`，测试通过。PTZ 部分是尽力而为：摄像头没有 PTZ 时不算失败。
- 整机验证：`cargo run --package nvr`，API 在 `:18080`，要先登录拿 token。添加设备时 `input_type=onvif`，`input_value` 填 `OnvifConfig` JSON。日志里出现 `onvif <id>: resolved rtsp uri` 即开始录制；dashboard 的 PTZ 按钮会让 dummy 打出 `onvif PTZ: ContinuousMove/Stop/GotoPreset`；用错误密码点「探测」会报 "authentication rejected"。
