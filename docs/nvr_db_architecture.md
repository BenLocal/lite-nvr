# nvr-db 数据库访问层抽象库文档

`nvr-db` 是 Lite-NVR 项目的专有数据库操作和迁移（Migrations）层。它不包含业务的 API 暴露，单纯负责数据持久化访问对象的封装，以确保数据库和上层业务逻辑（如 `lite-nvr` 守护进程）之间有干净的解耦。

## 1. 工程具体功能

- **ORM 及驱动隔离**：负责与底层数据库的具体绑定和连接。整个后端的数据来源主要经由此处。
- **静态嵌入式自动化迁移 (Migrations)**：包含一个用 `rust_embed` 打包嵌入的 SQL 自动迁移系统。系统在第一次启动或二进制更新时，自动按版本号执行新增的 DB schema 变更，无需额外配置独立的外部迁移工具。
- **KV 核心配置存储**：提供 NVR 运行需要的系统级配置读取（Key-Value 存储封装）。
- **用户权限验证访问**：负责从 DB 提取用户信息，用作上层 API 路由的安全拦截鉴权。

## 2. 核心实现逻辑

- **基于 Turso (libsql/SQLite) 构建**：`nvr-db` 采用兼容 SQLite API 的 Turso rust 客户端 (`turso` crate) 作为连接池。应用初始化时默认开启 `journal_mode=wal` (Write-Ahead Logging) 以保障在 Lite-NVR 中的高并发读写性能。
- **自定义轻量迁移引擎 (Migrations)**:
  - 在 `migrations.rs` 中使用 `rust_embed` 宏在编译期将 `migrations/` 目录下的所有 `*.sql` 文件嵌入到最终的二进制执行档中。
  - 使用内置特殊的 `_migrations` 数据表记录各脚本的执行版本号。每次应用启动时比对哪些版本还没有 apply，并借由 SQLite Transaction (`tx`) 批量顺序自动执行建表或字段增删。
- **基础 Dao 操作模型 (`kv.rs`, `user.rs`)**:
  - `Kv` 模块：系统动态键值对查询 `SELECT id, module, key, sub_key, value FROM kvs WHERE ...`。
  - `User` 模块：实现基于用户凭证的鉴权。

## 3. 架构协同与交互调用

```mermaid
graph TD
    A[lite-nvr (Main App)] -->|1. 初始化时连接并应用迁移| B(nvr-db: migrations::migrate)
    A -->|2. HTTP 请求处理| C[路由 Handler]
    C -->|3. 查询登录鉴权| D(nvr-db: user::by_username)
    C -->|4. 读取录像机系统配置| E(nvr-db: kv::by_module_and_key)

    B --> F[(SQLite: nvr.db)]
    D --> F
    E --> F
    
    subgraph Compiled Binary ["编译好的 NVR 独立程序层"]
        B
        G[内置的 migrations/xxxx.sql 文件] -.->|rust_embed| B
    end
```

## 4. 后续修改指南与使用规范

### 新增功能表与数据库迁移步骤
如果后续需要给系统新增一张数据表（例如新增“录像报警记录”表），**不要**去改动旧的 `.sql` 文件，而是遵循下面的规范添加 Migrations：

1. 进入 `nvr-db/migrations/` 目录。
2. 按照格式 `[Version号]_[小写带下划线描述].sql` 新建文件。例如：`2_create_alarm_records.sql`。
3. 在脚本内用标准 SQLite 语法书写建表语句 `CREATE TABLE IF NOT EXISTS xxx (...)`。
4. 重新 `cargo build`。程序在下次被调起时就会自动扫描新的 Version 号并创建该表，保持向后兼容和升级的无缝衔接。

## 5. 日常用法与故障排查（对于存储）

由于直接使用了 SQLite 作为本地方案（配合 WAL），数据的可靠性较高。如果出现问题，通常发生在以下场景：

### (1) Sqlite DB Locked (数据库死锁)
- 如果程序报错 `database is locked`：
  - 排查：由于 `nvr-db` 启用了 `WAL` 模式，通常只在写操作互斥时发生。确认 `lite-nvr` 里的业务代码中是否有长时间运行且未提交（commit）的 `Transaction`，或者后台有外部脚本同时在使用 SQLite 命令写入 `nvr.db` 文件。

### (2) 查看现存放的 SQLite 数据信息
如果需要 Debug 发现业务逻辑状态不对：
你可以直接从 shell 进入项目主目录并利用官方 sqlite3 组件：
```bash
# 宿主机上安装 sqlite3 命令行工具后：
sqlite3 nvr.db

# 在交互界面中输入：
sqlite> .tables              # 看表结构是否全部迁移成功
sqlite> SELECT * FROM kvs;   # 查看系统所有的配置映射
sqlite> .exit
```

### (3) Database Migration Error (迁移执行失败)
- 错误表现：启动时程序崩掉并提示 `Error migrating database`。
- 排查：大概率是刚加入的 SQL 迁移脚本由于不严谨（比如建立了一个与旧版本冲突的外键或重复建表去掉了 `IF NOT EXISTS`）。检查 `nvr.db` 中的 `_migrations` 表所停留在的最后一个正确 version，然后修正对应的 `.sql`，删掉旧数据库或手动修复即可。
