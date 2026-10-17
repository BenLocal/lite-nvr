# Rockchip 真机 e2e

从当前源码重新构建 Rockchip 包，部署到板端独立 `/userdata` 目录，串行验证 API、真实媒体、录像浏览器回放及硬件。每次输出 TAP 和 `report.json`，FAIL 非零退出，SKIP 单独统计。测试结束停止本次实例；保留实例目录和证据，便于定位失败。不会管理旧实例或板端既有服务。

## 运行

主机侧入口、HTTP 检查、部署与变异流程均为 Python 脚本。开发机需要 Python 3.9+、FFmpeg/ffprobe、readelf、OpenSSH，以及项目 `make package` 所需 Docker/Cross/npm/Rust/Bash；复用包时不执行构建。Cross/Rustup 缓存需要可写。板上需要 BusyBox sh、wget、timeout、netstat、ldd、gzip/tar；FFmpeg 来自安装包。

从 git-ignored `dev-env.local.md` 获取板端地址和认证。不要把密码写进命令历史或脚本。SSH 也可使用密钥或指定配置文件；首次连接默认 `StrictHostKeyChecking=accept-new`，主机公钥保存在 `.cache/rockchip-e2e-known_hosts`。

```bash
export RK_HOST=<board-address> RK_USER=<ssh-user>
export RK_PASSWORD_FILE=<0600-password-file>     # 使用密钥时省略
export RK_DRM_CARD=<actual-drm-card>
export RK_HDMI_CONNECTOR=<wired-output-connector>
export RK_HDMI_CAPTURE_DEVICE=<wired-capture-node>
export RK_HDMI_SECOND_CONNECTOR=<actual-second-output> # T3.4
python3 e2e/rockchip/run.py --baseline
python3 e2e/rockchip/run.py                            # T0 → T1 → T2 → T3
python3 e2e/rockchip/run.py T1                         # 每组先跑 T0，基线失败立即停止
python3 e2e/rockchip/run.py T2.6                       # 单项检查，仍先验证 T0
python3 e2e/rockchip/run.py --mutate drop-state-field
python3 e2e/rockchip/run.py --mutate drop-output-normalize
```

`run.sh` 仅作为兼容入口转交给 Python。默认每次重新打包。可以明确复用本套脚本生成的包：

```bash
RK_PACKAGE="$PWD/dist/lite-nvr-0.1.0-linux-arm64-rockchip.tar.gz" python3 e2e/rockchip/run.py T2
```

必须同时存在 `<包名>.build.json`，其 commit、源码摘要、包 SHA256 和 rockchip feature 与当前源码一致。未附构建证据的旧包会被拒绝。构建日志在本轮证据目录；构建期间不得运行其他 Cargo 命令（Rockchip wrapper 临时改 Cargo.lock 和 .cargo/config.toml）。HDMI 两个 examples 在包构建结束后单独通过 `make rockchip-e2e-hdmi` 构建，不需要动态 libdrm。

真实浏览器跨分段检查使用 Python Playwright 和 Chromium。可在本机可写缓存中安装，避免修改系统 Python：

```bash
pip install --target .cache/rockchip-e2e-python --cache-dir .cache/pip playwright
export PYTHONPATH="$PWD/.cache/rockchip-e2e-python"
export PLAYWRIGHT_BROWSERS_PATH="$PWD/.cache/rockchip-e2e-browsers"
python3 -m playwright install chromium
```

## 配置

### 测试已运行的实例

板端已经部署且正在采集时，可用 `live.py` 复用 API / 媒体断言，无需重新构建、部署或停止现有实例。沿用上面的 SSH、API 账号、RTSP 端口和 Playwright 环境变量，明确提供当前安装目录：

```bash
python3 e2e/rockchip/live.py --install-dir <board-install-dir> --group all
```

