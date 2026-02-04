use std::collections::HashMap;

use ffmpeg_next::{Dictionary, Rational};

use crate::{packet::RawPacket, stream::AvStream};

pub struct AvOutput {
    inner: ffmpeg_next::format::context::Output,
    streams: HashMap<usize, AvStream>,
    interleaved: bool,
    have_written_header: bool,
    have_written_trailer: bool,
}

impl AvOutput {
    pub fn new(
        url: &str,
        _format: Option<String>,
        _options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let output = ffmpeg_next::format::output(url)?;
        Ok(Self {
            inner: output,
            streams: HashMap::new(),
            interleaved: false,
            have_written_header: false,
            have_written_trailer: false,
        })
    }

    pub fn add_stream(&mut self, stream: &AvStream) -> anyhow::Result<()> {
        let codec_parameters = stream.parameters();
        let mut writer_stream = self
            .inner
            .add_stream(ffmpeg_next::encoder::find(codec_parameters.id()))?;
        writer_stream.set_parameters(codec_parameters.clone());
        self.streams.insert(stream.index(), stream.clone());
        Ok(())
    }

    fn stream_time_base(&mut self, stream_index: usize) -> Rational {
        self.streams.get(&stream_index).unwrap().time_base()
    }

    pub fn write_packet(
        &mut self,
        writer_stream_index: usize,
        mut packet: RawPacket,
    ) -> anyhow::Result<()> {
        if !self.have_written_header {
            self.inner.write_header()?;
            self.have_written_header = true;
        }
        let time_base = packet.time_base();

        let p = packet.get_mut();
        let destination_stream = match self.streams.get(&writer_stream_index) {
            Some(stream) => stream,
            None => return Err(anyhow::anyhow!("stream not found")),
        };
        p.set_stream(destination_stream.index());
        p.set_position(-1);
        let out_time_base = self.inner.stream(writer_stream_index).unwrap().time_base();
        p.rescale_ts(time_base, out_time_base);
        if self.interleaved {
            p.write_interleaved(&mut self.inner)?;
        } else {
            p.write(&mut self.inner)?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.have_written_header && !self.have_written_trailer {
            self.have_written_trailer = true;
            self.inner.write_trailer()?;
        }
        Ok(())
    }
}
