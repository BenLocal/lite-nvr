# AGENT.md

本文件给自动化编码代理（AI Agent）快速上手 `nvr-server` 项目使用。

## 1. 项目定位

`nvr-server` 是一个 Rust 实现的轻量 NVR（Network Video Recorder）：
- 接入多种视频源（RTSP/RTMP、文件、屏幕、测试源、V4L2）
- 基于 FFmpeg 做解复用、编解码、封装
- 输出到 ZLMediaKit / 网络流
- 提供 HTTP API 与内置 Dashboard

## 2. 仓库结构（Workspace）

- `nvr-server/`: 主服务（API、pipe 管理、ZLMediaKit 集成）
- `ffmpeg-bus/`: 媒体处理核心（input/decoder/encoder/output）
- `nvr-db/`: 数据库访问与 migration（SQLite）
- `nvr-dashboard/`: 前端 Dashboard（Vue）
- `scripts/`: 依赖安装与辅助脚本
- `rest/api.rest`: API 调试样例

根 `Cargo.toml` 是 workspace，成员为以上 4 个 crate。

## 3. 环境与依赖

- Rust edition 2024
- FFmpeg 7.x 动态库
- 可选：ZLMediaKit（默认 feature `zlm` 启用）

推荐先执行：

```bash
bash scripts/pre_install_deps.sh
```

## 4. 常用开发命令

### 启动主服务

```bash
cargo run --package nvr-server
```

默认行为：
- API 监听 `0.0.0.0:8080`
- 启动时自动执行数据库迁移
- 默认数据库文件：`nvr.db`
- 默认挂载前端路由：`/nvr`

### 运行测试

```bash
cargo test --workspace
```

说明：部分媒体测试依赖本地 FFmpeg 环境与测试素材（如 `scripts/test.mp4`）。

### 前端开发（可选）

```bash
cd nvr-dashboard/app
npm install
npm run dev
```

## 5. API 快速验证

项目启动后可先验证：

- `GET /pipe/list`
- `POST /pipe/add`
- `GET /pipe/status/{id}`
- `GET /pipe/remove/{id}`

可参考 `rest/api.rest` 里的请求样例直接调试。

## 6. 代理执行任务时的建议流程

1. 先读 `README.md` 与目标 crate 的 `src/`，确认改动边界。
2. 优先做最小修改，避免跨 crate 无关重构。
3. 改完先跑与改动最相关的测试，再跑 `cargo test --workspace`（若耗时可说明未全量执行）。
4. 若涉及 API 行为变化，同步更新 `README.md` 或 `rest/api.rest`。
5. 输出结果时明确：改了哪些文件、为什么改、如何验证。

## 7. 高风险区域

- `ffmpeg-bus/` 的编解码与封装链路（容易引入时序/格式回归）
- `nvr-server/src/media/pipe.rs`（输入输出拼装、编码配置、raw 数据路径）
- feature `zlm` 相关逻辑（有条件编译，修改后需关注无 `zlm` 场景）

## 8. 提交前检查清单

- 能编译：`cargo check --workspace`
- 测试通过：至少相关测试通过
- 无无关文件改动
- 文档与接口示例已同步（若行为变化）

## 9. 严格提交信息模板

采用 Conventional Commits，建议格式：

```text
<type>(<scope>): <subject>

<body>

<footer>
```

约束：
- `type` 仅限：`feat` `fix` `refactor` `perf` `test` `docs` `chore`
- `scope` 建议使用 crate 名：`nvr-server` `ffmpeg-bus` `nvr-db` `nvr-dashboard`
- `subject` 用祈使句，<= 72 字符，不加句号
- `body` 必须写“动机 + 方案 + 风险/兼容性”
- 涉及 breaking change 时，在 `footer` 增加 `BREAKING CHANGE: ...`

示例：

