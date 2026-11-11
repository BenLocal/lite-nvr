# 环境变量配置

本文按 2026-11-11 的代码整理项目直接读取的环境变量、启动脚本配置和常用 Makefile 参数。默认值指未设置变量时的代码行为；运行平台、启动脚本或已有环境可能提供其他值。不包含 Cargo、Node.js 和第三方库的全部通用环境变量。

## 如何配置

### 源码运行

仓库根目录的 `.env` 由 Makefile 和 `scripts/build.sh` 加载。可在本机 `.env` 中配置：

```dotenv
NVR_RECORD_DIR=/mnt/data/nvr/records
NVR_ZLM_RTSP_PORT=8556
NVR_ZLM_RTMP_PORT=8557
RUST_LOG=info
```

然后运行 `make run`。Makefile 命令行参数可以覆盖文件中的同名 Make 变量，例如 `make run NVR_RECORD_DIR=/mnt/archive`。不要假定 shell 中的值一定覆盖 `.env`；Makefile 的文件赋值及构建脚本的 `source` 都可能覆盖已有值。

直接执行 `cargo run` 或二进制不会自动读取根目录 `.env`，需显式导出：

```bash
set -a
. ./.env
set +a
make run
```

`.env` 内容应同时兼容 Makefile 赋值和 shell 赋值。简单路径直接写 `KEY=value`；复杂引号、包含空格的值建议通过 shell 单独导出，并避免在 `.env` 中重复定义。账号、密码和 token 只写入未入库的本地文件。

### 安装包运行

可以在安装目录执行：

```bash
NVR_RECORD_DIR=/mnt/data/nvr/records ./start.sh
```

当前 [通用启动脚本](../packaging/common/start.sh) 还支持安装目录下的 `nvr.env`。已有进程环境优先，文件只为未设置的变量补默认值：

```dotenv
# 与 start.sh 放在同一目录
NVR_RECORD_DIR=/mnt/data/nvr/records
NVR_ZLM_RTSP_PORT=8556
NVR_ZLM_RTMP_PORT=8557
```

`nvr.env` 按行读取 `KEY=VALUE`，值原样使用。不要写 `export`、引号、行尾注释、`$HOME` 或命令替换；路径直接写绝对路径。旧安装包若没有读取 `nvr.env` 的逻辑，需继续显式传入变量。

### Docker 运行

环境变量中的目录指容器内路径，另用挂载对应到宿主机。例如在现有 `docker run` 参数中增加：

```bash
-e NVR_RECORD_DIR=/records \
--mount type=bind,src=/mnt/data/nvr/records,dst=/records
```

先创建宿主机目录并确认写权限。仅设置变量不会自动建立宿主机挂载。修改容器环境需要重新创建容器，普通 `docker restart` 不会更新其创建时的环境。

运行参数通常在启动或模型首次加载时读取、缓存。修改后应重启 NVR；通过 `./start.sh restart` 重启时也要传入所需变量，或保存在该版本支持的 `nvr.env` 中。

## 录像、端口与媒体

来源：[NVR 配置](../nvr/src/config.rs)、[ZLM 服务](../nvr/src/zlm/server.rs)、[编码队列](../crates/ffmpeg-bus/src/encoder.rs)、[硬解选择](../crates/ffmpeg-bus/src/hw.rs)、[DMABUF 采集](../crates/ffmpeg-bus/src/v4l2_dmabuf.rs)。

| 变量 | 默认值 | 作用和取值 |
| --- | --- | --- |
| `NVR_RECORD_DIR` | `<当前工作目录>/data/records` | 录像归档根目录。去除首尾空白，空值按未设置处理；建议使用绝对路径。目录自动创建，运行用户需有写权限 |
| `NVR_ZLM_RTSP_PORT` | `8554` | ZLM RTSP TCP 端口，范围 `1–65535`；非法值导致初始化失败 |
| `NVR_ZLM_RTMP_PORT` | `8555` | ZLM RTMP TCP 端口，范围 `1–65535`；非法值导致初始化失败 |
| `RUST_LOG` | Makefile、构建脚本和启动脚本提供 `info` | Rust 日志过滤，例如 `info,ffmpeg_bus=debug` |
| `FFMPEG_BUS_ENCODER_QUEUE_FRAMES` | `8` | 每个编码器前的帧队列容量，正整数；空值或非法值使用默认值。增大会增加内存占用 |
| `FFMPEG_BUS_DISABLE_HWDEC` | 未设置，允许硬解 | **只要变量存在就禁用硬解**，包括空值和 `0`；恢复硬解需 `unset`。不控制硬编码 |
| `NVR_V4L2_DMABUF` | 启用 | Rockchip V4L2 DMABUF 采集开关；仅去除首尾空白后的 `0` 关闭，`false` 不会关闭。设备不支持时回退普通 FFmpeg 采集 |
| `DISPLAY` | 继承系统环境 | X11 屏幕采集的显示地址，例如 `:0`、`:99`；还需对应显示服务可访问 |
| `YT_DLP_BIN` | `yt-dlp`，由 `PATH` 查找 | 直播平台页面解析使用的外部程序路径。离线包不附带该程序；不使用此功能时不需要设置 |

