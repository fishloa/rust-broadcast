//! [`T2miPump`] — owning-[`Bytes`] feed-and-iterate T2-MI pump.
//!
//! Feed raw bytes (TS-encapsulated or bare T2-MI stream) in; get back an
//! iterator of [`T2miEvent`]s — one per **CRC-valid** complete T2-MI packet.
//! Lazy zero-copy: events own their [`bytes::Bytes`] slice and expose typed
//! views ([`T2miEvent::header`], [`T2miEvent::payload`]) that borrow from it
//! on demand.
//!
//! ```no_run
//! use dvb_t2mi::pump::T2miPump;
//! use dvb_t2mi::payload::AnyPayload;
//!
//! let mut pump = T2miPump::new(0x0006); // T2-MI PID from the PMT
//! let ts_packet = [0u8; 188]; // a real TS packet from your source
//! for event in pump.feed_ts(&ts_packet) {
//!     if let Ok(AnyPayload::Bbframe(bb)) = event.payload() {
//!         println!("BBFrame plp_id={}", bb.plp_id);
//!     }
//! }
//! ```
//!
//! # CRC policy
//!
//! Every complete packet is validated against its 4-byte CRC-32 trailer
//! (ETSI TS 102 773 Annex A / [`crate::crc::validate_crc`]) before being
//! emitted.  Packets that fail CRC are silently dropped and counted in
//! [`Stats::crc_failures`].  The caller never sees a corrupted packet.
//!
//! # TS header parsing
//!
//! [`T2miPump::feed_ts`] extracts the MPEG-TS payload in-place — sync byte
//! 0x47, PID, PUSI flag, and adaptation-field skip per ISO/IEC 13818-1
//! §2.4.3.2 — and passes it to [`crate::ts::PacketReassembler`].  No
//! `dvb-si` dependency is introduced; the TS header reader is a private
//! helper below.

use bytes::Bytes;

use crate::crc;
use crate::packet::Header;
use crate::payload::AnyPayload;
use crate::ts::PacketReassembler;

// ── TS header constants (ISO/IEC 13818-1 §2.4.3.2) ──────────────────────────

/// TS sync byte.
const TS_SYNC_BYTE: u8 = 0x47;
/// Expected size of one MPEG-TS packet.
const TS_PACKET_SIZE: usize = 188;
/// Byte 1 bit 6 = PUSI (Payload Unit Start Indicator).
const PUSI_MASK: u8 = 0x40;
/// Byte 1 bits 4..=0 = PID upper 5 bits.
const PID_MASK_HI: u8 = 0x1F;
/// Byte 3 bit 5 = adaptation_field_control bit 1 (adaptation field present).
const ADAPTATION_FLAG: u8 = 0x20;
/// Byte 3 bit 4 = adaptation_field_control bit 0 (payload present).
const PAYLOAD_FLAG: u8 = 0x10;

/// Minimal result of TS header parsing needed by the pump.
struct TsInfo {
    pid: u16,
    pusi: bool,
    /// Byte offset within the 188-byte packet where the payload starts.
    payload_start: usize,
}

/// Parse the 4-byte MPEG-TS header and skip any adaptation field.
///
/// Returns `None` when:
/// - `buf` is shorter than [`TS_PACKET_SIZE`],
/// - the sync byte is not `0x47`,
/// - the payload-present flag is clear, or
/// - the adaptation field length overflows the packet.
///
/// Citation: ISO/IEC 13818-1:2019 §2.4.3.2 (transport_packet header) and
/// §2.4.3.5 (adaptation_field length).
fn parse_ts_header(buf: &[u8]) -> Option<TsInfo> {
    if buf.len() < TS_PACKET_SIZE || buf[0] != TS_SYNC_BYTE {
        return None;
    }
    let b1 = buf[1];
    let b3 = buf[3];

    let pusi = (b1 & PUSI_MASK) != 0;
    let pid = (((b1 & PID_MASK_HI) as u16) << 8) | (buf[2] as u16);
    let has_adaptation = (b3 & ADAPTATION_FLAG) != 0;
    let has_payload = (b3 & PAYLOAD_FLAG) != 0;

    if !has_payload {
        return None;
    }

    let mut cursor: usize = 4;
    if has_adaptation {
        let af_len = buf[cursor] as usize;
        cursor += 1 + af_len;
        if cursor > TS_PACKET_SIZE {
            return None;
        }
    }

    Some(TsInfo {
        pid,
        pusi,
        payload_start: cursor,
    })
}

