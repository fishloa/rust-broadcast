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

use alloc::vec::Vec;
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
/// Byte 1 bit 7 = TEI (Transport Error Indicator).
const TEI_MASK: u8 = 0x80;
/// Byte 1 bit 6 = PUSI (Payload Unit Start Indicator).
const PUSI_MASK: u8 = 0x40;
/// Byte 1 bits 4..=0 = PID upper 5 bits.
const PID_MASK_HI: u8 = 0x1F;
/// Byte 3 bit 5 = adaptation_field_control bit 1 (adaptation field present).
const ADAPTATION_FLAG: u8 = 0x20;
/// Byte 3 bit 4 = adaptation_field_control bit 0 (payload present).
const PAYLOAD_FLAG: u8 = 0x10;
/// Byte 3 bits `[3:0]` = 4-bit continuity_counter.
const CC_MASK: u8 = 0x0F;

/// Minimal result of TS header parsing needed by the pump.
struct TsInfo {
    pid: u16,
    pusi: bool,
    /// Transport Error Indicator — set by the demodulator when an
    /// uncorrectable error is present in the packet (W-T2-2).
    tei: bool,
    /// 4-bit continuity_counter, used to detect the one legal repeated
    /// packet ISO/IEC 13818-1 §2.4.3.3 allows (W-T2-2).
    cc: u8,
    /// Byte offset within the 188-byte packet where the payload starts.
    payload_start: usize,
}

/// Parse the 4-byte MPEG-TS header and skip any adaptation field.
///
/// A local, minimal decode rather than a dependency on `mpeg-ts`'s
/// `TsHeader`/`TsPacket` (this crate does not depend on `mpeg-ts`, and this
/// warning sweep may not add new dependencies) — but see `feed_ts` for the
/// TEI-drop and continuity-counter-duplicate handling ISO/IEC 13818-1
/// §2.4.3.3 requires (W-T2-2).
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

    let tei = (b1 & TEI_MASK) != 0;
    let pusi = (b1 & PUSI_MASK) != 0;
    let pid = (((b1 & PID_MASK_HI) as u16) << 8) | (buf[2] as u16);
    let has_adaptation = (b3 & ADAPTATION_FLAG) != 0;
    let has_payload = (b3 & PAYLOAD_FLAG) != 0;
    let cc = b3 & CC_MASK;

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
        tei,
        cc,
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
    /// Propagates [`crate::Error`] from [`broadcast_common::Parse::parse`] on [`Header`].
    pub fn header(&self) -> crate::Result<Header> {
        use broadcast_common::Parse;
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
    /// TS packets on the filtered PID dropped because
    /// `transport_error_indicator` was set (ISO/IEC 13818-1 §2.4.3.3): the
    /// demodulator has already flagged this packet as uncorrectable, so
    /// feeding it to the reassembler would only corrupt or CRC-fail a
    /// packet that was otherwise fine (W-T2-2).
    pub tei_dropped: u64,
    /// TS packets on the filtered PID skipped as the one legal repeated
    /// packet ISO/IEC 13818-1 §2.4.3.3 allows (same `continuity_counter` as
    /// the previous packet on this PID) — without this, the repeat would be
    /// appended a second time and CRC-fail an otherwise-good T2-MI packet
    /// (W-T2-2).
    pub cc_duplicates_skipped: u64,
    /// Raw-mode resynchronisations after a CRC failure (#1095, W-T2-1):
    /// incremented each time [`T2miPump::feed_raw`] recovers frame
    /// alignment by scanning forward for a CRC-valid packet start, because
    /// the CRC-failed packet's own header-implied length may itself have
    /// been corrupted.
    pub raw_resyncs: u64,
    /// Bytes discarded while raw-mode hunting, after the resync scan had
    /// already proved no CRC-valid packet start exists in them (the hunt
    /// buffer exceeded `RAW_HUNT_CAP` (private: four times the largest possible T2-MI packet) with corrupted data still arriving)
    /// (#1095, W-T2-1).
    pub raw_hunt_bytes_discarded: u64,
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
    /// TS-mode only: the last-seen `continuity_counter` on the filtered PID,
    /// for detecting the one legal repeated packet (W-T2-2).
    last_cc: Option<u8>,
    /// TS-mode only: the last-seen payload bytes on the filtered PID, paired
    /// with `last_cc` — ISO/IEC 13818-1 §2.4.3.3 defines the one legal
    /// repeated packet as byte-identical content with an unchanged counter,
    /// not merely an unchanged counter (W-T2-2).
    last_payload: Vec<u8>,
    /// Raw-mode-only: bytes accumulated while resynchronising after framing
    /// was lost (a CRC failure whose header-implied `payload_len_bits` may
    /// itself have been corrupted, so the true next packet's start is
    /// unknown). TS-encapsulated mode never needs this: it self-heals from
    /// the next PUSI (ETSI TS 102 773 §6.1.1); raw mode has no such signal,
    /// so without this a single corrupted length desyncs the stream
    /// permanently (#1095, W-T2-1). Empty means "not currently hunting";
    /// while hunting, every `feed_raw` byte lands here instead of the
    /// reassembler, so the two framings never interleave.
    raw_hunt_buf: Vec<u8>,
    /// Raw-mode-only: next unscanned candidate offset within
    /// [`Self::raw_hunt_buf`], maintained by [`hunt_extract`] so no byte is
    /// ever examined twice.
    raw_hunt_next: usize,
    /// Raw-mode-only: the framing is trusted — the reassembler extends
    /// packets normally. False from a CRC failure until the resync scan
    /// picks a new framing point, whether by validating packets or by
    /// syncing speculatively on the straddling candidate; while false,
    /// every `feed_raw` routes through the scan instead
    /// (#1095, W-T2-1).
    raw_in_sync: bool,
}