录像归档路径为 `<NVR_RECORD_DIR>/<stream>/<file_name>`，其中 `stream` 通常是设备 ID。修改目录只影响后续归档，不自动迁移历史文件或修改已有数据库记录。此变量控制归档副本，不控制 ZLM 产生原始 HLS 分片的位置，详见 [录像设计](agents-dot-md/design-recorder.md)。

API `18080`、ZLM HTTP `8553` 和数据库路径 `nvr.db` 当前没有对应的运行环境变量；不要使用尚未实现的 `NVR_API_PORT`、`NVR_DB_URL` 等名字。设备的输入、编码、录像开关和保留策略通过 API、后台页面及数据库配置。

## ASR、检测与语音合成

来源：[ASR 配置](../nvr/src/asr/mod.rs)、[ASR 默认值](../crates/nvr-asr/src/lib.rs)、[检测配置](../nvr/src/detect/mod.rs)、[Chat TTS](../nvr/src/chat/tts.rs)。

| 变量 | 默认值 | 作用和取值 |
| --- | --- | --- |
| `ASR_MODELS_DIR` | `third_party/asr-models` | ASR 模型根目录，包含 Silero VAD、SenseVoice 模型子目录及可选的标点模型子目录 |
| `ASR_MAX_SPEECH_SECS` | `10.0` 秒 | VAD 连续语音段的强制分段上限，正浮点数；非法值记录警告并保留默认值 |
| `ASR_EARLY_COMMIT` | `true` | 说话未结束时，提前确认稳定句子。`1`、`true`、`on` 开启；`0`、`false`、`off` 关闭；区分大小写 |
| `ASR_PARTIAL_PUNCT` | `strip` | 配置了标点模型时，中间结果的标点策略。`strip` 或 `none` 去标点；`recognizer` 保留识别器自带标点。不区分大小写。未配置标点模型时保留识别器原文 |
| `DETECT_MODELS_DIR` | `third_party/detect-models` | 检测模型及 `models.json` 所在目录；清单不存在时不阻止 NVR 启动 |
| `TTS_MODELS_DIR` | `third_party/tts-models` | Chat 语音合成模型根目录，依次尝试 `MeloTTS-RKNN2`、`vits-melo-tts-zh_en` |
| `NVR_TTS_RKNN_LIB` | `librknnrt.so` | MeloTTS RKNN 后端运行库，可以是完整路径或动态库搜索路径中的名字 |
| `NVR_CHAT_SYSTEM_PROMPT` | 内置中文语音助手提示词 | Chat 系统提示词，默认要求简短、口语化、适合朗读，避免 Markdown、列表、代码和表情 |

这些目录中的相对路径基于 NVR 工作目录解析。安装包启动脚本会切换到安装目录。模型文件仍需另行准备；设置目录不会下载模型。ASR 的其他库参数和命令行选项不自动对应环境变量，详见 [ASR 使用说明](asr.md)。

## Rockchip 板端 LLM

来源：[LLM 环境解析](../nvr/src/llm/config.rs)、[RKLLM 默认值](../crates/nvr-rkllm/src/lib.rs)。需要编译 `rockchip` feature 及兼容的模型、运行库和板端驱动。

