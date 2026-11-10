# RK3588 上的 ASR 实测：CPU 与 NPU

> 2026-11-10 在 RK3588 测试板上对比 SenseVoice 的 CPU（int8 ONNX）与 NPU（RKNN）推理，以及本仓库实时转写引擎（VAD + 离线模型反复重识别 + 提前定稿）在两种后端下的延迟和 CPU 占用。

## 结论

- **NPU 更省 CPU，但单次识别更慢。** 同样每 500ms 刷新一次中间结果，连续说话时 NPU 占用约 0.42 个 CPU 核，CPU 后端约 0.91 个核。NPU 每次识别固定约 460ms，CPU 后端随音频长度从约 110ms 增长到约 600ms。
- **NPU 不能用默认的 300ms 刷新间隔。** 每次识别约 466ms，按 300ms 刷新时处理耗时是音频时长的 1.41 倍，跟不上实时；在 nvr 服务里会导致音频广播通道积压、丢帧。刷新间隔要不小于 500ms（处理耗时占音频时长 90%，余量小），建议 600ms（75%）。
- **NPU 识别每次仍要约 210ms CPU。** 原因未测：可能是特征提取、RKNN 模型里退回 CPU 执行的算子，或运行时等待结果的方式（待验证）。
- **NPU 核数设置不影响速度。** `num_threads` 设 1、2、3、-1，单次耗时都约 450ms，负载采样显示每次只用一个 NPU 核（占用 50–60%）。
- **识别结果一致。** 同一音频两种后端输出相同文字，提前定稿在相同位置触发。
- **提前定稿在板上有效。** 18.4 秒连续语音，CPU 后端 partial 最长耗时从 770ms 降到 369ms，VAD 关段时的最终识别从 777ms（11.6 秒音频）降到最长 284ms，第一句提前约 7 秒定稿。

## 测试环境

| 项目 | 值 |
|---|---|
| 板子 | Rockchip RK3588 EVB1 LP4 V10，8 核，内存 7.6 GB，Alpine Linux |
| NPU | RKNPU 驱动 v0.9.8；`librknnrt.so` 取自 Rockchip 发布包 `lib/` |
| 推理库 | sherpa-onnx 1.13.3。CPU：`linux-aarch64-static-lib`；NPU：`rknn-linux-aarch64-shared` |
| CPU 模型 | `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/model.int8.onnx`，2 线程 |
| NPU 模型 | `sherpa-onnx-rk3588-10-seconds-sense-voice-zh-en-ja-ko-yue-2024-07-17/model.rknn`（480 MB，输入固定补齐到 10 秒） |
| VAD | `silero_vad.onnx`，CPU |
| 运行方式 | 一次性容器 `docker run --rm --device /dev/dri ubuntu:24.04`；同时运行着 lite-nvr（大模型已加载、空闲）、jellyfin 等其它容器，数据只作参考 |
| 未配置 | 标点模型（CT-Transformer）；`use_itn` 开启 |

测试音频（16 kHz 单声道）：

| 文件 | 时长 | 内容 |
|---|---|---|
| `short2s.wav` | 2.0 秒 | `continuous2.wav` 的前 2 秒 |
| `zh.wav` | 5.6 秒 | 模型自带测试音频，「开放时间早上9点至下午5点。」 |
| `continuous2.wav` | 9.2 秒 | 截取 `zh.wav` 的 0.65–5.25 秒（去掉首尾静音），首尾相接 2 遍 |
| `continuous4.wav` | 18.4 秒 | 同上，4 遍，模拟不停顿的连续说话 |
| `silence.wav` | 0.5 秒 | 静音，用于扣除模型加载开销 |

## 单次识别：官方命令行

用 RKNN 版 sherpa-onnx 自带的 `sherpa-onnx-offline`，CPU 与 NPU 用同一个程序。每段音频识别 3 遍，取平均：

| 音频 | NPU 单次耗时 | CPU 单次耗时 |
|---|---|---|
| 2.0 秒 | 456ms | 134ms |
| 5.6 秒 | 464ms | 336ms |
| 9.2 秒 | 469ms | 598ms |

