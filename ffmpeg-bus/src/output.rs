use ffmpeg_next::Dictionary;

pub struct AvOutput {
    inner: ffmpeg_next::format::context::Output,
}

impl AvOutput {
    pub fn new(
        url: &str,
        _format: Option<String>,
        _options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let output = ffmpeg_next::format::output(url)?;
        Ok(Self { inner: output })
    }
}