```text
fix(nvr-server): avoid pipe status race on cancel

在并发 remove/status 请求下，pipe 可能返回已释放句柄。
通过在 manager 增加状态快照与原子切换，避免读取悬垂状态。
风险：状态字段新增，需确认旧接口兼容。

Refs: #123
```

## 10. 必须执行的测试矩阵

按改动范围执行，未覆盖项需在结果中明确说明原因。

### A. 全局最小门禁（所有改动必跑）

```bash
cargo check --workspace
cargo test -p nvr-db
```

### B. Rust 服务侧改动（`nvr-server/`）

```bash
cargo test -p nvr-server
```

若改动涉及 `/pipe/*`、`/user/*`、`/system/*` 路由：
- 使用 `rest/api.rest` 或 `curl` 至少验证 1 条成功链路与 1 条失败链路。

### C. 媒体链路改动（`ffmpeg-bus/` 或 `nvr-server/src/media/`）

```bash
cargo test -p ffmpeg-bus
cargo test -p nvr-server media::pipe
```

并补充：
- 至少一次端到端冒烟（输入 -> 编解码/转封装 -> 输出）
- 明确记录测试输入（如 `scripts/test.mp4`、lavfi 参数）

### D. 前端改动（`nvr-dashboard/app/`）

```bash
cd nvr-dashboard/app
npm run type-check
npm run build
```

若改动页面交互：
- 补充关键路径手测结果（登录、列表、状态展示等）。

## 11. 改动影响分级（必须标注）

每次任务在结果里标注影响级别，并匹配执行策略：

- `L0` 文档/注释/非行为改动
  - 要求：对应模块最小检查（编译或静态检查）
- `L1` 局部逻辑改动（不改接口）
  - 要求：目标 crate 测试通过 + 关联用例通过
- `L2` 接口/数据结构变更（API、DB、配置）
  - 要求：相关 crate 全量测试 + API/迁移验证 + 文档同步
- `L3` 媒体主链路或并发模型变更
  - 要求：workspace 检查与核心测试全跑 + 端到端冒烟 + 回滚方案说明

## 12. 输出规范（Agent 回报格式）

每次执行完成后，按以下顺序输出：

1. 变更摘要（文件 + 核心行为变化）
2. 影响级别（`L0/L1/L2/L3`）
3. 验证结果（实际执行的命令与结论）
4. 风险与回滚点（如有）
5. 未完成项与原因（如有）

## 13. 前端规则优先级（nvr-dashboard）

凡是修改 `nvr-dashboard/app/` 的任务，Agent 必须优先读取并遵循：

- `nvr-dashboard/app/RULES.md`

执行优先级：
- 若本文件与 `nvr-dashboard/app/RULES.md` 在前端实现细节上有冲突，以 `RULES.md` 为准。
- 若用户明确提出与规则冲突的需求，先执行用户需求，并在结果中标注偏离点。

前端关键规则摘要（详细以 `RULES.md` 原文为准）：
- API 请求分层：
  - 所有 HTTP 请求必须放在 `src/api`。
  - 按业务域拆分文件（如 `src/api/user.ts`）。
  - 公共请求行为（base URL、鉴权头、错误处理）统一在 `src/api/request.ts`。
  - 组件/页面/composable 不得直接写请求逻辑。
- 表单与校验：
  - 默认使用 `@primevue/forms`。
  - 错误态需使用 `invalid`（输入框红边）。
  - 字段错误信息显示在输入框下方，使用 PrimeVue 风格组件。
- 交互反馈：
  - 禁止使用 `window.alert`、`window.confirm`、`alert()`、`confirm()`。
  - 确认框使用 `primevue/confirmdialog`。
  - 警告/提示使用 `primevue/toast`。
  - 根组件挂载 `ConfirmDialog` 与 `Toast`，页面内通过 `useConfirm`、`useToast` 调用。
- 表单宽度一致性：
  - 统一使用 `class="field-input"`。
  - 对包裹型组件（如 `Password/InputNumber/Select`）保证外层与内层均为 `width: 100%`。
