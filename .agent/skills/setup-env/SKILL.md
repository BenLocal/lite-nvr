---
name: setup-env
description: 问答式准备 lite-nvr 本地开发环境（含 Rockchip 测试主机）。先检测现状，再问用户要配哪些能力（构建代理、FFmpeg/ZLM、ASR、检测模型、集成测试账号、交叉打包、Rockchip 板端测试主机），按回答写 `.env` / `dev-env.local.md` 并安装依赖。用户说「准备环境 / 搭环境 / 新机器初始化 / 配置本地环境 / 配 .env / 配 Rockchip 测试机」时使用；只需编译时用 build-nvr。
---

# setup-env

目标：每个用户选中的能力都落到 **已配置且已验证**；没选的一律不动。

## 1. 摸底

```bash
bash .agent/skills/setup-env/scripts/check-env.sh
```

只读，输出 `OK / MISSING / INFO`。把结果压成一张短表给用户看，作为后面提问的依据：已经 OK 的项不再问。

## 2. 问要配什么

用 `AskUserQuestion`（`multiSelect: true`）问「这次要准备哪些能力」，选项只列 `MISSING` 或用户可能想改的，单次最多 4 个，超出就分两轮问：

| 能力 | 需要用户回答的 | 落到哪里 |
|---|---|---|
| 构建代理 | HTTP 代理地址（必须 HTTP，不能 SOCKS5）；需要绕过代理的内网主机 / 镜像 | `.env` 的 `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY`（大小写各一份 no_proxy） |
| 基础依赖 FFmpeg / ZLM | 用 `./ffmpeg`、`./zlm` 默认目录，还是已有目录 | `make install-deps`；已有目录写 `FFMPEG_DIR` / `ZLM_DIR` |
| ASR | 是否下载预编译库、模型放哪 | `make download-asr-libs`、`make download-asr-models`；非默认目录写 `ASR_MODELS_DIR` |
| 目标检测 | 模型目录（需含 `models.json`） | 非默认目录写 `DETECT_MODELS_DIR`，清单格式见 `docs/agents-dot-md/design-detect.md` |
| 集成测试账号 | 要跑哪类：RTSP 摄像头 / ONVIF / FTP / 小米 / 屏幕采集 | `.env` 的 `RTSP_TEST_URL`、`ONVIF_TEST_*`、`FTP_TEST_*`、`XIAOMI_*`；屏幕采集用 `make xvfb` |
| 交叉打包 | 是否要换 Cross 镜像、apt 镜像 | `.env` 的 `CROSS_TARGET_*_IMAGE`、`APT_MIRROR`；本机需 `cross` + Docker |
| Rockchip | 见第 3 步 | `dev-env.local.md`，必要时 `.env` 的 `RK_FFMPEG_URL` |

每个选中的能力再追问一次具体值；有约定默认值的给出默认值作为首选项（标 `(Recommended)`），让用户一键确认。

## 3. Rockchip 分支：问测试主机

Rockchip 能力分「开发机交叉打包」和「板端实测」两半。先问用户要哪半；要板端实测时，用 `AskUserQuestion` 问测试主机配置：

- 连接：主机地址、SSH 端口、用户名、认证方式（密钥 / 密码 / 跳板机）。
- 系统：Ubuntu 24.04 原生 / Alpine 等 musl 系统（需 Docker 里跑 Ubuntu 24.04）/ 不确定。
- 部署方式：原生 `start.sh` / Docker（bridge 可用还是只能 `--network host`）。
- 安装目录与端口：新实例放哪个目录；18080、8553、8554、8555 是否被板上已有服务占用，被占时映射到哪些端口。

用户答「不确定」的项，连上之后实测补齐，不要猜：

```bash
ssh -p <port> <user>@<host> 'uname -m; getconf GNU_LIBC_VERSION; cat /etc/os-release | head -3; \
  cat /proc/device-tree/compatible | tr "\0" " "; echo; ls -l /dev/mpp_service /dev/rga /dev/dri /dev/dma_heap; \
  command -v docker && docker info --format "{{.ServerVersion}}"; ss -ltn | grep -E ":(18080|8553|8554|8555) "'
```

判定规则以 `docs/rockchip.md` 为准（`aarch64`、glibc ≥ 2.39、设备节点、Docker 下要挂设备树）；不满足的项直接告诉用户后果，由用户决定换系统还是走容器。

结果写进仓库根的 `dev-env.local.md`（已 gitignore，`docs/rockchip.md` 约定测试机信息只放这里）。文件已存在就先读，只增改相关小节：

```markdown
# 本地开发环境（不入库）

## Rockchip 测试主机
- 地址 / 端口：<host>:<port>
- 用户 / 认证：<user>，<密钥路径 | 密码：...>
- SoC / 系统 / 内核：<RK3588>，<Alpine 3.23.2>，<5.10>
- glibc：<版本 | musl>
- 运行方式：<原生 | Docker host 网络 | Docker bridge>
- 端口映射：NVR 18080→<>，ZLM HTTP/RTSP/RTMP→<>
- 安装目录：<~/lite-nvr-rk-...>
- 设备节点：<mpp_service / rga / dri / dma_heap 实测结果>
- 记录日期：<YYYY-MM-DD>
```

## 4. 落地

- 写 `.env`：没有就 `cp .env.example .env`；已有的键先给用户看旧值再改，追加的新键放到对应注释段下。改完 diff 给用户过目。
- 凭据（摄像头 / 小米 / ONVIF / FTP 账号、测试机密码）只进 `.env` 或 `dev-env.local.md`，两者都已 gitignore；写入前用 `git check-ignore .env dev-env.local.md` 确认。
- 下载类命令（`make install-deps`、`download-asr-*`、`scripts/download-rockchip-ffmpeg.sh`）走 `.env` 代理；超过 2 分钟无进度就停下，问用户换镜像还是改 `RK_FFMPEG_URL` / `FFMPEG_URL`。
- 系统包缺失（`clang`、`pkg-config`、`libsmbclient-dev` 等）需要 root：写成 `/tmp/*.sh` 脚本让用户在外部终端执行，回来再复查。

## 5. 验证

逐项对应用户选中的能力，全部通过才算完成：

- 重跑 `check-env.sh`，选中的项全部变 `OK`。
- 基础依赖：`make build BUILD_MODE=check BUILD_ARGS="-p nvr"`。
- ASR / 检测：对应目录存在，检测目录含 `models.json`。
- 交叉打包：`cross --version`、`docker info` 可用（完整 `make package` 耗时长，问用户是否现在跑）。
- Rockchip 板端：SSH 能连上，`dev-env.local.md` 每一行都已填实测值；后续部署与硬件验证按 `docs/rockchip.md` 走。

最后给用户一张表：能力 → 状态 → 写入了哪些键 / 文件，未完成项写明原因和下一步。
