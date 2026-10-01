# 代码 checklist（强制自检）

> 写完 / 改完代码必须逐条对照的硬性自检清单；新增条目直接追加本文件。

本文件由 `AGENTS.md`（与 `CLAUDE.md` 同一份）拆出，收敛「写代码时必须逐条自检」的硬性条目。顶层只留一条强制指针，条目细节都在这里；新增条目直接追加到本文件，不要写回 `AGENTS.md`。

**通用条目**是跨语言的硬红线；**语言专项**由 agents-dot-md skill 的 `scaffold.py` 按本仓库实际语言注入，是该语言公认的固定规范。落地时请按本项目实际情况增删改——不适用的直接删掉，别留着占位；踩过的坑按下面的格式追加。

## 怎么写一条 checklist

每条包含三部分，缺一不可——只写规则不给例子，下一个 agent 照样会踩：

1. **规则**：一句话说清「必须怎样 / 禁止怎样」。
2. **反例**：真实踩过的写法，标明后果（报什么错、线上出过什么事故）。
3. **正例**：照抄就对的写法。

条目来源优先级：**线上事故 > 评审反复提的意见 > 团队口头约定**。没踩过的坑不必预防性地写进来，这里只放硬性红线；软性建议归 `coding-guidelines.md`。

---

# 通用条目（跨语言）

## 1. SQL 必须兼容 turso / SQLite

数据库是 SQLite（经 `turso` crate，WAL 模式），不是 MySQL / Postgres。写 SQL 只用 SQLite 支持的语法；表结构变更一律新增迁移文件 `nvr-db/migrations/YYYYMMDD_<name>.sql`，**不改已发布的迁移**。

- 反例：直接修改 `20260210_init.sql` 加列——已部署实例不会重跑，线上表结构与代码不一致。
- 正例：新建 `nvr-db/migrations/<日期>_<name>.sql` 写 `ALTER TABLE ... ADD COLUMN ...`，并在 `nvr-db` 补 `_test.rs`。

## 2. 禁止 1+N：循环里不查库、不调远程接口

循环体内逐条查询 / HTTP 调用（含调 ZLM HTTP API），量一上来就是超时。

- 反例：遍历设备列表，每个设备单独查一次库。
- 正例：一次批量查询后在内存里按 key 分组。

## 3. 对外接口的返回结构 / 错误码 / 鉴权方式不得随意改动

dashboard 依赖这些契约。确需变更时同步改前端 `nvr-dashboard/app/src/api/*.ts`，或新增接口并存，不要就地改语义。

- 统一返回体：`nvr::handler::BaseResponse { code, message, data }`，handler 返回 `ApiJsonResult<T>`，成功用 `ok_json` / `ok_empty`（`code: 0`），失败经 `ApiError` → HTTP 500。
- 鉴权：`auth::require_auth` 包住整个 `/api`；免登录路径只在 `EXEMPT_PATHS` 里加，且需说明理由。

## 4. 敏感信息不入库

摄像头 / 小米 / ONVIF / FTP 账号、token 一律不写进受版本控制的文件（含测试、注释、`rest/api.rest`、提交信息）。走环境变量或 `.env`（见 `environment.md`）。

提交前自检：`git diff` 里有没有密码、token、私钥字面量。

## 5. 沿用既有技术选型，不引入第二套同类方案

新增依赖前先看根 `Cargo.toml` 的 `[workspace.dependencies]` 是否已有同类（HTTP 用 reqwest、错误用 anyhow/thiserror、异步用 tokio）；新依赖加到 workspace 依赖里再在 crate 中 `workspace = true` 引用。前端同理沿用 PrimeVue 与 `src/api/request.ts`。

## 6. 往 ZLM / ffmpeg 写数据的后台任务必须可取消、可 join，并进关停链

进程退出时 ZLM/ffmpeg 的 C 静态析构会立即执行，仍在写 ZLM 的线程会触发 use-after-free 段错误。

