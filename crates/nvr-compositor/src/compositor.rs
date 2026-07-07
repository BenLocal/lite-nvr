//! The compositor: builds a libavfilter graph (black canvas + one `overlay` per
//! region), then on a fixed output clock feeds each region's *currently active*
//! source frame in and pulls one composited frame out, encoding + muxing it to
//! ZLM as a single persistent stream.
//!
//! Every region is a switchable slot over a shared source pool: any source can
//! be switched into any region live (`Director::switch`). Frames are pre-scaled
//! to their region's exact size + YUV420P before entering the graph, so each
//! region's buffer source is fixed-size — switching to a differently-sized
//! source never rebuilds the graph and never interrupts the published stream.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ffmpeg_bus::encoder::{Encoder, Settings};
use ffmpeg_bus::frame::RawFrame;
use ffmpeg_bus::output::AvOutput;
use ffmpeg_bus::scaler::Scaler;
use ffmpeg_bus::stream::AvStream;
use ffmpeg_next::filter;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;
use tokio_util::sync::CancellationToken;

use crate::layout::{Layout, even};
use crate::source::LatestFrame;

/// Where/how the composited program is encoded and published.
#[derive(Clone)]
pub struct CompositorConfig {
    pub publish_url: String,
    pub format: String,
    pub fps: u32,
    pub bitrate: Option<u64>,
}

/// A source in the shared pool the compositor can sample by id.
pub struct SourceFeed {
    pub id: String,
    pub latest: LatestFrame,
}

/// Geometry of one region; its source is chosen at runtime, not fixed here.
struct RegionGeom {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// A region's cached scaler, keyed by its input `(width, height, pixel format)`
/// so it is rebuilt only when the active source's geometry changes.
type RegionScaler = Option<((u32, u32, Pixel), Scaler)>;

/// A cheap, cloneable handle to switch a running compositor's regions live.
#[derive(Clone)]
pub struct Director {
    /// Per-region active source id, indexed by region order.
    slots: Vec<Arc<Mutex<String>>>,
    pool_ids: Arc<HashSet<String>>,
}

impl Director {
    /// Switch region `index` to `source_id`. An empty `source_id` clears the
    /// region to black. Returns false if the index is out of range or (for a
    /// non-empty id) the source is not in the pool.
    pub fn switch(&self, index: usize, source_id: &str) -> bool {
        if index >= self.slots.len() {
            return false;
        }
        if !source_id.is_empty() && !self.pool_ids.contains(source_id) {
            return false;
        }
        *self.slots[index].lock().unwrap() = source_id.to_string();
        true
    }

    /// Current active source id per region, by region order.
    pub fn active(&self) -> Vec<String> {
        self.slots
            .iter()
            .map(|s| s.lock().unwrap().clone())
            .collect()
    }

    pub fn region_count(&self) -> usize {
        self.slots.len()
    }
}

/// A running compositor.
pub struct Compositor {
    director: Director,
    cancel: CancellationToken,
    handle: tokio::task::JoinHandle<Result<()>>,
}

impl Compositor {
    /// Start compositing. `layout.regions[i].source_id` is region i's initial
    /// source; any source in `pool` can later be switched into any region.
    /// `template` (any source's video stream) supplies the encoder frame rate.
    pub fn start(
        cfg: CompositorConfig,
        layout: Layout,
        pool: Vec<SourceFeed>,
        template: AvStream,
    ) -> Self {
        let canvas_w = even(layout.width).max(2);
        let canvas_h = even(layout.height).max(2);
        let geoms: Vec<RegionGeom> = layout
            .regions
            .iter()
            .map(|r| RegionGeom {
                x: even(r.x),
                y: even(r.y),
                w: even(r.w).max(2),
                h: even(r.h).max(2),
            })
            .collect();
        let slots: Vec<Arc<Mutex<String>>> = layout
            .regions
            .iter()
            .map(|r| Arc::new(Mutex::new(r.source_id.clone())))
            .collect();
        let pool_ids = Arc::new(pool.iter().map(|f| f.id.clone()).collect::<HashSet<_>>());
        let pool_map: HashMap<String, LatestFrame> =
            pool.into_iter().map(|f| (f.id, f.latest)).collect();

        let director = Director {
            slots: slots.clone(),
            pool_ids,
        };
        let cancel = CancellationToken::new();
        let loop_cancel = cancel.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let r = run(
                cfg,
                canvas_w,
                canvas_h,
                geoms,
                slots,
                pool_map,
                template,
                loop_cancel,
            );
            if let Err(ref e) = r {
                log::error!("compositor exited: {e:#}");
            }
            r
        });