NPU 核数（`--num-threads`，`zh.wav` 识别 5 遍的总耗时）：

| 设置 | 1 | 2 | 3 | -1 |
|---|---|---|---|---|
| 5 遍总耗时 | 2.250s | 2.347s | 2.265s | 2.342s |

单次识别消耗的 CPU 时间（同一模型分别识别 1 遍和 6 遍，user+sys 之差除以 5，扣掉加载开销）：

| 音频 | NPU | CPU |
|---|---|---|
| 2.0 秒 | 208ms | 219ms |
| 5.6 秒 | 215ms | 688ms |
| 9.2 秒 | 192ms | 1234ms |

## 实时转写：本仓库引擎

`nvr-asr-demo --realtime`：每次送 100ms 音频，处理完再送下一块，处理时间不与音频重叠。所以 partial 与最终识别的耗时总和必须小于音频时长，才能跟上直播。提前定稿开启，`max_speech_duration` 10 秒。NPU 版用 `sherpa-shared` 特性动态链接 RKNN 版 sherpa-onnx，`--provider rknn --num-threads 1`。

模型加载：CPU 约 2.5 秒，NPU 约 1.4 秒。加载阶段 CPU 时间（`silence.wav` 的 user+sys）：CPU 2.71 秒，NPU 1.87 秒。

### `continuous4.wav`（18.4 秒连续语音）

| 后端 | 刷新间隔 | partial 次数 | partial 平均 / 最长 | 最终识别次数 / 总耗时 | 识别总耗时 / 音频时长 | 识别占用 CPU（扣除加载） | 折合 CPU 核 |
|---|---|---|---|---|---|---|---|
| CPU | 300ms | 53 | 209ms / 369ms | 3 / 0.61s | 11.69s / 64% | 23.7s | 1.29 |
| CPU | 500ms | 33 | 230ms / 416ms | 3 / 0.59s | 8.17s / 44% | 16.7s | 0.91 |
| NPU | 300ms | 53 | 466ms / 491ms | 3 / 1.34s | 26.04s / 141% | 12.2s | 0.66 |
| NPU | 500ms | 33 | 462ms / 488ms | 3 / 1.32s | 16.57s / 90% | 7.8s | 0.42 |
| NPU | 600ms | 27 | 462ms / 502ms | 3 / 1.41s | 13.89s / 75% | 6.8s | 0.37 |

`time` 原始数据（墙钟 / user / sys）：CPU 300ms 33.3 / 26.06 / 0.36；CPU 500ms 29.8 / 19.06 / 0.33；NPU 300ms 47.6 / 12.29 / 1.75；NPU 500ms 38.1 / 8.02 / 1.64；NPU 600ms 35.5 / 6.87 / 1.76。NPU 300ms 运行期间，0.5 秒间隔的负载采样 95 次里有 89 次 NPU 非空闲。

所有配置的最终输出相同：

```
#0 [ 0.00s +4.56s] 开放时间早上9点至下午5点。      ← 提前定稿
#1 [ 4.56s +4.62s] 开放时间早上9点至下午5点，      ← 提前定稿
#2 [ 9.18s +2.56s] 开放时间早上9点。               ← VAD 强制切段
#3 [11.91s +1.73s] 至下午5点。
#4 [13.86s +4.54s] 开饭时间早上9点至下午5点。
```

（NPU 300ms 那一轮的 #1 末尾是「。」，其余是「，」。）

### `zh.wav`（5.6 秒），刷新间隔 300ms

| 后端 | partial 次数 | partial 平均 / 最短 / 最长 | 最终识别 | 输出 |
|---|---|---|---|---|
| CPU | 13 | 196 / 107 / 303ms | 268ms | 派饭时间早上9点至下午5点。 |
| NPU | 13 | 469 / 450 / 487ms | 443ms | 开饭时间早上9点至下午5点。 |

### 提前定稿开 / 关（CPU，300ms，`continuous4.wav`）

