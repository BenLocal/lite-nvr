# GB28181 设计要点

> NVR 作为 GB28181 上级平台（SIP UAS）：`crates/gb28181` 只管信令，`nvr/src/gb` 做按需拉流桥，媒体全交给 ZLM `RtpServer`；目前只做了实时点播和 PTZ，平台侧回放没有做；本地测试用 `make dummy`。

## 范围与阶段边界

- **已实现**
  - 平台侧：REGISTER（Open 或共享密码 digest）、Keepalive / 离线判定、Catalog 查询、实时 Play INVITE → BYE。
  - 媒体收流支持 UDP、TCP-passive、TCP-active，按次选择（P4a）。
  - PTZ DeviceControl：方向、变倍、预置位的设置 / 调用 / 删除（P2）。
  - `GET /gb/streams` 返回流状态（P4b），dashboard 有国标设备选择器、云台面板和「拉流中 / 空闲」状态列。
- **只在 crate 里实现、nvr 不使用**
  - `GbClient`（下级设备 / 级联角色）：注册、Keepalive，应答 Catalog / DeviceInfo / RecordInfo / INVITE（含 Playback / Download 的 `t=` 时间窗）。
  - 以上只有 loopback 测试和 dummy-camera 在用，nvr 没有接入级联。
- **未做（原计划 P3）**：平台侧录像回放，即 RecordInfo 查询、Playback INVITE、INFO seek / scale。
  - `GbServer` 没有 playback 方法。`build_play_offer` 虽然能生成 `s=Playback`，但没有调用方。
  - 设备侧回放要用 WVP 之类的平台去驱动 dummy。
- **有意推迟或不做**
  - SIP 只走 UDP，没有 SIP over TCP / TLS / WS。
  - 告警订阅、移动位置、语音对讲。
  - PTZ 的 FI 聚焦 / 光圈、TeleBoot、Record / Guard / Alarm 等其它 DeviceControl 子类型、PTZ 位置查询。
  - SDP 的 `f=` / `u=` 行（`sdp.rs` 里有 TODO）。
  - 按设备配置密码：nvr 只用一个共享密码。crate 的 `AuthConfig::Provider` 已有，nvr 没用。
  - 单端口复用 RTP：每路流一个 `RtpServer`。
  - 拉流后用 `rtp_info` 校验并重试。
  - 由 `UnRegist` 驱动拆流。

## 职责划分（规则）

- **crate `gb28181`**
  - 负责 SIP、MANSCDP XML、GB 方言 SDP、digest、registrar、SSRC 分配。
  - **不碰 RTP 字节，不依赖 nvr / ZLM 类型**。媒体交接只靠纯数据 `MediaSpec`（收流地址、SSRC、transport，以及应答后回填的 `negotiated_remote`）。
  - 对外是 async 方法加单一事件流：`bind()` 直接返回 `(facade, UnboundedReceiver<GbEvent>)`，不提供 `.events()` getter，因为只有一个消费者。
- **`nvr/src/gb`**
  - 持有全局 `GbBridge`（`OnceLock`），里面有 `GbServer`、`stream_id → (GB device_id, channel_id, transport)` 映射表，以及活跃会话（`ReceiverHandle` + `MediaSession`）。
  - 负责 REST（`/api/gb/{devices,catalog/{id},ptz,play,streams}`，只用 GET / POST）。
  - ZLM 依赖隔离在 `MediaReceiver` trait 后面，桥的测试用 fake receiver 加真实 `GbServer↔GbClient` loopback。
- **ZLM**
  - 负责收 PS/RTP 并发布到 **`rtp` app**：`RtpServer::new(0, mode, stream_id)`，端口由 ZLM 自己挑。
  - 所有 `RtpServer` 的创建、connect、关闭以及 `rtp_get_info`，都在专用 OS 线程 `zlm-rtp`（`ZlmControl`，`nvr/src/zlm/cmd.rs`）上执行。
  - **为什么用专用线程**：`RtpServer` 是裸 FFI 指针，不跨线程就不需要 `unsafe impl Send`。备选方案是每次操作 `spawn_blocking`，但那样还得保留 unsafe Send 包装，所以没采用。

## 关键决策与理由

- **按需拉流，没有常驻管道**
  - `input_type=gb28181` 的设备在 `ensure_device_pipe` 里只登记映射，不建 pipe。
  - 有观众打开 `/media/rtp/<nvr设备id>.live.flv` 时，ZLM 触发 `on_media_not_found`，桥会依次 `open_rtp`、`invite_play`、（TCP-active 时）`connect_rtp`。
  - 最后一个观众离开时，ZLM 触发 `on_media_no_reader`，桥发 BYE 并 drop handle，释放端口。
- **`stream_id` 就是 nvr 设备 id，不是 GB 编码**
  - 钩子只按 stream 名匹配，与 app 无关。
  - 可播 URL 走 `/media` 反代（`build_gb_flv_url`），不是直连 `127.0.0.1:8553`。
- **`on_media_not_found` 拉流失败也返回 true**
  - 流是我们负责的，ZLM 应该继续等。
  - 失败时 `active` 里什么都没插，ZLM 再次触发就会自然重试。