        Self {
            director,
            cancel,
            handle,
        }
    }

    /// A cloneable handle for switching regions.
    pub fn director(&self) -> Director {
        self.director.clone()
    }

    /// Switch region `index` to `source_id`. See [`Director::switch`].
    pub fn switch(&self, index: usize, source_id: &str) -> bool {
        self.director.switch(index, source_id)
    }

    /// Current active source id per region, by region order.
    pub fn active(&self) -> Vec<String> {
        self.director.active()
    }

    pub fn region_count(&self) -> usize {
        self.director.region_count()
    }

    /// Stop compositing (the loop flushes the encoder and stops publishing).
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// Wait for the compositing task to finish (after [`stop`](Self::stop)).
    pub async fn join(self) -> Result<()> {
        self.handle
            .await
            .map_err(|e| anyhow!("compositor task join error: {e}"))?
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    cfg: CompositorConfig,
    canvas_w: u32,
    canvas_h: u32,
    geoms: Vec<RegionGeom>,
    slots: Vec<Arc<Mutex<String>>>,
    pool: HashMap<String, LatestFrame>,
    template: AvStream,
    cancel: CancellationToken,
) -> Result<()> {
    if geoms.is_empty() {
        anyhow::bail!("layout has no regions");
    }
    let mut graph = build_graph(canvas_w, canvas_h, cfg.fps, &geoms)?;

    // Persistent encoder + muxer, sized to the canvas.
    let settings = Settings {
        width: canvas_w,
        height: canvas_h,
        keyframe_interval: cfg.fps.max(1) as u64,
        codec: Some("h264".to_string()),
        pixel_format: Pixel::YUV420P,
    };
    let mut opts = ffmpeg_next::Dictionary::new();
    opts.set("preset", "ultrafast");
    opts.set("tune", "zerolatency");
    if let Some(b) = cfg.bitrate {
        opts.set("b", &b.to_string());
    }
    let mut encoder = Encoder::new(&template, settings, Some(opts))?;
    let out_stream = make_out_stream(&template, canvas_w, canvas_h);
    let mut output = AvOutput::new(&cfg.publish_url, Some(&cfg.format), None)?;
    output.add_stream(&out_stream)?;
    log::info!(
        "compositor: {canvas_w}x{canvas_h} @ {}fps, {} regions -> {}",
        cfg.fps,
        geoms.len(),
        cfg.publish_url
    );

    // Per-region scaler, cached by the active source's (w, h, pixel format);
    // rebuilt when a region switches to a differently-shaped source.
    let n = geoms.len();
    let mut scalers: Vec<RegionScaler> = (0..n).map(|_| None).collect();
    let in_names: Vec<String> = (0..n).map(|i| format!("in{i}")).collect();

    let tick_us = (1_000_000i64 / cfg.fps.max(1) as i64).max(1);
    let interval = Duration::from_secs_f64(1.0 / cfg.fps.max(1) as f64);
    let mut frame_no: i64 = 0;
    let mut out_pts: i64 = 0;
    let mut next = Instant::now();

    while !cancel.is_cancelled() {
        // Feed the black background then each region's active frame — all with
        // the same pts, so overlay's framesync emits exactly one composited
        // frame per tick.
        let mut bg = black_frame(canvas_w, canvas_h);
        bg.set_pts(Some(frame_no));
        graph
            .get("bg")
            .ok_or_else(|| anyhow!("missing bg"))?
            .source()
            .add(&bg)?;

        for i in 0..n {
            let g = &geoms[i];
            let src_id = slots[i].lock().unwrap().clone();
            let cached = pool.get(&src_id).and_then(|c| c.lock().unwrap().clone());

            // Pre-scale the active source frame to this region's exact size +
            // YUV420P; fall back to a black tile if the source has no frame yet.
            let mut dst = Video::empty();
            let filled = if let Some(RawFrame::Video(mut rvf)) = cached {
                let src = rvf.get_mut();
                let key = (src.width(), src.height(), src.format());
                let need_new = scalers[i].as_ref().map(|(k, _)| *k != key).unwrap_or(true);
                if need_new {
                    let ctx = ffmpeg_next::software::scaling::Context::get(
                        src.format(),
                        src.width(),
                        src.height(),
                        Pixel::YUV420P,
                        g.w,
                        g.h,
                        ffmpeg_next::software::scaling::flag::Flags::BILINEAR,
                    )?;
                    scalers[i] = Some((key, Scaler::new(ctx)));
                }
                scalers[i].as_mut().unwrap().1.run(src, &mut dst)?;
                true
            } else {
                false
            };
            if !filled {
                dst = black_frame(g.w, g.h);
            }
            dst.set_pts(Some(frame_no));
            graph
                .get(&in_names[i])
                .ok_or_else(|| anyhow!("missing {}", in_names[i]))?
                .source()
                .add(&dst)?;
        }

        // Pull the composited frame; encode + mux it.
        let mut out = Video::empty();
        if graph
            .get("out")
            .ok_or_else(|| anyhow!("missing sink"))?
            .sink()
            .frame(&mut out)
            .is_ok()
        {
            out.set_pts(Some(out_pts));
            encoder.send_frame(RawFrame::Video(out.into()))?;
            while let Some(pkt) = encoder.encoder_receive_packet()? {
                output.write_packet(0, pkt)?;
            }
            out_pts += tick_us;
        }

        frame_no += 1;
        next += interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now; // fell behind; don't accumulate debt
        }
    }

    log::info!("compositor: stopping, flushing");
    let _ = encoder.send_eof();
    while let Ok(Some(pkt)) = encoder.encoder_receive_packet() {
        let _ = output.write_packet(0, pkt);
    }
    let _ = output.finish();
    Ok(())
}

