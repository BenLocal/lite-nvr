---
name: build-nvr
description: 编译 lite-nvr 工程（Rust workspace + 内嵌 Vue 后台）。用户说「编译 / 构建 / build / cargo build / 编一下 / 检查能不能编过 / 打 release 包」，或改完代码需要验证能否编译时使用；负责准备 FFmpeg / ZLM / .env 环境、选择 check / build / release / 单 crate，并按常见报错排查。
---

# build-nvr

用 `scripts/build.sh` 编译，它会加载 `.env`、拼好 `FFMPEG_DIR` / `ZLM_DIR` / `LD_LIBRARY_PATH` / `SHERPA_ONNX_LIB_DIR`（与 Makefile 同一套逻辑），先检查前置依赖再调 cargo。

```bash
bash .agent/skills/build-nvr/scripts/build.sh                 # cargo build --workspace（debug）
bash .agent/skills/build-nvr/scripts/build.sh check           # cargo check --workspace，最快
bash .agent/skills/build-nvr/scripts/build.sh release         # cargo build --workspace --release
bash .agent/skills/build-nvr/scripts/build.sh -p nvr          # 只编某个 crate；其余参数原样透传给 cargo
bash .agent/skills/build-nvr/scripts/build.sh check -p nvr --features smb
```

## 怎么选

- 只是验证改动能编过：`check`，并只带改动涉及的 crate（`-p <crate>`）。
- 要运行 / 交付二进制：默认 build 或 `release`，产物在 `target/{debug,release}/nvr`。
- 改了 `nvr/src/transport/smb.rs`：额外跑 `check -p nvr --features smb`（需系统装 libsmbclient 开发包）。
- 想强制重建前端：前面加 `FORCE_REBUILD=1`。

编译通过后如需验证测试，跑 `cargo test --workspace --lib --tests --no-fail-fast`（同样需要脚本里那套环境变量，可 `source` 后执行，见下）。

## 前置依赖

| 依赖 | 缺失时 |
|---|---|
| FFmpeg 共享库 | 依次用 `FFMPEG_DIR`、`./ffmpeg`（`make install-deps` 装的）、系统 pkg-config 里的 FFmpeg；都没有才报错退出 |
| ZLMediaKit | 可选：`./zlm` 或 `ZLM_DIR` 存在就用；否则 `rszlm-sys` 构建时自动下载预编译库 |
| Node.js / npm | `nvr-dashboard/build.rs` 在 `app/dist` 缺失或前端源码变化时会跑 `npm ci` + `npm run build`，没有 npm 会 panic |
| sherpa-onnx（`nvr-asr`） | 可选：`make download-asr-libs` 下载到 `third_party/`，脚本自动设置 `SHERPA_ONNX_LIB_DIR`；否则 `sherpa-onnx-sys` 构建时联网下载 |

构建期下载走 `.env` 里的 `HTTP_PROXY` / `HTTPS_PROXY`（模板 `.env.example`），必须是 HTTP 代理。

## 常见报错

- `Could not find ffmpeg` / pkg-config 找不到 `libavcodec`：没装 FFmpeg 或 `FFMPEG_DIR` 不对 → `make install-deps`。
- `SOCKS feature disabled`：`.env` 配了 SOCKS5 代理 → 换成 HTTP 代理。
- `sherpa-onnx-sys` 报目录不存在：设置了空的 `SHERPA_ONNX_LIB_DIR` → unset 它，或先 `make download-asr-libs`。
- `npm ci failed` / `npm build failed`：进 `nvr-dashboard/app` 单独跑 `npm ci && npm run build` 看真实错误（常见是 type-check 失败）。
- 运行时报 `libavcodec.so.*: cannot open shared object file`：编译期没问题，是运行时 `LD_LIBRARY_PATH` 没带 `ffmpeg/lib` → 用 `make run` 或先 `source` 脚本里的环境。

## 只要环境变量、不编译

```bash
source .agent/skills/build-nvr/scripts/build.sh env   # 在当前 shell 导出环境后返回，不执行 cargo
```