| 变量 | 默认值 | 作用和取值 |
| --- | --- | --- |
| `NVR_LLM_MODEL` | 未设置，关闭 LLM | `.rkllm` 模型路径；空字符串也视为关闭 |
| `NVR_LLM_LIB` | `librkllmrt.so` | RKLLM 运行库路径或动态库名，须与模型转换版本兼容 |
| `NVR_LLM_TEMPLATE` | `qwen` | `qwen` 或 `deepseek-r1`，须与模型匹配 |
| `NVR_LLM_MAX_CONTEXT` | `4096` | 上下文长度，解析为整数；实际可用长度还受模型和运行库限制 |
| `NVR_LLM_MAX_NEW_TOKENS` | `512` | 单次生成 token 上限，请求可进一步降低该上限 |
| `NVR_LLM_STOP_TOKENS` | `151643,151645` | 逗号分隔的整数 token ID；必须与模型 tokenizer 匹配 |
| `NVR_LLM_IGNORE_EOS` | `false` | 仅 `1`、`true` 开启，其他非空值关闭。开启后依赖停止 token 和生成上限结束 |

这一组变量先去除首尾空白，空值按未设置处理。模型未配置时不会解析其他 LLM 参数；模型已配置而整数或模板解析失败时会报错。`RKLLM_LOG_LEVEL` 属于底层 RKLLM 运行库的变量，Rust 层未解析它，也没有在这里为它规定默认值。

## GB28181 上级平台

来源：[GB 配置](../nvr/src/gb/config.rs)。

| 变量 | 默认值 | 作用和取值 |
| --- | --- | --- |
| `NVR_GB_ENABLE` | 关闭 | 必须等于 `1` 才开启 |
| `NVR_GB_SIP_ID` | 无 | 开启时必填，平台 SIP 编码 |
| `NVR_GB_DOMAIN` | 无 | 开启时必填，SIP 域和摘要认证 realm |
| `NVR_GB_PORT` | `5060` | SIP UDP 监听端口；解析失败时使用默认值 |
| `NVR_GB_PASSWORD` | 无 | 共享注册密码；未设置或空值时使用开放认证 |
| `NVR_GB_MEDIA_IP` | `127.0.0.1` | 在 SDP 中告知设备的 RTP 接收地址，跨主机使用时填设备可达的本机 IP |

即使设置了 `NVR_GB_ENABLE=1`，缺少非空的 SIP ID 或域时仍不会启用平台。其他设备协议的账号和连接参数主要由设备配置管理。

## 原生运行库与构建依赖

| 变量 | 默认或发现方式 | 使用范围 |
| --- | --- | --- |
| `FFMPEG_DIR` | Makefile 默认仓库 `ffmpeg/`；构建脚本优先本地 SDK，再回退系统 pkg-config | FFmpeg SDK 根目录；部分运行时工具查找也会使用其 `bin/` |
| `ZLM_DIR` | 构建脚本优先仓库 `zlm/`；否则由 `rszlm-sys` 下载 | ZLMediaKit SDK 根目录 |
| `SHERPA_ONNX_LIB_DIR` | Makefile、构建脚本查找本机预编译库；未找到时由依赖处理下载 | sherpa-onnx 原生库的 `lib/` 目录，手动设置时不能是空值或不存在的路径 |
| `LD_LIBRARY_PATH` | 启动脚本在原有值前加入包内 `lib/`、`ffmpeg/lib/` | Linux 动态库搜索路径；源码运行还需考虑 ZLM 库目录 |
| `ORT_DYLIB_PATH` | 安装包启动脚本默认 `<安装目录>/lib/libonnxruntime.so` | ONNX Runtime 动态库文件路径，其他运行方式按本机实际库设置 |
| `ALSA_CONFIG_PATH` | 包内存在 ALSA 配置时默认 `<安装目录>/share/alsa/alsa.conf` | ALSA 主配置文件；已有非空变量优先 |
| `ALSA_CONFIG_DIR` | 包内存在 ALSA 配置时默认 `<安装目录>/share/alsa` | ALSA 配置目录；已有非空变量优先 |
| `ZLM_RELEASE_URL` | 见 [Cargo 配置](../.cargo/config.toml) | `rszlm-sys` 预编译下载地址；使用本地 `ZLM_DIR` 时通常无需修改 |
| `ZLM_BUILD_FROM_SOURCE` | 未设置 | 当前 `rszlm-sys` 支持用 `1`、`true`、`ON` 强制源码构建；已有本地 SDK 时优先核对依赖的选择逻辑 |
| `ZLM_GIT_ZONE`、`ZLM_GIT` | 依赖默认仓库 | 源码构建的镜像选择及 Git 仓库覆盖项；`ZLM_GIT_ZONE=gitee` 选择 Gitee 镜像 |
| `ZLM_BRANCH` | 按 `rszlm-sys` feature 选择 | 依赖选择的 ZLM 分支；自定义完整 release URL 时不要假定修改分支能切换该 URL 的资产 |
| `HTTP_PROXY`、`HTTPS_PROXY` | 未设置 | 下载和构建代理；依赖使用的下载客户端是否读取代理以其实现为准 |
| `http_proxy`、`https_proxy` | Makefile、构建脚本可从大写变量补齐 | curl、wget 等命令的代理配置 |
| `NO_PROXY`、`no_proxy` | 继承环境 | 代理绕过列表；由具体 HTTP 客户端解释 |
| `CARGO_HTTP_PROXY` | 未设置 | Cargo 下载代理。`.env.example` 中其他候选代理名不能保证被 Cargo 读取 |

