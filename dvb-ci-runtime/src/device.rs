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

/// CA-device slot status (subset of the Linux `ca_slot_info` the runtime needs).
///
/// The DVB-CA slot reports two independent bits (uapi `linux/dvb/ca.h`
/// `ca_slot_info.flags`): `CA_CI_MODULE_PRESENT` (a module is physically
/// inserted) and `CA_CI_MODULE_READY` (that module has completed its own
/// init and is usable). A module can be present-but-not-ready briefly after
/// insertion; the runtime's hot-plug edge detection
/// ([`Notification::CamPresent`](crate::event::Notification::CamPresent)/
/// [`CamRemoved`](crate::event::Notification::CamRemoved)) keys off
/// `module_present`, since that is the field that toggles on physical
/// insert/removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotInfo {
    /// Slot number.
    pub num: u8,
    /// `true` once a module is present and ready (`CA_CI_MODULE_READY`).
    pub module_ready: bool,
    /// `true` while a module is physically inserted (`CA_CI_MODULE_PRESENT`),
    /// regardless of whether it has finished initialising.
    pub module_present: bool,
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

    /// Query slot status (ioctl `CA_GET_SLOT_INFO`).
    fn slot_info(&mut self) -> io::Result<SlotInfo>;

    /// Wait up to `timeout` for the device to become readable; `Ok(true)` if
    /// readable. The runtime's poll loop calls this between reads.
    fn poll(&mut self, timeout: std::time::Duration) -> io::Result<bool>;
}

/// One recorded device operation, for assertions + differential testing.
#[derive(Debug, Clone, PartialEq, Eq)]
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
