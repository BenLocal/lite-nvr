# 录像设计要点

> 线上录像链路（ZLM HLS 分片归档）与独立的 `nvr-recorder` crate 各自的分段、元数据、保留、外送语义和调用方约束；动录像、回放、清理或外送前先看本文。

原设计稿：2026-07-15「RTSP segment recorder crate (`nvr-recorder`)」及其实施计划。**注意**：该 crate 已经实现，但**至今没有接进 `nvr`**（`nvr/Cargo.toml` 不依赖它）。线上录像走的是第一节描述的 ZLM 链路。

## 一、线上录像链路（nvr + ZLM）

- 开关：设备的 `record` 字段就是 ZLM `Media` 的 `hls_enabled`。录像依赖的正是 ZLM 产出的 HLS 分片；直播预览走 FLV，与 HLS 无关，所以关掉 HLS 就等于关掉录像。
- 分段：ZLM 全局设置 `hls.segDur=60`、`hls.broadcastRecordTs=1`，每产出一个 `.ts` 分片触发一次 `on_record_ts`。
- 归档：`persist_record_ts` 把 ZLM 的分片**复制**（不是移动）到 `<record_dir>/<stream>/<file_name>`。`record_dir` 由 `NVR_RECORD_DIR` 指定，默认为 `<cwd>/data/records`。随后对归档文件执行 ffprobe 取编码和分辨率，并按 `file_path` upsert 到 `record_segments`。
- 元数据语义：`start_time` 是 ZLM 给出的 epoch **秒**；`file_size` 取 ZLM 上报值与归档文件实际大小中较大的那个；`stream` 等于设备 id，回放和外送都按它分组。
- 回放：
  - 列表和播放列表会过滤掉文件已不存在的记录（只打 warn，不删行）。
  - 单段播放列表是一个只含一条 `#EXTINF` 的 VOD 播放列表，拖动进度依靠 `/segment/{id}` 支持的 Range 请求。
  - 设备当日播放列表按**本地时区**划分日界。每两个分片之间都要插入 `#EXT-X-DISCONTINUITY`：每次 ZLM 重新推流 PTS 都会归零，相邻分片的时间戳会重叠，不插的话 hls.js 拼接失败，整天的播放列表都播不下去。

## 二、保留（`nvr/src/cleanup.rs`）

- 策略存在 KV 的 `record_cleanup` 中，可以在 Settings 页修改。**默认关闭**。规则依次执行：先删掉 `create_time` 早于 `max_age_days` 天的记录；如果设置了 `max_total_gb`，再从最旧的开始删，直到总量低于上限。每删一条，同时删文件（尽力而为）和数据库行。
- 执行周期每轮从配置里重新读取，改配置不需要重启；首轮延迟 30 秒，避免和启动过程争抢资源。
- 总量按数据库里的 `file_size` 求和，不扫描磁盘。不在数据库里的文件既不计入总量，也不会被清理。
- 清理**不看外送状态**：还没送出的分片照样会被删除。如果离线外送很重要，保留期要设得比外送积压的时长更长。

## 三、外送（`nvr/src/transport/`）

- 只复制不删除：本地文件保留，回放不受影响。每 30 秒扫一轮；每个目标每轮最多处理 20 个分片，按 `start_time` 从旧到新；失败最多重试 5 次，之后不再自动重试。
- 待送集合是“该目标下还没有 job，或者 job 失败且未达到重试上限”的分片。所以**新建或启用一个目标时，会把库里所有历史分片都补送一遍**。
- 远端路径为 `<base_path>/<stream>/<file_name>`。SMB 后端需要开启 `--features smb` 编译。

## 四、`nvr-recorder` crate（独立库，未接入）

- 定位：从一路 RTSP 拉流，按时间切片写盘，只做 stream-copy，不转码。按段通过 `mpsc<SegmentInfo>` 发出结果，manifest 和写库策略都由调用方负责（例子 `examples/record.rs` 写 `manifest.jsonl`）。`SegmentInfo` 的字段刻意与 `record_segments` 对齐，方便以后直接映射，但这一步至今没做。
- 刻意不做：写入 `nvr-db` / 接 REST、多路调度、转码（容器装不下的编码属于配置错误）、保留清理。
- 默认值：TCP、音视频都录、60 秒一段、TS 容器、超时 5 秒、无限重连（退避 1 秒到 16 秒）。选 TS 作默认是因为抗崩溃：未写完的段也能播到断点；MP4 的 `moov` 要到 `finish()` 才写，崩溃会丢掉当前段。
- 切段规则：
  - 有视频时只在**视频关键帧**处切段（从 GOP 中间开始的 stream-copy 无法播放），第一个文件要等到第一个关键帧才打开；纯音频时任何包都可以切。
  - 不对齐模式下，按媒体时长达到 `segment_time` 切。
  - 对齐墙钟模式下，到达下一个 epoch 整数倍边界时切，因此第一段通常偏短。
- 时间戳：每段用触发该段的那个包的 DTS（没有则用 PTS）作为**公共基准**，各条流分别换算到自己的 time base 后统一减去，效果相当于 `-reset_timestamps 1`。音视频共用同一个基准才能保持同步。减出来的负值钳到 0（开段关键帧之前的音频包属于预期情况）。
- 元数据的口径：
  - `duration` 等于主流（有视频取视频）最后一个 PTS 减第一个 PTS，比真实时长少约一帧。
  - `size_bytes` 是累计写入的包载荷大小，不是文件实际大小。
  - 文件名用 strftime 格式化段开始的 **UTC** 时间，精度到秒。
- 调用方约束：
  - **必须持续消费接收端**：通道容量为 16，发送时会 await。不消费会卡住收包循环，进而导致输入广播 lag，会话被判定断开后重连。
  - 重连计数**整个生命周期内只增不减**，成功录一段也不会归零。因此有限的 `max_retries` 实际限制的是“累计失败次数”而不是“连续失败次数”，退避时长也会一直停在 `max_delay`。退避没有随机抖动。
  - 任何会话内错误（找不到所选轨道、输出打不开、`add_stream` 失败、EOF、广播 lag）都走同一条重连路径。在默认的无限重连下，永久性错误会无限重试；需要尽快失败的话，就设一个有限的 `max_retries` 或 `Some(0)`。