- 规则：新任务接收 `CancellationToken`，保留 `JoinHandle`；在 `nvr/src/main.rs` 的 teardown 中按「生产者先于 manager」的顺序调用其 `shutdown()`。
- 规则：替换同 id 的设备源前，先 `stop` 再 `join` 旧 Entry，确保旧 ZLM `Media` 已释放（见 `manager.rs`）。

## 7. 测试与源码同目录，命名 `<module>_test.rs`

- 反例：新建 `tests/` 目录或在源文件里写内联 `mod tests { ... }`（`crates/gb28181` 里既有的内联测试不强行迁移，新增测试仍按本条）。
- 正例：`foo.rs` 旁建 `foo_test.rs`，在 `foo.rs` 末尾用 `#[cfg(test)] #[path = "foo_test.rs"] mod foo_test;` 引入。依赖真实设备 / 网络的测试读 `*_TEST_*` 环境变量，未设置时跳过。

---

# Rust 专项

## R1. 可恢复错误用 Result，并保留错误来源

库代码把可恢复失败返回为 `Result`，使用 `?` 传播；补上下文时保留 source，便于沿错误链定位。`unwrap` / `expect` 只用于测试或已经由不变量证明不可能失败的分支，并写清该不变量。

- 反例：对文件、网络响应或用户输入直接 `.unwrap()`。
- 正例：沿用项目既有错误类型，例如 `map_err(|source| ConfigError::Read { path: path.to_owned(), source })?`；项目已经使用 `anyhow` 时才用 `with_context(...)`。

## R2. clone 必须表达所有权需要，避免为绕过借用检查器复制

先调整借用范围、参数类型或数据结构，再考虑 `clone`。大对象、集合和热路径上的 clone 要说明所有权为何必须独立；共享只读数据优先借用，跨线程共享再按需要使用 `Arc`。

- 反例：为消除一次借用错误，把整个 `Vec<Record>` 在循环中反复 clone。
- 正例：函数接收 `&[Record]`，或缩短可变借用范围后再访问其他字段。

## R3. unsafe 块要小，并写明 SAFETY 不变量

把 `unsafe` 收敛在最小边界，紧邻写 `// SAFETY:` 说明调用者保证、生命周期、对齐、别名和线程安全条件；对外暴露安全封装。新增 unsafe 后补覆盖边界条件的测试，能用安全 API 实现时优先使用安全 API。

- 反例：用一个大 `unsafe` 块包住整段业务逻辑，没有说明前置条件。
- 正例：只把必须的指针解引用放进小块，并在上一行写清调用点如何满足 SAFETY 不变量。

## R4. async 任务中不执行阻塞操作，不跨 await 持锁

文件重 I/O、阻塞系统调用和 CPU 重任务放到专用线程或 `spawn_blocking`；释放 mutex guard 后再 `.await`，避免阻塞执行器或形成死锁。启动的任务要有取消、超时和 join/回收路径。

- 反例：在 Tokio task 里直接跑重 CPU 工作，或持有 `MutexGuard` 时调用异步网络请求。
- 正例：阻塞工作交给 `spawn_blocking`；在 `.await` 前用小作用域释放 guard。

## R5. 并发边界明确 Send / Sync 与锁范围

共享可变状态用适合的同步原语保护，把临界区压到最小；不要为绕过编译错误随意添加 `unsafe impl Send/Sync`。处理 poisoned lock 或 channel 关闭时保留业务上下文，不用无条件 unwrap。

- 反例：给含非线程安全指针的类型直接写 `unsafe impl Send` 来通过编译。
- 正例：重新设计所有权边界；确需共享时使用满足约束的 `Arc` 与同步原语，并保持锁区最小。

## R6. 索引、转换和算术在边界处显式校验

外部输入转整数使用 `TryFrom` / `try_into`，可能溢出的运算使用 `checked_*` / `saturating_*` 并选择符合业务的语义；不可信索引用 `.get()`，避免直接 `[]` 导致 panic。

