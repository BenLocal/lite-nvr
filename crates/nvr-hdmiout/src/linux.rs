use std::{
    fs::{File, OpenOptions},
    os::fd::{AsFd, BorrowedFd},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use drm::{
    Device as BasicDevice,
    buffer::{Buffer as _, DrmFourcc},
    control::{
        Device as ControlDevice, Event, Mode, ModeTypeFlags, PageFlipFlags, connector, crtc,
        dumbbuffer::DumbBuffer, framebuffer,
    },
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::{ConnectionState, Connector, DisplayMode, Error, OutputConfig, Result, VideoFrame};

struct Card(File, Mutex<()>);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

fn open_card(path: &Path) -> Result<Arc<Card>> {
    Ok(Arc::new(Card(
        OpenOptions::new().read(true).write(true).open(path)?,
        Mutex::new(()),
    )))
}

struct Master {
    card: Arc<Card>,
    released: bool,
}
impl Master {
    fn release(&mut self) -> Result<()> {
        if !self.released {
            self.card.release_master_lock()?;
            self.released = true;
        }
        Ok(())
    }
}
impl Drop for Master {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn display_mode(mode: Mode) -> DisplayMode {
    let (width, height) = mode.size();
    DisplayMode {
        width: width.into(),
        height: height.into(),
        refresh_hz: mode.vrefresh(),
    }
}

fn hdmi(info: &connector::Info) -> bool {
    matches!(
        info.interface(),
        connector::Interface::HDMIA | connector::Interface::HDMIB
    )
}

pub fn list_connectors(path: impl AsRef<Path>) -> Result<Vec<Connector>> {
    let card = open_card(path.as_ref())?;
    let resources = card.resource_handles()?;
    let mut result = Vec::new();
    for handle in resources.connectors() {
        let info = card.get_connector(*handle, true)?;
        if hdmi(&info) {
            result.push(Connector {
                id: info.handle().into(),
                name: info.to_string(),
                state: match info.state() {
                    connector::State::Connected => ConnectionState::Connected,
                    connector::State::Disconnected => ConnectionState::Disconnected,
                    connector::State::Unknown => ConnectionState::Unknown,
                },
                modes: info.modes().iter().copied().map(display_mode).collect(),
            });
        }
    }
    Ok(result)
}

struct Scanout {
    card: Arc<Card>,
    buffer: Option<DumbBuffer>,
    framebuffer: Option<framebuffer::Handle>,
}
impl Scanout {
    fn new(card: Arc<Card>, mode: DisplayMode) -> Result<Self> {
        let buffer = card.create_dumb_buffer((mode.width, mode.height), DrmFourcc::Xrgb8888, 32)?;
        let mut result = Self {
            card,
            buffer: Some(buffer),
            framebuffer: None,
        };
        result.framebuffer = Some(result.card.add_framebuffer(
            result.buffer.as_ref().ok_or(Error::Closed)?,
            24,
            32,
        )?);
        // Start with black rather than uninitialized scanout contents.
        result
            .card
            .map_dumb_buffer(result.buffer.as_mut().ok_or(Error::Closed)?)?
            .fill(0);
        Ok(result)
    }
    fn handle(&self) -> Result<framebuffer::Handle> {
        self.framebuffer.ok_or(Error::Closed)
    }
    fn write(&mut self, frame: VideoFrame<'_>, mode: DisplayMode) -> Result<()> {
        let buffer = self.buffer.as_mut().ok_or(Error::Closed)?;
        let pitch = usize::try_from(buffer.pitch()).map_err(|_| Error::InvalidFrame)?;
        let mut mapping = self.card.map_dumb_buffer(buffer)?;
        crate::copy_frame(frame, mode, mapping.as_mut(), pitch)
    }
}
impl Drop for Scanout {
    fn drop(&mut self) {
        if let Some(fb) = self.framebuffer.take() {
            let _ = self.card.destroy_framebuffer(fb);
        }
        if let Some(buffer) = self.buffer.take() {
            let _ = self.card.destroy_dumb_buffer(buffer);
        }
    }
}

/// Owns DRM master and two scanout buffers. `present` blocks until the page flip
/// completes (at most two seconds). Explicit `close` reports restoration errors;
/// Drop also attempts restoration. Do not share this card with a compositor.
pub struct HdmiOutput {
    card: Arc<Card>,
    crtc: crtc::Info,
    previous_connectors: Vec<connector::Handle>,
    mode: DisplayMode,
    buffers: [Scanout; 2],
    front: usize,
    active: bool,
    master: Option<Arc<Master>>,
    failed: bool,
}

impl HdmiOutput {
    pub fn open(config: &OutputConfig) -> Result<Self> {
        let card = open_card(&config.card)?;
        card.acquire_master_lock()?;
        let master = Arc::new(Master {
            card: Arc::clone(&card),
            released: false,
        });
        Self::open_shared(card, master, config, false)
    }

    /// Open another HDMI connector on this output's card, sharing DRM master.
    /// Presentations are serialized to preserve the shared DRM event queue.
    pub fn open_sibling(&self, connector: &str, mode: Option<DisplayMode>) -> Result<Self> {
        if !self.active || self.failed {
            return Err(Error::Closed);
        }
        let config = OutputConfig {
            card: std::path::PathBuf::new(),
            connector: Some(connector.to_owned()),
            mode,
        };
        Self::open_shared(
            Arc::clone(&self.card),
            Arc::clone(self.master.as_ref().ok_or(Error::Closed)?),
            &config,
            true,
        )
    }

    fn open_shared(
        card: Arc<Card>,
        master: Arc<Master>,
        config: &OutputConfig,
        sibling: bool,
    ) -> Result<Self> {
        let resources = card.resource_handles()?;
        let infos = resources
            .connectors()
            .iter()
            .map(|h| card.get_connector(*h, true))
            .collect::<std::io::Result<Vec<_>>>()?;
        let info = infos
            .iter()
            .find(|info| {
                hdmi(info)
                    && info.state() == connector::State::Connected
                    && config
                        .connector
                        .as_ref()
                        .is_none_or(|name| *name == info.to_string())
            })
            .ok_or_else(|| Error::NoConnector(config.connector.clone()))?;
        if sibling
            && let Some(encoder) = info.current_encoder()
            && let Some(handle) = card.get_encoder(encoder)?.crtc()
            && card.get_crtc(handle)?.framebuffer().is_some()
        {
            return Err(Error::NoCrtc);
        }
        let mode = if let Some(wanted) = config.mode {
            info.modes().iter().find(|m| display_mode(**m) == wanted)
        } else {
            info.modes()
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .or_else(|| info.modes().first())
        }
        .copied()
        .ok_or(Error::NoMode)?;
        let mut chosen = None;
        for encoder in info.encoders() {
            let encoder = card.get_encoder(*encoder)?;
            for handle in resources.filter_crtcs(encoder.possible_crtcs()) {
                let state = card.get_crtc(handle)?;
                let used_elsewhere = infos
                    .iter()
                    .filter(|other| other.handle() != info.handle())
                    .filter_map(|other| other.current_encoder())
                    .map(|e| card.get_encoder(e))
                    .collect::<std::io::Result<Vec<_>>>()?
                    .iter()
                    .any(|e| e.crtc() == Some(handle));
                if !used_elsewhere {
                    chosen = Some(state);
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let crtc = chosen.ok_or(Error::NoCrtc)?;
        let mut previous_connectors = Vec::new();
        for other in &infos {
            if let Some(e) = other.current_encoder()
                && card.get_encoder(e)?.crtc() == Some(crtc.handle())
            {
                previous_connectors.push(other.handle());
            }
        }
        let selected = display_mode(mode);
        let buffers = [
            Scanout::new(Arc::clone(&card), selected)?,
            Scanout::new(Arc::clone(&card), selected)?,
        ];
        let mut output = Self {
            card,
            crtc,
            previous_connectors,
            mode: selected,
            buffers,
            front: 0,
            active: false,
            master: Some(master),
            failed: false,
        };
        output.card.set_crtc(
            output.crtc.handle(),
            Some(output.buffers[0].handle()?),
            (0, 0),
            &[info.handle()],
            Some(mode),
        )?;
        output.active = true;
        Ok(output)
    }

    pub fn mode(&self) -> DisplayMode {
        self.mode
    }

    pub fn present(&mut self, frame: VideoFrame<'_>) -> Result<()> {
        if !self.active || self.failed {
            return Err(Error::Closed);
        }
        let _presentation = self
            .card
            .1
            .lock()
            .map_err(|_| Error::Io(std::io::Error::other("DRM presentation lock poisoned")))?;
        let next = 1 - self.front;
        self.buffers[next].write(frame, self.mode)?;
        self.card.page_flip(
            self.crtc.handle(),
            self.buffers[next].handle()?,
            PageFlipFlags::EVENT,
            None,
        )?;
        // After queuing a flip, never reuse either buffer unless completion is observed.
        self.failed = true;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(Error::FlipTimeout)?;
            let timeout = Timespec::try_from(remaining).map_err(|_| Error::FlipTimeout)?;
            let mut fds = [PollFd::new(&*self.card, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => return Err(Error::FlipTimeout),
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(Error::Io(error.into())),
            }
            for event in self.card.receive_events()? {
                if matches!(event, Event::PageFlip(event) if event.crtc == self.crtc.handle()) {
                    self.front = next;
                    self.failed = false;
                    return Ok(());
                }
            }
        }
    }

    pub fn close(&mut self) -> Result<()> {
        if self.active {
            let framebuffer = self.crtc.framebuffer();
            // After an unclean exit the kernel can retain a mode/connector
            // without a framebuffer. Restore that state as a disabled CRTC.
            let connectors = if framebuffer.is_some() {
                self.previous_connectors.as_slice()
            } else {
                &[]
            };
            self.card.set_crtc(
                self.crtc.handle(),
                framebuffer,
                self.crtc.position(),
                connectors,
                framebuffer.and(self.crtc.mode()),
            )?;
            self.active = false;
        }
        if let Some(master) = self.master.take() {
            match Arc::try_unwrap(master) {
                Ok(mut master) => master.release()?,
                Err(shared) => drop(shared),
            }
        }
        Ok(())
    }
}
impl Drop for HdmiOutput {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "linux_test.rs"]
mod linux_test;
