# 实时转写与完整音频转写

`crates/nvr-asr` 使用 SenseVoice INT8 ONNX 和 Silero VAD。两种模式复用同一套模型，都能在没有网络的环境中运行。

| 模式 | 入口 | 识别时机 | 结果 |
| --- | --- | --- | --- |
| 实时转写 | `AsrEngine::accept()`、`flush()` | 说话过程中周期性重识别，段落结束后再识别一次 | 可替换的 `Partial`、确认后的 `Final` |
| 完整音频转写 | `AsrModels::transcribe_audio()`、`transcribe_file()` | 每个 VAD 语音段结束后识别一次 | `Transcription`，包含全文和带时间信息的分段 |

这些是 Rust 库接口和命令行能力。本次没有增加 Web 文件上传接口。

## 输入与模型

实时接口和 `transcribe_audio()` 接收 16kHz、单声道、`f32` PCM。采样值通常在 `[-1, 1]`。

`transcribe_file()` 接收 16kHz WAV，支持整数 PCM 和浮点 WAV，多声道会平均混合为单声道。当前没有 MP3/AAC 解码或重采样，其他格式需先转换：

```bash
ffmpeg -i input.mp3 -ar 16000 -ac 1 speech-16k.wav
```

模型默认目录与服务端使用的目录一致：

```text
third_party/asr-models/
  silero_vad.onnx
  sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/
    model.int8.onnx
    tokens.txt
```

可用 `make download-asr-libs` 准备本机原生库，`make download-asr-models` 下载模型。可选的 CT-Transformer 标点模型通过 `AsrConfig::punct_model` 指定，只应用于最终结果。配置了标点模型时，中间结果默认去掉标点；设置 `AsrConfig::partial_punct = PartialPunct::Recognizer`（服务端用环境变量 `ASR_PARTIAL_PUNCT=recognizer`，demo 用 `--partial-punct recognizer`）可保留 SenseVoice 自带的标点，不增加推理开销，但句尾标点可能随后续音频变化。

## 库接口

模型只需加载一次，后续转写复用返回的 `Arc<AsrModels>`：

```rust
use nvr_asr::{AsrConfig, AsrEngine, AsrModels, Transcript, split_sentences};

let cfg = AsrConfig::new("model.int8.onnx", "tokens.txt", "silero_vad.onnx");
let models = AsrModels::load(cfg)?;

// 完整音频文件。
let result = models.transcribe_file("speech-16k.wav")?;
println!("{}", result.text);
for segment in &result.segments {
    println!("{}s +{}s: {}", segment.start, segment.duration, segment.text);
}

// 已经完成解码的整段 PCM，也可直接传入。
let result = models.transcribe_audio(&pcm)?;

// 每条实时音频流使用自己的引擎。
let mut live = AsrEngine::new(models.clone())?;
for chunk in audio_stream {
    for event in live.accept(&chunk) {
        match event {
            Transcript::Partial { segment, text } => update_segment(segment, text),
            Transcript::Final { segment, text, start, duration } => {
                for s in split_sentences(&text, start, duration) {
                    commit_sentence(segment, s);
                }
            }
        }
    }
}
for event in live.flush() {
    // 接收最后一段；不应在普通音频分片结束时调用 flush。
    handle_final(event);
}
```

示例中的 `pcm`、`audio_stream` 和界面回调由调用方提供。所有识别接口都包含阻塞的 CPU 工作；在 Tokio 服务中应放到专用线程或 `spawn_blocking` 中运行。

## 实时纠错的范围

默认 `partial_interval` 为 300ms，表示积累相应长度的语音后尝试重识别。实际显示间隔还受推理耗时和调度影响。

每次 `Partial` 是当前语音段的完整临时文字。调用方应替换上一条临时文字，而不是追加。例如，先显示“今天讨论试试”，收到新结果后替换为“今天讨论实时转写”。

