use anyhow::Result;
use bytes::Bytes;
use ffmpeg_next::{
    Packet, Rational,
    codec::Parameters,
    ffi::{
        AVBSFContext, av_bsf_alloc, av_bsf_free, av_bsf_get_by_name, av_bsf_init,
        av_bsf_receive_packet, av_bsf_send_packet,
    },
};
use std::ptr;

use crate::packet::RawPacket;

/// Check if the codec parameters indicate AVCC/HVCC format (needs conversion to Annex B).
///
/// Returns true if the stream is in MP4 container format and needs BSF conversion.
/// Returns false if already in Annex B format or format cannot be determined.
pub fn needs_annexb_conversion(codec_params: &Parameters) -> bool {
    unsafe {
        let extradata = codec_params.extradata();
        if extradata.is_none() || extradata.unwrap().is_empty() {
            // No extradata usually means already Annex B or raw stream
            return false;
        }

        let extradata = extradata.unwrap();
        if extradata.len() < 4 {
            return false;
        }

        // AVCC format detection:
        // - First byte is typically 0x01 (configurationVersion)
        // - For AVCC, extradata starts with configuration box, not start codes
        // Annex B format detection:
        // - Starts with 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01 (start codes)

        // Check for Annex B start codes
        if (extradata[0] == 0x00
            && extradata[1] == 0x00
            && extradata[2] == 0x00
            && extradata[3] == 0x01)
            || (extradata[0] == 0x00 && extradata[1] == 0x00 && extradata[2] == 0x01)
        {
            // Already Annex B format
            return false;
        }

        // AVCC typically has configurationVersion = 1 as first byte
        // and the structure is well-defined
        if extradata[0] == 0x01 && extradata.len() >= 7 {
            // Likely AVCC format, needs conversion
            return true;
        }

        // HVCC (HEVC) also starts with configurationVersion = 1
        // Conservative approach: if we're not sure, don't convert
        false
    }
}

/// Check if packet data is in Annex B format by looking at the start codes.
pub fn is_annexb_packet(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // Check for 4-byte start code: 0x00 0x00 0x00 0x01
    if data[0] == 0x00 && data[1] == 0x00 && data[2] == 0x00 && data[3] == 0x01 {
        return true;
    }

    // Check for 3-byte start code: 0x00 0x00 0x01
    if data[0] == 0x00 && data[1] == 0x00 && data[2] == 0x01 {
        return true;
    }

    false
}

/// Bitstream Filter for converting H.264/H.265 from MP4 (AVCC) to Annex B format.
/// This is required for ZLMediaKit and other streaming servers that expect Annex B format.
pub struct BitstreamFilter {
    ctx: *mut AVBSFContext,
    time_base: Rational,
}

unsafe impl Send for BitstreamFilter {}

impl BitstreamFilter {
    /// Create a new H.264 MP4 to Annex B bitstream filter.
    ///
    /// # Arguments
    /// * `codec_params` - Codec parameters from the input stream (contains extradata with SPS/PPS)
    /// * `time_base` - Time base for the stream
    pub fn new_h264_mp4toannexb(codec_params: &Parameters, time_base: Rational) -> Result<Self> {
        unsafe {
            let filter_name = std::ffi::CString::new("h264_mp4toannexb")?;
            let bsf = av_bsf_get_by_name(filter_name.as_ptr());
            if bsf.is_null() {
                return Err(anyhow::anyhow!("h264_mp4toannexb filter not found"));
            }

            let mut ctx: *mut AVBSFContext = ptr::null_mut();
            let ret = av_bsf_alloc(bsf, &mut ctx);
            if ret < 0 {
                return Err(anyhow::anyhow!("av_bsf_alloc failed: {}", ret));
            }

            // Copy codec parameters (includes extradata with SPS/PPS)
            ffmpeg_next::ffi::avcodec_parameters_copy((*ctx).par_in, codec_params.as_ptr());

            let ret = av_bsf_init(ctx);
            if ret < 0 {
                av_bsf_free(&mut ctx);
                return Err(anyhow::anyhow!("av_bsf_init failed: {}", ret));
            }

            Ok(Self { ctx, time_base })
        }
    }

