# 鉴权设计要点

> 会话 token 的存储、过期、吊销规则，`?token=` 的取舍，以及哪些入口刻意不鉴权；改登录、用户管理或新增绕开 `/api` 的入口前先看本文。

原设计稿：2026-07-13「Auth: real sessions, API-wide enforcement, user management」（已落地）。实现位置：`nvr/src/auth.rs`、`nvr-db/src/session.rs`、`nvr-db/src/user.rs`、`nvr/src/handler/user.rs`，前端 `src/auth/token.ts`、`src/api/request.ts`。

## 一、范围

- 已做：登录签发真实会话；`/api` 下所有接口统一鉴权；登出、改密码、用户增删查。
- **刻意不做**（原因写在这里，别当成遗漏去“修”）：
  - `/media` 代理和 ZLM 自己的端口（8553/8554/8555）不鉴权。直播流不走 `/api`，强行加鉴权会弄坏直播预览；要做得另起设计（基于 ZLM hook 的鉴权）。
  - `/asr` 的 Socket.IO namespace 不鉴权（ASR 的 HTTP 控制接口在 `/api` 下，照常鉴权）。
  - 不做角色和权限：任何已登录用户都能管理用户。这是 lite 版的范围，已确认。

## 二、token 与会话

- token 是 `uuid v4`，**以明文作为 KV 主键**存储：`module="session"`、`key=<token>`、`sub_key=<username>`（这样按用户批量删除只需一条 SQL）、`value=JSON{username, expires_at}`。拿到数据库就等于拿到所有有效会话，因此数据库文件要按机密处理。
- **TTL 固定 30 天，不滑动续期**。原因：续期意味着每个请求都要写一次数据库。
- **进程内缓存放在数据库前面**：`app_db_conn` 每次调用都会新开一个 turso 连接，如果每个请求都查库就太浪费。缓存未命中时回落到数据库查询，所以重启后会话依然有效。
  - 由此带来的约束：吊销会话**必须走 `auth::revoke` / `revoke_user`**（同时清缓存和数据库）。直接删数据库行，在进程重启之前对已缓存的 token 不起作用。
- 过期清理：`validate` 遇到过期 token 时顺手删掉；每次登录时尽力执行一次 `delete_expired`，失败也不影响登录。没有专门的后台 GC。
- 前端：勾选“记住我”时 token 存 `localStorage`，否则存 `sessionStorage`。`request.ts` 收到 HTTP 401 时清掉本地 token 并跳转到 `${BASE_URL}login`。

## 三、token 来源与 `?token=`

- 中间件挂在 `/api` 路由上，看到的是 nest 去掉前缀之后的路径；豁免路径只有 `/user/login`。失败时返回 HTTP 401，body 为 `BaseResponse{code:401}`；成功时往 extensions 里放入 `AuthUser{username, token}`。
- token 的读取顺序：先读 `Authorization: Bearer`，再读 `?token=`。之所以保留 query 参数，是因为 hls.js / Safari 原生 `<video>` 没法可靠地带上请求头。
- 回放的 m3u8（`segment_playlist` / `playback_playlist`）会把收到的 `?token=` **原样拼到每个分片 URI 后面**，这样不能带头的播放器也能一路播下去。能这样回显，是因为请求进入 handler 时已经通过中间件校验，这是一个已知有效的 token。
- 已接受的风险：query 里的 token 会出现在 URL 中（浏览器历史、代理日志、播放列表内容里）。目前 nvr 自身不记录请求 URI。新增日志或 tracing 时不要把 query 串打出来。
- 前端的回放 URL 构造函数会拼上 `?token=`（已做 `encodeURIComponent`），hls.js 另外通过 `xhrSetup` 再带一份 Bearer 头。

## 四、口令与吊销规则

- 口令使用 argon2（`Argon2::default()` 加随机盐），哈希与校验集中在 `nvr_db::user`。登录时，用户不存在和密码错误返回**同一条**错误信息。
- 改密码：必须先校验旧密码；改完后吊销该用户的**其他**会话，当前会话保留。
- 删用户：禁止删除自己（因此至少会留下一个能登录的用户）；删除用户的同时吊销其全部会话。
- 新增用户：用户名和密码都不能为空，不能与已有用户重名。
- **默认账号**：每次启动都会执行 `ensure_default_admin_user`，只要 `admin` 用户不存在就重新创建，密码为 `admin`。所以“删掉 admin”**不等于**禁用默认口令，下次重启它会以 admin/admin 回来。要收紧就保留 `admin` 并修改其密码。
