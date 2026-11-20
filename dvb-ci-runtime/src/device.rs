//! The hardware-abstraction boundary: [`CaDevice`].
//!
//! EN 50221 runs over the Linux CA device (`/dev/dvb/adapterN/caM`): the
//! application reads/writes the TPDU link-layer byte stream and issues a few
//! ioctls (reset, slot info, capabilities). The runtime is written entirely
//! against this trait so it can be driven by a real device (the `linux`
//! feature) *or* by an in-memory mock — which is what makes the state machines
//! testable without hardware, and enables differential testing against an
//! external reference (feed both the same mock, compare the emitted
//! write/ioctl sequences).

use std::io;

/// Largest legal link-layer frame `[slot, connection_id, <TPDU>]` a
/// [`CaDevice`] can deliver: the 2-byte link header, plus the largest TPDU
/// EN 50221 Table 1 can encode — `tpdu_tag`(1), `length_field` (up to 3 bytes
/// total, its maximum-value long form: size indicator plus 2 length bytes),
/// and the `length_field`'s own max value, 65535, of `t_c_id` plus data (§7
/// semantics: "length_field codes the length of all following fields", i.e.
/// t_c_id + data).
///
/// A [`Driver`](crate::Driver)'s read buffer is sized to this, so no frame a
/// device accepts is handed back as an error purely for not fitting it
/// (audit a20-21).
pub const MAX_CA_FRAME: usize = 2 + 1 + 3 + 65_535;

/// CA-device slot status (subset of the Linux `ca_slot_info` the runtime needs).
///
/// The DVB-CA slot reports two independent bits (uapi `linux/dvb/ca.h`
/// `ca_slot_info.flags`): `CA_CI_MODULE_PRESENT` (a module is physically
/// inserted) and `CA_CI_MODULE_READY` (that module has completed its own
/// init and is usable). A module can be present-but-not-ready briefly after
/// insertion; the runtime's hot-plug edge detection
/// ([`Notification::HotPlug`](crate::event::Notification::HotPlug) carrying
/// [`HotPlug::CamPresent`](crate::event::HotPlug::CamPresent)/
/// [`CamRemoved`](crate::event::HotPlug::CamRemoved)) keys off
/// `module_present`, since that is the field that toggles on physical
/// insert/removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SlotInfo {
    /// Slot number.
    pub num: u8,
    /// `true` once a module is present and ready (`CA_CI_MODULE_READY`).
    pub module_ready: bool,
    /// `true` while a module is physically inserted (`CA_CI_MODULE_PRESENT`),
    /// regardless of whether it has finished initialising.
    pub module_present: bool,
}

/// A failed device operation, with enough context to pinpoint it from a log
/// line alone: **which operation** (`write`, `read`, `poll`, `ioctl
/// CA_RESET`, ...), **which device** (the `/dev/dvb/adapterN/...` path, or
/// the fd role for a `from_file` wrapper) and the **errno**. A bare
/// `Invalid argument (os error 22)` from `Driver::pump` (seen on a DD CXD2099
/// during init) names none of those, and the same errno means different
/// things per operation: the kernel's `dvb_ca_en50221` returns `EINVAL` from
/// `write` while the slot is not yet `RUNNING`, from `read` when the buffer is
/// smaller than the pending frame, and from `CA_GET_SLOT_INFO` for a slot it
/// does not have.
///
/// It is carried as the payload of an [`io::Error`] of the *same*
/// [`io::ErrorKind`] as the original, so `kind()`-based handling is
/// unchanged; the errno stays reachable through [`errno`](Self::errno) /
/// [`DeviceOpError::errno_of`] (a wrapped error no longer answers
/// `raw_os_error()` itself).
#[derive(Debug)]
pub struct DeviceOpError {
    /// The operation that failed, e.g. `"write (17 bytes)"`.
    pub op: String,
    /// The device, e.g. `"/dev/dvb/adapter3/ca0"`.
    pub device: String,
    source: io::Error,
}

impl DeviceOpError {
    /// Wrap `source` (an error from `op` on `device`) into an [`io::Error`] of
    /// the same kind that displays all three.
    #[must_use]
    pub fn wrap(op: impl Into<String>, device: &str, source: io::Error) -> io::Error {
        let kind = source.kind();
        io::Error::new(
            kind,
            DeviceOpError {
                op: op.into(),
                device: device.to_owned(),
                source,
            },
        )
    }

    /// The OS errno of the underlying failure, if it had one.
    #[must_use]
    pub fn errno(&self) -> Option<i32> {
        self.source.raw_os_error()
    }