默认 `--group api`；`media` 包含真实解码、录像和浏览器跨段播放。测试会新建独立命名的设备、合成器和板端证据目录，结束后删除本轮设备与合成器，并校验原有设备配置、进程 PID 和二进制摘要未变。测试录像与证据目录保留。`runtime.json` 记录实际运行二进制 SHA-256；报告中的 commit 表示测试代码版本，不能用来证明现有实例由该源码构建。

此入口主动禁用 HDMI/V4L2 有信号及无信号 fixture，避免争用现有采集；普通运行也跳过需要变异构建的 T1.9。硬件全量和故障注入继续使用 `run.py` 的独立部署流程。

### 语音对话

`chat.py` 连接已运行实例的 Socket.IO `/chat`（需 `NVR_LLM_MODEL` 和 TTS 模型），用 WAV 按住说话或直接发文字，输出识别结果、回复、松开后首句语音延迟，并把回复语音存成 WAV；有错误或某轮没有语音时非零退出。依赖 `python-socketio[client]` 和 `requests`，可装进上面的 `.cache/rockchip-e2e-python`。API 账号沿用 `RK_API_USER` / `RK_API_PASSWORD`。

```bash
ffmpeg -i in.m4a -ar 16000 -ac 1 -sample_fmt s16 question.wav   # 需要 16 kHz 单声道 16 位
python3 e2e/rockchip/chat.py --url http://<board>:18080 --wav question.wav --rounds 20
python3 e2e/rockchip/chat.py --url http://<board>:18080 --text "你好"
```

| 环境变量 | 默认 / 用途 |
|---|---|
| `RK_HOST`, `RK_USER` | 必填，不在代码中存板端地址/认证 |
| `RK_PASSWORD_FILE` | 可选，传给 sshpass -f；应设置为 0600 |
| `RK_SSH_CONFIG` | `/dev/null`；需要 SSH 跳板/密钥选项时传自己的配置 |
| `RK_API_USER`, `RK_API_PASSWORD` | 独立新库的默认 admin 账号；非默认时显式配置 |
| `RK_RTSP_PORT`, `RK_RTMP_PORT` | 8554 / 8555；API 18080 和 HTTP 8553 固定，占用时部署中止 |
| `RK_VIDEO_DEVICE` | 可选的已确认空闲且有信号的 V4L2 节点；未设置时使用显式配置的 HDMI 回环 |
| `RK_NO_SIGNAL_DEVICE` | 已确认空闲且无信号的节点；未提供则 T2.7 SKIP |
| `RK_HDMI_LOOPBACK` | 默认 1，HDMI OUT→HDMI IN 回环接线；设 0 禁用回环 |
| `RK_DRM_CARD`, `RK_HDMI_CONNECTOR` | HDMI 测试必填：按实际板端 DRM 卡和接线填写输出口，无默认值 |
| `RK_HDMI_CAPTURE_DEVICE` | 回环测试必填：实际接到所选输出口的 V4L2 采集节点，无默认值；未配置时 T2.6 / T3.3 SKIP |
| `RK_HDMI_SECOND_CONNECTOR` | T3.4 必填：同一 DRM 卡上的第二路输出口，须与第一路不同，无默认值 |
| `RK_RECORD_TIMEOUT` | 210 秒；ZLM 分段 60 秒，需要至少两个真正分段 |
| `RK_RESOURCE_SECONDS` | 每档 20 秒，最少 10；1/2/4 路 1080p 资源观察 |
| `RK_PYTHON` | python3；可指定装了 Playwright 的 Python |
| `RK_ARTIFACT_DIR` | `third_party/rockchip-e2e/<UTC时间>-<随机ID>`，包括报告和失败日志；指定目录必须为空 |
| `RK_PACKAGE` | 显式复用有匹配构建证据的包；默认重新构建 |

HTTP 与 RTSP 分别探测；任何一路不通就为两者建立 SSH 隧道，绑定主机 loopback 的空闲端口并等待 readiness。FLV 使用同源 `/media/device/<id>.live.flv`，RTSP 是 `/device/<id>`，强制 TCP。每项检查前后对比板端 boot_id，变化即 FAIL 并中止后续。

