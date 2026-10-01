# AGENTS.md

> 本文件是 Claude Code 唯一自动加载的项目契约（`AGENTS.md` 与 `CLAUDE.md` 互为软链，同一份内容）。**核心准则写在本文件**；细化规范拆到 `docs/agents-dot-md/*`、经验沉淀放 `docs/memory/*`（都不自动加载，按需查阅）。下方《项目 Skill 索引》《模块文档索引》《记忆索引》由 agents-dot-md skill 的 `reindex.py` 生成，勿手工编辑标记之间的内容。

## 项目概要
Rust 写的轻量 NVR：从 RTSP / 文件 / 屏幕采集 / 测试图案 / V4L2 / GB28181 / ONVIF / 小米摄像头 / 直播平台页面取流，经 FFmpeg 转码，通过 ZLMediaKit 分发（RTSP / RTMP / HLS / FLV），附带导播切换、多画面合成、混音、录像与转存、ASR 实时字幕、目标检测，以及 REST API 与 Vue 3 后台。

- 主程序 `nvr`：API 监听 **18080**，接口挂在 `/api/*`，后台 SPA 在 `/nvr/`，`/media/*` 反代到 ZLM HTTP。
- 查代码结构、符号、调用链先用 CodeGraph（`codegraph_explore`，仓库根有 `.codegraph/` 索引），再退回 grep / 读文件；文档不重复代码结构。

## 常用命令
```bash
make install-deps                                  # 装 FFmpeg / ZLMediaKit 依赖（scripts/pre_install_deps.sh）
make run                                           # = cargo run --package nvr（Makefile 会自动带上 .env 与 LD_LIBRARY_PATH）
cargo check --workspace                            # 快速编译检查
cargo test --workspace --lib --tests --no-fail-fast  # 与 CI 一致的全量测试
cargo test -p <crate>                              # 单 crate 测试
cargo fmt                                          # 格式化（CI 不查，提交前自己跑）
cd nvr-dashboard/app && npm run lint && npm run type-check && npm test   # 前端检查
```
本地直接用 cargo 跑时，`LD_LIBRARY_PATH` 需包含 `ffmpeg/lib`（以及设置了 `ZLM_DIR` 时的 `$ZLM_DIR/lib`）。

## 工作区规则
- 作用范围：本规则适用于当前仓库及所有子目录。
- 交流语言：默认使用简体中文；代码注释、标识符、提交信息沿用仓库既有的英文风格（详见 `docs/agents-dot-md/translation.md`）。
- 修改原则：仅做最小、精确改动，避免无关重构。
- 安全原则：未经明确授权，不执行破坏性 git / 文件操作。
- 验证要求：代码改动后，尽量执行对应 crate 的 `cargo check` / `cargo test -p <crate>`；动了前端跑 lint + type-check。
- 自检要求：代码改动后必须逐条对照 `docs/agents-dot-md/code-checklist.md`，全部符合才算完成。
- 测试位置：测试与源码同目录、以 `_test.rs` 结尾（如 `crates/ffmpeg-bus/src/bus.rs` → `bus_test.rs`），在源文件**末尾**引入：
  ```rust
  #[cfg(test)]
  #[path = "module_name_test.rs"]
  mod module_name_test;
  ```
- 提交信息：Conventional Commits，scope 用模块名（如 `fix(detect): ...`、`feat(dashboard): ...`）。
- 前端：TypeScript + Vue Composition API + PrimeVue，组件文件名 PascalCase；请求统一放 `src/api/<domain>.ts`，规则见 `docs/dashboard-rules.md`。
- 后台产物 `nvr-dashboard/app/dist/` 由 `nvr-dashboard/build.rs` 在源码变化时自动构建，并经 `rust-embed` 嵌入二进制。

## 编码行为准则（Karpathy 防错指南）
1. **先想后写**：不臆测、不藏困惑；多种解读先摆选项让用户拍板；有更简单实现就直说。
2. **简单优先**：只写当前所需最小代码，不提前抽象、不加没要求的防御。
3. **外科手术式改动**：只动该动的；不顺手重构没坏的；只清理本次改动产生的孤儿。
4. **目标驱动**：把任务转成可验证目标（先写复现 / 非法输入测试再改），多步任务先给带验证点的计划。