    /// The OS errno of `err` whether or not it is wrapped in a
    /// [`DeviceOpError`] (a plain OS error answers for itself).
    #[must_use]
    pub fn errno_of(err: &io::Error) -> Option<i32> {
        err.raw_os_error().or_else(|| {
            err.get_ref()
                .and_then(|inner| inner.downcast_ref::<DeviceOpError>())
                .and_then(DeviceOpError::errno)
        })
    }
}

impl std::fmt::Display for DeviceOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} failed: {}", self.device, self.op, self.source)
    }
}

impl std::error::Error for DeviceOpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The link-layer device the EN 50221 runtime drives.
///
/// All methods mirror the operations a host performs on the CA file descriptor
/// per EN 50221. Implementations: [`MockCaDevice`] (in-memory, for tests +
/// differential harness) and the `linux` `CaDevice` over `/dev/dvb/.../ca`.
pub trait CaDevice {
    /// Read one link-layer TPDU frame into `buf`; returns the byte count.
    /// `Ok(0)` means no data available (non-blocking).
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Write one link-layer TPDU frame.
    fn write(&mut self, buf: &[u8]) -> io::Result<()>;

    /// Reset the interface / slot (ioctl `CA_RESET`).
    fn reset(&mut self) -> io::Result<()>;

    /// How long the module needs after [`reset`](Self::reset) returns before
    /// it accepts `Create_T_C` (the DD/cxd2099 class of drivers only
    /// re-initialise the slot seconds after `CA_RESET`).
    ///
    /// `reset` itself must not sleep for this: the sans-IO
    /// [`Driver`](crate::Driver) reads this value after `reset` and holds the
    /// transport quiet until it elapses on its injectable clock, so a CAM
    /// hot-plug edge no longer freezes `pump` (audit r10-O-10). Defaults to
    /// [`Duration::ZERO`](std::time::Duration::ZERO) — an implementation whose
    /// `reset` already blocks until the module is usable keeps working
    /// unchanged.
    fn reset_settle(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    /// Query slot status (ioctl `CA_GET_SLOT_INFO`).
    fn slot_info(&mut self) -> io::Result<SlotInfo>;

    /// Wait up to `timeout` for the device to become readable; `Ok(true)` if
    /// readable. The runtime's poll loop calls this between reads.
    fn poll(&mut self, timeout: std::time::Duration) -> io::Result<bool>;
}

/// One recorded device operation, for assertions + differential testing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceOp {
    /// A `write()` of these exact bytes.
    Write(Vec<u8>),
    /// A `reset()` ioctl.
    Reset,
    /// A `slot_info()` ioctl.
    SlotInfo,
}

/// In-memory [`CaDevice`] for tests and the differential harness.
///
/// - `inbound` is a scripted queue of frames the "module" (mock CAM) sends up;
///   each [`read`](CaDevice::read) pops one.
/// - every host-side operation is appended to `ops` so a test (or a differential
///   comparison against an external reference) can assert the exact emitted
///   `write`/ioctl sequence.
#[derive(Debug, Default)]
pub struct MockCaDevice {
    /// Scripted frames the module sends to the host (FIFO).
    pub inbound: std::collections::VecDeque<Vec<u8>>,
    /// Recorded host-side operations, in order.
    pub ops: Vec<DeviceOp>,
    /// Slot status returned by [`slot_info`](CaDevice::slot_info).
    pub slot: SlotInfo,
}

impl MockCaDevice {
    /// New mock with a ready module in slot 0 and the given inbound script.
    #[must_use]
    pub fn new(inbound: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            inbound: inbound.into_iter().collect(),
            ops: Vec::new(),
            slot: SlotInfo {
                num: 0,
                module_ready: true,
                module_present: true,
            },
        }
    }

    /// The bytes written by the host so far, concatenated (convenience for
    /// byte-exact differential comparison against the C reference).
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        self.ops
            .iter()
            .filter_map(|o| match o {
                DeviceOp::Write(b) => Some(b.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

impl CaDevice for MockCaDevice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inbound.pop_front() {
            Some(frame) => {
                let n = frame.len().min(buf.len());
                buf[..n].copy_from_slice(&frame[..n]);
                Ok(n)
            }
            None => Ok(0),
        }
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        self.ops.push(DeviceOp::Write(buf.to_vec()));
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        self.ops.push(DeviceOp::Reset);
        Ok(())
    }

    fn slot_info(&mut self) -> io::Result<SlotInfo> {
        self.ops.push(DeviceOp::SlotInfo);
        Ok(self.slot)
    }

    fn poll(&mut self, _timeout: std::time::Duration) -> io::Result<bool> {
        Ok(!self.inbound.is_empty())
    }
}

/// One link-layer event for diagnostics, captured in both directions by
/// [`RecordingCaDevice`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkEvent {
    /// Host → module: a frame the host wrote.
    Tx(Vec<u8>),
    /// Module → host: a frame the host read.
    Rx(Vec<u8>),
    /// A `reset()` ioctl.
    Reset,
    /// A `slot_info()` ioctl and the status it returned.
    SlotInfo(SlotInfo),
}