## 覆盖和断言

| 组 | 主要断言 | 对应改动 |
|---|---|---|
| T0.1–6 | 实例 PID/路径、所有端口、管理页、动态库、ELF GLIBC 需求、正反登录、OS/版本、硬编硬解注册 | 部署基线 / f27cbc7 |
| T0.7–8 | 无 X11 环境查询快速返回，添加和更新明确拒绝且不持久化 | b33e7d7 |
| T1.1–8 | CRUD；state/flv_url；空 output 省略；非法值精确文案；原生转发拒绝非空 output；拒绝更新不改旧配置；metrics；不存在设备 | 23217e1 |
| T2.1–4 | lavfi raw→H.264 真解码；1080p 缩至 720p@25；预编码 RTSP 音视频源不设 output 时编码/尺寸保持；H.265 真出流 | b33e7d7 / 23217e1 |
| T2.5–7 | FLV、HLS 分片真解码；V4L2 有信号 H.264；无信号旧管线更新和删除各 <15 秒 | b33e7d7 |
| T2.8 | 至少两个真实录像分段可解码；分段卡片自然 ended 后请求第二段并继续播放；日回放跨过真实分段边界、索引变化 | 01f3d31 |
| T2.9–11 | 合成响应 RTSP 端口等于实际配置、合成出帧；无关更新保留 detect；AAC 32kHz / Opus 48kHz 真解码 | 23217e1 |
| T3.1–2 | 显式 h264_rkmpp 编码、解码（不能以软件回退冒充） | RKMPP 基线 |
| T3.3–4 | Rust HDMI connector 枚举、present、回环视频模式和上半部彩条内容；双输出关闭隔离 | 23217e1 |
| T3.5 | 1/2/4 路 1080p 转码，RSS/load/meminfo、进程存活、各流真解码和 OOM 错误 | 资源观察 |

业务错误沿用当前 HTTP 500 和 `{code,message,data}` 包装；鉴权失败是 401。`config.output=None` 在 JSON 中省略，空对象不是其回显。T1.6 覆盖保存前拒绝更新，不声称覆盖写库后的回滚分支。output 保持比例、只缩小；lavfi 属于 raw 采集，不作为编码透传基线。

TAP 的每项包含 ID 和所守 commit；报告包含耗时、期望/实际错误信息、原始响应截断 500 字符和 nvr.log 尾部 20 行。响应和日志中的当前会话 token、JSON password/token 字段脱敏。浏览器证据只用于本次新库；不保存 storage_state。

## SKIP 和实际限制

- HDMI 没连接或无可用模式：T3.3 SKIP，报告 EDID 字节数和 modes 数量；这不证明物理 sink 不存在，也不是 Rust HDMI 库已通过。模式就绪会强制 DRM 探测并等待连续两次有效模式，最多 12 秒，以处理接收端 HPD 切换后的空缓存。回环禁用时仅执行输出，回环验收 SKIP。T2.6 默认也使用同一 Rust 彩条输出提供 HDMI IN 信号；OUT 无模式时它同样 SKIP。
- 双 HDMI 必须两路都有模式；不使用无模式的 connected 状态冒充通过。
- V4L2 节点存在不能证明有信号，也不能证明没被既有服务使用。回环输出口与采集节点按实际接线分别配置，不根据端口编号猜测映射；OUT 产生彩条后，连续三次从所选输入抓到对应彩条才启动 NVR 采集。输入未捕获到生成的图案时 SKIP，保留实际采样和输出日志。
- 有残留 DISPLAY/socket 候选时不适用 2 秒无候选快速路径，X11 添加/更新仍单独检查。
- Playwright/浏览器缺失时，录像文件已经校验后，将浏览器验收记 SKIP。浏览器能启动但真实回放失败应 FAIL。
- 资源观察是有限时长观测，不等于长期容量或无 OOM 保证。
- 普通 HEAD 测试不改生产代码的回滚行为；T1.9 仅在 `--mutate drop-rollback` 的受控应用失败注入模式验证写库后的配置和出流恢复，不验证实际目标检测推理（只守配置保留）。

