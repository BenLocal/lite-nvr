# Documentation Index

Project docs live here; some stay at fixed paths because tooling expects them (Cargo, npm, Claude Code, GitHub).

## In this folder

- [agents-dot-md/](agents-dot-md/00-index.md) — architecture rules, tech stack, code checklist, per-subsystem design notes (GB28181, ONVIF, detection, recorder, auth, media pipe)
- [memory/](memory/00-index.md) — troubleshooting conclusions and known pitfalls
- [dashboard-rules.md](dashboard-rules.md) — frontend coding rules
- [rockchip.md](rockchip.md) — Rockchip arm64 GNU deployment, RK FFmpeg and test procedure

## Top-level

- [../README.md](../README.md) — project README
- [../AGENTS.md](../AGENTS.md) (= `CLAUDE.md`) — agent guidance, auto-loaded by Claude Code
- [../.agent/skills/](../.agent/skills/) — project skills (e.g. `build-nvr`)

## Per-crate / per-package

- [../crates/ffmpeg-bus/README.md](../crates/ffmpeg-bus/README.md) — media engine
- [../crates/nvr-asr/README.md](../crates/nvr-asr/README.md) — speech-to-text
- [../crates/nvr-detect/README.md](../crates/nvr-detect/README.md) — object detection
- [../examples/dummy-camera/README.md](../examples/dummy-camera/README.md) — GB28181 emulated camera
- [../examples/dummy-onvif-camera/README.md](../examples/dummy-onvif-camera/README.md) — ONVIF emulated camera
- [../nvr-dashboard/app/README.md](../nvr-dashboard/app/README.md) — dashboard package
- [../nvr-dashboard/app/AGENTS.md](../nvr-dashboard/app/AGENTS.md) (= `CLAUDE.md`) — dashboard agent guidance
