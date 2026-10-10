# 技术栈与实现规范

> 选型约定、构建期环境变量、构建与验证命令；写代码前先看本文，按既有方式扩展。

本文回答「这个项目用了什么、新代码该照着什么写、改完怎么验证」。**依赖方向与设计决策在 `architecture.md`**；代码结构用 CodeGraph 查，不在文档里重复。

## 一、选型约定

crate 清单、依赖与版本以各 `Cargo.toml` / `package.json` 为准，用 CodeGraph 查；这里只记选型规则。

- 新依赖先查根 `Cargo.toml` 的 `[workspace.dependencies]`，加在那里再用 `workspace = true` 引用；同类能力不引入第二套（HTTP 用 reqwest，异步用 tokio）。
- 后台任务统一用 `tokio_util::sync::CancellationToken` 取消。
- 错误：应用层（`nvr`）用 `anyhow`，库 crate 可用 `thiserror`。
- 数据库是 turso（SQLite，WAL），迁移 SQL 命名 `YYYYMMDD_name.sql`。
- 前端：Vue 3 + TS + PrimeVue，规则见 `docs/dashboard-rules.md`。

## 二、配置与环境变量

Makefile 会自动 `include .env` 并导出（`.env.example` 为模板，主要是构建代理 `HTTP(S)_PROXY`；必须是 HTTP 代理，ureq 不支持 SOCKS5）。

| 变量 | 作用 |
|---|---|
| `FFMPEG_DIR` | FFmpeg 安装目录，默认 `./ffmpeg` |
| `ZLM_DIR` | ZLMediaKit 安装目录，默认 `./zlm` |
| `LD_LIBRARY_PATH` | 本地运行需含 `ffmpeg/lib`、`zlm/lib`（Makefile 自动拼） |
| `RUST_LOG` | 日志级别，如 `info`、`ffmpeg_bus=debug` |
| `SHERPA_ONNX_LIB_DIR` | sherpa-onnx 预编译库，存在 `third_party/sherpa-onnx/*/lib` 时 Makefile 自动设置 |
| `NVR_ZLM_RTSP_PORT` | NVR 内嵌 ZLM 的 RTSP 监听端口，默认 8554；端口冲突时可设置其他非零 TCP 端口 |

运行期开关（`NVR_GB_*`、`NVR_RECORD_DIR`、`DETECT_MODELS_DIR`、`ASR_MODELS_DIR` 等）以代码里的 `std::env::var` 为准，用 CodeGraph / grep 查。

集成测试会读 `RTSP_TEST_URL`、`ONVIF_TEST_*`、`FTP_TEST_*`、`DETECT_TEST_*`、`ASR_SMOKE_MEDIA`、`XIAOMI_*`、`DISPLAY`（屏幕采集，`make xvfb` 起 `:99`），未设置时相应测试跳过或不跑。

## 三、构建与验证命令

```bash
make install-deps            # FFmpeg & ZLMediaKit 依赖
make download-asr-libs       # 可选：nvr-asr 预编译库
make download-asr-models     # 可选：ASR 模型
cargo build --workspace
make run                     # cargo run --package nvr，API :18080
make watch                   # 改 .rs 自动重启（需 cargo-watch）
make dummy                   # 跑 GB28181 模拟摄像头对接本机 NVR

cargo check --workspace
cargo test --workspace --lib --tests --no-fail-fast   # 与 CI（rust-check.yml）一致
cargo test -p nvr / -p ffmpeg-bus / -p nvr-asr ...
cargo fmt                    # CI 不查 fmt/clippy，提交前自己跑（make fmt-check）
cargo build -p nvr --features smb   # 需要 SMB 转存时

cd nvr-dashboard/app
npm ci && npm run build      # 平时由 nvr-dashboard/build.rs 自动构建
npm run dev
npm run lint && npm run type-check && npm test   # 与 CI（frontend-check.yml）一致
```

## 四、其他约定

- 提交：Conventional Commits，scope 为模块名（`detect`、`dashboard`、`device-config`…）。
- 接口调试：`rest/api.rest`。
- 端到端脚本：`scripts/detect_e2e.sh`、`scripts/start_dummy_*.sh`、`scripts/start_livesrc_push.sh`。

## 五、快速参考：新增一个子系统 HTTP 接口

1. 能力本身放 `crates/<name>`（根 `Cargo.toml` 加 member），可脱离 ZLM 单测。
2. `nvr/src/<name>/mod.rs` 写胶水 / 全局状态，`nvr/src/<name>/api.rs` 写 `<name>_router()`，handler 返回 `ApiJsonResult<T>`。
3. `nvr/src/api.rs` 里 `.nest("/<name>", ...)`；若往 ZLM 写数据，在 `main.rs` 关停链里加 `shutdown()`。
4. 前端：`nvr-dashboard/app/src/api/<name>.ts` + 对应 view。
5. 验证：`cargo test -p nvr`，`rest/api.rest` 补一条请求实测。