// ── T2miEvent ─────────────────────────────────────────────────────────────────

/// One complete, CRC-valid T2-MI packet. Owns its bytes — `'static`, cheap clone.
///
/// Only constructed after CRC-32 validation (ETSI TS 102 773 Annex A).
/// [`T2miEvent::header`] and [`T2miEvent::payload`] are lazy: they borrow from
/// the owned [`Bytes`] on demand.
#[derive(Debug, Clone)]
pub struct T2miEvent {
    bytes: Bytes,
}

impl T2miEvent {
    /// The full packet bytes (header + payload + CRC trailer).
    #[must_use]
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// The raw `packet_type` byte (byte 0 of the T2-MI header per §5.1).
    ///
    /// Never panics — events are only built for CRC-valid packets which are at
    /// least `6` (header) + `4` (CRC) = 10 bytes.
    #[must_use]
    pub fn packet_type(&self) -> u8 {
        self.bytes[0]
    }

    /// Parse the 6-byte T2-MI packet header (lazy, borrows this event's bytes).
    ///
    /// # Errors
    ///
    /// Propagates [`crate::Error`] from [`dvb_common::Parse::parse`] on [`Header`].
    pub fn header(&self) -> crate::Result<Header> {
        use dvb_common::Parse;
        Header::parse(&self.bytes)
    }

    /// Extract the `packet_type` byte and payload slice from this event's
    /// bytes — shared logic for [`payload`](Self::payload) and
    /// [`payload_with`](Self::payload_with).
    ///
    /// Uses [`Header::raw_payload_bytes`] so that genuinely-private
    /// `packet_type` values (not in [`PacketType`](crate::packet::PacketType))
    /// are not rejected.  The packet is already CRC-validated by the pump.
    fn payload_parts(&self) -> crate::Result<(u8, &[u8])> {
        let payload_bytes = Header::raw_payload_bytes(&self.bytes)?;
        let packet_type = self.bytes[0];
        Ok((packet_type, payload_bytes))
    }

    /// Parse the payload by dispatching on `packet_type`.
    ///
    /// Extracts the payload slice via [`Header::raw_payload_bytes`] (no
    /// `packet_type` enum conversion), then calls
    /// [`AnyPayload::dispatch`].  Unrecognised packet types produce
    /// [`AnyPayload::Unknown`] with the raw payload bytes.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error`] from extracting the payload slice or from the
    /// typed payload parser.
    pub fn payload(&self) -> crate::Result<AnyPayload<'_>> {
        let (packet_type, payload_bytes) = self.payload_parts()?;
        Ok(match AnyPayload::dispatch(packet_type, payload_bytes) {
            Some(result) => result?,
            None => AnyPayload::Unknown {
                packet_type,
                body: payload_bytes,
            },
        })
    }

    /// Parse the payload by dispatching on `packet_type`, preferring the
    /// registry's custom parsers over the built-in dispatch.
    ///
    /// Like [`payload`](Self::payload), but calls
    /// [`AnyPayload::dispatch_with`] so that runtime-registered custom
    /// packet types are resolved to [`AnyPayload::Other`].  Unrecognised
    /// packet types produce [`AnyPayload::Unknown`] with the raw payload
    /// bytes, exactly as [`payload`](Self::payload) does.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error`] from extracting the payload slice or from the
    /// typed payload parser (built-in or custom).
    pub fn payload_with(
        &self,
        registry: &crate::payload::PayloadRegistry,
    ) -> crate::Result<AnyPayload<'_>> {
        let (packet_type, payload_bytes) = self.payload_parts()?;
        Ok(
            match AnyPayload::dispatch_with(registry, packet_type, payload_bytes) {
                Some(result) => result?,
                None => AnyPayload::Unknown {
                    packet_type,
                    body: payload_bytes,
                },
            },
        )
    }
}