构建代理使用 HTTP 代理地址；部分依赖的 ureq 构建不包含 SOCKS 支持。`ORT_LIB_LOCATION`、`ORT_PREFER_DYNAMIC_LINK` 等是第三方构建参数，不是 NVR 运行配置；需要手动调整时核对当前依赖版本的构建脚本。

## 编译与交叉打包参数

以下多数是 Makefile 参数，推荐使用 `make 目标 NAME=value`，不要把它们当作已编译 NVR 的运行配置。来源：[Makefile](../Makefile)、[构建脚本](../scripts/build.sh)、[打包脚本](../scripts/package-linux.sh)。

| 参数 | 默认值 | 作用 |
| --- | --- | --- |
| `BUILD_MODE` | `build` | `make build` 的模式，支持 `check`、`build`、`release` |
| `BUILD_ARGS` | 空 | 传给 Cargo 的参数，例如 `-p nvr --features rockchip` |
| `BUILD_JOBS` | CPU 数的一半，至少 `1` | 本机构建并发数 |
| `BUILD_NICE` | `10` | 本机构建和打包进程的 nice 优先级 |
| `FORCE_REBUILD` | 未设置 | 前端构建脚本执行时，只要该变量存在，包括 `0`，就要求重建前端；并非运行时开关 |
| `CARGO_TARGET_DIR` | Cargo 默认 `target/` | 本机 Cargo 产物目录；打包脚本另为各架构指定 `target/package/<arch>` |
| `CARGO_HOME` | `$HOME/.cargo` | Rust 工具及缓存位置；打包脚本从其 `bin/` 查找工具 |
| `CARGO_BUILD_JOBS`、`CMAKE_BUILD_PARALLEL_LEVEL` | Makefile 根据 `BUILD_JOBS` 或 `PACKAGE_JOBS` 赋值 | Cargo、CMake 并发控制；直接运行打包脚本时 Cargo 并发默认 `2` |
| `PACKAGE_ARCHS` | `amd64 arm64 rockchip` | 需要打包的 Linux 架构列表 |
| `PACKAGE_ARCHIVE` | `0` | `1` 时生成或复用 `.tar.gz` 和校验和 |
| `PACKAGE_OFFLINE` | `0` | `1` 时收集离线启动所需依赖，仍不含模型和 yt-dlp |
| `PACKAGE_JOBS` | `2` | 打包构建的 Cargo 和 CMake 并发数 |
| `PACKAGE_CONTAINER_OPTS` | `--cpus=2 --memory=4g --memory-swap=4g` | Makefile 传给 Cross 构建容器的资源参数 |
| `CROSS_CONTAINER_OPTS` | 直接运行打包脚本时提供 `--cpus=2 --memory=4g` | Cross 容器附加参数；`make package` 会用 `PACKAGE_CONTAINER_OPTS` 赋值 |
| `APT_MIRROR` | 未设置 | 构建容器的 Ubuntu apt 镜像地址，要求 `http://` 或 `https://` |
| `CROSS_TARGET_X86_64_UNKNOWN_LINUX_GNU_IMAGE` | 匹配宿主架构的项目构建镜像 | amd64 目标的 Cross 镜像，空值回退默认 |
| `CROSS_TARGET_AARCH64_UNKNOWN_LINUX_GNU_IMAGE` | 匹配宿主架构的项目构建镜像 | arm64、Rockchip 目标的 Cross 镜像，空值回退默认 |
| `RK_FFMPEG_URL` | 脚本内固定的 FFmpeg 8.1 shared RK release 资产地址 | 覆盖 Rockchip FFmpeg SDK 下载地址 |
| `RK_FFMPEG_SHA256` | 从 release 的 `checksums.sha256` 获取 | 指定 Rockchip FFmpeg 归档的 SHA-256 |
| `FFMPEG_URL` | 由安装脚本根据平台选择 | `make install-deps` 的 FFmpeg 下载地址覆盖项 |
| `ASR_LIBS_ARGS`、`ASR_MODELS_ARGS` | 空 | 分别传给 ASR 原生库、模型下载脚本的命令行参数 |
| `PACKAGE_TEST_IMAGE` | `ubuntu:22.04` | 离线安装包验证脚本的基础镜像 |

