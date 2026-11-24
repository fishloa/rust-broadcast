//! A worked example of extending [`isobmff`] from outside the crate.
//!
//! The crate defines two things `isobmff` has never heard of:
//!
//! - [`AcmeCalibrationBox`] — a private FullBox (`acal`) that lives inside
//!   `moov`'s `udta`, with two 16.16 fixed-point values and a short label;
//! - [`AcmeTelemetryEntry`] — a private sample entry (`actm`) for a metadata
//!   track, carrying one 32-bit serial number.
//!
//! Both implement [`BoxDef`], so a [`BoxRegistry`] built by the caller parses
//! them; without a registry the same bytes read back as an opaque sample entry
//! and an unparsed `udta` child, and write back unchanged.
//!
//! This is the documentation a third party copies: no `isobmff` source has to
//! change for either type to be typed. See `README.md` and
//! `tests/round_trip.rs`.

use broadcast_common::Parse;
use broadcast_common::Serialize;

use isobmff::Error;
use isobmff::box_types::{BOX_HEADER_MIN_SIZE, write_box_header, write_full_box_header};
use isobmff::parse_box;
use isobmff::registry::{BoxDef, BoxRegistry};

/// The four-CC of [`AcmeCalibrationBox`].
pub const CALIBRATION_FOURCC: [u8; 4] = *b"acal";
/// The four-CC of [`AcmeTelemetryEntry`].
pub const TELEMETRY_FOURCC: [u8; 4] = *b"actm";
/// The handler a telemetry track declares.
pub const TELEMETRY_HANDLER: [u8; 4] = *b"meta";
/// Bytes before the payload word in a sample entry (reserved + `data_reference_index`).
pub const ENTRY_PREFIX_LEN: usize = 8;
/// Width of a FullBox `version`/`flags` field.
pub const VERSION_AND_FLAGS_LEN: usize = 4;
/// Width of a 16.16 fixed-point or 32-bit field.
pub const FIELD_LEN: usize = 4;
/// Offset of the gain inside a calibration body.
pub const GAIN_OFFSET: usize = 0;
/// Offset of the shift inside a calibration body.
pub const SHIFT_OFFSET: usize = GAIN_OFFSET + FIELD_LEN;
/// Offset of the label length inside a calibration body.
pub const LABEL_LEN_OFFSET: usize = SHIFT_OFFSET + FIELD_LEN;
/// Offset of the label bytes inside a calibration body.
pub const LABEL_OFFSET: usize = LABEL_LEN_OFFSET + 1;
/// Bytes of a calibration box before its label: the FullBox
/// `version`/`flags` and the body fields up to the label.
pub const CALIBRATION_FIXED_LEN: usize = VERSION_AND_FLAGS_LEN + LABEL_OFFSET;
/// Bytes of the telemetry payload word.
pub const TELEMETRY_PAYLOAD_LEN: usize = 4;
/// `acal` is a `udta` atom.
pub const CALIBRATION_PARENT: [u8; 4] = *b"udta";
/// `actm` is a sample entry inside `stsd`.
pub const TELEMETRY_PARENT: [u8; 4] = *b"stsd";

/// A private FullBox: two 16.16 fixed-point values and a counted label.
#[derive(Debug, Clone, PartialEq)]
pub struct AcmeCalibrationBox {
    /// The gain, 16.16 fixed point.
    pub gain: i32,
    /// The offset applied after the gain, 16.16 fixed point.
    pub shift: i32,
    /// A short ASCII label.
    pub label: Vec<u8>,
}

impl<'a> Parse<'a> for AcmeCalibrationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let body = parse_box(bytes)?.0.body;
        let rest = body.get(4..).ok_or(Error::InvalidInput(
            "acal body is shorter than its FullBox header",
        ))?;
        let field = |at: usize| -> Result<i32, Error> {
            let slice = rest
                .get(at..at + FIELD_LEN)
                .ok_or(Error::InvalidInput("acal body is shorter than its fields"))?;
            let array: [u8; FIELD_LEN] = slice
                .try_into()
                .map_err(|_| Error::InvalidInput("acal field width"))?;
            Ok(i32::from_be_bytes(array))
        };
        let gain = field(GAIN_OFFSET)?;
        let shift = field(SHIFT_OFFSET)?;
        let len = usize::from(
            *rest
                .get(LABEL_LEN_OFFSET)
                .ok_or(Error::InvalidInput("acal has no label length"))?,
        );
        let label = rest
            .get(LABEL_OFFSET..LABEL_OFFSET + len)
            .ok_or(Error::InvalidInput("acal label runs past the box"))?
            .to_vec();
        Ok(Self { gain, shift, label })
    }
}

impl Serialize for AcmeCalibrationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + CALIBRATION_FIXED_LEN + self.label.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let need = self.serialized_len();
        let mut written = write_full_box_header(buf, &CALIBRATION_FOURCC, need, 0, 0, false)?;
        buf[written..written + FIELD_LEN].copy_from_slice(&self.gain.to_be_bytes());
        written += FIELD_LEN;
        buf[written..written + FIELD_LEN].copy_from_slice(&self.shift.to_be_bytes());
        written += FIELD_LEN;
        buf[written] = broadcast_common::len::fit_u8(self.label.len(), "acal label")?;
        written += 1;
        buf[written..written + self.label.len()].copy_from_slice(&self.label);
        written += self.label.len();
        Ok(written)
    }
}

impl BoxDef for AcmeCalibrationBox {
    const FOURCC: [u8; 4] = CALIBRATION_FOURCC;
    const PARENT: Option<[u8; 4]> = Some(CALIBRATION_PARENT);
}

/// A private sample entry for a metadata track: one 32-bit serial number.
#[derive(Debug, Clone, PartialEq)]
pub struct AcmeTelemetryEntry {
    /// The capture's serial number.
    pub serial: u32,
}

impl<'a> Parse<'a> for AcmeTelemetryEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let body = parse_box(bytes)?.0.body;
        let slice = body
            .get(ENTRY_PREFIX_LEN..ENTRY_PREFIX_LEN + TELEMETRY_PAYLOAD_LEN)
            .ok_or(Error::InvalidInput(
                "actm body is shorter than its entry prefix",
            ))?;
        let array: [u8; FIELD_LEN] = slice
            .try_into()
            .map_err(|_| Error::InvalidInput("actm field width"))?;
        Ok(Self {
            serial: u32::from_be_bytes(array),
        })
    }
}

impl Serialize for AcmeTelemetryEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + ENTRY_PREFIX_LEN + TELEMETRY_PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let need = self.serialized_len();
        let mut written = write_box_header(buf, &TELEMETRY_FOURCC, need)?;
        buf[written..written + ENTRY_PREFIX_LEN].fill(0);
        buf[written + 1] = 1;
        written += ENTRY_PREFIX_LEN;
        buf[written..written + TELEMETRY_PAYLOAD_LEN].copy_from_slice(&self.serial.to_be_bytes());
        written += TELEMETRY_PAYLOAD_LEN;
        Ok(written)
    }
}

impl BoxDef for AcmeTelemetryEntry {
    const FOURCC: [u8; 4] = TELEMETRY_FOURCC;
    const PARENT: Option<[u8; 4]> = Some(TELEMETRY_PARENT);
}

/// The registry a reader needs to type both types above.
#[must_use]
pub fn acme_registry() -> BoxRegistry {
    let mut registry = BoxRegistry::new();
    registry
        .register::<AcmeCalibrationBox>()
        .expect("a private four-CC this crate does not type");
    registry
        .register::<AcmeTelemetryEntry>()
        .expect("a private four-CC this crate does not type");
    registry
}