## 变异验证

`--mutate` 先构建并验证原版目标组，然后临时修改指定源码、重新构建和独立部署。目标断言必须由 PASS 变 FAIL，部署基线仍 PASS，非预期 FAIL 会拒绝验收。结束或收到信号恢复原始文件内容，停止本次实例与隧道；报告保留 baseline/mutant 两份和 `mutation.json`。变异不用于日常循环，不与其他 Cargo/编辑器修改同一源文件并行。

默认建议使用 `drop-state-field`→T1.2、`drop-output-normalize`→T1.3，不依赖现场 HDMI/V4L2 信号。还支持 `drop-raw-encode`→T2.1（及依赖 raw 采集的项目）、`drop-x11-guard`→T0.8（API 接受并持久化不可用 X11 输入）。`drop-rollback`→T1.9 先在两个版本中注入相同的写库后应用失败，原版必须恢复旧配置并保持可解码出流，变异版删除持久化恢复后必须被发现；注入代码随原文件一起恢复，不加入生产构建。无信号 fixture 和共享 raw 路径会引起合理的多项失败，不能要求恰好只有一项失败。

“已验证抓到的 bug”只依据真实 `mutation.json` 记录；静态自检和 mock 自检不能充当板端变异证据。

2026-10-12，HEAD `23217e12296441067729d02bc927aac20cf7bf8a` 的真机配对记录（证据保留在本机、未入库）：

| 变异 | 原版目标 | 变异目标 | 证据目录（`third_party/rockchip-e2e/` 下） |
|---|---|---|---|
| drop-state-field | T1.2 PASS | T1.2 FAIL，其他适用项 PASS | `20261012T043049Z-14484d77/` |
| drop-output-normalize | T1.3 PASS | T1.3 FAIL，其他适用项 PASS | `20261012T043905Z-889e1b23/` |
| drop-rollback（两版注入相同应用失败） | T1.9 PASS，旧配置/出流恢复 | T1.9 FAIL，其他项 PASS | `20261012T044718Z-7acf7f5d/` |

## 脚本自检

