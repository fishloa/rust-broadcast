use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;

pub(crate) trait PacketSink {
    type Event;
    type Stats;

    fn feed_packet(&mut self, pkt: &[u8], out: &mut VecDeque<Self::Event>);
    fn sink_stats(&self) -> Self::Stats;
}

impl PacketSink for dvb_si::demux::SiDemux {
    type Event = dvb_si::demux::SectionEvent;
    type Stats = dvb_si::demux::Stats;

    fn feed_packet(&mut self, pkt: &[u8], out: &mut VecDeque<Self::Event>) {
        out.extend(self.feed(pkt));
    }

    fn sink_stats(&self) -> Self::Stats {
        self.stats()
    }
}

impl PacketSink for dvb_t2mi::pump::T2miPump {
    type Event = dvb_t2mi::pump::T2miEvent;
    type Stats = dvb_t2mi::pump::Stats;

    fn feed_packet(&mut self, pkt: &[u8], out: &mut VecDeque<Self::Event>) {
        out.extend(self.feed_ts(pkt));
    }

    fn sink_stats(&self) -> Self::Stats {
        self.stats()
    }
}

pub(crate) struct PacketPump<S, P: PacketSink> {
    pub(crate) frames: S,
    sink: P,
    queue: VecDeque<P::Event>,
    io_error: Option<std::io::Error>,
    done: bool,
}

impl<S: Stream<Item = std::io::Result<Bytes>> + Unpin, P: PacketSink> PacketPump<S, P> {
    pub(crate) fn new(frames: S, sink: P) -> Self {
        Self {
            frames,
            sink,
            queue: VecDeque::new(),
            io_error: None,
            done: false,
        }
    }

    pub(crate) fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<P::Event>> {
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Poll::Ready(Some(ev));
            }
            if self.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut self.frames).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => self.done = true,
                Poll::Ready(Some(Err(e))) => {
                    self.io_error = Some(e);
                    self.done = true;
                }
                Poll::Ready(Some(Ok(pkt))) => self.sink.feed_packet(&pkt, &mut self.queue),
            }
        }
    }

    pub(crate) fn stats(&self) -> P::Stats {
        self.sink.sink_stats()
    }

    pub(crate) fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.io_error.take()
    }
}