// ── Stats ─────────────────────────────────────────────────────────────────────

/// Accumulated pump statistics (monotonically growing across all `feed` calls).
///
/// New counter fields may be added in a future release; construction is via
/// [`Default`] only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    /// TS packets fed via [`T2miPump::feed_ts`].
    pub ts_packets: u64,
    /// Complete T2-MI packets produced by the reassembler (pre-CRC check).
    pub t2mi_packets: u64,
    /// Packets dropped due to CRC-32 mismatch (ETSI TS 102 773 Annex A).
    pub crc_failures: u64,
    /// Malformed inputs: bad TS sync byte, truncated TS packet, overflowed
    /// adaptation field, or `feed_ts` called on a raw-mode pump.
    pub malformed_packets: u64,
}

// ── T2miPump ──────────────────────────────────────────────────────────────────

/// Feed-and-iterate T2-MI pump.
///
/// Supports two operating modes:
///
/// - **TS-encapsulated** (most common): construct with [`T2miPump::new`],
///   passing the 13-bit PID carrying T2-MI (from the PMT).  Feed 188-byte
///   MPEG-TS packets with [`T2miPump::feed_ts`].  The pump filters by PID,
///   strips the TS header per ISO/IEC 13818-1 §2.4.3.2, and forwards the
///   payload to the internal [`PacketReassembler`] (ETSI TS 102 773 §6.1.1).
///
/// - **Raw** (un-encapsulated): construct with [`T2miPump::raw`].  Feed
///   arbitrary byte slices with [`T2miPump::feed_raw`].  The pump buffers bytes
///   and emits events once a full packet (determined by the header's
///   `payload_len_bits`) is available.
///
/// # PID note
///
/// PIDs are 13-bit values (0x0000–0x1FFF per ISO/IEC 13818-1 §2.4.3.2).
/// This type uses `u16` directly; no newtype is introduced.  Values above
/// 0x1FFF are accepted without error — the PID filter simply never matches.
pub struct T2miPump {
    mode: PumpMode,
    reasm: PacketReassembler,
    stats: Stats,
    scratch: Vec<T2miEvent>,
    /// Raw-mode sync flag: true once the first raw feed has initialised the
    /// reassembler via a PUSI=true, pointer=0 signal.
    raw_started: bool,
}

enum PumpMode {
    /// TS-encapsulated: filter packets to this PID.
    Ts { pid: u16 },
    /// Un-encapsulated raw byte stream.
    Raw,
}

impl T2miPump {
    /// Create a TS-encapsulated pump that filters to `pid`.
    ///
    /// `pid` is the 13-bit T2-MI PID from the PMT (e.g. 0x0006 for data
    /// piping).
    ///
    /// # PID range
    ///
    /// Valid MPEG-TS PIDs are 13-bit (0x0000–0x1FFF); this parameter is `u16`.
    /// No newtype is introduced to keep the API lightweight.
    #[must_use]
    pub fn new(pid: u16) -> Self {
        Self {
            mode: PumpMode::Ts { pid },
            reasm: PacketReassembler::new(),
            stats: Stats::default(),
            scratch: Vec::new(),
            raw_started: false,
        }
    }