/// Build the compositing graph: `bg` + one `overlay` per region (each fed a
/// pre-scaled, region-sized `in{i}` buffer source), then `outfmt` → `buffersink`.
fn build_graph(
    canvas_w: u32,
    canvas_h: u32,
    fps: u32,
    geoms: &[RegionGeom],
) -> Result<filter::Graph> {
    let buffer = filter::find("buffer").ok_or_else(|| anyhow!("no buffer filter"))?;
    let buffersink = filter::find("buffersink").ok_or_else(|| anyhow!("no buffersink filter"))?;
    let overlay = filter::find("overlay").ok_or_else(|| anyhow!("no overlay filter"))?;
    let fmt = filter::find("format").ok_or_else(|| anyhow!("no format filter"))?;

    let mut g = filter::Graph::new();
    g.add(
        &buffer,
        "bg",
        &buffer_args(canvas_w, canvas_h, Pixel::YUV420P, fps),
    )?;
    for (i, geom) in geoms.iter().enumerate() {
        g.add(
            &buffer,
            &format!("in{i}"),
            &buffer_args(geom.w, geom.h, Pixel::YUV420P, fps),
        )?;
        g.add(
            &overlay,
            &format!("ov{i}"),
            &format!("x={}:y={}:eof_action=pass:repeatlast=1", geom.x, geom.y),
        )?;
    }
    g.add(&fmt, "outfmt", "pix_fmts=yuv420p")?;
    g.add(&buffersink, "out", "")?;

    let n = geoms.len();
    // Overlay chain: bg is the base of ov0; each ov{i} overlays in{i}.
    link(&mut g, "bg", 0, "ov0", 0);
    link(&mut g, "in0", 0, "ov0", 1);
    for i in 1..n {
        link(&mut g, &format!("ov{}", i - 1), 0, &format!("ov{i}"), 0);
        link(&mut g, &format!("in{i}"), 0, &format!("ov{i}"), 1);
    }
    link(&mut g, &format!("ov{}", n - 1), 0, "outfmt", 0);
    link(&mut g, "outfmt", 0, "out", 0);

    g.validate()?;
    Ok(g)
}

fn link(g: &mut filter::Graph, src: &str, src_pad: u32, dst: &str, dst_pad: u32) {
    let mut a = g.get(src).unwrap();
    let mut b = g.get(dst).unwrap();
    a.link(src_pad, &mut b, dst_pad);
}

fn buffer_args(w: u32, h: u32, pix: Pixel, fps: u32) -> String {
    let pix_int: ffmpeg_next::ffi::AVPixelFormat = pix.into();
    format!(
        "video_size={w}x{h}:pix_fmt={}:time_base=1/{}:pixel_aspect=1/1",
        pix_int as i32,
        fps.max(1)
    )
}

/// H.264 output stream descriptor with the canvas dimensions.
fn make_out_stream(template: &AvStream, w: u32, h: u32) -> AvStream {
    let s = AvStream::for_encoder_output(template, ffmpeg_next::codec::Id::H264);
    // for_encoder_output copies the template's (source) dimensions; overwrite
    // them with the canvas size so the muxed stream advertises the right geometry.
    unsafe {
        let p = s.parameters().as_ptr() as *mut ffmpeg_next::ffi::AVCodecParameters;
        (*p).width = w as i32;
        (*p).height = h as i32;
    }
    s
}

/// A black YUV420P frame (Y=16, U=V=128, limited range).
fn black_frame(w: u32, h: u32) -> Video {
    let mut v = Video::new(Pixel::YUV420P, w, h);
    v.data_mut(0).fill(16);
    v.data_mut(1).fill(128);
    v.data_mut(2).fill(128);
    v
}
