use futures::{Sink, Stream};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use crate::media::types::VideoRawFrame;

pub struct RawSinkSource {
    pub writer: tokio::sync::mpsc::Sender<Vec<u8>>,
    inner: Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
}

impl RawSinkSource {
    pub fn new() -> Self {
        Self::with_capacity(32)
    }

    pub fn with_capacity(buffer_size: usize) -> Self {
        let (writer, receiver) = tokio::sync::mpsc::channel(buffer_size);
        Self {
            writer,
            inner: Mutex::new(receiver),
        }
    }

    pub fn stream(&self) -> RawFrameStream<'_> {
        RawFrameStream { source: self }
    }
}

impl Default for RawSinkSource {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RawFrameStream<'a> {
    source: &'a RawSinkSource,
}

impl Stream for RawFrameStream<'_> {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = self.source.inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

impl Stream for RawSinkSource {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut guard = self.get_mut().inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

/// Wrapper to use `Arc<RawSinkSource>` as Stream (orphan rule workaround).
pub struct RawSinkSourceStream(pub Arc<RawSinkSource>);

impl Stream for RawSinkSourceStream {
    type Item = VideoRawFrame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let source = &self.0;
        let mut guard = source.inner.lock().unwrap();
        guard.poll_recv(cx).map(|opt| opt.map(VideoRawFrame::new))
    }
}

impl RawSinkSource {
    /// Returns a stream that yields VideoRawFrame. Use this when you have `Arc<RawSinkSource>`.
    pub fn as_stream(this: Arc<Self>) -> RawSinkSourceStream {
        RawSinkSourceStream(this)
    }
}

impl Sink<Vec<u8>> for RawSinkSource {
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.get_mut().writer.capacity() > 0 {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        self.get_mut()
            .writer
            .try_send(item)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "channel closed"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
