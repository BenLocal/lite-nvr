# 目标检测踩坑

> 检测模型加载、models.json、tap 生命周期与 lease、auto-start 相关的非显然问题；设计见 `docs/agents-dot-md/design-detect.md`。

- 2026-07-21：单测和启动都正常，`POST /api/detect/{pipe}/start` 却在运行时加载模型失败 → usls 用 `ort-load-dynamic`（避免与 nvr-asr 的 sherpa-onnx ORT 符号冲突），运行时要自己找 `libonnxruntime.so`，版本须对应 `ort =2.0.0-rc.10`（ORT 1.22.0）→ 设 `ORT_DYLIB_PATH` 或把 `.so` 目录加进 `LD_LIBRARY_PATH`；升级 usls/ort 时同步改 `scripts/detect_e2e.sh` 的 `ORT_VER`（证据：crates/nvr-detect/Cargo.toml、scripts/detect_e2e.sh:25）
- 2026-07-21：models.json 某项不写 `version`，start 报 "No clear YOLO Version specified" 且所有模型一起失败 → usls 靠 version 选解码头；hub 首次 start 一次性构建全部模型，一个失败整批报错 → 每项都写 `version`；清单只在启动时读一次，改完要重启（证据：crates/nvr-detect/src/usls_backend.rs:44-50、nvr/src/detect/hub.rs:118-148）
- 2026-07-23：`cargo test -p nvr detect::tap_test` 显示 0 个用例，看着像全绿 → 同目录 `*_test.rs` 注册路径是 `detect::tap::tap_test::…` → 过滤用父模块路径，如 `detect::tap`（证据：nvr/src/detect/tap.rs 末尾）
- 2026-07-23：tap 因 EOF / 设备断开自行结束后，再 start 一直返回 "already running" → 登记槽没释放；不分代清理又会让退出中的旧 tap 删掉新 tap 的登记 → tap 退出时按 `TapEpoch` 调 `unregister_tap`，给 `tap::run` 加参数时保留 epoch（证据：commit 4b15a3b、nvr/src/detect/hub.rs:256-266）
- 2026-08-10：关掉预览把配置驱动的检测也停了，或旧预览停掉了替换后的新 tap → 不带 lease 的 stop 是无条件的 → 只有 start 返回 `started` 且带 `x-detection-tap-lease` 时才拥有该 tap，stop 要带 lease；`already running` 或已持久化启用时只轮询，不调 stop（证据：nvr/src/detect/api.rs:84-101、DetectionOverlay.vue、commit 3e162ab）
- 2026-07-23：auto-start 重试期间有人手动 start 且失败（管线未就绪），之后该设备不再自动启动 → `start_tap(…, None)` 先 `cancel_auto_start` 再检查管线 → 修改或重存一次设备配置触发重新 reconcile（证据：nvr/src/detect/control.rs:100-102）
