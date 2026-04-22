# lite-nvr

轻量级网络视频录像机（NVR），基于 Rust 构建。支持从多种来源（RTSP、文件、屏幕捕获、测试图案、V4L2 设备）采集视频，通过 FFmpeg 转码，并推送到 [ZLMediaKit](https://github.com/ZLMediaKit/ZLMediaKit) 进行 RTSP/RTMP/HLS 分发。

A lightweight Network Video Recorder built with Rust. Ingests video from RTSP, files, screen capture, test patterns, and V4L2 devices, transcodes via FFmpeg, and distributes through ZLMediaKit for RTSP/RTMP/HLS streaming.

## 特性 Features

- 🎥 多种输入源：RTSP/RTMP 网络流、本地文件、屏幕捕获、V4L2 设备、测试图案
- 🔄 基于 FFmpeg 的实时转码（H.264/libx264）
- 📡 通过 ZLMediaKit 分发流媒体（RTSP/RTMP/HLS）
- 🌐 RESTful API 和 Vue 3 Web 控制台
- 💾 SQLite 数据库持久化
- ⚡ 异步管道架构（Tokio + async channels）

## 架构 Architecture

```
┌──────────┐    ┌───────────┐    ┌─────────┐    ┌─────────────┐
│  Input   │───▶│  Decoder  │───▶│ Encoder │───▶│   Output    │
│ (source) │    │ (ffmpeg)  │    │(libx264)│    │(ZLM/file/…) │
└──────────┘    └───────────┘    └─────────┘    └─────────────┘
```

### 工作空间结构 Workspace Crates

| Crate             | 描述 Description                                                                 |
| ----------------- | -------------------------------------------------------------------------------- |
| **nvr**           | 主应用 — REST API、管道管理、ZLMediaKit 集成<br>Main app — REST API, pipeline management, ZLMediaKit integration |
| **ffmpeg-bus**    | 媒体引擎 — 输入解复用、解码器、编码器、复用器，通过异步通道连接<br>Media engine — input demux, decoder, encoder, muxer via async channels |
| **nvr-db**        | 数据库层 — SQLite/Turso，内嵌 SQL 迁移，KV 存储<br>Database layer — SQLite/Turso with embedded migrations, KV store |
| **nvr-dashboard** | Web 控制台 — Vue 3 SPA，通过 rust-embed 嵌入<br>Web dashboard — Vue 3 SPA embedded via rust-embed |

## 快速开始 Quick Start

### 前置要求 Prerequisites

- **Rust** (edition 2024)
- **FFmpeg 7.x** 共享库（头文件 + 库文件）
- **ZLMediaKit**（可选，默认通过 `zlm` feature 启用）

### 1. 安装依赖 Install Dependencies

```bash
# 自动下载适合您平台的 FFmpeg 和 ZLMediaKit
# Auto-download FFmpeg & ZLMediaKit for your platform
bash scripts/pre_install_deps.sh
```

在 macOS 上，此脚本通过 Homebrew 安装 `ffmpeg@7` 并在 `./ffmpeg/` 创建符号链接。

### 2. 构建和运行 Build & Run

```bash
# 构建所有 crates
# Build all crates
cargo build --workspace

# 运行主应用（API 服务器在 :8080）
# Run main app (API server on :8080)
cargo run --package nvr
```

API 服务器默认启动在 `http://localhost:8080`。

Web 控制台访问：`http://localhost:8080/nvr/`

### 3. 创建管道 Create a Pipeline

```bash
# RTSP 输入 → ZLMediaKit 输出
# RTSP input → ZLMediaKit output
curl -X POST http://localhost:8080/pipe/add \
  -H "Content-Type: application/json" \
  -d '{
    "id": "cam1",
    "input": { "t": "net", "i": "rtsp://192.168.1.100:554/stream" },
    "outputs": [{
      "t": "zlm",
      "zlm": { "app": "live", "stream": "cam1" }
    }]
  }'
```

播放流：

```bash
# 通过 ffplay 播放
ffplay rtsp://127.0.0.1:8554/live/cam1

# 或通过 HLS (需要等待几秒生成切片)
# Or via HLS (wait a few seconds for segments)
ffplay http://127.0.0.1:8080/live/cam1/hls.m3u8
```

## REST API

### 管道管理 Pipeline Management

| 方法 Method | 端点 Endpoint       | 描述 Description           |
| ----------- | ------------------- | -------------------------- |
| GET         | `/pipe/list`        | 列出活动管道 List active pipelines |
| POST        | `/pipe/add`         | 创建新管道 Create a new pipeline |
| GET         | `/pipe/remove/{id}` | 移除管道 Remove a pipeline     |
| GET         | `/pipe/status/{id}` | 获取管道状态 Get pipeline status |

### 输入类型 Input Types

| 类型 Type           | `t` 字段 | `i` 字段示例 Example                      |
| ------------------- | -------- | ----------------------------------------- |
| 网络流 Network      | `net`    | `rtsp://host:554/path`                    |
| 文件 File           | `file`   | `/path/to/video.mp4`                      |
| 屏幕捕获 Screen     | `x11grab`| `:99`                                     |
| 测试图案 Test       | `lavfi`  | `testsrc=size=1920x1080:rate=10,realtime` |
| V4L2 设备 Device    | `v4l2`   | `/dev/video0`                             |

### 输出类型 Output Types

| 类型 Type       | `t` 字段 | 配置 Config                                        |
| --------------- | -------- | -------------------------------------------------- |
| ZLMediaKit      | `zlm`    | `{ "app": "live", "stream": "cam1" }`              |
| 网络流 Network  | (default)| `{ "net": { "url": "rtsp://...", "format": "rtsp" } }` |
| 文件 File       | `file`   | `{ "file": { "path": "/path/to/output.mp4" } }`    |
| Sink (测试)     | `sink`   | `{}`                                               |

### 编码选项 Encode Options (可选)

```json
{
  "encode": {
    "preset": "ultrafast",
    "bitrate": 2000000,
    "width": 1920,
    "height": 1080
  }
}
```

预设 Presets: `ultrafast`, `superfast`, `veryfast`, `fast`, `medium`（越慢质量越好）

## 开发 Development

### 测试 Testing

```bash
# 运行所有测试
# Run all tests
cargo test --workspace --lib --tests --no-fail-fast

# 运行单个 crate 的测试
# Run tests for a single crate
cargo test -p nvr
cargo test -p ffmpeg-bus

# 代码格式化
# Format code
cargo fmt

# 快速编译检查
# Fast compile check
cargo check --workspace
```

### 前端开发 Frontend Development

```bash
cd nvr-dashboard/app

# 安装依赖
# Install dependencies
npm ci

# 开发服务器（热重载）
# Dev server with hot reload
npm run dev

# 构建生产版本（嵌入到 Rust 二进制）
# Build for production (embedded into Rust binary)
npm run build

# 类型检查和 lint
# Type check and lint
npm run type-check
npm run lint
```

## 环境变量 Environment Variables

| 变量 Variable | 描述 Description                                                 |
| ------------- | ---------------------------------------------------------------- |
| `RUST_LOG`    | 日志级别过滤器 Log level filter (e.g. `info`, `debug`, `ffmpeg_bus=debug`) |
| `FFMPEG_DIR`  | FFmpeg 安装路径 Path to FFmpeg installation (default: `./ffmpeg`) |
| `ZLM_DIR`     | ZLMediaKit 安装路径 Path to ZLMediaKit installation (default: `./zlm`) |
| `LD_LIBRARY_PATH` | 运行时库路径，需包含 `ffmpeg/lib` 和 `zlm/lib`<br>Runtime library path, must include `ffmpeg/lib` and `zlm/lib` |

## 配置 Configuration

运行时配置存储在 SQLite 数据库的 `kvs` 表中。可通过 REST API 的 `/system/config` 端点管理。

Runtime configuration is stored in the `kvs` table of the SQLite database. Manage via the `/system/config` REST API endpoint.

## 许可证 License

MIT
