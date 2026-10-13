# nvr-hdmiout

Rockchip HDMI 视频与音频输出，视频直接使用 Linux DRM/KMS，音频使用 ALSA，不依赖 X11、Wayland、ZLM 或 FFmpeg。

- `list_connectors(card)`：探测 HDMI 连接状态与可用显示模式。
- `HdmiOutput::open(&config)`：选择连接器和模式，默认 `/dev/dri/card0` 的已连接 HDMI 及首选模式。
- `present(VideoFrame)`：双缓冲、等待垂直刷新切换，超时后需关闭并重新打开。
- `open_sibling(connector, mode)`：在同一个 card 上开启另一个 HDMI 输出，共享 DRM master，并串行处理翻页事件。
- `close()` / Drop：恢复原 CRTC 状态；最后一个输出关闭后释放 DRM master。显式 `close()` 可报告恢复错误。

输入帧尺寸必须与输出模式一致，格式为 XRGB8888（Rockchip 小端内存排列为 B、G、R、X）；支持行 padding。缩放、解码及像素格式转换由调用方完成。输出方法是阻塞方法，异步应用应放在专用线程中调用。需要 DRM 节点权限及 DRM master，不能与占用同一 card 的桌面合成器同时输出。非 Linux 返回 `Unsupported`。

```bash
cargo run -p nvr-hdmiout --example hdmiout -- list
# 输出彩条与移动标记，30 秒后退出
cargo run -p nvr-hdmiout --example hdmiout -- pattern /dev/dri/card0 HDMI-A-1 30
```

示例也可接收 stdin 的完整 `bgr0` 原始帧。先用 `list` 确认输出模式，再让 FFmpeg 输出相同尺寸；例如首选模式为 1080p60 时：

```bash
ffmpeg -re -i input.mp4 -an -vf scale=1920:1080 -pix_fmt bgr0 -f rawvideo - |
  target/debug/examples/hdmiout stdin /dev/dri/card0 HDMI-A-1 30
```

Docker 中运行需映射 `/dev/dri/card0`，并具备节点权限；HDMI OUT→HDMI IN 回环抓帧还需映射对应 `/dev/video*`。

验证：`cargo test -p nvr-hdmiout`。Linux 主机设置 `NVR_HDMI_TEST_CARD=/dev/dri/card0` 和可选的 `NVR_HDMI_TEST_CONNECTOR=HDMI-A-1` 可运行真实硬件测试（短暂输出黑画面、关闭及重开）；未设置时跳过硬件测试。

双输出示例（OUT 1 红色、OUT 2 蓝色，均带移动白条）：

先按实际接线设置 `DRM_CARD`、`FIRST_CONNECTOR` 和 `SECOND_CONNECTOR`，两路 connector 须不同且属于同一 DRM 卡。

```bash
cargo run -p nvr-hdmiout --example dual_hdmiout -- 300 "$DRM_CARD" "$FIRST_CONNECTOR" "$SECOND_CONNECTOR"
```

硬件测试另设 `NVR_HDMI_TEST_SIBLING=HDMI-A-2` 可验证双输出，以及关闭一路后另一路继续输出。

## 音频输出

`AudioOutput::open(&AudioConfig)` 打开指定 ALSA 播放设备；`write(&[i16])` 接收完整的交错 PCM 帧，默认 48000 Hz、双声道。音频写入放在专用线程，`close()` 丢弃排队音频并立即停止，非 Linux 返回 `UnsupportedAudio`。HDMI 音频需要对应 HDMI 视频输出已启用。Linux 构建需要 `libasound2-dev`，运行需要 ALSA 库及 `/dev/snd` 权限。

双路视频与测试音（OUT 1 为 440 Hz，OUT 2 为 880 Hz）：

```bash
dual_hdmiout 0 /dev/dri/card0 HDMI-A-1 HDMI-A-2 \
  hw:CARD=rockchiphdmi0,DEV=0 hw:CARD=rockchiphdmi1,DEV=0
```

设备名称须以主机 `/proc/asound/cards` 为准。时长 `0` 表示持续运行，SIGINT / SIGTERM 会停止并回收音频线程、恢复视频状态；不传 ALSA 参数仍仅输出视频。单路示例可在 `pattern` 的时长后追加 ALSA 设备与可选音调频率。Docker 需额外映射 `/dev/snd`。
