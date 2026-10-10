# 在 Rockchip 板子上运行 lite-nvr

本文面向运行 64 位 Linux 的 Rockchip 板子。使用独立的 Linux arm64 Rockchip GNU 安装包，并从 [BenLocal/FFmpeg-Builds 的 latest release](https://github.com/BenLocal/FFmpeg-Builds/releases/tag/latest) 获取 Rockchip 版 FFmpeg。

## 运行条件

- `uname -m` 应为 `aarch64`。32 位系统不能运行 arm64 包。
- 当前 Cross 官方镜像基于 Ubuntu 24.04。优先使用 Ubuntu 24.04 的板端系统；其他系统先检查 glibc 和共享库兼容性，不能按 SoC 型号判断是否兼容。
- 当前包以 glibc 2.39 及以上作为部署环境要求，检查：`getconf GNU_LIBC_VERSION`。老 Debian / Ubuntu 系统可能需要更换系统或针对该系统重新编译，单独替换 FFmpeg 不能解决 NVR 本身的 glibc 版本要求。
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

在板子上解压到一个新的目录，避免覆盖已有实例的数据：

```bash
RK_INSTALL_ROOT="$HOME/lite-nvr-rk-$(date +%Y%m%d-%H%M%S)"
mkdir "$RK_INSTALL_ROOT"
tar -xzf /tmp/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz -C "$RK_INSTALL_ROOT"
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

## 启动与检查

```bash
RUST_LOG=info ./start.sh
```

后台地址为 `http://<板子地址>:18080/nvr/`。NVR API 使用 18080；内置 ZLM 的 HTTP、RTSP、RTMP 分别为 8553、8554、8555。

在另一个终端检查，先进入实际安装目录并设置库路径：

```bash
cd /实际安装目录/lite-nvr-0.1.0-linux-arm64-rockchip
export LD_LIBRARY_PATH="$PWD/lib:$PWD/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
curl -f http://127.0.0.1:18080/nvr/ -o /dev/null
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
