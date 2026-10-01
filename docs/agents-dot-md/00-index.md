# docs/agents-dot-md 模块索引

> AGENTS.md 的细化模块目录（不会被 Claude Code 自动加载，按需查阅）。本文件由 agents-dot-md skill 的 `reindex.py` 生成，勿手工编辑。

- [系统架构](architecture.md) — 依赖方向硬规则、启动与关停顺序的约束、关键设计决策；动结构、加模块、接外部依赖前先看本文。
- [代码 checklist（强制自检）](code-checklist.md) — 写完 / 改完代码必须逐条对照的硬性自检清单；新增条目直接追加本文件。
- [编码行为准则（Karpathy 防错指南 · 展开）](coding-guidelines.md) — 写 / 改 / 审代码的可执行细则；AGENTS.md 顶层只留四条高压线，细则在此。
- [鉴权设计要点](design-auth.md) — 会话 token 的存储、过期、吊销规则，`?token=` 的取舍，以及哪些入口刻意不鉴权；改登录、用户管理或新增绕开 `/api` 的入口前先看本文。
- [目标检测设计要点](design-detect.md) — 实时多模型 YOLO 检测（`crates/nvr-detect` + `nvr/src/detect`）与按设备检测配置（device-config Phase 1）的范围、取舍、清单与环境变量语义、API/UI 契约及端到端验证步骤；启动顺序、auto-start 重试、大栈线程、清单缺失不阻塞启动见 `architecture.md`。
- [GB28181 设计要点](design-gb28181.md) — NVR 作为 GB28181 上级平台（SIP UAS）：`crates/gb28181` 只管信令，`nvr/src/gb` 做按需拉流桥，媒体全交给 ZLM `RtpServer`；目前只做了实时点播和 PTZ，平台侧回放没有做；本地测试用 `make dummy`。
- [媒体管线拆分设计要点](design-media-pipe.md) — `media-pipe-core` / `media-pipe-zlm` 为什么这样拆、边界在哪、调用方必须遵守的约束；改管线输出或接新媒体服务器前先看本文。
- [ONVIF 设计要点](design-onvif.md) — NVR 只做 ONVIF 客户端：负责发现摄像头、解析 RTSP URI、下发 PTZ，媒体照旧走 RTSP→ZLM 管道；本地测试用 dummy-onvif-camera 模拟 ONVIF 服务。
- [录像设计要点](design-recorder.md) — 线上录像链路（ZLM HLS 分片归档）与独立的 `nvr-recorder` crate 各自的分段、元数据、保留、外送语义和调用方约束；动录像、回放、清理或外送前先看本文。
- [开发环境](environment.md) — 本地依赖安装、代理与凭据存放约定；变量清单见 `tech-stack.md`。
- [技术栈与实现规范](tech-stack.md) — 选型约定、构建期环境变量、构建与验证命令；写代码前先看本文，按既有方式扩展。
- [交流与语言约定](translation.md) — 默认简体中文交流；代码注释、标识符、提交信息沿用仓库既有英文风格。