`Partial` 和 `Final` 都带 `segment`（语音段 id）。同一段的 `Final` 取代该段所有 `Partial`，之后 id 加一。`split_sentences` 按 `。！？!?…`（以及后跟空白或结尾的 `.`）把 `Final` 切成短句，按字数比例分配起止时间。

VAD 检测到段落结束时，引擎使用该段完整音频重新识别，输出带起始时间和时长的 `Final`。连续说话时，段落超过 `max_speech_duration`（默认 10 秒，服务端可用 `ASR_MAX_SPEECH_SECS` 覆盖）后 VAD 会更容易切段，但这不是硬上限：sherpa-onnx 只是把阈值调到 0.9、最短静音调到 0.1 秒，实测出现过比设定值长 1–2 秒的段。已经确认的段落不会因下一段语音而再次修改。

连续说话不停顿时，`early_commit`（默认开启，服务端 `ASR_EARLY_COMMIT=0` 关闭，demo 用 `--no-early-commit`）会提前定稿：中间结果里出现一个后面还跟着文字的 `。！？!?.…，,；;`，且相邻两次重识别的前缀一致，就把这段前缀作为 `Final` 发出，按 SenseVoice 的逐字时间戳在下一字开始前 0.12 秒处切掉对应音频，之后只重识别剩下的部分。末尾那个标点不算：SenseVoice 每次识别都会在结尾补一个句号。中间结果会带上语音开始前约 `min_speech_duration + 0.3` 秒的音频，避免 VAD 检测延迟吃掉第一个字。

每次定稿后，切点前 `commit_overlap`（默认 0.5 秒，demo 用 `--commit-overlap`）的已定稿音频会留在缓冲开头，作为下次识别的左侧上下文，让切点后的字识别得更准；时间戳落在这段上下文里的字（容差 0.02 秒）丢掉不用。VAD 关段时的最终识别也同样带上这段上下文，再跳过已经定稿的部分。

`max_decode_secs`（默认不限，demo 用 `--max-decode-secs`）是单次识别的音频上限。RKNN 模型按固定长度转换（5 秒、10 秒等），超出部分会被直接截掉，所以用 NPU 时要设成模型的长度。设了上限后：

- 如果下一次中间结果的识别（当前缓冲 + 一个刷新间隔 + 0.25 秒余量）会超过上限，就强制定稿。切点只在缓冲后半段（新音频部分）里找：优先选最后一个句末或分句标点，其次选相邻两字时间戳间隔最大处（说话人的停顿），都没有就选最后一个合法边界。切点后剩下的部分不会以标点开头。
- VAD 关段时，超过上限的段按同样的规则分成几次识别，每段输出一个 `Final`。离线入口（`--mode offline`）也走这条路径。
- 被强制切开的前半句没有句末标点，例如「开放时间早上9点至」「下午5点，」。前端目前会把它们显示成两行。

`commit_overlap` 最多取上限的四分之一。实测数据见 [asr_rk3588.md](asr_rk3588.md)。

服务端 `/asr` 命名空间的 `partial` 事件为 `{pipe, id, final: false, text}`；`final` 事件为 `{pipe, id, final: true, text, start, duration, sentences: [{text, start, duration}]}`。当前没有跨段纠错、说话人分离或逐词时间戳。

RK3588 上 CPU 与 NPU（RKNN）的实测对比见 [asr_rk3588.md](asr_rk3588.md)。

## 完整音频处理

离线入口自动创建独立 VAD，不改变已有实时引擎的状态。它跳过临时语音缓存和周期性重识别，只在 VAD 结束一个语音段时识别；文件结束后调用 `flush()` 处理尾段。

长音频仍按语音边界分段，默认单段上限为 20 秒。结果结构为：

```text
Transcription
  text: 全部段落的文字，以换行符连接
  segments:
    text: 该段文字
    start: 相对输入开头的秒数
    duration: 该段语音的秒数
```

没有检测到语音时，全文和分段列表为空。文件读取失败、采样率不符或 VAD 初始化失败会返回错误。

当前 WAV 文件会一次读入内存，离线引擎再按有限大小的块送入 VAD。这限制了引擎的待处理缓存，但不等于磁盘流式读取；长录音仍需要容纳整份 PCM 的内存。

