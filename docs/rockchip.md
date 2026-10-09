# 在 Rockchip 板子上运行 lite-nvr

本文面向运行 64 位 Linux 的 Rockchip 板子。使用 Linux arm64 GNU 安装包，并从 [BenLocal/FFmpeg-Builds 的 latest release](https://github.com/BenLocal/FFmpeg-Builds/releases/tag/latest) 获取 Rockchip 版 FFmpeg。

## 运行条件

- `uname -m` 应为 `aarch64`。32 位系统不能运行 arm64 包。
- 当前 Cross 官方镜像基于 Ubuntu 24.04。优先使用 Ubuntu 24.04 的板端系统；其他系统先检查 glibc 和共享库兼容性，不能按 SoC 型号判断是否兼容。
- 当前包以 glibc 2.39 及以上作为部署环境要求，检查：`getconf GNU_LIBC_VERSION`。老 Debian / Ubuntu 系统可能需要更换系统或针对该系统重新编译，单独替换 FFmpeg 不能解决 NVR 本身的 glibc 版本要求。
- 使用 **FFmpeg 8.1 shared RK** 包，名称匹配 `linuxarm64-gpl-shared-8.1-rk.tar.xz`。工程使用 `ffmpeg-next = 8`，不要替换成 release 中的 6.1 / 7.1；不带 `shared` 的包不能替代 NVR 链接的共享库。
- ZLMediaKit 使用 [BenLocal/ZLMediaKit-Build release](https://github.com/BenLocal/ZLMediaKit-Build/releases/tag/autobuild-2026-06-24) 的 Linux arm64 预编译库，无需在板子上编译 ZLM。

## 生成并上传 arm64 GNU 包

开发机需准备 rustup 管理的 Rust 工具链、`cross`、Docker 和 Node/npm。在仓库根目录执行：

```bash
make package PACKAGE_ARCHS=arm64
# 可选：按环境变量替换 Cross 容器内的 Ubuntu apt 源。
APT_MIRROR=https://mirrors.tuna.tsinghua.edu.cn/ubuntu make package PACKAGE_ARCHS=arm64
```

默认 Cargo 并发为 1，构建容器最多使用 2 核 CPU、4 GiB 内存。产物为 `dist/lite-nvr-0.1.0-linux-arm64-gnu.tar.gz`，版本号来自 `nvr/Cargo.toml`。也可运行不带 `PACKAGE_ARCHS` 的 `make package`，按顺序生成 amd64、arm64 两个包。

包中有 NVR、内嵌管理后台、通用 FFmpeg、ZLM、ONNX Runtime CPU 库、C++ / OpenMP 运行库及 `start.sh`。ASR 和检测模型文件需要另外配置。

测试机的地址、账号与密码只保存在仓库根目录的 `dev-env.local.md`，该文件已被 Git 忽略。先按该文件的连接方式登录，再在开发机上传安装包：

```bash
RK_HOST=<测试机地址>
RK_USER=<SSH用户名>
scp dist/lite-nvr-0.1.0-linux-arm64-gnu.tar.gz "$RK_USER@$RK_HOST:/tmp/"
```

在板子上解压到一个新的目录，避免覆盖已有实例的数据：

```bash
RK_INSTALL_ROOT="$HOME/lite-nvr-rk-$(date +%Y%m%d-%H%M%S)"
mkdir "$RK_INSTALL_ROOT"
tar -xzf /tmp/lite-nvr-0.1.0-linux-arm64-gnu.tar.gz -C "$RK_INSTALL_ROOT"
cd "$RK_INSTALL_ROOT/lite-nvr-0.1.0-linux-arm64-gnu"
```

## 使用 Rockchip FFmpeg

从上述 release 选择 8.1 shared RK 资产。2026-10-09 检查到的文件名如下；`latest` 的文件名会更新，下载前以 release 页面为准：

```bash
RK_FFMPEG_ASSET=ffmpeg-d90e3a1c18-latest-linuxarm64-gpl-shared-8.1-rk.tar.xz
RK_FFMPEG_URL="https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/$RK_FFMPEG_ASSET"
curl -fL --retry 3 -o "$RK_FFMPEG_ASSET" "$RK_FFMPEG_URL"
curl -fL -o checksums.sha256 https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/checksums.sha256
awk -v asset="$RK_FFMPEG_ASSET" '$2 == asset' checksums.sha256 | sha256sum -c -
mkdir -p ffmpeg-rk
tar -xJf "$RK_FFMPEG_ASSET" -C ffmpeg-rk --strip-components=1
```

在有网络的开发机上下载，再用 `scp` 上传，也可以避免板子直接访问 GitHub。开发机下载优先使用其 HTTP 代理；板子内的 `127.0.0.1` 是板子自身，若使用开发机代理，需要填开发机的可达地址。

保留安装包里的原始 FFmpeg，然后让 `start.sh` 使用 RK 目录。以下步骤用于新解压的安装目录，只执行一次：

```bash
mv ffmpeg ffmpeg.generic
ln -s ffmpeg-rk ffmpeg
export LD_LIBRARY_PATH="$PWD/lib:$PWD/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
./ffmpeg/bin/ffmpeg -hide_banner -version
ldd ./bin/nvr
ldd ./ffmpeg/lib/libavcodec.so
```

两次 `ldd` 都不应出现 `not found` 或符号版本错误。Rockchip FFmpeg 若还需要 MPP / RGA 等共享库，应使用该 release 随附的库，或安装与板端驱动匹配的运行库，并加入 `LD_LIBRARY_PATH`；不要从不同系统随意拷贝 glibc。替换 FFmpeg 后必须重新检查实际加载的库。

如遇 ABI / 符号错误，优先用同一套 RK FFmpeg 头文件与共享库重新编译 NVR。在板子或匹配的 arm64 构建环境中，准备 Rust、Node/npm、Clang、pkg-config 与 C/C++ 工具链后，在新的源码目录运行：

```bash
# 在新终端重新设置 release URL，文件名以 release 页面为准。
RK_FFMPEG_URL=https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/ffmpeg-d90e3a1c18-latest-linuxarm64-gpl-shared-8.1-rk.tar.xz
FFMPEG_URL="$RK_FFMPEG_URL" make install-deps
CARGO_BUILD_JOBS=1 CMAKE_BUILD_PARALLEL_LEVEL=1 \
  bash .agent/skills/build-nvr/scripts/build.sh release -p nvr
# 填写前面已经准备好的安装目录，把新二进制放回该目录。
RK_PACKAGE_DIR=/实际安装目录/lite-nvr-0.1.0-linux-arm64-gnu
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
cd /实际安装目录/lite-nvr-0.1.0-linux-arm64-gnu
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

## NVR 当前的硬件加速范围

本项目通过 FFmpeg **库**处理媒体，替换 PATH 中的 `ffmpeg` 命令并不会切换 NVR 的编解码器。`start.sh` 的库路径以及 `ldd ./bin/nvr` 的结果才决定实际加载的 FFmpeg。

目前 `crates/ffmpeg-bus/src/hw.rs` 的自动候选包含 VideoToolbox、NVENC、QSV、VAAPI，尚未包含 `h264_rkmpp` / `hevc_rkmpp`。因此本文支持在合适的 RK 系统上部署 arm64 GNU NVR、加载 RK FFmpeg，以及单独验证 RK 编解码器；不代表 NVR 的默认转码自动使用 MPP，也不代表检测已经使用 RK NPU。需要 NVR 内自动硬件加速时，还需实现并验证 RK 编解码候选与帧格式处理。

## 测试记录

- 2026-10-09：已在本地 Ubuntu 24.04 arm64 容器中运行 RK FFmpeg，确认注册了 `h264_rkmpp` / `hevc_rkmpp` 编码器；容器没有板端设备节点，此项不是硬件编码实测。
- 2026-10-09：已下载上述 8.1 arm64 shared RK 资产，并与 release 的 `checksums.sha256` 校验一致；包内包含 `librockchip_mpp`、`librockchip_vpu` 和 `librga`。
- 2026-10-09：尝试连接 `dev-env.local.md` 中的测试板，SSH 22 端口连接超时。板端系统、NVR 启动、媒体转码和 MPP / RGA 实测尚未完成，不能将本文视为已完成的板端验证报告。
