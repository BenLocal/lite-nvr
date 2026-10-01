# docs/memory 记忆索引

> 本仓库的记忆区：**纯 Markdown、零外部服务、零 LLM key**，由 agent 自己维护。
> 何时记 / 怎么记 / 不记什么见 `AGENTS.md` 的《记忆记录（Memory）》一节。本文件由 agents-dot-md skill 的 `reindex.py` 生成，勿手工编辑。

- [目标检测踩坑](detection.md) — 检测模型加载、models.json、tap 生命周期与 lease、auto-start 相关的非显然问题；设计见 `docs/agents-dot-md/design-detect.md`。
- [设备协议踩坑（GB28181 / ONVIF）](device-protocols.md) — GB28181 拉流、注册、PTZ 与 ONVIF 鉴权、profile 相关的非显然问题；设计见 `design-gb28181.md`、`design-onvif.md`。
- [媒体管线与录像踩坑](media-and-recording.md) — media-pipe / ZLM sink、ffmpeg-bus 输出、nvr-recorder 重连，以及默认账号相关的非显然问题；设计见 `design-media-pipe.md`、`design-recorder.md`、`design-auth.md`。