/// Bound on [`T2miPump::raw_hunt_buf`]'s growth while corrupted raw-mode
/// data keeps arriving without ever producing a CRC-valid resync point: a
/// T2-MI packet's `payload_len_bits` is 16 bits, so the largest possible
/// packet is `HEADER_LEN(6) + 8192 + CRC_LEN(4)` = 8202 bytes; four times
/// that bounds memory while still giving the scan room to find a resync
/// point spanning more than one packet's worth of corrupted data.
const RAW_HUNT_CAP: usize = 8202 * 4;

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
            last_cc: None,
            last_payload: Vec::new(),
            raw_hunt_buf: Vec::new(),
            raw_hunt_next: 0,
            raw_in_sync: true,
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
            last_cc: None,
            last_payload: Vec::new(),
            raw_hunt_buf: Vec::new(),
            raw_hunt_next: 0,
            raw_in_sync: true,
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
    /// is incremented). A packet with `transport_error_indicator` set, or the
    /// one legal repeated packet with an unchanged `continuity_counter`
    /// (ISO/IEC 13818-1 §2.4.3.3), is counted ([`Stats::tei_dropped`] /
    /// [`Stats::cc_duplicates_skipped`]) and not forwarded to the
    /// reassembler (W-T2-2).
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
                    Some(info) if info.pid == filter_pid => {
                        let payload = &packet[info.payload_start..TS_PACKET_SIZE];
                        if info.tei {
                            // The demodulator has already flagged this
                            // packet as uncorrectable; feeding it would only
                            // corrupt or CRC-fail an otherwise-good packet.
                            self.stats.tei_dropped += 1;
                        } else if self.last_cc == Some(info.cc) && self.last_payload == payload {
                            // The one legal repeat (§2.4.3.3): the spec
                            // defines this as byte-identical payload with an
                            // unchanged counter, not merely an unchanged
                            // counter — a corrupted/non-incrementing counter
                            // whose payload actually differs is a
                            // conformance error elsewhere (indicator 1.4),
                            // not something to silently drop here.
                            self.stats.cc_duplicates_skipped += 1;
                        } else {
                            self.last_cc = Some(info.cc);
                            self.last_payload.clear();
                            self.last_payload.extend_from_slice(payload);
                            self.reasm.feed(payload, info.pusi);
                            self.drain_reasm();
                        }
                    }
                    Some(_) => {
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
    /// Unlike TS-encapsulated mode, this stream has no PUSI to resynchronise
    /// from: if a CRC failure means the failed packet's own header-implied
    /// length may have been corrupted, [`T2miPump`] scans forward for the
    /// next CRC-valid packet start instead of staying desynchronised forever
    /// (#1095, W-T2-1; see [`Stats::raw_resyncs`]).
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
                    self.drain_reasm();
                } else if self.raw_hunt_buf.is_empty() && self.raw_in_sync {
                    // In-sync continuation: feed without PUSI — bytes extend
                    // the current T2-MI packet in progress.
                    self.reasm.feed(data, false);
                    self.drain_reasm();
                } else if !self.raw_hunt_buf.is_empty() {
                    // Hunting: the framing is untrusted, so all new bytes go
                    // to the hunt buffer (never the reassembler) and the
                    // resync scan retries over the extended region.
                    self.raw_hunt_buf.extend_from_slice(data);
                    self.hunt();
                } else {
                    // Untrusted framing, scan buffer empty: everything since
                    // the last resync point is buffered in the reassembler
                    // (a straddle-sync left it there mid-packet). Fold it
                    // back into the scan buffer and let the scan judge the
                    // whole region again — including any packet that
                    // completed in the meantime (#1095, W-T2-1).
                    let buffered = self.reasm.take_buffered();
                    self.reasm.drop_buffered();
                    self.raw_hunt_buf = buffered.to_vec();
                    self.raw_hunt_next = 0;
                    self.raw_hunt_buf.extend_from_slice(data);
                    self.hunt();
                }
            }
        }

        self.scratch.drain(..)
    }

    /// Drain all pending packets from the reassembler, CRC-validate each one,
    /// and push valid packets to `scratch`. In raw mode, a CRC failure
    /// enters the resync hunt (`hunt`) instead of trusting the rest of this
    /// batch, since the failed packet's own header-implied length — which
    /// the reassembler used to decide where every packet after it starts —
    /// may itself have been the corrupted field (#1095, W-T2-1).
    fn drain_reasm(&mut self) {
        let raw_mode = matches!(self.mode, PumpMode::Raw);
        while let Some(raw) = self.reasm.pop_packet() {
            self.stats.t2mi_packets += 1;
            match crc::validate_crc(&raw) {
                Ok(()) => self.scratch.push(T2miEvent { bytes: raw }),
                Err(_) => {
                    self.stats.crc_failures += 1;
                    if raw_mode {
                        // Enter hunting with the failed packet followed by
                        // whatever the reassembler still holds (its own
                        // bytes, or already-reframed bytes that must be
                        // dropped, never mixed in), then scan.
                        let buffered = self.reasm.take_buffered();
                        let mut hunt = Vec::with_capacity(raw.len() + buffered.len());
                        hunt.extend_from_slice(&raw);
                        hunt.extend_from_slice(&buffered);
                        self.reasm.drop_buffered();
                        self.raw_hunt_buf = hunt;
                        self.raw_hunt_next = 0;
                        self.hunt();
                        return;
                    }
                }
            }
        }
    }

    /// Advance the raw-mode resync scan over [`Self::raw_hunt_buf`] and
    /// drain whatever CRC-valid packets it can now decide.
    ///
    /// Extraction happens directly from the hunt buffer: the normal
    /// [`PacketReassembler`] is *not* reframed while hunting, because its
    /// framing is exactly what is untrusted. When the scan resyncs, the
    /// reassembler is seeded with the bytes from the resync point as a
    /// fresh PUSI-style pointer-field-0 init (exactly like the first
    /// raw-mode call), so a T2-MI packet straddling the resync boundary
    /// completes via the reassembler's own continuation handling
    /// (#1095, W-T2-1).
    fn hunt(&mut self) {
        let mut emitted: Vec<T2miEvent> = Vec::new();
        match hunt_extract(
            &mut self.raw_hunt_buf,
            &mut self.raw_hunt_next,
            &mut emitted,
        ) {
            HuntStep::Found { remainder } => {
                // The scan validated every packet that fits the buffer;
                // their bytes go straight to `scratch` (the CRC gate is the
                // scan itself). Framing then restarts from the resync point
                // with only the undecided tail: the reassembler is seeded
                // with the §6.1.1 pointer-field-0 init carrying the tail, so
                // the next `feed_raw` completes the straddling packet
                // through its continuation handling (#1095, W-T2-1).
                for event in emitted {
                    self.scratch.push(event);
                }
                let tail_len = remainder.len();
                self.raw_hunt_buf = remainder;
                self.raw_hunt_next = 0;
                self.stats.raw_resyncs += 1;
                self.raw_in_sync = true;
                self.reasm.drop_buffered();
                if tail_len > 0 {
                    let mut init = Vec::with_capacity(1 + tail_len);
                    init.push(0x00); // pointer_field = 0
                    init.extend_from_slice(&self.raw_hunt_buf);
                    self.reasm.feed(&init, true);
                    // The seed *is* the undecided tail (the scan proved no
                    // complete packet remains), so nothing may frame yet.
                    debug_assert!(
                        self.reasm.pop_packet().is_none(),
                        "a pointer-0 seed of the undecided tail cannot frame a packet"
                    );
                }
            }
            HuntStep::Straddle { offset } => {
                // The scan walked every candidate whose implied packet
                // *fits* the buffer and rejected them all. `offset` is the
                // first offset that may still grow into a valid packet:
                // either a plausible header (all 6 bytes readable —
                // allocated `packet_type`, zero RFU bytes, ETSI TS 102 773
                // §5.1 Table 1) whose implied packet overruns the remaining
                // buffer, or, in the final bytes where no complete header
                // fits, a plausible partial header
                // ([`crate::ts::partial_header_plausible`]) with fewer bytes
                // remaining than the smallest possible packet. Nothing
                // behind it is decidable either — a complete valid packet's
                // end always coincides with a buffer end, which the scan
                // judges on the way — so it is *the* straddle point. Sync
                // on it now: the prefix is proven start-free and dropped,
                // and the straddling bytes are seeded into the reassembler
                // behind a §6.1.1 pointer-field-0 init so the next
                // `feed_raw` extends them as an ordinary continuation,
                // instead of the framing the scan exists to distrust piling
                // undecided bytes forever. Speculative: nothing emits
                // without a CRC pass, and the framing stays untrusted —
                // every later feed first folds the reassembler's buffer
                // back into the scan ([`Self::feed_raw`]) until a pass
                // validates a packet (#1095, W-T2-1).
                let straddler = self.raw_hunt_buf.split_off(offset);
                let straddle_len = straddler.len();
                self.raw_hunt_buf.clear();
                self.raw_hunt_next = 0;
                self.stats.raw_resyncs += 1;
                self.raw_in_sync = false;
                self.reasm.drop_buffered();
                if straddle_len > 0 {
                    let mut init = Vec::with_capacity(1 + straddle_len);
                    init.push(0x00); // pointer_field = 0
                    init.extend_from_slice(&straddler);
                    self.reasm.feed(&init, true);
                }
            }
            HuntStep::Nothing => {
                // The scan traversed everything decidable and found
                // nothing valid: bound memory while corrupted data keeps
                // arriving. The discarded prefix was proven free of valid
                // packet starts (a valid packet's end always coincides with
                // a buffer end, which the scan judges before anything
                // behind it), so it can never begin the resync point.
                if self.raw_hunt_buf.len() > RAW_HUNT_CAP {
                    let keep_from = self.raw_hunt_buf.len() - RAW_HUNT_CAP;
                    self.raw_hunt_buf.drain(..keep_from);
                    self.raw_hunt_next = self.raw_hunt_next.saturating_sub(keep_from);
                    self.stats.raw_hunt_bytes_discarded += keep_from as u64;
                }
            }
        }
    }
}

