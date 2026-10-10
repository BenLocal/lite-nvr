# 在 Rockchip 板子上运行 lite-nvr

本文面向运行 64 位 Linux 的 Rockchip 板子。使用独立的 Linux arm64 Rockchip GNU 安装包，并从 [BenLocal/FFmpeg-Builds 的 latest release](https://github.com/BenLocal/FFmpeg-Builds/releases/tag/latest) 获取 Rockchip 版 FFmpeg。

## 运行条件

- `uname -m` 应为 `aarch64`。32 位系统不能运行 arm64 包。
- 板端需要 glibc（musl 系统见下文 Alpine 一节），最低版本取决于打包用的 Cross 镜像：
  - 官方镜像 `ghcr.io/cross-rs/aarch64-unknown-linux-gnu:main`（Ubuntu 24.04）：需要 glibc 2.39 及以上。
  - Ubuntu 20.04 的 Cross 镜像（在 `.env` 设置 `CROSS_TARGET_AARCH64_UNKNOWN_LINUX_GNU_IMAGE`）：需要 glibc 2.30 及以上，可原生运行在 Buildroot 等较老的系统上。该镜像的 GCC 9 缺少 sherpa-onnx 预编译库需要的 libstdc++ 符号，由 `crates/nvr-asr/src/libstdcxx_compat.cpp` 补齐。
  - 包内 RK FFmpeg、ZLM、ONNX Runtime 的要求都不高于 glibc 2.28。
- 检查板端 glibc：`getconf GNU_LIBC_VERSION`；BusyBox 系统没有 `getconf`，可执行 `/lib/libc.so.6` 查看版本。检查包实际需要的版本：`strings bin/nvr | grep -o 'GLIBC_2\.[0-9]*' | sort -V | tail -1`。不能按 SoC 型号判断是否兼容，单独替换 FFmpeg 也不能解决 NVR 本身的 glibc 版本要求。
- 使用 **FFmpeg 8.1 shared RK** 包，名称匹配 `linuxarm64-gpl-shared-8.1-rk.tar.xz`。工程使用 `ffmpeg-next = 8`，不要替换成 release 中的 6.1 / 7.1；不带 `shared` 的包不能替代 NVR 链接的共享库。
- ZLMediaKit 使用 [BenLocal/ZLMediaKit-Build release](https://github.com/BenLocal/ZLMediaKit-Build/releases/tag/autobuild-2026-06-24) 的 Linux arm64 预编译库，无需在板子上编译 ZLM。

## 生成并上传 arm64 Rockchip 包

开发机需准备 rustup 管理的 Rust 工具链、`cross`、Docker 和 Node/npm。在仓库根目录执行：

```bash
make package PACKAGE_ARCHS=rockchip
# 可选：按环境变量替换 Cross 容器内的 Ubuntu apt 源。
APT_MIRROR=https://mirrors.tuna.tsinghua.edu.cn/ubuntu make package PACKAGE_ARCHS=rockchip
```

默认 Cargo 并发为 1，构建容器最多使用 2 核 CPU、4 GiB 内存。产物为 `dist/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz`，版本号来自 `nvr/Cargo.toml`。也可运行不带 `PACKAGE_ARCHS` 的 `make package`，按顺序生成 amd64 GNU、arm64 GNU 和 arm64 Rockchip 三个包。

包中有 NVR、内嵌管理后台、RK FFmpeg、ZLM、ONNX Runtime CPU 库、C++ / OpenMP 运行库及 `start.sh`。ASR 和检测模型文件需要另外配置。

测试机的地址、账号与密码只保存在仓库根目录的 `dev-env.local.md`，该文件已被 Git 忽略。先按该文件的连接方式登录，再在开发机上传安装包：

```bash
RK_HOST=<测试机地址>
RK_USER=<SSH用户名>
scp dist/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz "$RK_USER@$RK_HOST:/tmp/"
```

首次安装时，在板子上解压到一个新的目录，避免覆盖已有实例的数据。安装目录要放在空间充足的数据分区（例如 `/userdata`），不要放在根分区：数据库和录像默认都写在安装目录下（见下文「在板子上启动」）。

```bash
RK_INSTALL_ROOT=/userdata/lite-nvr-rk-$(date +%Y%m%d)
mkdir -p "$RK_INSTALL_ROOT"
# BusyBox 的 tar 不支持 -z，统一用 gzip 管道解包。
gzip -dc /tmp/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz | tar -x -C "$RK_INSTALL_ROOT"
cd "$RK_INSTALL_ROOT/lite-nvr-0.1.0-linux-arm64-rockchip"
```

## Alpine 板子的 Docker 运行方式

测试板实际运行 Alpine 3.23.2（musl）。当前 GNU 包及 RK FFmpeg、ZLM、ONNX Runtime 都使用 glibc，不能直接在 Alpine 原生运行；可用 Ubuntu 24.04 arm64 容器提供 glibc，并传入硬件设备。

在已经解压的安装目录中执行：

```bash
set --
for device in /dev/mpp_service /dev/rga /dev/dri/renderD128 /dev/dri/renderD129 /dev/dma_heap/*; do
  [ ! -e "$device" ] || set -- "$@" --device "$device"
done
docker run -d --name lite-nvr-rockchip --cpus=1 \
  --security-opt systempaths=unconfined \
  -v /sys/firmware/devicetree/base:/sys/firmware/devicetree/base:ro \
  -v /proc/device-tree:/proc/device-tree:ro \
  -p 18084:18080 -p 18553:8553 -p 18554:8554 -p 18555:8555 \
  -v "$PWD:/opt/package" "$@" ubuntu:24.04 /opt/package/start.sh
```

`systempaths=unconfined` 取消 Docker 对系统路径的默认屏蔽，让 MPP 能读取上面只读挂载的设备树并识别 SoC；也会取消其他默认系统路径屏蔽，部署时需评估容器访问权限。若设备树被屏蔽，H.264 可能仍可用，但 HEVC 会报 MPP 初始化失败。

上面的映射需要 Docker 桥接网络。测试板 Docker 的 bridge 不可用，创建网络返回 `operation not supported`，因而实测使用 `--network host`（去掉所有 `-p` 参数）。使用 host 网络前检查 18080、8553、8554、8555 是否被占用，已有服务占用时不要直接替换或停止它们。测试板已有服务占用 8554、8555，NVR 的页面和 HTTP 媒体服务可用，但该测试容器的 RTSP / RTMP 监听失败，不能按完整部署验收。

Alpine 的 musl 与 GNU ABI 不同；改 Rust target 不能转换已有 glibc 共享库。如需 musl 原生包，还需重建 FFmpeg/MPP/RGA、ZLM、ONNX Runtime、sherpa-onnx 及其 C/C++ 依赖。

## 使用 Rockchip FFmpeg

Rockchip 包已在编译期使用 RK FFmpeg 的头文件与共享库，启用 `nvr/rockchip` → `ffmpeg-bus/rockchip`，无需再替换包内的 FFmpeg。下载脚本默认使用 2026-10-09 验证的 8.1 shared RK 资产；如 release 文件名变化，在 `.env` 设置 `RK_FFMPEG_URL`。

```bash
export LD_LIBRARY_PATH="$PWD/lib:$PWD/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
./ffmpeg/bin/ffmpeg -hide_banner -version
ldd ./bin/nvr
ldd ./ffmpeg/lib/libavcodec.so
```

两次 `ldd` 都不应出现 `not found` 或符号版本错误。MPP / RGA 库随 RK FFmpeg 打包；硬件使用仍需要板端内核驱动和设备节点。

如遇 ABI / 符号错误，优先用同一套 RK FFmpeg 头文件与共享库重新编译 NVR。以下原生构建命令要求 glibc Linux 环境；Alpine 板子应在 Ubuntu 容器中执行。在匹配的 arm64 构建环境中，准备 Rust、Node/npm、Clang、pkg-config 与 C/C++ 工具链后，在新的源码目录运行：

```bash
# 在新终端重新设置 release URL，文件名以 release 页面为准。
RK_FFMPEG_URL=https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/ffmpeg-d90e3a1c18-latest-linuxarm64-gpl-shared-8.1-rk.tar.xz
FFMPEG_URL="$RK_FFMPEG_URL" make install-deps
make build BUILD_JOBS=1 BUILD_MODE=release BUILD_ARGS="-p nvr --features rockchip"
# 填写前面已经准备好的安装目录，把新二进制放回该目录。
RK_PACKAGE_DIR=/实际安装目录/lite-nvr-0.1.0-linux-arm64-rockchip
cp target/release/nvr "$RK_PACKAGE_DIR/bin/nvr"
cd "$RK_PACKAGE_DIR"
export LD_LIBRARY_PATH="$PWD/lib:$PWD/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
ldd ./bin/nvr
```

此方式使用现有依赖安装脚本下载 FFmpeg 和 ZLM，不需要自行编译 ZLM。如果在另一台 arm64 机器构建，用 `scp` 将 `target/release/nvr` 上传到板端安装目录的 `bin/nvr`。后续启动与检查步骤均在安装目录执行，`start.sh` 将运行刚复制的 `bin/nvr`。

## 在板子上启动

以下命令都在安装目录（`.../lite-nvr-0.1.0-linux-arm64-rockchip`）中执行。包内的 `start.sh`（源码在仓库 `scripts/rockchip/start.sh`，POSIX `sh`，BusyBox 可直接运行）会先 `cd` 到安装目录，设置 `LD_LIBRARY_PATH`（包内 `lib/`、`ffmpeg/lib/`）和 `ORT_DYLIB_PATH`，再运行 `bin/nvr`：

| 命令 | 作用 |
|---|---|
| `./start.sh` 或 `./start.sh run` | 前台运行，日志输出到终端，Ctrl-C 退出（Docker 方式也用它） |
| `./start.sh start` | 后台运行，日志写到 `nvr.log`，上一份日志改名为 `nvr.log.prev` |
| `./start.sh stop` | 停止本目录启动的 nvr；15 秒内未退出则强制结束 |
| `./start.sh restart` | 先停止再后台启动 |
| `./start.sh status` | 进程、端口和管理页面检查；未运行时退出码为 3 |

`stop` / `status` 按可执行文件路径（`本目录/bin/nvr`）识别进程，不会误伤板上其他服务；请用它们代替 `pkill -f nvr` 这类宽泛匹配。

### 端口与数据位置

- 端口：管理后台和 API 为 18080；内置 ZLM 的 HTTP、RTSP、RTMP 分别为 8553、8554、8555。启动前确认没有被占用：`netstat -ltn | grep -E ':(18080|8553|8554|8555) '`。板上已有的录播服务可能占用 80、554、1935 等端口，与 NVR 不冲突，不要停止它们。
- 数据：数据库 `nvr.db*` 和录像 `data/records/` 都在安装目录下；录像目录可用 `NVR_RECORD_DIR` 改到别处。

### 启动

首次部署建议先前台运行，直接看日志；确认正常后改为后台运行：

```bash
./start.sh            # 前台，Ctrl-C 退出
./start.sh start      # 后台
./start.sh status
```

启动约 5 秒后，日志里会有一行主机信息，例如：

```text
metrics: host Linux (Buildroot 2018.02-rc3) 5.10.252-... (Rockchip RK3588 EVB1 LP4 V10 Board), 8 cpus
```

常用环境变量，写在命令前面，例如 `RUST_LOG=debug ./start.sh start`：

| 变量 | 作用 |
|---|---|
| `RUST_LOG` | 日志级别，默认 `info`；如 `ffmpeg_bus=debug` |
| `NVR_RECORD_DIR` | 录像目录，默认 `安装目录/data/records` |
| `FFMPEG_BUS_ENCODER_QUEUE_FRAMES` | 编码器前的帧队列长度，默认 8；路数与内存见 [rockchip-capacity.md](rockchip-capacity.md) |
| `FFMPEG_BUS_DISABLE_HWDEC=1` | 强制软件解码，排查硬解问题时用 |
| `DETECT_MODELS_DIR` / `ASR_MODELS_DIR` | 检测 / ASR 模型目录，默认在安装目录下的 `third_party/` |

### 检查是否正常运行

`./start.sh status` 已检查进程、端口和管理页面。还可以调用主机信息接口确认后端正常（板上通常没有 `curl`，用 BusyBox 自带的 `wget`）：

```bash
TOKEN=$(wget -q -O - --header 'Content-Type: application/json' \
  --post-data '{"username":"admin","password":"admin"}' \
  http://127.0.0.1:18080/api/user/login | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
wget -q -O - --header "Authorization: Bearer $TOKEN" http://127.0.0.1:18080/api/system/os
```

后台地址为 `http://<板子地址>:18080/nvr/`，默认账号 `admin` / `admin`，部署后应修改密码。

### 原地升级

在开发机重新打包并上传后，停止 NVR，把新包解压到**同一个**安装根目录，覆盖 `bin/`、`lib/`、`ffmpeg/` 和 `start.sh`；`nvr.db*` 和 `data/` 不在包里，会原样保留，设备配置和录像不会丢失：

```bash
cd "$RK_INSTALL_ROOT/lite-nvr-0.1.0-linux-arm64-rockchip"
./start.sh stop
cd "$RK_INSTALL_ROOT"
gzip -dc /tmp/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz | tar -x
cd lite-nvr-0.1.0-linux-arm64-rockchip
./start.sh start
```

### 开机自启

安装包不配置开机自启。需要时按板端系统的方式自行添加：systemd 系统写一个 service 单元，`ExecStart` 用 `安装目录/start.sh run`；Buildroot / BusyBox 系统可在 `/etc/init.d/` 下加脚本，`start` / `stop` 分别调用 `安装目录/start.sh start` / `安装目录/start.sh stop`。

### 硬件检查

先进入实际安装目录并设置库路径：

```bash
cd /实际安装目录/lite-nvr-0.1.0-linux-arm64-rockchip
export LD_LIBRARY_PATH="$PWD/lib:$PWD/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
wget -q -O /dev/null http://127.0.0.1:18080/nvr/ && echo admin page OK
./ffmpeg/bin/ffmpeg -hide_banner -decoders | grep rkmpp
./ffmpeg/bin/ffmpeg -hide_banner -encoders | grep rkmpp
ls -l /dev/dri /dev/mpp_service /dev/rga
```

设备节点是否存在取决于板端内核和驱动。列出 `rkmpp` 编解码器只证明 FFmpeg 编译了它们；可再用测试图验证实际编码：

```bash
./ffmpeg/bin/ffmpeg -hide_banner \
  -f lavfi -i testsrc2=size=1280x720:rate=25 \
  -vf format=nv12 -frames:v 50 -c:v h264_rkmpp \
  /tmp/lite-nvr-rk-encode-test.mp4
```

在板子的源码目录中，还可验证 Rust 库实际使用硬件编码、硬件解码和普通内存输出。先用上面的命令生成测试视频，下载 SDK 后执行：

```bash
export FFMPEG_DIR="$(bash scripts/download-rockchip-ffmpeg.sh)"
export LD_LIBRARY_PATH="$FFMPEG_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
FFMPEG_BUS_RK_TEST_VIDEO=/tmp/lite-nvr-rk-encode-test.mp4 CARGO_BUILD_JOBS=1 \
  bash scripts/with-rockchip-patch.sh cargo test -p ffmpeg-bus --features rockchip rockchip_hardware -- --test-threads=1
```

只有设置 `FFMPEG_BUS_RK_TEST_VIDEO` 才执行这两项板上测试；未设置时跳过。硬编码默认测试 H.264，可用 `FFMPEG_BUS_RK_TEST_ENCODER=hevc_rkmpp` 或 `mjpeg_rkmpp` 切换目标编码器。测试要求实际选中 RKMPP、输出帧/编码包，硬件测试中的软件回退会导致失败。

## NVR 当前的硬件加速范围

本项目通过 FFmpeg **库**处理媒体，替换 PATH 中的 `ffmpeg` 命令并不会切换 NVR 的编解码器。`start.sh` 的库路径以及 `ldd ./bin/nvr` 的结果才决定实际加载的 FFmpeg。

启用 `rockchip` feature 时，H.264 / HEVC / MJPEG 编码优先使用 `h264_rkmpp` / `hevc_rkmpp` / `mjpeg_rkmpp`，也可在编码配置中显式选择这些名字。解码优先尝试 RKMPP 的 H.264、HEVC、MJPEG、MPEG-1/2/4、VP8/9、AV1，具体支持范围由芯片决定。硬件打开失败时沿用软件回退；发送帧/包时失败会标记该硬件编解码器并回退。

解码选择普通内存像素格式，让 RK FFmpeg 从 MPP 缓冲复制像素，继续供 CPU 滤镜、缩放、合成和检测使用；当前不提供端到端零拷贝或 RGA 滤镜加速。`FFMPEG_BUS_DISABLE_HWDEC=1` 仍可强制软件解码。每个编码器前有一个解码帧队列，默认 8 帧（1080p 约 25MB），可用 `FFMPEG_BUS_ENCODER_QUEUE_FRAMES` 调整；调大会按每帧约 3MB（1080p）增加每路内存，原先的 128 帧曾使 RK3588 在 12 路 1080p 转码时内存耗尽。feature 未启用时不加入 RKMPP 候选。RKMPP 与 RK NPU 检测是独立能力。

V4L2 采集走 FFmpeg 的 v4l2 输入，它只支持单平面（`VIDEO_CAPTURE`）设备。RK3588 的 rkcif（MIPI/LVDS 摄像头接口）和 hdmirx 节点只提供多平面接口（capabilities `0x84201000`），FFmpeg 读取时 `VIDIOC_DQBUF` 报 `Invalid argument`，因此不能作为 V4L2 设备接入；管理后台的 V4L2 节点列表只列出单平面采集节点。可用 `media-ctl -p` 查看 rkcif 是否接了传感器（例如测试板上的 LT6911C HDMI 转 MIPI 芯片）。

直接复用 crate 时写 `ffmpeg-bus = { path = "...", features = ["rockchip"] }`。Cargo feature 不能替依赖 crate 修改 `FFMPEG_DIR`：直接执行 Cargo 前必须将其设为 RK FFmpeg SDK，并配置 `LD_LIBRARY_PATH`；原生构建脚本和 `make package PACKAGE_ARCHS=rockchip` 会自动下载并选择 SDK。启用 feature 后启动检查 RKMPP 编解码器是否已注册，防止误用通用 FFmpeg。

参考：[RK FFmpeg 解码说明](https://github.com/nyanmisaka/ffmpeg-rockchip/wiki/Decoder)、[编码说明](https://github.com/nyanmisaka/ffmpeg-rockchip/wiki/Encoder)。

## 测试记录

- 2026-10-09：已在本地 Ubuntu 24.04 arm64 容器中运行 RK FFmpeg，确认注册了 `h264_rkmpp` / `hevc_rkmpp` 编码器；容器没有板端设备节点，此项不是硬件编码实测。
- 2026-10-09：已下载上述 8.1 arm64 shared RK 资产，并与 release 的 `checksums.sha256` 校验一致；包内包含 `librockchip_mpp`、`librockchip_vpu` 和 `librga`。
- 2026-10-09：尝试连接 `dev-env.local.md` 中的测试板，SSH 22 端口连接超时。板端系统、NVR 启动、媒体转码和 MPP / RGA 实测尚未完成，不能将本文视为已完成的板端验证报告。

- 2026-10-09：后续 SSH 已连接成功，确认测试板为 RK3588、Alpine 3.23.2、Linux 5.10，MPP/RGA 设备节点存在。Ubuntu 24.04 arm64 测试容器中 NVR 管理页面返回 HTTP 200；host 网络下已有服务占用 8554/8555，RTSP/RTMP 未完成验证。
- 2026-10-09：板上 RK FFmpeg 实际完成 H.264 硬编码和硬解码各 50 帧、MJPEG 硬编码 10 帧；Rust `ffmpeg-bus` RKMPP 硬解码及普通内存帧输出测试通过。HEVC 在 320×240、1280×720 下均报 `Failed to init MPP context: -1`，Rust 编码选择会退回 libx265，此项硬编码尚未通过。
- 2026-10-09：RK 8.1 头文件新增 NV15/NV20，使上游 ffmpeg-next 8.1.0 的像素格式匹配不完整；使用保留原许可证的补丁 `patches/ffmpeg-rockchip-8.1.0.patch`，根据 SDK 头文件启用对应枚举映射，避免使用旧绑定缓存掩盖兼容问题。

- 2026-10-09：HEVC 初始化失败的原因已验证为 Docker 默认屏蔽 `/sys/firmware`、MPP 读不到设备树。传入只读设备树并用 `--security-opt systempaths=unconfined` 解除屏蔽后，HEVC 硬编码成功，Rust 库同时通过 H.264/HEVC 硬编码、H.264 硬解码和内存帧格式测试（4 项通过）。

Rockchip 打包和项目构建脚本会自动下载或复用原始 crate，并将补丁应用到 `.cache/rockchip-rust/`；无需保留 `vendor/`。普通构建不启用补丁。手动 Cargo/Cross 命令需通过 `scripts/with-rockchip-patch.sh`，完成后会恢复 Cargo.lock；构建期间请勿并行运行其他 Cargo 命令。

- 2026-10-10：用 Ubuntu 20.04 的 Cross 镜像打包（nvr 只需 glibc 2.30），在 RK3588 + Buildroot 2018.02（glibc 2.33、无 Docker）上原生运行通过：管理后台、RKMPP 硬解 / 硬编、`/api/system/os` 均正常；1080p 转码路数与资源占用见 [rockchip-capacity.md](rockchip-capacity.md)。原地升级保留了 `nvr.db` 中的设备配置。
- 2026-10-10：该板 12 个 `/dev/video*`（rkcif、hdmirx）均为多平面接口，FFmpeg v4l2 输入无法读取；对无信号节点抓帧时 ffmpeg 会阻塞在驱动调用里，`timeout` 发出的 SIGTERM 无效，只能 `kill -9`。