| | 开 | 关 |
|---|---|---|
| partial 次数 / 最长耗时 | 53 / 369ms | 53 / 770ms |
| 最终识别 | 2.56s、1.73s、4.54s 音频，最长 284ms | 11.64s、1.73s、4.54s 音频，最长 777ms |
| 第一句定稿 | 说话中 4.56 秒处 | 第一个 VAD 段结束（约 11.7 秒）后 |

基线（CPU，`zh.wav`）的 partial 耗时随音频长度线性增长：0.87 秒音频 200ms（首次，含预热）、1.19 秒 110ms、2.47 秒 176ms、3.75 秒 242ms、4.71 秒 307ms，约每秒音频 55ms 加 60ms 固定开销。

## 发现的问题

- **VAD 强制切段会切断句子。** 提前定稿裁掉了已定稿的音频，但 VAD 内部的「最长段」计时不会重置。第三遍在 11.64 秒被强制切开，变成「开放时间早上9点。」「至下午5点。」，提前定稿开或关都一样。
- **NPU 输入上限 10 秒。** VAD 强制切段仍可能产生 11.6 秒的段；接入 NPU 前，超过 10 秒的音频要先拆开。超出部分 sherpa-onnx 如何处理没有验证。
- **NPU 与大模型、TTS 共用。** RKLLM 占 3 个 NPU 核，MeloTTS-RKNN 也用 NPU，以前出现过 NPU 内存分配失败（`docs/memory/voice-chat.md`）。ASR 的 RKNN 模型要多占约 480 MB，本次没有测与它们同时运行的情况。
- **多路并发未测。** 每次识别只用一个 NPU 核，按 500ms 刷新时一路就占该核约 90% 时间；多路能否分配到不同核没有验证。

## 待验证

- NPU 每次识别约 210ms 的 CPU 时间具体花在哪里，能否降低。
- 5 秒档 RKNN 模型用于 partial 的耗时（理论上补齐更少，可能更快）。
- 「短音频用 CPU、接近 10 秒用 NPU」的混合方案。
- 与语音对话（RKLLM + TTS）同时运行时的延迟、NPU 占用和 NPU 内存。

## 复现

板上测试目录 `/tmp/asr-demo-test/`。demo 用 `lite-nvr-cross:host-native` 镜像交叉编译：

```bash
# CPU 版（静态链接）
CROSS_CONTAINER_OPTS="--volume \"$PWD/third_party:/project/third_party:ro\"" \
SHERPA_ONNX_LIB_DIR=/project/third_party/sherpa-onnx/sherpa-onnx-v1.13.3-linux-aarch64-static-lib/lib \
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
cross build --release --locked -p nvr-asr --bin nvr-asr-demo --target aarch64-unknown-linux-gnu --target-dir target/package/asr-demo

# NPU 版：<rknn-lib> 内放 rknn-linux-aarch64-shared 的 lib/*.so 和 librknnrt.so
CROSS_CONTAINER_OPTS="--volume \"<rknn-lib>:/rknnlib:ro\"" SHERPA_ONNX_LIB_DIR=/rknnlib ... \
cross build --release --locked -p nvr-asr --features sherpa-shared --bin nvr-asr-demo \
  --target aarch64-unknown-linux-gnu --target-dir target/package/asr-demo-rknn

# 板上运行（NPU）
docker run --rm --device /dev/dri -e RUST_LOG=nvr_asr=debug \
  -e LD_LIBRARY_PATH=/t/rknn/lib:/rk -v /tmp/asr-demo-test:/t -v <asr-models>:/m:ro -v <package>/lib:/rk:ro \
  ubuntu:24.04 /t/nvr-asr-demo-rknn --provider rknn --model /t/rknn/model/model.rknn \
  --tokens /t/rknn/model/tokens.txt --num-threads 1 --vad /m/silero_vad.onnx \
  --wav /t/continuous4.wav --realtime --partial-ms 600
```

NPU 负载：`cat /sys/kernel/debug/rknpu/load`。