/// A [`CaDevice`] decorator that records every frame in **both** directions
/// (plus ioctls) for live-CAM diagnostics. Wrap a real device, run, then dump
/// the [`log`](Self::log) — or decode it with
/// [`trace::decode_log`](crate::trace::decode_log) — to get an annotated byte
/// trace without hand-instrumenting the device:
///
/// ```no_run
/// # use dvb_ci_runtime::{Driver, device::RecordingCaDevice, trace};
/// # fn real_device() -> dvb_ci_runtime::MockCaDevice { dvb_ci_runtime::MockCaDevice::new([]) }
/// let mut driver = Driver::new(RecordingCaDevice::new(real_device()));
/// driver.init().unwrap();
/// // ... pump ...
/// println!("{}", trace::decode_log(driver.device().log()));
/// ```
#[derive(Debug, Default)]
pub struct RecordingCaDevice<D> {
    inner: D,
    /// The captured link events, in order.
    pub log: Vec<LinkEvent>,
    /// Last logged slot status, so repeated identical `slot_info()` polls
    /// (the driver now samples every [`pump`](crate::Driver::pump) for
    /// hot-plug edge detection — #726) don't swamp the trace; only a change
    /// is recorded, same rationale as `poll()` below.
    last_slot: Option<SlotInfo>,
}

impl<D: CaDevice> RecordingCaDevice<D> {
    /// Wrap `inner`, recording all I/O.
    pub fn new(inner: D) -> Self {
        Self {
            inner,
            log: Vec::new(),
            last_slot: None,
        }
    }

    /// The recorded link events, in order.
    #[must_use]
    pub fn log(&self) -> &[LinkEvent] {
        &self.log
    }

    /// Borrow the wrapped device.
    pub fn inner(&self) -> &D {
        &self.inner
    }
}

