use std::fmt::{Display, Formatter};

#[derive(Clone, Debug)]
pub struct VideoInputSpec {
    /// Format name for `-f` (e.g. "v4l2", "lavfi").
    pub format: String,
    /// Description of this input format.
    pub description: String,
    /// Possible values for `-i` (device path, lavfi graph, etc.). Empty if not enumerable.
    pub inputs: Vec<String>,
}

impl VideoInputSpec {
    pub fn format(&self) -> &str {
        &self.format
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn inputs(&self) -> &[String] {
        &self.inputs
    }
}

/// Input video format (e.g. v4l2, lavfi). Use `input_spec()` to get both `-f` and `-i` parameters.
pub struct VideoDevice {
    inner: ffmpeg_next::Format,
}

impl VideoDevice {
    fn new(inner: ffmpeg_next::Format) -> Self {
        Self { inner }
    }

    pub fn name(&self) -> &str {
        self.inner.name()
    }

    pub fn description(&self) -> &str {
        self.inner.description()
    }

    pub fn extensions(&self) -> Vec<&str> {
        self.inner.extensions()
    }

    pub fn input_spec(&self) -> VideoInputSpec {
        let format = self.name().to_string();
        let description = self.description().to_string();
        let inputs = inputs_for_format(self.name());
        VideoInputSpec {
            format,
            description,
            inputs,
        }
    }
}

/// Resolve possible `-i` values for a given format name.
fn inputs_for_format(format_name: &str) -> Vec<String> {
    match format_name {
        #[cfg(target_os = "linux")]
        "v4l2" => v4l2_device_paths()
            .map(|paths| {
                paths
                    .into_iter()
                    .filter_map(|p| p.into_os_string().into_string().ok())
                    .collect()
            })
            .unwrap_or_default(),
        #[cfg(not(target_os = "linux"))]
        "v4l2" => Vec::new(),
        "lavfi" => vec![
            "color=c=blue:s=1280x720".to_string(),
            "testsrc=duration=5".to_string(),
        ],
        _ => Vec::new(),
    }
}

impl Display for VideoDevice {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "name: {}, description: {}, extensions: {:?}",
            self.name(),
            self.description(),
            self.extensions()
        )?;
        Ok(())
    }
}

pub struct AudioDevice {
    inner: ffmpeg_next::Format,
}

impl AudioDevice {
    fn new(inner: ffmpeg_next::Format) -> Self {
        Self { inner }
    }

    pub fn name(&self) -> &str {
        self.inner.name()
    }

    pub fn description(&self) -> &str {
        self.inner.description()
    }

    pub fn extensions(&self) -> Vec<&str> {
        self.inner.extensions()
    }
}

impl Display for AudioDevice {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "name: {}, description: {}, extensions: {:?}",
            self.name(),
            self.description(),
            self.extensions()
        )?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn v4l2_device_paths() -> anyhow::Result<Vec<std::path::PathBuf>> {
    use std::path::PathBuf;
    let mut paths: Vec<PathBuf> = std::fs::read_dir("/dev")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("video"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    Ok(paths)
}

pub fn input_video_list() -> anyhow::Result<Vec<VideoDevice>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::input::video() {
        devices.push(VideoDevice::new(device));
    }
    Ok(devices)
}

pub fn input_video_specs() -> anyhow::Result<Vec<VideoInputSpec>> {
    input_video_list().map(|devices| devices.iter().map(VideoDevice::input_spec).collect())
}

pub fn output_video_list() -> anyhow::Result<Vec<VideoDevice>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::output::video() {
        devices.push(VideoDevice::new(device));
    }
    Ok(devices)
}

pub fn input_audio_list() -> anyhow::Result<Vec<AudioDevice>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::input::audio() {
        devices.push(AudioDevice::new(device));
    }
    Ok(devices)
}

pub fn output_audio_list() -> anyhow::Result<Vec<AudioDevice>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::output::audio() {
        devices.push(AudioDevice::new(device));
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_video_list() {
        let devices = input_video_list().unwrap();
        for device in devices.iter() {
            println!("{}", device);
        }
        assert!(!devices.is_empty());
    }

    #[test]
    fn test_output_video_list() {
        let devices = output_video_list().unwrap();
        for device in devices.iter() {
            println!("{}", device);
        }
        assert!(!devices.is_empty());
    }

    #[test]
    fn test_input_audio_list() {
        let devices = input_audio_list().unwrap();
        for device in devices.iter() {
            println!("{}", device);
        }
        assert!(!devices.is_empty());
    }

    #[test]
    fn test_output_audio_list() {
        let devices = output_audio_list().unwrap();
        for device in devices.iter() {
            println!("{}", device);
        }
        assert!(!devices.is_empty());
    }
}