/// What one pass of [`hunt_extract`] decided.
enum HuntStep {
    /// At least one CRC-valid packet was found (moved into the caller's
    /// `emitted` list); the payload is whatever followed the last one
    /// (empty when it reached the buffer end).
    Found { remainder: Vec<u8> },
    /// The first offset that may still grow into a valid packet — sync
    /// there speculatively and wait for the rest; see [`T2miPump::hunt`].
    Straddle { offset: usize },
    /// Nothing decidable and nothing plausible in the tail; keep buffering.
    Nothing,
}

/// Advance the raw-mode resync scan over `hunt` — the resync buffer —
/// starting at and updating `next` (the next unscanned candidate offset).
///
/// A candidate passes when its header is plausible (allocated
/// `packet_type`, per ETSI TS 102 773 §5.1 Table 1, zero RFU bytes), its
/// implied length fits the buffer, and its CRC-32 trailer validates. The
/// scan walks forward one offset at a time — one cheap plausibility test
/// per byte, so garbage costs O(1) per byte rather than one full CRC per
/// candidate — and *fully decides* every offset whose implied packet fits
/// the buffer: an over-long `payload_len_bits` (a bit error, or a lost UDP
/// datagram) is a decided rejection, not a reason to stop, because a valid
/// packet starting behind it would still have to end at a buffer end, which
/// the scan judges on the way. The original bug was exactly trusting that
/// claim to block the candidates behind it (#1095, W-T2-1).
///
/// The scan stops at the first offset that may still become valid — the
/// straddle point — in either of two forms:
///
/// - a plausible header (all 6 bytes readable) whose implied packet overruns
///   the remaining buffer: the arriving bytes may complete it;
/// - in the final bytes, where no complete header fits at all, a plausible
///   partial header ([`crate::ts::partial_header_plausible`]) with fewer
///   bytes remaining than the smallest possible packet (header + CRC + one
///   payload byte): the arriving bytes can only extend it.
///
/// An *implausible* header is rejected even when it overruns (random garbage
/// must stay O(1) per byte). The pump syncs the reassembler on the straddle
/// point and keeps scanning until a pass validates a packet; a wrong guess
/// just fails the CRC later. Bytes the scan skipped between rejected
/// candidates are proven start-free and dropped.
fn hunt_extract(hunt: &mut Vec<u8>, next: &mut usize, emitted: &mut Vec<T2miEvent>) -> HuntStep {
    // Smallest byte count a real T2-MI packet can occupy: 6-byte header plus
    // 4-byte CRC plus at least one payload bit (one byte), per ETSI TS 102
    // 773 §5.1 — `Header::total_len` of a zero-payload header.
    const MIN_PACKET_LEN: usize = crate::ts::HEADER_LEN + crate::crc::CRC_LEN + 1;
    let mut offset = *next;
    let mut last_end = 0usize;
    loop {
        if offset + crate::ts::HEADER_LEN > hunt.len() {
            // No complete header fits here (nor at any later offset — this
            // is the first such offset the scan reached). Judge the tail
            // straddle exactly once.
            if offset > 0
                && hunt.len() - offset < MIN_PACKET_LEN
                && crate::ts::partial_header_plausible(&hunt[offset..])
            {
                *next = offset;
                return HuntStep::Straddle { offset };
            }
            break;
        }
        let Some(total_len) = crate::ts::implied_packet_len(&hunt[offset..]) else {
            break;
        };
        let end = offset + total_len;
        if end > hunt.len() {
            // Overruns the buffer: a plausible header is the straddle
            // candidate (stop); an implausible one is rejected and the scan
            // keeps walking, which is what stops a corrupt over-long
            // `payload_len_bits` from blocking the good packet behind it.
            if emitted.is_empty() && crate::ts::header_plausible(&hunt[offset..]) {
                *next = offset;
                return HuntStep::Straddle { offset };
            }
            offset += 1;
            continue;
        }
        if crate::ts::header_plausible(&hunt[offset..])
            && crc::validate_crc(&hunt[offset..end]).is_ok()
        {
            emitted.push(T2miEvent {
                bytes: hunt[offset..end].to_vec().into(),
            });
            // Continue scanning right after the packet: a healthy stream
            // resumes back-to-back.
            offset = end;
            last_end = end;
            continue;
        }
        offset += 1;
    }
    if !emitted.is_empty() {
        let remainder = hunt.split_off(last_end);
        *next = 0;
        return HuntStep::Found { remainder };
    }
    *next = offset.min(hunt.len());
    HuntStep::Nothing
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::crc32_mpeg2;

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
        ts_packet_cc(pid, t2mi_data, pusi, pointer_field, 0)
    }

    /// Like [`ts_packet`], with an explicit `continuity_counter` (needed to
    /// exercise the W-T2-2 duplicate-vs-distinct-packet distinction, which
    /// `ts_packet`'s fixed `cc = 0` cannot).
    fn ts_packet_cc(
        pid: u16,
        t2mi_data: &[u8],
        pusi: bool,
        pointer_field: u8,
        cc: u8,
    ) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = if pusi { PUSI_MASK } else { 0 };
        pkt[1] |= ((pid >> 8) as u8) & PID_MASK_HI;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = PAYLOAD_FLAG | (cc & CC_MASK); // payload present, no adaptation field
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

    // ── (b2) raw mode: a corrupted header length must not desync forever ─────

    #[test]
    fn raw_mode_resyncs_after_corrupted_header_length() {
        // W-T2-1: in raw mode there is no PUSI to heal from. A corrupted
        // `payload_len_bits` makes the first packet extract short, its CRC
        // fail, and the reassembler believe every later packet starts at
        // the wrong offset. Before the fix the pump trusted this framing
        // forever and the good packet was never emitted.
        let good = make_t2mi_packet(0x20, &[0x00u8; 11]);
        let filler = make_t2mi_packet(0x20, &[0x5Au8; 300]);
        let mut corrupt = filler.clone();
        // payload_len_bits 2400 -> 1376: the first "packet" is cut 128
        // bytes short.
        let deflated = (((corrupt[4] as u16) << 8) | corrupt[5] as u16) - 1024;
        corrupt[4] = (deflated >> 8) as u8;
        corrupt[5] = deflated as u8;
        // The 0x20 (Timestamp) payload is all zeros, which would make the
        // mis-cut packet's trailing zeros accidentally plausible; with the
        // filler body forced to 0x5A, only the good packet's real start
        // can validate.
        for b in corrupt.iter_mut().skip(6) {
            *b = 0x5A;
        }
        let mut stream = corrupt.clone();
        stream.extend_from_slice(&good);

        // Feed up to the mis-cut packet's implied end: it extracts there
        // and CRC-fails, putting the pump into hunting.
        let mis_cut_len = 6 + (deflated as usize).div_ceil(8) + 4;
        let mut pump = T2miPump::raw();
        assert!(
            pump.feed_raw(&stream[..mis_cut_len]).next().is_none(),
            "nothing valid may be emitted yet"
        );
        assert_eq!(
            pump.stats().crc_failures,
            1,
            "the mis-cut packet must CRC-fail"
        );
        assert_eq!(pump.stats().raw_resyncs, 0, "no resync point yet");

        // The rest of the stream (including the good packet) arrives while
        // still hunting: the scan must find the good packet's true start.
        let more: Vec<_> = pump.feed_raw(&stream[mis_cut_len..]).collect();

        let stats = pump.stats();
        assert_eq!(stats.raw_resyncs, 1, "the pump must find the realignment");
        assert_eq!(
            more.len(),
            1,
            "the good packet following the corruption must still be emitted"
        );
        assert_eq!(
            &more[0].bytes[..],
            &good[..],
            "with the original bytes intact"
        );

        // Stream continues normally after resync.
        let after: Vec<_> = pump.feed_raw(&good).collect();
        assert_eq!(after.len(), 1, "post-resync feeds work as usual");
        assert_eq!(pump.stats().raw_resyncs, 1, "no spurious extra resyncs");
    }

    #[test]
    fn raw_mode_resync_completes_a_packet_straddling_the_boundary() {
        // The good packet's tail straddles the resync boundary: the first
        // feed ends mid-packet (undecided remainder), the second completes
        // it. The straddling packet must still come out whole.
        let good = make_t2mi_packet(0x20, &[0x00u8; 11]);
        let mut corrupt = make_t2mi_packet(0x20, &[0x5Au8; 300]);
        let deflated = (((corrupt[4] as u16) << 8) | corrupt[5] as u16) - 1024;
        corrupt[4] = (deflated >> 8) as u8;
        corrupt[5] = deflated as u8;
        for b in corrupt.iter_mut().skip(6) {
            *b = 0x5A;
        }

        let mut pump = T2miPump::raw();
        // Feed the corrupt packet plus the first 5 bytes of the good one.
        let mut first = corrupt.clone();
        first.extend_from_slice(&good[..5]);
        assert!(pump.feed_raw(&first).next().is_none());
        assert_eq!(pump.stats().raw_resyncs, 1, "resync at the good header");

        // The remaining bytes complete the straddling packet.
        let rest: Vec<_> = pump.feed_raw(&good[5..]).collect();
        assert_eq!(rest.len(), 1, "the straddling packet must complete intact");
        assert_eq!(&rest[0].bytes[..], &good[..]);
    }

    #[test]
    fn raw_mode_hunt_for_garbage_is_bounded_and_cheap() {
        // W-T2-1: 10 000 feeds of LCG garbage must terminate promptly (the
        // resync scan gates candidates on header plausibility, so garbage
        // costs O(1) per byte, not one full CRC per candidate offset),
        // must bound its hunt buffer, and must never panic.
        struct Lcg(u64);
        impl Lcg {
            fn next_u8(&mut self) -> u8 {
                self.0 = self
                    .0
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (self.0 >> 33) as u8
            }
        }
        let started = std::time::Instant::now();
        let mut pump = T2miPump::raw();
        let mut lcg = Lcg(0xCAFE_F00D_1234_5678u64);
        for _ in 0..10_000u32 {
            let len = 1 + (lcg.next_u8() as usize % 256);
            let data: Vec<u8> = (0..len).map(|_| lcg.next_u8()).collect();
            let _: Vec<_> = pump.feed_raw(&data).collect();
            assert!(
                pump.raw_hunt_buf.len() <= RAW_HUNT_CAP,
                "the hunt buffer must stay bounded while hunting"
            );
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "garbage resync must be O(1) per byte, took {elapsed:?}"
        );
        let stats = pump.stats();
        assert!(
            stats.raw_hunt_bytes_discarded > 0,
            "bounded hunting must discard bytes; stats: {stats:?}"
        );
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
        // Distinct continuity_counter values: two genuinely different TS
        // packets, not the one legal repeat (W-T2-2) — see
        // `cc_duplicate_packet_is_skipped_not_double_counted` below for that.
        let pkt0 = ts_packet_cc(0x0006, &t2mi, true, 0, 0);
        let pkt1 = ts_packet_cc(0x0006, &t2mi, true, 0, 1);

        let mut pump = T2miPump::new(0x0006);
        pump.feed_ts(&pkt0).for_each(drop);
        pump.feed_ts(&pkt1).for_each(drop);

        let stats = pump.stats();
        assert_eq!(stats.ts_packets, 2);
        // The reassembler resets on PUSI so we get 2 complete packets.
        assert_eq!(stats.t2mi_packets, 2);
        assert_eq!(stats.cc_duplicates_skipped, 0);
    }

    /// W-T2-2: ISO/IEC 13818-1 §2.4.3.3 allows exactly one repeated packet
    /// (same `continuity_counter`) on a PID. Before the fix, `feed_ts` fed
    /// every packet unconditionally, so a legal repeat was reassembled
    /// twice, and (for a PUSI'd repeat) the reassembler's next-packet-start
    /// bookkeeping would even see the same T2-MI bytes as two separate
    /// packets.
    #[test]
    fn cc_duplicate_packet_is_skipped_not_double_counted() {
        let payload = [0x00u8, 0x00, 0x00];
        let t2mi = make_t2mi_packet(0x00, &payload);
        let pkt = ts_packet_cc(0x0006, &t2mi, true, 0, 5);

        let mut pump = T2miPump::new(0x0006);
        let first: Vec<_> = pump.feed_ts(&pkt).collect();
        let second: Vec<_> = pump.feed_ts(&pkt).collect(); // same CC: legal repeat

        assert_eq!(first.len(), 1, "the first (genuine) packet must be emitted");
        assert!(
            second.is_empty(),
            "the repeated packet must not be reassembled a second time"
        );
        let stats = pump.stats();
        assert_eq!(stats.ts_packets, 2);
        assert_eq!(stats.t2mi_packets, 1);
        assert_eq!(stats.cc_duplicates_skipped, 1);
    }

    // ── payload_with registry seam ───────────────────────────────────────────

    #[test]
    fn payload_with_dispatches_custom_registered_type() {
        use crate::payload::registry::PayloadRegistry;
        use crate::traits::PayloadDef;
        use broadcast_common::Parse;

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

        impl PayloadDef<'_> for TestPrivatePayload {
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
        use broadcast_common::Parse;

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

        impl PayloadDef<'_> for PrivatePayload {
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