> 可执行细则与「改动落地规则」见 `docs/agents-dot-md/coding-guidelines.md`。

## 代码 checklist（强制）
> ⛔ 每次写完 / 改完代码，**必须逐条对照 `docs/agents-dot-md/code-checklist.md` 自检**，全部符合才算完成；不符合的先改到符合，不要留给评审或线上发现。新增条目直接追加到该文件。

## 架构与技术栈
> 动结构 / 加模块 / 接外部依赖前读 `docs/agents-dot-md/architecture.md`（依赖方向、启动与关停约束、设计决策）；
> 写代码前读并遵循 `docs/agents-dot-md/tech-stack.md`（选型约定、构建期环境变量、构建与验证命令）。与本文件冲突时以本文件为准。
> 各子系统的设计要点（范围、取舍、协议约束、测试流程）在 `docs/agents-dot-md/design-*.md`，改对应子系统前先读。

## 技能整理（Skill 维护）
本仓库自带的项目 skill 放在仓库任意位置的 `skills/<name>/SKILL.md`（含各业务模块子目录下的 `skills/`；YAML frontmatter 至少含 `name` / `description`，可带脚本 / 资源同目录）。它们随仓库分发、对所有克隆生效；全局 skill（`~/.claude/skills/*`）不入库，本索引不收录。

- 新增 / 改名 / 删除 skill，或改了 `SKILL.md` 的 `description` 后，在仓库根运行 `python3 ~/.claude/skills/agents-dot-md/scripts/reindex.py .`（Windows 用 `python`）重建下方《项目 Skill 索引》《模块文档索引》与 `docs/agents-dot-md/00-index.md`。
- **不要手工编辑** `<!-- SKILLS:START -->…<!-- SKILLS:END -->` 与 `<!-- MODULES:START -->…<!-- MODULES:END -->` 之间的内容——会被脚本覆盖。
- `description` 写清「何时用 / 触发词」，首句作为索引摘要（脚本取首句）；触发要精准，避免与既有 skill 语义重叠。

## 记忆记录（Memory）
本仓库的记忆区是 `docs/memory/*.md`——**纯 Markdown，不依赖外部记忆服务，也不需要 LLM key**，你自己用读写文件的工具维护，检索时直接 `grep` / 读文件。

- **何时记**：完成一次排查 / 根因分析 / 踩坑修复后，把「非显然、下次能省事」的结论写下来，别让下一个会话重新推导。
- **记在哪**：按主题聚合到一个文件（如 `docs/memory/build-and-deploy.md`、`docs/memory/known-pitfalls.md`、`docs/memory/external-integrations.md`），不要一条一个文件。文件头两行必须是 `# 标题` 与 `> 一句话摘要`（否则进不了索引）。
- **和记忆服务的分工**：本环境另有记忆服务（如 mem0）时，只和本仓库有关的结论写这里，随 git 共享给团队和其他机器；跨项目的个人偏好和本机环境事实（代理、凭据放在哪、本机工具版本）写记忆服务，没有记忆服务就不记，不要写进仓库。同一条事实只写一处。
- **每条怎么写**：一行一条，**绝对日期**打头，写清「现象 → 原因 → 结论 / 做法」；能附证据就附（`文件:行`、命令、报错原文）。未验证的猜测标「待验证」。
- **不记什么**：代码结构、git 历史、本文件或模块里已写过的内容；凭据 / 密钥只落未入库的本地文件（如 `dev-env.local.md`），**绝不**写进 `docs/memory/` 或任何入库文件。
- 增删主题文件后在仓库根运行 `python3 ~/.claude/skills/agents-dot-md/scripts/reindex.py .` 重建下方《记忆索引》。与某个任务关联的结论，可按需另记到任务系统（如 `vikunja`）的评论里。

## 📇 项目 Skill 索引（全仓 SKILL.md，脚本生成）
<!-- SKILLS:START -->
- **build-nvr** — 编译 lite-nvr 工程（Rust workspace + 内嵌 Vue 后台） （`.agent/skills/build-nvr`）
<!-- SKILLS:END -->