`CC_*`、`CXX_*`、`CARGO_TARGET_*_LINKER`、`CARGO_TARGET_*_RUSTFLAGS`、`PKG_CONFIG_ALLOW_CROSS` 和 `BINDGEN_EXTRA_CLANG_ARGS` 由打包脚本为目标架构设置，常规打包不需要手动配置。GNU 编译工具链和目标 glibc 兼容性见 [Rockchip 部署](rockchip.md) 和 [离线发布](releases.md)。

### 模型下载脚本

| 脚本 | 环境变量 | 默认和说明 |
| --- | --- | --- |
| `scripts/download_asr_models.sh` | `DEST` | 仓库 `third_party/asr-models` |
| 同上 | `SENSE_VOICE_MODEL` | `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`，下载的 release 模型名称 |
| 同上 | `PUNCT_MODEL` | `sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12` |
| 同上 | `ASR_MODELS_BASE_URL`、`PUNCT_MODELS_BASE_URL` | 分别为脚本中的 GitHub `asr-models`、`punctuation-models` release 下载根地址 |
| `scripts/download_sherpa_onnx_libs.sh` | `DEST` | 仓库 `third_party/sherpa-onnx` |
| 同上 | `VERSION` | 优先读取 Cargo.lock 中的版本，否则 `1.13.3` |
| 同上 | `LINK` | `static`；也支持脚本列出的 `shared` 组合，仍须匹配 Cargo feature |
| 同上 | `OS`、`ARCH` | 自动检测宿主；可用脚本参数指定目标，支持组合见脚本帮助 |
| `scripts/download_tts_models.sh` | `DEST` | 仓库 `third_party/tts-models` |
| 同上 | `HF_ENDPOINT` | `https://huggingface.co`，模型下载站点 |

`DEST` 是各下载脚本自己的输出目录，不能代替服务运行时的 `ASR_MODELS_DIR`、`TTS_MODELS_DIR`。TTS 下载的 `--int8`、`--melo-rknn` 是命令行选项；脚本内部的 `INT8`、`MELO_RKNN` 不是环境配置入口。

### 前端开发和 ASR 演示

| 变量或 Make 参数 | 默认值 | 用途 |
| --- | --- | --- |
| `NVR_API_TARGET` | `http://localhost:18080` | Vite 开发服务器的 `/api`、`/socket.io`、`/media` 代理目标；可在前端目录的 `.env` 或进程环境中设置，进程环境优先，不影响生产后台 |
| `VITE_BASE_URL` | `/nvr/` | 前端构建部署前缀；当前 Vite 配置直接读取进程环境，推荐 `VITE_BASE_URL=/nvr/ npm run build`；修改后需重新构建内嵌后台 |
| `ASR_SENSEVOICE`、`ASR_PUNCT_DIR` | 模型目录下匹配到的第一个子目录 | `make asr-demo` 自动寻找模型目录 |
| `ASR_MODEL`、`ASR_TOKENS`、`ASR_VAD`、`ASR_PUNCT` | 自动发现的对应模型、词表和 VAD 路径 | 仅覆盖 `make asr-demo` 的文件参数；`ASR_PUNCT=` 关闭演示的标点模型 |
| `ASR_WAV` | SenseVoice 模型自带 `test_wavs/zh.wav` | `make asr-demo` 的输入音频 |
| `ASR_ARGS` | `--realtime` | 演示程序附加参数；离线演示可用 `ASR_ARGS="--mode offline"` |

## 集成测试与模拟设备

这些变量不改变正常运行的 NVR。需要模型或真实设备的测试，部分默认忽略并要求 `--ignored`，部分在变量缺失时直接返回；设置变量不等于自动启用全部测试。

