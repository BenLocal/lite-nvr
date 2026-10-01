# 开发环境

> 本地依赖安装、代理与凭据存放约定；变量清单见 `tech-stack.md`。

## 本地依赖
- `make install-deps` 安装 FFmpeg 与 ZLMediaKit 到 `./ffmpeg`、`./zlm`（均已 gitignore）。
- ASR 相关的预编译库和模型下载到 `third_party/`（已 gitignore），见 `make download-asr-libs` / `make download-asr-models`。
- 本地数据：`nvr.db*`、`data/` 已 gitignore，不要提交。

## 代理
- 构建期下载（sherpa-onnx、ZLM 源码等）走 `.env` 里的 `HTTP_PROXY` / `HTTPS_PROXY`，模板 `.env.example`；Makefile 会同步导出小写变量给 curl / wget。
- 必须是 HTTP 代理，SOCKS5 会让 ureq 报 "SOCKS feature disabled"。

## 凭据
> ⚠️ 摄像头账号、小米账号（`XIAOMI_*`）、ONVIF / FTP 测试账号等**不入库**：放 `.env`（已 gitignore）或本地未入库文件（如 `dev-env.local.md`）。新增此类凭据一律如此，勿写进任何受版本控制的文件。