## 📂 模块文档索引（docs/agents-dot-md/，脚本生成）
<!-- MODULES:START -->
- [系统架构](docs/agents-dot-md/architecture.md) — 依赖方向硬规则、启动与关停顺序的约束、关键设计决策；动结构、加模块、接外部依赖前先看本文。
- [代码 checklist（强制自检）](docs/agents-dot-md/code-checklist.md) — 写完 / 改完代码必须逐条对照的硬性自检清单；新增条目直接追加本文件。
- [编码行为准则（Karpathy 防错指南 · 展开）](docs/agents-dot-md/coding-guidelines.md) — 写 / 改 / 审代码的可执行细则；AGENTS.md 顶层只留四条高压线，细则在此。
- [鉴权设计要点](docs/agents-dot-md/design-auth.md) — 会话 token 的存储、过期、吊销规则，`?token=` 的取舍，以及哪些入口刻意不鉴权；改登录、用户管理或新增绕开 `/api` 的入口前先看本文。
- [目标检测设计要点](docs/agents-dot-md/design-detect.md) — 实时多模型 YOLO 检测（`crates/nvr-detect` + `nvr/src/detect`）与按设备检测配置（device-config Phase 1）的范围、取舍、清单与环境变量语义、API/UI 契约及端到端验证步骤；启动顺序、auto-start 重试、大栈线程、清单缺失不阻塞启动见 `architecture.md`。
- [GB28181 设计要点](docs/agents-dot-md/design-gb28181.md) — NVR 作为 GB28181 上级平台（SIP UAS）：`crates/gb28181` 只管信令，`nvr/src/gb` 做按需拉流桥，媒体全交给 ZLM `RtpServer`；目前只做了实时点播和 PTZ，平台侧回放没有做；本地测试用 `make dummy`。
- [媒体管线拆分设计要点](docs/agents-dot-md/design-media-pipe.md) — `media-pipe-core` / `media-pipe-zlm` 为什么这样拆、边界在哪、调用方必须遵守的约束；改管线输出或接新媒体服务器前先看本文。
- [ONVIF 设计要点](docs/agents-dot-md/design-onvif.md) — NVR 只做 ONVIF 客户端：负责发现摄像头、解析 RTSP URI、下发 PTZ，媒体照旧走 RTSP→ZLM 管道；本地测试用 dummy-onvif-camera 模拟 ONVIF 服务。
- [录像设计要点](docs/agents-dot-md/design-recorder.md) — 线上录像链路（ZLM HLS 分片归档）与独立的 `nvr-recorder` crate 各自的分段、元数据、保留、外送语义和调用方约束；动录像、回放、清理或外送前先看本文。
- [开发环境](docs/agents-dot-md/environment.md) — 本地依赖安装、代理与凭据存放约定；变量清单见 `tech-stack.md`。
- [技术栈与实现规范](docs/agents-dot-md/tech-stack.md) — 选型约定、构建期环境变量、构建与验证命令；写代码前先看本文，按既有方式扩展。
- [交流与语言约定](docs/agents-dot-md/translation.md) — 默认简体中文交流；代码注释、标识符、提交信息沿用仓库既有英文风格。
<!-- MODULES:END -->

## 🧠 记忆索引（docs/memory/，脚本生成）
<!-- MEMORY:START -->
- [目标检测踩坑](docs/memory/detection.md) — 检测模型加载、models.json、tap 生命周期与 lease、auto-start 相关的非显然问题；设计见 `docs/agents-dot-md/design-detect.md`。
- [设备协议踩坑（GB28181 / ONVIF）](docs/memory/device-protocols.md) — GB28181 拉流、注册、PTZ 与 ONVIF 鉴权、profile 相关的非显然问题；设计见 `design-gb28181.md`、`design-onvif.md`。
- [媒体管线与录像踩坑](docs/memory/media-and-recording.md) — media-pipe / ZLM sink、ffmpeg-bus 输出、nvr-recorder 重连，以及默认账号相关的非显然问题；设计见 `design-media-pipe.md`、`design-recorder.md`、`design-auth.md`。
<!-- MEMORY:END -->
