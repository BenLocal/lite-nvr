use std::fmt::{Display, Formatter};
/// Input video format (e.g. v4l2, lavfi). Use `input_spec()` to get both `-f` and `-i` parameters.
pub struct VideoDeviceFormat {
    inner: ffmpeg_next::Format,
}

impl VideoDeviceFormat {
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

    pub fn mime_types(&self) -> Vec<&str> {
        self.inner.mime_types()
    }
}

impl Display for VideoDeviceFormat {
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

pub struct AudioDeviceFormat {
    inner: ffmpeg_next::Format,
}

impl AudioDeviceFormat {
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

    pub fn mime_types(&self) -> Vec<&str> {
        self.inner.mime_types()
    }
}

impl Display for AudioDeviceFormat {
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

pub fn input_video_format_list() -> anyhow::Result<Vec<VideoDeviceFormat>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::input::video() {
        devices.push(VideoDeviceFormat::new(device));
    }
    Ok(devices)
}

pub fn output_video_format_list() -> anyhow::Result<Vec<VideoDeviceFormat>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::output::video() {
        devices.push(VideoDeviceFormat::new(device));
    }
    Ok(devices)
}

pub fn input_audio_format_list() -> anyhow::Result<Vec<AudioDeviceFormat>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::input::audio() {
        devices.push(AudioDeviceFormat::new(device));
    }
    Ok(devices)
}

pub fn output_audio_format_list() -> anyhow::Result<Vec<AudioDeviceFormat>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::output::audio() {
        devices.push(AudioDeviceFormat::new(device));
    }
    Ok(devices)
}