    /// Create an un-encapsulated raw-stream pump.
    ///
    /// Use [`T2miPump::feed_raw`] to supply bytes.  The pump buffers internally
    /// and emits events by packet boundary, not by call boundary — a packet
    /// split across two `feed_raw` calls produces exactly one event.
    #[must_use]
    pub fn raw() -> Self {
        Self {
            mode: PumpMode::Raw,
            reasm: PacketReassembler::new(),
            stats: Stats::default(),
            scratch: Vec::new(),
            raw_started: false,
        }
    }

    /// Accumulated statistics.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Feed one 188-byte MPEG-TS packet. Infallible: malformed packets are
    /// counted in [`Stats::malformed_packets`] and discarded.
    ///
    /// Packets on the wrong PID are silently ignored (only [`Stats::ts_packets`]
    /// is incremented).
    ///
    /// Returns a draining iterator over any T2-MI events completed by this feed.
    pub fn feed_ts(&mut self, packet: &[u8]) -> impl Iterator<Item = T2miEvent> + '_ {
        self.scratch.clear();

        match self.mode {
            PumpMode::Raw => {
                // feed_ts on a raw-mode pump is a caller error.
                self.stats.malformed_packets += 1;
            }
            PumpMode::Ts { pid: filter_pid } => {
                self.stats.ts_packets += 1;
                match parse_ts_header(packet) {
                    None => {
                        self.stats.malformed_packets += 1;
                    }
                    Some(info) => {
                        if info.pid == filter_pid {
                            let payload = &packet[info.payload_start..TS_PACKET_SIZE];
                            self.reasm.feed(payload, info.pusi);
                            Self::drain_reasm_into(
                                &mut self.reasm,
                                &mut self.stats,
                                &mut self.scratch,
                            );
                        }
                        // Wrong PID: ignored cheaply — no stats beyond ts_packets.
                    }
                }
            }
        }