- **transport 按次请求**
  - `POST /gb/play {device_id, transport?}` 只改映射上的 transport 并返回 URL，真正的拉流仍是惰性的。
  - 被否决的方案：按设备配置、全局默认、编码进 stream_id。
  - transport 只存在内存里：重启后，或设备被 update（`ensure_device_pipe` 会以 `Udp` 重新登记）后，都回到 UDP。
  - dashboard 不调 `/gb/play`，非 UDP 只能通过 API 切换。
  - 切换 transport 要等上一路拉流拆掉之后才生效。
- **TCP-active 两阶段**
  - 先开 Active 模式的 `RtpServer`，再 INVITE。
  - 从 200 OK 的应答 SDP（`c=` / `m=`）里拿设备媒体地址，然后 `connect_rtp`。
  - connect 结果在 ZLM 线程的回调里异步返回（oneshot 移进回调），外层用 5 s 超时兜底，超时后 drop handle 释放端口。
- **流状态以 ZLM 为准**
  - `MediaCache::is_live` 实时调 `MediaSource::for_each` 查询，不在本地缓存。
  - P4b 原设计是用 `on_media_changed` 维护本地集合，后来改掉了，因为事件丢失、迟到或强制关闭都会让缓存漂移。
  - 只有 live 的流才额外调 `rtp_info` 补充 peer / ssrc / port。
- **同流拉取与拆除串行化**
  - `handle_media_not_found`、无读者拆流和注销映射共用按 `stream_id` 的异步操作门；重复 hook 等待正在进行的 INVITE，随后复用活跃会话。
  - 不同流独立；同步映射锁不跨 await。关停先禁止新拉取，再等待进行中的拉取完成并拆流。
- **`MediaSession` 用 RAII 管理**：`stop().await` 发 BYE；直接 Drop 会把 dialog 交给 janitor 补发 BYE；设备主动 BYE 时产生 `SessionClosed` 事件。
- **PTZ 重试策略**
  - Stop 和预置位命令在传输失败时重试一次。
  - **运动命令不重试**：如果响应丢了再重发，可能在操作员松手、Stop 已经送达之后，又让云台重新动起来（约 32 s 后）。
  - `device_control` 收到非 2xx 最终响应也返回 Ok：对控制类 MESSAGE 来说，200 只代表事务确认，不代表云台真的执行了。Catalog 也是同样的处理。

## 协议 / 互通要点

- **SIP / REGISTER**
  - 设备 id 取自 From 的 user。
  - 过期时间只读 `Expires` 头，缺省按 3600 处理，Contact 上的 `;expires` 参数被忽略。`Expires: 0` 表示注销。
  - 后续 MESSAGE / INVITE 都发往 **REGISTER 报文的来源地址**（`get_destination_from_request`），不用 Contact，这样能穿过 NAT。
- **digest 鉴权**
  - 401 challenge 不带 qop，按 RFC2617 qop-less MD5 校验，realm 用 `NVR_GB_DOMAIN`。
  - nonce 一次性使用，有效期 300 s，sweep 时会顺带清理，防止 REGISTER 洪泛把 nonce 表撑大。
- **离线判定**
  - 超过 `keepalive_grace`（默认 180 s）没有 Keepalive，标记为 Offline，但仍保留注册。
  - 注册过期才会删除设备和目的地址。
  - 设计稿里写的「miss ×3」并没有实现。
- **Keepalive 与其它 MESSAGE**
  - 来自未注册设备的 Keepalive 回 404，用来提示设备重新注册。
  - 不认识的 MANSCDP MESSAGE 一律回 200（宽松策略）。
- **MANSCDP 编解码**
  - 解码宽松：按 XML 声明的 `encoding` 转码，支持 GB2312 / GBK（统一按 gbk 解码）和 GB18030；忽略未知元素，缺字段也能容忍。
  - 编码严格，按 GB 标准输出 UTF-8。
  - GB 编码一律当不透明字符串处理，格式不对也不拒绝注册。
- **Catalog**
  - 多包回复按 `SN` 聚合，直到收满 `SumNum` 或 `query_timeout`（8 s）超时。
  - 超时时返回部分结果并带 `incomplete` 标志，但 `/api/gb/catalog` 会**丢掉这个标志**，静默返回部分列表。
  - 发送失败自动重试一次。
- **SSRC**
  - 10 位：实时流为 `0`、回放为 `1`，接着是平台 id 第 4–8 位（5 位），最后是 4 位序号。
  - 写进 SDP 的 `y=`，以及 Subject `通道id:ssrc,平台id:0`。
  - 应答里的 `y=` 会被解析，但不会使用。
- **SDP offer**
  - `m=video <port> RTP/AVP|TCP/RTP/AVP 96 98`，`a=recvonly`，rtpmap 为 96 PS、98 H264。
  - TCP 时带 `a=setup:passive|active` 和 `a=connection:new`。
  - `setup` 描述的是**平台自己的角色**：passive 表示设备连进来，active 表示 ZLM 主动连出去。
  - 只解析 IPv4 的 `c=IN IP4`。