impl<D: CaDevice> CaDevice for RecordingCaDevice<D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.log.push(LinkEvent::Rx(buf[..n].to_vec()));
        }
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        self.log.push(LinkEvent::Tx(buf.to_vec()));
        self.inner.write(buf)
    }

    fn reset(&mut self) -> io::Result<()> {
        self.log.push(LinkEvent::Reset);
        self.inner.reset()
    }

    /// Forwarded: without this the wrapper falls back to the trait default
    /// (`Duration::ZERO`) and `Driver::init` writes the first TPDU straight
    /// after `CA_RESET` (EINVAL on a CXD2099). Guarded by
    /// `tests/wrapper_forwards_defaults.rs`.
    fn reset_settle(&self) -> std::time::Duration {
        self.inner.reset_settle()
    }

    fn slot_info(&mut self) -> io::Result<SlotInfo> {
        let si = self.inner.slot_info()?;
        if self.last_slot != Some(si) {
            self.log.push(LinkEvent::SlotInfo(si));
            self.last_slot = Some(si);
        }
        Ok(si)
    }

    fn poll(&mut self, timeout: std::time::Duration) -> io::Result<bool> {
        // Polls are not recorded (they would swamp the trace).
        self.inner.poll(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read buffer must fit the largest frame the link can encode: the
    /// 2-byte link header, the 1-byte `tpdu_tag`, the widest `length_field`
    /// (3 bytes) and that field's own maximum (65535) of `t_c_id` + data.
    #[test]
    fn max_ca_frame_fits_the_widest_link_frame() {
        assert_eq!(MAX_CA_FRAME, 65_541);
        assert!(MAX_CA_FRAME >= 2 + 1 + 3 + crate::transport::MAX_SPDU_LEN + 1);
    }

    /// A wrapped device failure must keep the errno reachable (it classifies
    /// `EINVAL`/`ENODEV` per operation), name the device and the operation in
    /// its Display, and keep the original error as its source.
    #[test]
    fn device_op_error_reports_errno_text_and_source() {
        let wrapped = DeviceOpError::wrap(
            "read (4096 bytes)",
            "/dev/dvb/adapter3/ca0",
            io::Error::from_raw_os_error(22),
        );
        let inner = wrapped
            .get_ref()
            .and_then(|e| e.downcast_ref::<DeviceOpError>())
            .unwrap();
        assert_eq!(inner.errno(), Some(22));
        assert_eq!(DeviceOpError::errno_of(&wrapped), Some(22));
        assert_eq!(
            DeviceOpError::errno_of(&io::Error::from_raw_os_error(5)),
            Some(5),
            "a plain OS error answers for itself"
        );
        assert_eq!(DeviceOpError::errno_of(&io::Error::other("x")), None);

        let text = wrapped.to_string();
        assert!(text.contains("/dev/dvb/adapter3/ca0"), "{text}");
        assert!(text.contains("read (4096 bytes)"), "{text}");
        assert!(text.contains("Invalid argument"), "{text}");
        assert_eq!(wrapped.kind(), io::ErrorKind::InvalidInput);
        assert!(std::error::Error::source(inner).is_some());
    }

    /// A `read` that finds nothing must not be recorded as an (empty) Rx event,
    /// or the trace fills with noise the CAM never sent.
    #[test]
    fn recording_device_ignores_an_empty_read() {
        let mut dev = RecordingCaDevice::new(MockCaDevice::new([]));
        let mut buf = [0u8; 16];
        assert_eq!(dev.read(&mut buf).unwrap(), 0);
        assert!(
            dev.log().is_empty(),
            "a zero-length read is not an Rx event: {:?}",
            dev.log()
        );
    }

    /// `slot_info` is sampled on every pump for hot-plug edge detection (#726),
    /// so the trace keeps one entry per *change*: the first sample is logged,
    /// a repeat of it is not, and a changed status is.
    #[test]
    fn recording_device_logs_slot_info_only_when_it_changes() {
        let ready = SlotInfo {
            num: 2,
            module_ready: true,
            module_present: true,
        };
        let mut mock = MockCaDevice::new([]);
        mock.slot = ready;
        let mut dev = RecordingCaDevice::new(mock);

        assert_eq!(dev.slot_info().unwrap(), ready);
        assert_eq!(
            dev.slot_info().unwrap(),
            ready,
            "the wrapper returns the device's own status"
        );
        assert_eq!(
            dev.log(),
            &[LinkEvent::SlotInfo(SlotInfo {
                num: 2,
                module_ready: true,
                module_present: true,
            })],
            "two identical samples must leave exactly one entry"
        );

        dev.inner.slot = SlotInfo {
            num: 2,
            module_ready: false,
            module_present: false,
        };
        dev.slot_info().unwrap();
        assert_eq!(dev.log().len(), 2);
        assert_eq!(
            dev.log()[1],
            LinkEvent::SlotInfo(SlotInfo {
                num: 2,
                module_ready: false,
                module_present: false,
            })
        );
    }

    /// `poll` must report the wrapped device's readiness, not a constant.
    #[test]
    fn recording_device_poll_follows_the_inner_device() {
        let mut dev = RecordingCaDevice::new(MockCaDevice::new([]));
        assert!(!dev.poll(std::time::Duration::ZERO).unwrap());
        dev.inner.inbound.push_back(vec![0x80, 0x02, 0x01, 0x00]);
        assert!(dev.poll(std::time::Duration::ZERO).unwrap());
    }

    /// The mock reports readability only while frames are queued, so a test
    /// that reads with no data sees `Ok(0)`.
    #[test]
    fn mock_poll_reports_only_queued_frames() {
        let mut dev = MockCaDevice::new([]);
        assert!(!dev.poll(std::time::Duration::ZERO).unwrap());
        dev.inbound.push_back(vec![0x81, 0x01, 0x01]);
        assert!(dev.poll(std::time::Duration::ZERO).unwrap());
    }

    #[test]
    fn recording_device_captures_both_directions() {
        let inner = MockCaDevice::new([vec![0x83, 0x01, 0x01]]);
        let mut dev = RecordingCaDevice::new(inner);
        dev.reset().unwrap();
        dev.write(&[0x82, 0x01, 0x01]).unwrap();
        let mut buf = [0u8; 16];
        dev.read(&mut buf).unwrap();
        assert_eq!(
            dev.log(),
            &[
                LinkEvent::Reset,
                LinkEvent::Tx(vec![0x82, 0x01, 0x01]),
                LinkEvent::Rx(vec![0x83, 0x01, 0x01]),
            ]
        );
    }

    #[test]
    fn mock_records_writes_and_replays_inbound() {
        let mut dev = MockCaDevice::new([vec![0x01, 0x02], vec![0x03]]);
        // host writes
        dev.write(&[0xAA, 0xBB]).unwrap();
        dev.reset().unwrap();
        // module frames replay in order
        let mut buf = [0u8; 16];
        assert_eq!(dev.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[0x01, 0x02]);
        assert_eq!(dev.read(&mut buf).unwrap(), 1);
        assert_eq!(dev.read(&mut buf).unwrap(), 0); // drained
        assert_eq!(dev.written(), vec![0xAA, 0xBB]);
        assert_eq!(dev.ops[1], DeviceOp::Reset);
    }
}
