//! `tokio_util::codec` adapters over the sans-IO cores (W1 SP1.1). The cores keep
//! their APIs; a codec only moves bytes between a `BytesMut` and a core.

use std::collections::VecDeque;
use std::future::Future;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use rtsp_types::Message;
use tokio_util::codec::{Decoder, Encoder};

use crate::client::{ClientEvent, ClientSession};
use crate::error::{Error, Result};
use crate::interleaved::{InterleavedFrame, MAGIC};
use crate::limits::MAX_MESSAGE_BYTES;

/// Runs `fut` for at most `limit`; expiry is `Error::Timeout { what }`.
pub(crate) async fn bounded<T>(
    limit: Duration,
    what: &'static str,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| Error::Timeout { what })?
}

/// Client-side codec: bytes in -> [`ClientEvent`]s out via
/// [`ClientSession::handle_data`]; requests (already serialized by the session)
/// are written verbatim.
#[derive(Debug)]
pub(crate) struct ClientCodec {
    pub(crate) session: ClientSession,
    queue: VecDeque<ClientEvent>,
}

impl ClientCodec {
    pub(crate) fn new(session: ClientSession) -> Self {
        Self {
            session,
            queue: VecDeque::new(),
        }
    }
}

impl Decoder for ClientCodec {
    type Item = ClientEvent;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ClientEvent>> {
        if let Some(ev) = self.queue.pop_front() {
            return Ok(Some(ev));
        }
        if src.is_empty() {
            return Ok(None);
        }
        // The session buffers partial messages itself; hand it everything.
        let chunk = src.split();
        self.queue.extend(self.session.handle_data(&chunk)?);
        Ok(self.queue.pop_front())
    }
}

impl Encoder<Vec<u8>> for ClientCodec {
    type Error = Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}

/// One server-side frame: a complete request, or an interleaved `$` block (§10.12).
pub(crate) enum ServerFrame {
    Request(Vec<u8>),
    Media { channel: u8, data: Vec<u8> },
}

/// Frames a request stream. A bounded framing check (`framing::head_end`, an
/// incremental `memchr` search resumed from the last scanned offset) finds the
/// blank line ending the header block and enforces the 64 KiB head cap on the
/// bytes buffered before it; only then is `rtsp_types::Message::parse` called,
/// once, and its `Incomplete(Some(n))` body need is remembered so a body is not
/// re-parsed until it can possibly be complete. This is not protocol parsing:
/// every header and body decision is `rtsp_types`'.
#[derive(Debug, Default)]
pub(crate) struct ServerCodec {
    /// Buffer length below which a re-parse cannot succeed yet (body wait).
    skip_until: usize,
    /// How much of the buffer the terminator search already covered.
    scanned: usize,
    /// `Message::parse` calls made (test instrumentation).
    #[cfg(test)]
    parse_calls: usize,
}

impl Decoder for ServerCodec {
    type Item = ServerFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ServerFrame>> {
        if src.is_empty() {
            return Ok(None);
        }
        if src[0] == MAGIC {
            return match InterleavedFrame::parse(src)? {
                Some((frame, used)) => {
                    src.advance(used);
                    Ok(Some(ServerFrame::Media {
                        channel: frame.channel,
                        data: frame.payload,
                    }))
                }
                None => Ok(None),
            };
        }
        if src.len() < self.skip_until {
            return Ok(None);
        }
        if crate::framing::head_end(src, self.scanned)?.is_none() {
            self.scanned = src.len();
            return Ok(None);
        }
        #[cfg(test)]
        {
            self.parse_calls += 1;
        }
        match Message::<Vec<u8>>::parse(src) {
            Ok((_, used)) => {
                self.skip_until = 0;
                self.scanned = 0;
                Ok(Some(ServerFrame::Request(src.split_to(used).to_vec())))
            }
            Err(rtsp_types::ParseError::Incomplete(Some(more))) => {
                let want = src.len().saturating_add(more.get());
                if want > MAX_MESSAGE_BYTES {
                    return Err(Error::MessageParse(format!(
                        "request of at least {want} bytes exceeds the {MAX_MESSAGE_BYTES}-byte maximum"
                    )));
                }
                self.skip_until = want;
                Ok(None)
            }
            // Terminator present yet no length: rtsp_types wants more input.
            Err(rtsp_types::ParseError::Incomplete(None)) => Ok(None),
            Err(rtsp_types::ParseError::Error) => {
                Err(Error::MessageParse("malformed RTSP request".into()))
            }
        }
    }
}

