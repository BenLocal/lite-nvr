use serde::Serialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command;

#[derive(Debug, Serialize)]
pub(crate) struct X11Display {
    display: String,
    current: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct X11Environment {
    pub(crate) available: bool,
    pub(crate) reason: String,
    pub(crate) displays: Vec<X11Display>,
}

fn x11_displays(sockets: &[String], current: Option<&str>) -> Vec<X11Display> {
    let current = current.map(str::trim).filter(|d| !d.is_empty());
    let mut numbers: Vec<u32> = sockets
        .iter()
        .filter_map(|name| name.strip_prefix('X')?.parse().ok())
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let mut displays = Vec::new();
    if let Some(display) = current {
        displays.push(X11Display {
            display: display.to_owned(),
            current: true,
        });
    }
    for n in numbers {
        let display = format!(":{n}");
        if !current.is_some_and(|c| c == display || c.strip_suffix(".0") == Some(display.as_str()))
        {
            displays.push(X11Display {
                display,
                current: false,
            });
        }
    }
    displays
}

fn ffmpeg_path() -> PathBuf {
    if let Ok(root) = std::env::var("FFMPEG_DIR") {
        let path = PathBuf::from(root).join("bin/ffmpeg");
        if path.is_file() {
            return path;
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(root) = exe.parent().and_then(|p| p.parent())
    {
        let path = root.join("ffmpeg/bin/ffmpeg");
        if path.is_file() {
            return path;
        }
    }
    PathBuf::from("ffmpeg")
}

// Probe in a child process: FFmpeg's optional XCB loader can abort on missing
// libraries. A successful probe must produce a frame, not merely exit zero.
async fn probe(program: &std::path::Path, display: &str, timeout: Duration) -> bool {
    let result = tokio::time::timeout(
        timeout,
        Command::new(program)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "x11grab",
                "-i",
                display,
                "-frames:v",
                "1",
                "-vf",
                "scale=1:1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "-",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    matches!(result, Ok(Ok(output)) if output.status.success() && output.stdout.len() == 3)
}

pub(crate) async fn environment() -> X11Environment {
    let mut sockets = Vec::new();
    if cfg!(target_os = "linux") {
        if let Ok(mut dir) = tokio::fs::read_dir("/tmp/.X11-unix").await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                sockets.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    let current = std::env::var("DISPLAY").ok();
    let candidates = if cfg!(target_os = "linux") {
        x11_displays(&sockets, current.as_deref())
    } else {
        Vec::new()
    };
    let had_candidates = !candidates.is_empty();
    let program = ffmpeg_path();
    let mut displays = Vec::new();
    for display in candidates {
        if probe(&program, &display.display, Duration::from_secs(3)).await {
            displays.push(display);
        }
    }
    let available = !displays.is_empty();
    let reason = if available {
        ""
    } else if had_candidates {
        "当前 X11 显示环境不可访问或缺少采集依赖，无法添加 X11 设备"
    } else {
        "当前 NVR 运行环境没有 X11 显示服务，无法添加 X11 设备"
    }
    .to_owned();
    X11Environment {
        available,
        reason,
        displays,
    }
}

pub(crate) async fn validate_input(input_type: &str, display: &str) -> anyhow::Result<()> {
    if input_type != "x11grab" {
        return Ok(());
    }
    let environment = environment().await;
    if !environment.available {
        anyhow::bail!("{}", environment.reason);
    }
    if !probe(&ffmpeg_path(), display, Duration::from_secs(3)).await {
        anyhow::bail!("当前 NVR 无法访问 X11 显示环境 {display}，不能添加或启动该设备");
    }
    Ok(())
}

#[cfg(test)]
#[path = "x11_test.rs"]
mod x11_test;