    /// Create a new H.265 MP4 to Annex B bitstream filter.
    pub fn new_hevc_mp4toannexb(codec_params: &Parameters, time_base: Rational) -> Result<Self> {
        unsafe {
            let filter_name = std::ffi::CString::new("hevc_mp4toannexb")?;
            let bsf = av_bsf_get_by_name(filter_name.as_ptr());
            if bsf.is_null() {
                return Err(anyhow::anyhow!("hevc_mp4toannexb filter not found"));
            }

            let mut ctx: *mut AVBSFContext = ptr::null_mut();
            let ret = av_bsf_alloc(bsf, &mut ctx);
            if ret < 0 {
                return Err(anyhow::anyhow!("av_bsf_alloc failed: {}", ret));
            }

            ffmpeg_next::ffi::avcodec_parameters_copy((*ctx).par_in, codec_params.as_ptr());

            let ret = av_bsf_init(ctx);
            if ret < 0 {
                av_bsf_free(&mut ctx);
                return Err(anyhow::anyhow!("av_bsf_init failed: {}", ret));
            }

            Ok(Self { ctx, time_base })
        }
    }

    /// Filter a packet, converting from MP4 format to Annex B format.
    ///
    /// Returns a vector of filtered packets. Usually one packet in, one packet out,
    /// but the first keyframe may produce multiple packets (SPS/PPS + IDR).
    ///
    /// # Arguments
    /// * `packet` - Input packet in MP4 (AVCC/HVCC) format
    pub fn filter(&mut self, packet: &RawPacket) -> Result<Vec<FilteredPacket>> {
        unsafe {
            let pkt = packet.get_mut() as *mut Packet;
            let ret = av_bsf_send_packet(self.ctx, (*pkt).as_mut_ptr());
            if ret < 0 {
                return Err(anyhow::anyhow!("av_bsf_send_packet failed: {}", ret));
            }

            let mut filtered_packets = Vec::new();
            loop {
                let mut out_pkt = Packet::empty();
                let ret = av_bsf_receive_packet(self.ctx, out_pkt.as_mut_ptr());
                if ret == ffmpeg_next::ffi::AVERROR(ffmpeg_next::ffi::EAGAIN) {
                    // Need more input
                    break;
                } else if ret == ffmpeg_next::ffi::AVERROR_EOF {
                    // End of stream
                    break;
                } else if ret < 0 {
                    return Err(anyhow::anyhow!("av_bsf_receive_packet failed: {}", ret));
                }

                // Extract data from filtered packet
                let data = if let Some(d) = out_pkt.data() {
                    Bytes::copy_from_slice(d)
                } else {
                    Bytes::new()
                };

                filtered_packets.push(FilteredPacket {
                    data,
                    pts: out_pkt.pts(),
                    dts: out_pkt.dts(),
                    is_key: out_pkt.is_key(),
                    size: out_pkt.size(),
                });
            }

            Ok(filtered_packets)
        }
    }

    /// Flush the filter (call at end of stream).
    pub fn flush(&mut self) -> Result<Vec<FilteredPacket>> {
        unsafe {
            // Send NULL packet to signal EOF
            let ret = av_bsf_send_packet(self.ctx, ptr::null_mut());
            if ret < 0 && ret != ffmpeg_next::ffi::AVERROR_EOF {
                return Err(anyhow::anyhow!("av_bsf_send_packet(flush) failed: {}", ret));
            }

            let mut filtered_packets = Vec::new();
            loop {
                let mut out_pkt = Packet::empty();
                let ret = av_bsf_receive_packet(self.ctx, out_pkt.as_mut_ptr());
                if ret == ffmpeg_next::ffi::AVERROR_EOF {
                    break;
                } else if ret < 0 {
                    break;
                }

                let data = if let Some(d) = out_pkt.data() {
                    Bytes::copy_from_slice(d)
                } else {
                    Bytes::new()
                };

                filtered_packets.push(FilteredPacket {
                    data,
                    pts: out_pkt.pts(),
                    dts: out_pkt.dts(),
                    is_key: out_pkt.is_key(),
                    size: out_pkt.size(),
                });
            }

            Ok(filtered_packets)
        }
    }

    /// Get the time base for this filter.
    pub fn time_base(&self) -> Rational {
        self.time_base
    }
}

impl Drop for BitstreamFilter {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                av_bsf_free(&mut self.ctx);
            }
        }
    }
}

/// A filtered packet in Annex B format.
#[derive(Debug, Clone)]
pub struct FilteredPacket {
    /// Packet data in Annex B format (with 0x00 0x00 0x00 0x01 start codes)
    pub data: Bytes,
    /// Presentation timestamp
    pub pts: Option<i64>,
    /// Decoding timestamp
    pub dts: Option<i64>,
    /// Whether this is a keyframe
    pub is_key: bool,
    /// Size of the packet
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bsf_creation() {
        // This test just verifies that the filter can be looked up
        unsafe {
            let filter_name = std::ffi::CString::new("h264_mp4toannexb").unwrap();
            let bsf = av_bsf_get_by_name(filter_name.as_ptr());
            assert!(!bsf.is_null(), "h264_mp4toannexb filter should exist");
        }
    }
}