- **`NVR_GB_MEDIA_IP` 必须设成设备能访问到的本机 IP**：它会写进 SDP 的 `c=`。默认值 127.0.0.1 只适合同机的 dummy 测试。
- **PTZCmd 8 字节**（`ptz.rs` 文档里有字节表）
  - 地址字节固定为 1。
  - 变倍速度只有 4 bit，取 `speed>>4` 并且最小钳到 1，否则置了变倍位、速度却为 0，镜头不会动。
  - **位布局是按标准写的，单测只能锁住校验和不漂移**。真实设备方向不对时，只改 `encode_ptz_cmd` 一处，并同步更新测试向量。
  - 前端只有四个方向键；斜向（`up_left` 等）只能通过 API 发送。
  - 关闭云台弹窗（`@hide`）时必须发 stop，这是兜底：松手事件丢了，摄像头也不会一直转下去。

## 配置（环境变量，`nvr/src/gb/config.rs`）

- 只有 `NVR_GB_ENABLE=1`，**并且** `NVR_GB_SIP_ID`（20 位平台编码）和 `NVR_GB_DOMAIN`（realm）都非空时才会启用 GB。否则 `bridge()` 返回 None：`/gb/play`、`/gb/ptz` 报 "GB support is not enabled"，devices / catalog 返回空列表。
- `NVR_GB_PORT`：SIP UDP 端口，默认 5060，监听 `0.0.0.0`。
- `NVR_GB_PASSWORD`：为空表示 Open，不发 challenge；非空表示所有设备共用这一个密码。
- `NVR_GB_MEDIA_IP`：见上节，默认 127.0.0.1。
- 启动顺序：`gb::init` 在 ZLM 之后、`init_device_pipes` 之前执行，所以设备初始化时桥已经就绪。ZLM 钩子在调用时才通过 `gb::bridge()` 晚绑定，因此钩子先注册也没问题。
- bind 失败只记 error 日志，gb 设备会告警 "GB support is not active"。

## 冒烟测试（dummy-camera）

自动化测试只覆盖信令和桥的编排（fake receiver）。**ZLM 能否真的按三种 transport 收到流、PTZ 方向对不对，只能靠这个手测或真机验证。**

1. 启动 NVR：
   ```bash
   NVR_GB_ENABLE=1 NVR_GB_SIP_ID=34020000002000000001 NVR_GB_DOMAIN=3402000000 \
     NVR_GB_PORT=5060 NVR_GB_MEDIA_IP=127.0.0.1 make run
   ```
   这里的值和 `make dummy` 的默认值对应，dummy 的 domain 默认取 server-id 的前 10 位。要测鉴权时，NVR 加 `NVR_GB_PASSWORD=12345678`，dummy 用 `make dummy DUMMY_PASSWORD=12345678`。
2. 运行 `make dummy`（默认设备 / 通道 id 都是 `34020000001320000001`）。NVR 日志应出现 `gb28181: device registered: 3402…0001`。
3. 登录并添加设备（API 端口 18080，必须带 Bearer token）：
   ```bash
   TOKEN=$(curl -s -X POST localhost:18080/api/user/login -H 'Content-Type: application/json' \
     -d '{"username":"admin","password":"admin"}' | jq -r .data.token)
   H=(-H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json')
   curl -s "${H[@]}" localhost:18080/api/device/add -d '{"id":"cam-dummy","name":"Dummy","input_type":"gb28181",
     "input_value":"{\"device_id\":\"34020000001320000001\",\"channel_id\":\"34020000001320000001\"}"}'
   ```
   也可以在 dashboard 里选「国标 GB28181」，然后用选择器（数据来自 registrar + Catalog）选设备和通道。
4. 选择 transport：`curl -s "${H[@]}" localhost:18080/api/gb/play -d '{"device_id":"cam-dummy","transport":"tcp_active"}'`。可选值为 `udp`（缺省）、`tcp_passive`、`tcp_active`。
5. 播放 `http://localhost:18080/media/rtp/cam-dummy.live.flv`（例如用 `ffplay`）。NVR 日志应出现 `gb28181: pulling … -> stream cam-dummy (TcpActive port N)`。此时 `GET /api/gb/streams` 返回 `live:true` 和 `rtp` 块，设备列表显示「拉流中」。
6. 关掉播放器。经过 ZLM 的 no-reader 延迟后，日志出现 `gb28181: released stream cam-dummy`，dummy 收到 BYE 并停止推流。要切换 transport，就在这之后回到第 4 步。
7. 异常路径：
   - 停掉 dummy 后再打开流，日志出现 `pull for stream … failed`，端口被释放，UI 仍然可用。
   - 删除设备后再打开流，不会触发拉流。
   - 不设置 `NVR_GB_ENABLE` 时，`/api/gb/play` 返回未启用错误。
8. PTZ 只有 dummy 打日志，方向是否正确必须用真实云台机验证。验证内容：各方向按住时移动、松开时停止；变倍；调速；预置位的设置 / 调用 / 删除；按住方向键时按 Esc 关闭弹窗，摄像头必须停下。
