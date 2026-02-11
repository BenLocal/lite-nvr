use std::fmt::{Display, Formatter};

pub struct VideoDevice {
    inner: ffmpeg_next::Format,
}

impl VideoDevice {
    fn new(inner: ffmpeg_next::Format) -> Self {
        Self { inner }
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn extensions(&self) -> Vec<&str> {
        self.inner.extensions()
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

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn extensions(&self) -> Vec<&str> {
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

pub fn input_video_list() -> anyhow::Result<Vec<VideoDevice>> {
    let mut devices = Vec::new();
    for device in ffmpeg_next::device::input::video() {
        devices.push(VideoDevice::new(device));
    }
    Ok(devices)
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
