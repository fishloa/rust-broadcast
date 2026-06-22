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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotInfo {
    /// Slot number.
    pub num: u8,
    /// `true` once a module is present and ready (CA_CI_MODULE_READY).
    pub module_ready: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

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
