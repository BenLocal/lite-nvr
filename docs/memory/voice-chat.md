# 板端大模型与语音对话踩坑

> RKLLM 进程内推理、`/chat` 语音对话（ASR → LLM → TTS）在 Rockchip 板上的非显然问题；设计与验收见 `docs/voice-chat-spec.md`。

- 2026-10-19：20 主机 `/chat` 同一会话连续提问，首句语音从约 5 s 涨到约 14.5 s → RKLLM runtime 1.3.0 + Qwen3.5-0.8B 的 prefill 只有约 30 token/s（401 token 用 12.8 s，外部 `rkllm-server` 同样慢），而 `nvr-rkllm` 每轮重新渲染并 prefill 完整历史 → 估算延迟按「prompt token ÷ 30」算，历史长度直接决定首字时间；要降延迟只能复用 KV cache 或缩短历史（待实施）。
- 2026-10-19：`scripts/download_asr_models.sh` 走 GitHub release，经本地代理 TLS 握手失败（`curl: (35) unexpected eof`）→ 同样的文件在 Hugging Face `csukuangfj/<模型名>` 仓库（silero VAD 在 `csukuangfj/vad`），用 `https://hf-mirror.com/csukuangfj/<repo>/resolve/main/<file>` 直连下载即可，只需 `model.int8.onnx`、`tokens.txt`、标点模型 `model.onnx`，不用下约 1 GB 的 tarball。
- 2026-10-19：给 `nvr-rkllm` 加 KV cache 续写后，第一次上板没有命中缓存 → Qwen3.5-0.8B 的 `<|im_end|>` id 是 248046，不是默认停止列表里 Qwen2/3 的 151645，回复靠 runtime 自己的 EOS 结束，而代码只缓存「命中停止 token」的回复 → 改为缓存所有自然结束（未取消、未达 max_tokens）的回复。用 `RKLLM_LOG_LEVEL=3` 数 token 验证过：自然结束时 EOS 不进 KV cache（KV 增量 = 回复 token 数，48+34=82），所以续写开头要补 `<|im_end|>\n`；`RKLLM_LOG_LEVEL=3` 会打印 prompt token 和 `newly added user cache size`，`=1` 只打印 prefill/generate 性能表。