        self.scratch.drain(..)
    }

    /// Feed raw T2-MI bytes (un-encapsulated mode).
    ///
    /// The slice may contain a partial packet; bytes are buffered internally.
    /// A packet split across two `feed_raw` calls produces exactly one event.
    ///
    /// Returns a draining iterator over any T2-MI events completed by this feed.
    pub fn feed_raw(&mut self, data: &[u8]) -> impl Iterator<Item = T2miEvent> + '_ {
        self.scratch.clear();

        match self.mode {
            PumpMode::Ts { .. } => {
                // feed_raw on a TS-mode pump is a caller error.
                self.stats.malformed_packets += 1;
            }
            PumpMode::Raw => {
                if !self.raw_started {
                    // First call: initialise the reassembler with PUSI=true and
                    // pointer_field=0.  PacketReassembler::feed interprets the
                    // first byte of the payload as the pointer_field when PUSI is
                    // set (ETSI TS 102 773 §6.1.1).  We prepend a 0x00 byte so the
                    // reassembler sees pointer=0 and treats the rest as the start
                    // of a new T2-MI packet.
                    let mut buf = Vec::with_capacity(1 + data.len());
                    buf.push(0x00); // pointer_field = 0
                    buf.extend_from_slice(data);
                    self.reasm.feed(&buf, true);
                    self.raw_started = true;
                } else {
                    // Continuation: feed without PUSI — bytes extend the
                    // current T2-MI packet in progress.
                    self.reasm.feed(data, false);
                }
                Self::drain_reasm_into(&mut self.reasm, &mut self.stats, &mut self.scratch);
            }
        }

        self.scratch.drain(..)
    }

    /// Drain all pending packets from the reassembler, CRC-validate each one,
    /// and push valid packets to `scratch`.
    fn drain_reasm_into(
        reasm: &mut PacketReassembler,
        stats: &mut Stats,
        scratch: &mut Vec<T2miEvent>,
    ) {
        for raw in reasm.drain_packets() {
            stats.t2mi_packets += 1;
            match crc::validate_crc(&raw) {
                Ok(()) => scratch.push(T2miEvent { bytes: raw }),
                Err(_) => stats.crc_failures += 1,
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::crc32_mpeg2;

    // ── Test helpers ─────────────────────────────────────────────────────────

    /// Build a syntactically valid T2-MI packet (header + payload + CRC-32).
    ///
    /// `packet_type` is the raw byte (Table 1 of TS 102 773).
    /// `payload` is the post-header, pre-CRC data.
    /// Returns the full byte vector including the 4-byte CRC trailer.
    fn make_t2mi_packet(packet_type: u8, payload: &[u8]) -> Vec<u8> {
        let payload_len_bits = (payload.len() * 8) as u16;
        let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
        pkt.push(packet_type);
        pkt.push(0x01); // packet_count
        pkt.push(0x00); // superframe_idx=0, rfu=0, t2mi_stream_id=0
        pkt.push(0x00); // rfu byte = 0
        pkt.extend_from_slice(&payload_len_bits.to_be_bytes());
        pkt.extend_from_slice(payload);
        let crc = crc32_mpeg2::compute(&pkt);
        pkt.extend_from_slice(&crc.to_be_bytes());
        pkt
    }

    /// Wrap a T2-MI payload slice in a single 188-byte MPEG-TS packet.
    ///
    /// Sets PUSI=true and pointer_field=0 so the reassembler treats
    /// the T2-MI data as starting at byte 0 of the payload.
    /// The T2-MI bytes must fit in 183 bytes (188 − 4 header − 1 pointer).
    fn ts_packet(pid: u16, t2mi_data: &[u8], pusi: bool, pointer_field: u8) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = if pusi { PUSI_MASK } else { 0 };
        pkt[1] |= ((pid >> 8) as u8) & PID_MASK_HI;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = PAYLOAD_FLAG; // payload present, no adaptation field
        if pusi {
            pkt[4] = pointer_field;
            let start = 5 + pointer_field as usize;
            assert!(
                start + t2mi_data.len() <= 188,
                "T2-MI data too large for one TS packet"
            );
            pkt[start..start + t2mi_data.len()].copy_from_slice(t2mi_data);
        } else {
            let start = 4;
            assert!(
                start + t2mi_data.len() <= 188,
                "T2-MI data too large for one TS packet"
            );
            pkt[start..start + t2mi_data.len()].copy_from_slice(t2mi_data);
        }
        pkt
    }

    // ── (a) valid T2-MI packet in TS → one event, typed payload ──────────────

    #[test]
    fn ts_packet_emits_one_event_with_typed_payload() {
        // Build a valid BBFrame T2-MI packet.
        // BbframePayload minimum: frame_idx(1) + plp_id(1) + flags(1) = 3 bytes.
        let bbframe_payload = [0x01u8, 0x02, 0x00];
        let t2mi = make_t2mi_packet(0x00, &bbframe_payload);

        let pkt = ts_packet(0x0006, &t2mi, true, 0);
        let mut pump = T2miPump::new(0x0006);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();

        assert_eq!(events.len(), 1, "expected exactly one event");
        assert_eq!(events[0].packet_type(), 0x00);

        let payload = events[0].payload().expect("payload parse should succeed");
        assert!(
            matches!(payload, AnyPayload::Bbframe(_)),
            "expected Bbframe, got {payload:?}"
        );

        let stats = pump.stats();
        assert_eq!(stats.ts_packets, 1);
        assert_eq!(stats.t2mi_packets, 1);
        assert_eq!(stats.crc_failures, 0);
        assert_eq!(stats.malformed_packets, 0);
    }

    // ── (b) corrupted CRC → zero events, crc_failures=1 ─────────────────────

    #[test]
    fn corrupted_crc_drops_packet_and_counts() {
        let payload = [0x00u8, 0x00, 0x00]; // minimal BBFrame payload
        let mut t2mi = make_t2mi_packet(0x00, &payload);
        // Corrupt the last CRC byte.
        *t2mi.last_mut().unwrap() ^= 0xFF;

        let pkt = ts_packet(0x0006, &t2mi, true, 0);
        let mut pump = T2miPump::new(0x0006);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();

        assert_eq!(events.len(), 0, "corrupted packet must not emit");
        let stats = pump.stats();
        assert_eq!(stats.crc_failures, 1);
        assert_eq!(stats.t2mi_packets, 1); // reassembler produced it, CRC gate dropped it
    }

    // ── (c) feed_raw with packet split across two calls → one event ──────────

    #[test]
    fn feed_raw_split_across_two_calls_emits_one_event() {
        // Use a timestamp payload (11 bytes, all zeros), packet_type=0x20.
        let ts_payload = [0x00u8; 11];
        let t2mi = make_t2mi_packet(0x20, &ts_payload);

        // Split at an arbitrary boundary (e.g. after the header).
        let split = 6;
        let first = &t2mi[..split];
        let second = &t2mi[split..];

        let mut pump = T2miPump::raw();

        let ev1: Vec<_> = pump.feed_raw(first).collect();
        assert_eq!(ev1.len(), 0, "no complete packet yet after first chunk");

        let ev2: Vec<_> = pump.feed_raw(second).collect();
        assert_eq!(
            ev2.len(),
            1,
            "one event after second chunk completes the packet"
        );

        let stats = pump.stats();
        assert_eq!(stats.t2mi_packets, 1);
        assert_eq!(stats.crc_failures, 0);
    }

    // ── (d) garbage TS packet → malformed counted, no panic ──────────────────

    #[test]
    fn garbage_ts_packet_counted_no_panic() {
        let mut pump = T2miPump::new(0x0006);
        let garbage = [0x00u8; 188]; // bad sync byte
        let events: Vec<_> = pump.feed_ts(&garbage).collect();
        assert_eq!(events.len(), 0);
        assert_eq!(pump.stats().malformed_packets, 1);
        assert_eq!(pump.stats().ts_packets, 1);
    }

    // ── (e) wrong-PID TS packet → ignored cheaply ────────────────────────────

    #[test]
    fn wrong_pid_ts_packet_ignored() {
        let payload = [0x00u8, 0x00, 0x00];
        let t2mi = make_t2mi_packet(0x00, &payload);
        let pkt = ts_packet(0x0100, &t2mi, true, 0); // PID 0x0100, pump listens on 0x0006

        let mut pump = T2miPump::new(0x0006);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();

        assert_eq!(events.len(), 0, "wrong-PID packet must not emit");
        // ts_packets incremented, but nothing else moves.
        let stats = pump.stats();
        assert_eq!(stats.ts_packets, 1);
        assert_eq!(stats.t2mi_packets, 0);
        assert_eq!(stats.crc_failures, 0);
        assert_eq!(stats.malformed_packets, 0);
    }

    // ── additional: header() lazy parse ──────────────────────────────────────

    #[test]
    fn event_header_lazy_parse_matches_packet_type() {
        let payload = [0x00u8; 11]; // Timestamp payload
        let t2mi = make_t2mi_packet(0x20, &payload);
        let pkt = ts_packet(0x0010, &t2mi, true, 0);

        let mut pump = T2miPump::new(0x0010);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();
        assert_eq!(events.len(), 1);

        let hdr = events[0].header().expect("header parse should succeed");
        assert_eq!(hdr.packet_type as u8, 0x20);
        assert_eq!(hdr.packet_count, 0x01);
    }

    // ── additional: stats() method ───────────────────────────────────────────

    #[test]
    fn stats_accumulate_across_feeds() {
        let payload = [0x00u8, 0x00, 0x00];
        let t2mi = make_t2mi_packet(0x00, &payload);
        let pkt = ts_packet(0x0006, &t2mi, true, 0);

        let mut pump = T2miPump::new(0x0006);
        pump.feed_ts(&pkt).for_each(drop);
        pump.feed_ts(&pkt).for_each(drop);

        let stats = pump.stats();
        assert_eq!(stats.ts_packets, 2);
        // The reassembler resets on PUSI so we get 2 complete packets.
        assert_eq!(stats.t2mi_packets, 2);
    }

    // ── payload_with registry seam ───────────────────────────────────────────

    #[test]
    fn payload_with_dispatches_custom_registered_type() {
        use crate::payload::registry::PayloadRegistry;
        use crate::traits::PayloadDef;
        use dvb_common::Parse;

        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct TestPrivatePayload {
            val: u8,
        }

        impl<'a> Parse<'a> for TestPrivatePayload {
            type Error = crate::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.is_empty() {
                    return Err(crate::Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "TestPrivatePayload",
                    });
                }
                Ok(Self { val: bytes[0] })
            }
        }

        impl<'a> PayloadDef<'a> for TestPrivatePayload {
            const PACKET_TYPE: u8 = 0x00;
            const NAME: &'static str = "TEST_PRIVATE";
        }

        let mut reg = PayloadRegistry::new();
        reg.register::<TestPrivatePayload>();

        let private_payload = [0x42u8, 0x02, 0x00];
        let t2mi = make_t2mi_packet(0x00, &private_payload);
        let pkt = ts_packet(0x0006, &t2mi, true, 0);

        let mut pump = T2miPump::new(0x0006);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();
        assert_eq!(events.len(), 1, "expected one event");

        let result = events[0].payload_with(&reg).expect("payload_with parse");
        match result {
            AnyPayload::Other {
                packet_type,
                ref value,
            } => {
                assert_eq!(packet_type, 0x00);
                let downcast = value.downcast_ref::<TestPrivatePayload>().unwrap();
                assert_eq!(downcast.val, 0x42);
            }
            other => panic!("expected Other, got {other:?}"),
        }

        let built_in = events[0].payload().expect("payload parse");
        assert!(
            matches!(built_in, AnyPayload::Bbframe(_)),
            "expected Bbframe via built-in dispatch, got {built_in:?}"
        );
    }

    // ── payload_with with genuinely-private packet type (not in PacketType) ──

    #[test]
    fn payload_with_dispatches_genuinely_private_packet_type() {
        use crate::payload::registry::PayloadRegistry;
        use crate::traits::PayloadDef;
        use dvb_common::Parse;

        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct PrivatePayload {
            val: u8,
        }

        impl<'a> Parse<'a> for PrivatePayload {
            type Error = crate::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.is_empty() {
                    return Err(crate::Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "PrivatePayload",
                    });
                }
                Ok(Self { val: bytes[0] })
            }
        }

        impl<'a> PayloadDef<'a> for PrivatePayload {
            const PACKET_TYPE: u8 = 0x42;
            const NAME: &'static str = "PRIVATE_0X42";
        }

        let mut reg = PayloadRegistry::new();
        reg.register::<PrivatePayload>();

        let private_body = [0xABu8];
        let t2mi = make_t2mi_packet(0x42, &private_body);
        let pkt = ts_packet(0x0006, &t2mi, true, 0);

        let mut pump = T2miPump::new(0x0006);
        let events: Vec<_> = pump.feed_ts(&pkt).collect();
        assert_eq!(events.len(), 1, "expected one event");

        let result = events[0].payload_with(&reg).expect("payload_with parse");
        match result {
            AnyPayload::Other {
                packet_type,
                ref value,
            } => {
                assert_eq!(packet_type, 0x42);
                let downcast = value.downcast_ref::<PrivatePayload>().unwrap();
                assert_eq!(downcast.val, 0xAB);
            }
            other => panic!("expected Other, got {other:?}"),
        }

        let no_reg = events[0].payload().expect("payload without registry");
        match no_reg {
            AnyPayload::Unknown {
                packet_type,
                body: _,
            } => {
                assert_eq!(packet_type, 0x42);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
