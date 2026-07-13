# Auth: real sessions, API-wide enforcement, user management

Date: 2026-07-13
Status: approved (playback streams: header + query token, fully protected;
user management: all logged-in users, no roles)

## Problem

Login verifies the password (argon2) but issues a throwaway UUID token that is
never stored or checked. No route enforces auth — every `/api/*` endpoint is
callable without credentials. `logout` is a no-op, `/user/info` returns a
placeholder string, and there is no way to change a password or manage users.

## Design

### Session store (nvr-db)

- New `nvr-db/src/session.rs`, KV-backed like `user.rs`: `module="session"`,
  `key=<token>`, `sub_key=<username>` (enables delete-by-user in SQL),
  `value=JSON { username, expires_at }`.
- TTL: 30 days, fixed (no sliding renewal — avoids a DB write per request).
- Ops: `insert`, `get_by_token`, `delete`, `delete_by_username(except)`,
  `delete_expired`.
- `user.rs` gains `update`, `delete`, `list`, and owns `hash_password` /
  `verify_password` (moved from `migrations.rs` / `nvr/handler/user.rs`).

### Auth layer (nvr/src/auth.rs)

- Process-wide cache `RwLock<HashMap<token, {username, expires_at}>>` in front
  of the DB (turso opens a connection per request — see `nvr/src/db.rs` —
  so per-request DB validation would be wasteful). Cache miss falls back to
  the DB, so sessions survive restarts.
- `create_session`, `validate`, `revoke`, `revoke_user(username, except)`.
- Axum middleware (`middleware::from_fn`) layered on the `/api` router
  (nested routers see the stripped path). Exempt: `/user/login` only.
- Token sources, in order: `Authorization: Bearer <t>` header, then `?token=`
  query param (for hls.js / Safari-native playback where headers can't be
  attached reliably).
- Failure → 401 with the standard `BaseResponse { code: 401 }` body.
- Success → inserts `AuthUser { username, token }` into request extensions.

### Handlers (nvr/src/handler/user.rs)

- `login` → verify, create session, return `{ token, username }`; also
  garbage-collects expired sessions.
- `logout` → revoke the current token.
- `info` → `{ username }` from `AuthUser`.
- New `POST /user/password` `{ old_password, new_password }` → verify old,
  re-hash, revoke the user's *other* sessions (current one stays).
- New `GET /user/list` → `[{ username, create_time, update_time }]` (no hashes).
- New `POST /user/add` `{ username, password }` → reject empty / duplicate.
- New `POST /user/remove/{username}` → reject self-removal; deletes the user
  and revokes all their sessions.
- No roles: any logged-in user may manage users (lite scope, approved).

### Playback token propagation (nvr/src/handler/playback.rs)

`segment_playlist` and `playback_playlist` accept `?token=` and append it to
the segment URIs they emit, so Safari-native `<video>` (which cannot set
headers) keeps working end-to-end. The token has already passed the middleware
by the time these handlers run, so it is a known-good stored token.

### Frontend

- `request.ts`: HTTP 401 → clear token, redirect to `${BASE_URL}login`.
- `api/user.ts`: `logout`, `getUserInfo`, `changePassword`, `listUsers`,
  `addUser`, `removeUser`.
- `api/playback.ts` URL builders append `?token=` (from `getAuthToken`).
- `PlaybackView`: `new Hls({ xhrSetup })` sets the Bearer header as well.
- `AppLayout`: the logout confirm now calls `POST /user/logout` before
  clearing local state.
- `SettingsView`: new「账户与安全」section — change-password form + user
  management table (list / add / remove, self-removal disabled).

### Out of scope (deliberate)

- `/media` proxy and ZLM's own ports (8553/8554/8555) — live streams are
  outside `/api`; protecting them would break live preview and needs its own
  design (ZLM hook-based auth).
- `/asr` Socket.IO namespace.
- Roles / permissions.

## Testing

- `nvr-db/src/session_test.rs`: insert/get/expiry/delete/delete-by-user.
- `nvr-db/src/user_test.rs`: update/delete/list + hash/verify round-trip.
- `nvr/src/auth_test.rs`: middleware 401 without token, pass with header and
  with query token, login exemption, expiry.
- Frontend: `npm run type-check` + `npm run lint:eslint`.