- 反例：把外部 `i64` 直接 `as usize`，随后用 `items[index]` 访问。
- 正例：`let index = usize::try_from(raw_index)?; let item = items.get(index).ok_or(...)?;`。

## R7. 提交前格式化、静态检查并运行测试

改动必须通过 `cargo fmt --check`、`cargo check --workspace` 和 `cargo test --workspace --lib --tests`（CI 只跑 check + test，fmt 需自己保证）；clippy 目前不强制，改动处不要新增 warning。涉及 feature、target 或 unsafe 的代码要覆盖对应组合，不能只验证默认 feature。

- 反例：只跑默认 feature 的 `cargo check`，却修改了可选 feature 或跨平台模块。
- 正例：动了 `transport/smb.rs` 时额外跑 `cargo check -p nvr --features smb`；本机无法验证的组合在交付时写明。

---

# JavaScript 专项

## S1. 一律 `===`，声明用 `const` / `let`

禁止 `==`（隐式类型转换）和 `var`（函数作用域 + 提升）。默认 `const`，需要重新赋值才用 `let`。

## S2. 不允许游离的 Promise

异步调用必须 `await`、`return`，或显式 `.catch(...)`。漏掉会变成 unhandled rejection——Node 18+ 默认直接让进程退出。

- 反例：`doAsync()` 单独一行
- 正例：`await doAsync()` / `void doAsync().catch(err => log.error(...))`

## S3. 循环里不串行 await（1+N 的 JS 版）

`for (const id of ids) { await fetchOne(id) }` 会把 N 次请求串起来。改用 `Promise.all(ids.map(...))`；量大时用并发上限池，别一次打爆下游。

## S4. 错误对象整体交给日志，不要只打 message

`console.error(err.message)` 丢掉堆栈和 cause。传整个 error 对象；跨层抛出时用 `new Error("上下文", { cause: err })` 保留链路。

## S5. 捕获要精确，不吞错

禁止空 `catch {}`。`try` 块只包住可能抛错的那几行，不要整个函数体裹一层。

## S6. 模块顶层不做副作用 I/O

顶层只做定义与导出；连接、读配置、发请求放进显式的初始化函数，避免 import 顺序决定运行结果。

## S7. 对外数据必须校验后再用

接口返回、`JSON.parse` 结果、用户输入在使用前先校验形状与必填字段，不要直接深层解构后透传到下游。

---

# TypeScript 专项

> 以下条目叠加在《JavaScript 专项》之上，JS 的条目同样适用。

## T1. `strict` 必开，禁止关掉 `strictNullChecks`

`tsconfig.json` 保持 `"strict": true`。新模块不得单独放宽严格性开关来绕过报错。

## T2. 禁 `any`，不确定用 `unknown` 再收窄

外部数据先声明为 `unknown`，用类型守卫 / schema 校验（zod 等）收窄后再使用。项目若开了 `no-explicit-any` lint 规则，不要用注释豁免。

## T3. 不用 `as` 强断言绕过类型系统

`as` 只在你比编译器多掌握信息、且写明理由时使用；禁止 `as any as T` 这种双重断言。需要运行时保证的地方写类型守卫函数（`x is T`）。

## T4. 用 `@ts-expect-error` 而非 `@ts-ignore`，且必须注明原因

`@ts-expect-error` 在错误消失后会自己报错提醒清理，`@ts-ignore` 会永远静默。两者都要跟一行说明「为什么必须忽略、何时可以删」。

## T5. 对外 API 显式标注返回类型

导出的函数、hook、service 方法写明返回类型，不依赖推断——推断结果会随实现改动悄悄变化，破坏调用方而编译不报错。

## T6. 用可辨识联合表达互斥状态

`{ status: "loading" } | { status: "ok"; data: T } | { status: "error"; error: Error }`，而不是一堆可选字段（`data?`、`error?`、`loading?`）互相组合出非法状态。

## T7. 类型与运行时校验在边界处对齐

接口响应、环境变量、localStorage 等外部输入，类型声明必须配一个运行时校验；只写 `interface` 不校验等于没有保证。
