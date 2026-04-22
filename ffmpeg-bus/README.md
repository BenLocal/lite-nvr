# ffmpeg-bus

异步 FFmpeg 媒体处理引擎，基于 Tokio 和 async channels 构建。

Async FFmpeg media processing engine built on Tokio and async channels.

## 概述 Overview

`ffmpeg-bus` 是一个 Rust 库，提供了基于 FFmpeg 的异步媒体管道。它通过 `Bus` 核心组件协调输入、解码、编码和输出，所有组件通过异步通道通信。

`ffmpeg-bus` is a Rust library that provides an async media pipeline based on FFmpeg. It coordinates input, decoding, encoding, and output through a central `Bus` component, with all components communicating via async channels.

## 架构 Architecture

```
                    ┌─────────────────────────────────────┐
                    │              Bus                    │
                    │  (Command dispatcher & coordinator) │
                    └─────────────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        │                           │                           │
        ▼                           ▼                           ▼
┌───────────────┐          ┌───────────────┐          ┌───────────────┐
│  Input Task   │          │ Decoder Task  │          │ Encoder Task  │
│  (demuxer)    │─────────▶│  (decode)     │─────────▶│  (encode)     │
└───────────────┘          └───────────────┘          └───────────────┘
        │                           │                           │
        │                           │                           │
        └───────────────────────────┴───────────────────────────┘
                                    │
                                    ▼
                          ┌───────────────────┐
                          │   Output Tasks    │
                          │ (mux/file/net/raw)│
                          └───────────────────┘
```

## 核心组件 Core Components

| 模块 Module      | 描述 Description                                                                 |
| ---------------- | -------------------------------------------------------------------------------- |
| **bus**          | 核心调度器，管理所有任务的生命周期和通信<br>Core dispatcher managing task lifecycle and communication |
| **input**        | 输入解复用器，支持网络流、文件、设备<br>Input demuxer supporting network streams, files, devices |
| **decoder**      | 视频/音频解码器任务<br>Video/audio decoder task |
| **encoder**      | 视频/音频编码器任务（libx264, etc.）<br>Video/audio encoder task (libx264, etc.) |
| **output**       | 输出复用器，支持文件、网络、原始流<br>Output muxer supporting file, network, raw streams |
| **frame**        | 视频帧处理和转换<br>Video frame processing and conversion |
| **packet**       | 数据包处理和转换<br>Packet processing and conversion |
| **scaler**       | 视频缩放和像素格式转换<br>Video scaling and pixel format conversion |
| **bsf**          | 比特流过滤器（h264_mp4toannexb, etc.）<br>Bitstream filters (h264_mp4toannexb, etc.) |
| **audio_mixer**  | 音频混音器<br>Audio mixer |
| **device**       | 设备枚举和管理<br>Device enumeration and management |

## 使用示例 Usage Example

### 基本管道 Basic Pipeline

```rust
use ffmpeg_bus::{Bus, input::AvInput, output::{AvOutput, OutputDest, OutputAvType}};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化 FFmpeg
    // Initialize FFmpeg
    ffmpeg_bus::init()?;

    // 创建 Bus 实例
    // Create Bus instance
    let bus = Bus::new("pipeline-1");

    // 添加输入（RTSP 流）
    // Add input (RTSP stream)
    let input = AvInput {
        url: "rtsp://192.168.1.100:554/stream".to_string(),
        format: None,
        options: Default::default(),
    };
    bus.add_input(input, None).await?;

    // 添加输出（文件）
    // Add output (file)
    let output = AvOutput {
        id: "output-1".to_string(),
        av_type: OutputAvType::Video,
        dest: OutputDest::File {
            path: "/tmp/output.mp4".to_string(),
        },
        encode: None,
    };
    let (av_output, stream) = bus.add_output(output).await?;

    // 等待处理完成
    // Wait for processing to complete
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
```

### 转码管道 Transcoding Pipeline

```rust
use ffmpeg_bus::{
    Bus,
    input::AvInput,
    output::{AvOutput, OutputDest, OutputAvType},
    encoder::Settings,
};

let bus = Bus::new("transcode");

// 输入
let input = AvInput {
    url: "input.mp4".to_string(),
    format: None,
    options: Default::default(),
};
bus.add_input(input, None).await?;

// 输出（带编码设置）
let output = AvOutput {
    id: "encoded".to_string(),
    av_type: OutputAvType::Video,
    dest: OutputDest::File {
        path: "output.mp4".to_string(),
    },
    encode: Some(Settings {
        preset: Some("fast".to_string()),
        bitrate: Some(2_000_000),
        width: Some(1920),
        height: Some(1080),
        ..Default::default()
    }),
};
bus.add_output(output).await?;
```

### 多输出管道 Multi-Output Pipeline

```rust
// 一个输入，多个输出
// One input, multiple outputs
let bus = Bus::new("multi-out");

bus.add_input(input, None).await?;

// 输出 1: 文件
bus.add_output(AvOutput {
    id: "file-out".to_string(),
    av_type: OutputAvType::Video,
    dest: OutputDest::File { path: "out1.mp4".to_string() },
    encode: None,
}).await?;

// 输出 2: RTSP 流
bus.add_output(AvOutput {
    id: "rtsp-out".to_string(),
    av_type: OutputAvType::Video,
    dest: OutputDest::Net {
        url: "rtsp://localhost:8554/live".to_string(),
        format: Some("rtsp".to_string()),
    },
    encode: None,
}).await?;
```

## 命令 Commands

```bash
# 运行所有测试
# Run all tests
cargo test

# 运行特定测试
# Run specific test
cargo test bus_test

# 构建
# Build
cargo build

# 检查
# Check
cargo check

# 查看文档
# View documentation
cargo doc --open
```

## 特性 Features

- ✅ 异步架构（Tokio）
- ✅ 多输入源支持（网络流、文件、设备）
- ✅ 实时转码（H.264/libx264）
- ✅ 多输出支持（文件、网络、原始流）
- ✅ 硬件加速支持（通过 `hw` 模块）
- ✅ 比特流过滤
- ✅ 音频混音
- ✅ 视频缩放和格式转换

## 依赖 Dependencies

- **ffmpeg-next** (7.x) - FFmpeg Rust 绑定
- **tokio** - 异步运行时
- **futures** - 异步流和工具
- **anyhow** - 错误处理
- **bytes** - 字节缓冲区
- **log** - 日志记录

## 测试 Testing

测试文件与源代码并置：

Test files are colocated with source code:

- `bus_test.rs` - Bus 核心功能测试
- `frame_test.rs` - 帧处理测试
- `bsf_test.rs` - 比特流过滤器测试
- `audio_mixer_test.rs` - 音频混音器测试

## 许可证 License

MIT
