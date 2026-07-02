# GB28181 P2 PTZ — manual smoke test

Prereqs: a real PTZ-capable GB28181 camera registered to nvr (P1-3 running with
`NVR_GB_ENABLE=1 …`), the device visible in the dashboard.

**⚠️ The PTZCmd byte layout is validated here, not by unit tests.** If a direction
moves the wrong way or nothing happens, fix the bit map in
`crates/gb28181/src/ptz.rs::encode_ptz_cmd` (and update its vectors).

1. **Directions:** open the 云台 dialog on the gb device row. Press-and-hold each
   of ↑ ↓ ← →: the camera moves in that direction while held and STOPS on release.
   Confirm each direction matches its button (no inversion/swap).
2. **Diagonals (API-only):** the 4-arrow pad emits single directions; diagonals
   (`up_left`/`up_right`/`down_left`/`down_right`) are a backend superset —
   exercise via `POST /api/gb/ptz` with `command:"up_left"` if the camera supports it.
3. **Zoom:** hold 放大 / 缩小 → the lens zooms in/out and stops on release.
4. **Speed:** raise the 速度 slider → the same direction moves faster.
5. **Presets:** point the camera somewhere, set 预置位 = 1, click 设置; move away;
   click 调用 → the camera returns to the saved position; click 删除 to remove it.
6. **Stop-on-close safety:** hold a direction (camera moving), then press **Esc**
   (or click outside) to close the dialog WITHOUT releasing → the camera must
   STOP (the dialog's `@hide` sends a stop). A camera that keeps moving after the
   dialog closes is a regression of the `@hide="ptzRelease"` net.
7. **Offline device:** stop the camera; a PTZ press returns an error (HTTP 500,
   "device offline") and the UI stays usable.
8. **GB disabled:** with `NVR_GB_ENABLE` unset, `POST /api/gb/ptz` returns the
   "GB support is not enabled" error (the 云台 button only shows for gb devices).