## 命令行

在仓库根目录构建：

```bash
make build BUILD_MODE=release BUILD_ARGS="-p nvr-asr --bin nvr-asr-demo"
```

完整 WAV 转写只输出最终全文，处理信息写到标准错误：

```bash
ASR_MODEL_ROOT=third_party/asr-models
target/release/nvr-asr-demo \
  --model "$ASR_MODEL_ROOT/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/model.int8.onnx" \
  --tokens "$ASR_MODEL_ROOT/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tokens.txt" \
  --vad "$ASR_MODEL_ROOT/silero_vad.onnx" \
  --wav speech-16k.wav --mode offline
```

将 `--mode offline` 换成 `--mode streaming` 可观察中间结果和最终分段。额外加 `--realtime` 会在分片间等待，模拟音频陆续到达；不加时尽快处理。原有调用默认仍为 `streaming`，`--partial-ms` 默认 300，`--chunk-ms` 默认 100。`offline` 不允许同时使用 `--realtime`。

## 验收

普通测试覆盖配置、WAV 混音和错误采样率。真实模型测试默认忽略，避免 CI 自动下载模型。准备好本机原生库和上面的模型目录后，在仓库根目录运行：

```bash
bash -c 'source scripts/build.sh env >/dev/null; cargo test -p nvr-asr complete_audio_matches_streaming_and_skips_partials -- --ignored --nocapture --test-threads=1'
```

默认使用模型随附的 `test_wavs/zh.wav`，也可通过绝对路径 `NVR_ASR_TEST_MODEL_DIR`、`NVR_ASR_TEST_WAV` 指定其他模型目录和测试音频。

测试使用同一份模型和音频，先预热，再核对：

- 实时模式确实产生 `Partial` 和最终分段。
- 离线全文、分段文字及时间信息与实时 `Final` 一致。
- 文件入口与 PCM 入口结果一致。
- 离线处理不积累临时语音缓存，不触发周期性重识别。
- 空音频和纯静音得到空结果，尾段由 `flush()` 处理。

输出的耗时不包含模型加载、文件读取和模拟到达的等待，只适合比较该机器上这份样本的两条处理路径。它不代表 Rockchip 板端的实时性能保证。

### 本次真实音频验收

2026-11-10 在 Apple M2 本机验证，使用 sherpa-onnx 1.13.3、默认 2 个识别线程、SenseVoice INT8 和 Silero VAD，未配置标点恢复模型。输入为模型附带的 `test_wavs/zh.wav`，时长 5.592 秒。

| 路径 | 中间结果数 | 最终分段数 | 预热后的处理耗时 |
| --- | --- | --- | --- |
| 实时纠错路径，100ms 分块，无到达等待 | 13 | 1 | 2.435 秒 |
| 完整音频路径，无中间重识别 | 0 | 1 | 0.311 秒 |

两条路径的最终文字、分段起始时间和时长一致；文件入口与 PCM 入口结果也一致。空音频、纯静音和错误采样率检查通过。这是单份样本的验证，不是字错率评测或板端延迟基准。

### Rockchip 板端验收

2026-11-10 更新 RK3588 测试板后，使用同一份 5.592 秒的中文音频验证：

- `/chat` Socket.IO 语音链路收到 13 次中间转写和非空最终文字。
- 板端 `nvr-asr-demo` 的 `offline` 与 `streaming` 最终文字一致，也与 `/chat` 最终文字一致。
- 离线路径得到 1 个分段，单次处理耗时为 0.339 秒，不含模型加载和文件读取。
- 管理页面和 Chat 页面均返回 HTTP 200，现有模型及运行数据继续使用原挂载目录。

本次额外将 `nvr-asr-demo` 放入测试板的安装目录 `bin/`；默认发布包仍只包含 NVR 主程序。完整文件转写通过命令行使用，Web 文件上传尚未接入。上述耗时来自单份样本，不能替代长录音或多路并发测试。
