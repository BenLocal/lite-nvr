pub struct Scaler {
    context: ffmpeg_next::software::scaling::Context,
}

impl Scaler {
    pub fn new(context: ffmpeg_next::software::scaling::Context) -> Self {
        Self { context }
    }

    pub fn run(
        &mut self,
        frame: &ffmpeg_next::frame::Video,
        dst: &mut ffmpeg_next::frame::Video,
    ) -> anyhow::Result<()> {
        self.context.run(frame, dst).map_err(|e| e.into())
    }
}

unsafe impl Send for Scaler {}