| 变量 | 用途和默认值 |
| --- | --- |
| `RTSP_TEST_URL` | 独立 `nvr-recorder` 库的 RTSP 录制冒烟测试输入 |
| `ONVIF_TEST_HOST`、`ONVIF_TEST_PORT`、`ONVIF_TEST_USER`、`ONVIF_TEST_PASS` | ONVIF 集成测试；端口默认 `80`，用户名和密码默认空 |
| `FTP_TEST_PORT`、`FTP_TEST_ROOT`、`FTP_TEST_LOCAL` | FTP 往返测试的端口、服务端根目录及本地临时目录；运行该忽略测试时需提供 |
| `DETECT_TEST_MODEL`、`DETECT_TEST_IMAGE` | 真实检测测试的 ONNX 模型及图片路径 |
| `ASR_SMOKE_MEDIA` | NVR ASR 媒体测试输入，模型根目录沿用 `ASR_MODELS_DIR` |
| `NVR_ASR_TEST_MODEL_DIR`、`NVR_ASR_TEST_WAV` | ASR 库真实模型测试；默认仓库 `third_party/asr-models` 及模型自带 `test_wavs/zh.wav` |
| `NVR_TTS_TEST_MODEL_DIR` | CPU VITS 语音合成测试模型目录 |
| `NVR_TTS_TEST_MELO_RKNN_DIR`、`NVR_TTS_TEST_RKNN_LIB` | 板端 MeloTTS 测试模型目录及 RKNN 库覆盖项 |
| `NVR_RKLLM_TEST_MODEL`、`NVR_RKLLM_TEST_LIB` | 板端 RKLLM 真实模型测试的模型与运行库路径 |
| `FFMPEG_BUS_RK_TEST_VIDEO`、`FFMPEG_BUS_RK_TEST_ENCODER` | RKMPP 编解码测试输入及编码器；编码器默认 `h264_rkmpp` |
| `FFMPEG_BUS_RK_TEST_V4L2` | Rockchip V4L2 DMABUF 测试节点，例如实际有信号的 `/dev/video*` |
| `NVR_HDMI_TEST_CARD`、`NVR_HDMI_TEST_CONNECTOR`、`NVR_HDMI_TEST_SIBLING` | HDMI 输出测试的 DRM 卡、连接器和第二连接器 |
| `XIAOMI_USER`、`XIAOMI_PASS`、`XIAOMI_REGION` | 小米 `validate` 工具的账号、密码和区域；账号密码也可由参数提供，区域默认空 |
| `ODDITY_RTSP_SERVER_BIN` | RTSP 模拟摄像头使用的服务程序，默认 `oddity-rtsp-server` |
| `DUMMY_SERVER_ADDR`、`DUMMY_SERVER_ID`、`DUMMY_DEVICE_ID`、`DUMMY_PASSWORD`、`DUMMY_ARGS` | `make dummy` 参数；地址默认 `127.0.0.1:5060`，平台和设备编码见 Makefile，密码默认空 |

直接运行模拟脚本时，还可用脚本各自的变量：

- `scripts/start_dummy_camera.sh`：`SERVER_ADDR`、`SERVER_ID`、`DEVICE_ID`、`CHANNEL_ID`、`PASSWORD`、`DOMAIN`、`MEDIA_IP`、`LISTEN`、`FPS`、`SOURCE_FILE`、`MANUFACTURER`、`MODEL`、`FIRMWARE`、`INSTANCE`。这些名字不会配置 NVR 服务端，默认值见脚本开头。
- `scripts/start_dummy_rtsp_camera.sh`：`PORT`、`STREAM_PATH`、`WIDTH`、`HEIGHT`、`FPS`、`PRESET`、`INSTANCE`。
- `scripts/start_livesrc_push.sh`：`HOST`、`PORT`、`APP`、`STREAM`、`WIDTH`、`HEIGHT`、`FPS`、`PRESET`。
- Rockchip 端到端工具的 `RK_HOST`、`RK_USER`、`RK_PASSWORD_FILE`、`RK_API_*`、`RK_RTSP_PORT`、`RK_RTMP_PORT`、`RK_HDMI_*`、超时及报告目录等，集中说明在 [板端验收环境变量表](../e2e/rockchip/README.md)。它们由测试工具使用，不直接替代 NVR 的 `NVR_*` 配置。

`NVR_RKLLM_WORKER_TEST` 是测试程序用来区分自身子进程的内部标记，正常部署不要手动设置。