固定版本的开发检查工具安装到独立缓存；`make rockchip-e2e-check` 在工具缺失时直接失败，不跳过 lint。Ruff 配置见 [配置说明](https://docs.astral.sh/ruff/configuration/)，本套脚本使用 Python 3.9 目标版本。

```bash
pip install --upgrade --target .cache/rockchip-e2e-tools --cache-dir .cache/pip -r e2e/rockchip/requirements-dev.txt
export PYTHONPATH="$PWD/.cache/rockchip-e2e-tools${PYTHONPATH:+:$PYTHONPATH}"
export PATH="$PWD/.cache/rockchip-e2e-tools/bin:$PATH"
make rockchip-e2e-check
```

```bash
python3 e2e/rockchip/run.py --help
for file in e2e/rockchip/board/*.sh; do sh -n "$file"; done
shellcheck e2e/rockchip/run.sh e2e/rockchip/board/*.sh
PYTHONDONTWRITEBYTECODE=1 python3 e2e/rockchip/host/suite_test.py
PYTHONDONTWRITEBYTECODE=1 python3 e2e/rockchip/host/hdmi_test.py
PYTHONDONTWRITEBYTECODE=1 python3 e2e/rockchip/lib/deploy_test.py
PYTHONDONTWRITEBYTECODE=1 python3 e2e/rockchip/lib/provenance_test.py
PYTHONDONTWRITEBYTECODE=1 python3 e2e/rockchip/lib/lifecycle_report_test.py
```

自检验证报告、脱敏、SKIP 和重启失败边界，以及显式接线与 YUV 色条采样回归。真正硬件结果以板端运行报告为准。

主机上的 lavfi 图案带 `realtime` 过滤器，避免最快生成导致队列丢帧/时间戳提前跨过采样窗口；连续采样保持 10 秒至少 200 帧的门槛。预编码源同时断言没有 encoder，防止相同 H.264 编码掩盖意外重编码。

音频转码用例用 FFmpeg `-copyts` 保留输入时间戳，单独映射视频并以 `setpts=PTS-STARTPTS` 从首帧计时解码 10 秒、至少 200 帧；`-vsync 0` 不补帧。音频另用 ffprobe 检查 codec、采样率和真实解码帧，保存 `audio-<codec>.json`。采样时间戳的注意事项见 [媒体踩坑记录](../../docs/memory/media-and-recording.md)。

## 真机验收记录

2026-10-15，20 主机运行中实例的 `live.py --group all` 回归：**20 PASS / 0 FAIL / 3 SKIP**，证据在本机 `third_party/rockchip-e2e/live-20261015T034901Z/`。覆盖 API、H.264/H.265、缩放、编码透传、FLV/HLS、真实录像和浏览器两种跨段播放、合成、检测配置保留、AAC/Opus 转码。跳过的是受控故障注入 T1.9、占用中采集节点的 T2.6，以及未提供无信号 fixture 的 T2.7；本轮不声称新增硬件全量验收。三路现有流另行只读解码，两秒分别为 121/121/41 帧，均无解码错误；结果在 `existing-streams.json`。原有三设备配置、进程 PID=14962、二进制 SHA-256 与 boot_id 均保持不变，测试 fixture 已清理。运行二进制摘要为 `42f78ef73bd3437bf35e9bf95beab35f4a801cf0381a29fb0eb145c7c7b5ab53`，与本地 Rockchip 包内二进制一致；这不替代源码构建证据。本轮未发现新问题；已有 [ZLM 生命周期崩溃 #2988](http://172.31.169.114:3456/tasks/2988) 仍等待上游修复，已指派给 benshi3，此串行回归不能证明该并发问题已修复。

2026-10-12 最终全量回归（HEAD `9493697`，包含本轮工作区修复，具体源码摘要在 `build.json`）：**33 PASS / 0 FAIL / 1 SKIP**，本机证据 `third_party/rockchip-e2e/20261012T072216Z-e028187b/report.json`。部署/API、全部媒体用例、真实录像及浏览器跨段、HDMI 回环颜色/模式/帧率、双输出关闭隔离和 1/2/4 路资源观察均通过；无板端重启，清理通过。唯一 SKIP 为普通运行未注入回滚故障的 T1.9，独立配对验证记录见上文。AAC 视频 228 帧 / 32kHz 音频 313 帧，Opus 视频 242 帧 / 48kHz 音频 500 帧，视频门槛仍为 10 秒至少 200 帧。

补正实际 sysfs 名称 `stream_hdmirx` 的无信号识别后，T2.7 专项 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T073635Z-f8c36368/`）。`no-signal-precondition.txt` 保存所选节点名称和四通道 Unlock 状态；随后管线更新和删除均在 15 秒内完成。不按 `/dev/video*` 编号推断驱动或接线。

以下保留此前排查记录，旧失败已由后续专项与上述全量复验关闭。

2026-10-12，HEAD `23217e12296441067729d02bc927aac20cf7bf8a`，默认重新构建原版后执行全量：**29 PASS / 1 FAIL / 4 SKIP**，证据为本机 `third_party/rockchip-e2e/20261012T045621Z-a6fd839c/report.json`。

- 旧采样命令的 T2.11 Opus 分支未通过视频连续性断言：实测 166 帧，要求至少 200。后续定位为采样时的时间戳调整问题，修复及最新复验见下文；AAC 32kHz 分支通过。
- 补充单项复验 `20261012T050918Z-b37789f8/`：9 PASS / 1 FAIL / 0 SKIP。`audio-aac.json` 为视频 248 帧、AAC 32kHz 音频 313 帧；`audio-opus.json` 为视频 157 帧、Opus 48kHz 音频 500 帧。音频编码和真实解码通过，失败仍是 Opus 转码时的视频连续性。
- T2.6 / T3.3：Rust HDMI OUT 已 present 1080p60 彩条，IN 四通道未稳定锁定且时序为 640x480，回环验收 SKIP。T3.4：第二路输出无可用显示模式，SKIP。
- 普通全量的 T1.9 未注入故障而 SKIP；写库后回滚已通过上面的独立配对验证，并成功捕获删除回滚的变异。
- 部署与 API、其他媒体路径、真实录像及浏览器两种跨段播放、RKMPP 硬编硬解、1/2/4 路资源观察和清理均通过；未观测到板端重启。
- 10 项 Python 自检、ShellCheck、Rockchip 包和 HDMI examples 交叉构建通过。全仓 `cargo fmt --all --check` 发现既有 Rust 格式差异；本次没有 Rust 源码差异。
- 提交前 `make test`：工作区 Rust 测试 451 项通过、9 项忽略、0 失败；Python 提交检查编译通过，因未安装 black/ruff/flake8 而跳过格式化与 lint。

2026-10-12 后续修复：T2.11 采样保留输入时间戳后，真机专项 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T060129Z-e2d9e5f6/`）。AAC 视频 229 帧、32kHz 音频 313 帧；Opus 视频 250 帧、48kHz 音频 500 帧。上述历史全量的 FAIL 已由此专项复验关闭，HDMI 验收仍需继续。以上记录不声称长期容量或无 OOM 保证。

HDMI 采集定位：本轮现场验证了 A1→video11 的接线，并在打开前等待模式恢复。T2.6 真机专项 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T061627Z-8a8df0d9/`），验证 V4L2→H.264→RTSP 的实际解码；T3.3 模式、帧率和彩条专项也为 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T062044Z-9b560f96/`）。这组现场接线不作为脚本默认映射，后续执行须显式提供实际接线配置。

取消默认接口映射后，显式配置的 T2.6 为 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T063833Z-1156353b/`）。T3.3 增加经 NVR 编码的回环颜色校验时，发现 YUV 直接缩为一行产生采样误报；改为先转 RGB 后缩放，原阈值不变，真机复验 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T065239Z-be769d9b/`）。

T3.4 取消卡和接口写死，两路模式同轮刷新并连续两次就绪；只对开始呈现前的 `NoConnector` 做最多三次等待，其他错误以及持续启动失败仍 FAIL，启动失败证据另存 `dual-startup-attempts.log`。真机专项 **10 PASS / 0 FAIL / 0 SKIP**（`20261012T070853Z-ef9180f7/`），验证双路呈现及关闭第一路后第二路仍可呈现。

修复后的本地检查：`make rockchip-e2e-check` 通过 Ruff lint、格式检查、ShellCheck 和 15 项 Python 测试；所有 Python 文件通过 3.9 语法解析。整理既有 Rust 格式差异后，`cargo fmt --all --check`、工作区编译检查和 `make test` 均通过（451 PASS / 9 ignored / 0 FAIL）。

本轮 `make test` 的 `nvr` 并发测试进程曾收到 SIGSEGV，首次日志为 `workspace-tests-after-fixes.log`，提交前又在 `commit-workspace-tests.log` 复现。串行 99 项、调试器下并发运行、原始并发重复运行及最新工作区全量复验均通过；专项 Memcheck 未发现内存错误。崩溃仍有间歇性，根因未确认，不宣称已修复；最新通过证据为本机 `third_party/rockchip-e2e/native-core-full-test.log`，排查日志保留在同目录。临时诊断源码已恢复。