impl Encoder<Vec<u8>> for ServerCodec {
    type Error = Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &[u8] = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS, DESCRIBE\r\n\r\n";

    #[tokio::test(start_paused = true)]
    async fn a_connect_that_never_completes_is_bounded() {
        let r: Result<()> = bounded(
            Duration::from_secs(3),
            "connect",
            std::future::pending::<Result<()>>(),
        )
        .await;
        assert!(matches!(r, Err(Error::Timeout { what: "connect" })));
    }

    #[test]
    fn client_codec_reassembles_a_response_split_at_every_byte_boundary() {
        for split in 1..RESPONSE.len() {
            let mut c = ClientSession::new();
            let _ = c.options("rtsp://h/s").unwrap();
            let mut codec = ClientCodec::new(c);
            let mut buf = BytesMut::new();
            buf.extend_from_slice(&RESPONSE[..split]);
            assert!(
                codec.decode(&mut buf).unwrap().is_none(),
                "split {split}: premature event"
            );
            buf.extend_from_slice(&RESPONSE[split..]);
            let ev = codec.decode(&mut buf).unwrap();
            assert!(
                matches!(ev, Some(ClientEvent::Response { cseq: 1, .. })),
                "split {split}: {ev:?}"
            );
        }
    }

    #[test]
    fn client_codec_one_byte_at_a_time() {
        let mut c = ClientSession::new();
        let _ = c.options("rtsp://h/s").unwrap();
        let mut codec = ClientCodec::new(c);
        let mut buf = BytesMut::new();
        let mut got = None;
        for b in RESPONSE {
            buf.extend_from_slice(&[*b]);
            if let Some(ev) = codec.decode(&mut buf).unwrap() {
                got = Some(ev);
            }
        }
        assert!(matches!(got, Some(ClientEvent::Response { cseq: 1, .. })));
    }

    #[test]
    fn server_codec_request_split_at_every_boundary_and_pipelined_frames() {
        let req: &[u8] = b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n";
        for split in 1..req.len() {
            let mut codec = ServerCodec::default();
            let mut buf = BytesMut::from(&req[..split]);
            assert!(codec.decode(&mut buf).unwrap().is_none(), "split {split}");
            buf.extend_from_slice(&req[split..]);
            assert!(
                matches!(codec.decode(&mut buf).unwrap(), Some(ServerFrame::Request(r)) if r == req)
            );
        }
        // request, then an interleaved frame, back to back in one buffer
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::new();
        buf.extend_from_slice(req);
        buf.extend_from_slice(&[0x24, 3, 0, 2, 0xAA, 0xBB]);
        assert!(matches!(
            codec.decode(&mut buf).unwrap(),
            Some(ServerFrame::Request(_))
        ));
        assert!(matches!(
            codec.decode(&mut buf).unwrap(),
            Some(ServerFrame::Media { channel: 3, ref data }) if data == &[0xAA, 0xBB]
        ));
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn server_codec_waits_for_a_body_without_reparsing_every_byte() {
        let head = b"ANNOUNCE rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 20\r\n\r\n";
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::from(&head[..]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(
            codec.skip_until,
            head.len() + 20,
            "knows how many bytes it still needs"
        );
        buf.extend_from_slice(&[b'x'; 20]);
        assert!(
            matches!(codec.decode(&mut buf).unwrap(), Some(ServerFrame::Request(r)) if r.len() == head.len() + 20)
        );
    }

    #[test]
    fn server_codec_caps_a_70_kib_unterminated_head_of_a() {
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::from(&vec![b'A'; 70 * 1024][..]);
        assert!(matches!(
            codec.decode(&mut buf),
            Err(Error::MessageParse(_))
        ));
    }

    #[test]
    fn server_codec_caps_9000_valid_header_lines_without_a_blank_line() {
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::from(&b"OPTIONS rtsp://h/s RTSP/1.0\r\n"[..]);
        buf.extend_from_slice(&b"X-A: b\r\n".repeat(9000));
        assert!(matches!(
            codec.decode(&mut buf),
            Err(Error::MessageParse(_))
        ));
    }

    #[test]
    fn server_codec_parses_once_per_message_even_when_fed_a_byte_at_a_time() {
        let req: &[u8] = b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n";
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::new();
        let mut got = 0;
        for b in req {
            buf.extend_from_slice(&[*b]);
            if codec.decode(&mut buf).unwrap().is_some() {
                got += 1;
            }
        }
        assert_eq!(got, 1);
        assert_eq!(codec.parse_calls, 1, "no parse before the terminator");
    }
}
