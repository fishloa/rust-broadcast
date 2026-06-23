//! The driver — the one place I/O happens. It pumps a [`CaDevice`] against the
//! sans-IO [`CiStack`]: reads frames in, executes the stack's [`Action`]s
//! (writes/ioctls) out, tracks the requested poll timer, and collects
//! [`Notification`]s for the host application.

use std::io;
use std::time::Duration;

use crate::device::CaDevice;
use crate::event::{Action, Event, HostRequest, Notification};
use crate::stack::CiStack;

/// Drives a [`CaDevice`] with the [`CiStack`].
pub struct Driver<D: CaDevice> {
    device: D,
    stack: CiStack,
    notifications: Vec<Notification>,
    /// Delay the stack last asked to be polled after (`None` = none pending).
    next_timer: Option<Duration>,
    /// Read buffer for one link-layer frame.
    buf: Vec<u8>,
}

impl<D: CaDevice> Driver<D> {
    /// New driver over `device`, single transport connection.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self {
            device,
            stack: CiStack::new(),
            notifications: Vec::new(),
            next_timer: None,
            buf: vec![0u8; 4096],
        }
    }

    /// Borrow the underlying device (e.g. to inspect a mock's recorded ops).
    pub fn device(&self) -> &D {
        &self.device
    }

    /// The poll delay the stack most recently requested, if any.
    pub fn next_timer(&self) -> Option<Duration> {
        self.next_timer
    }

    /// Drain the notifications collected so far.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        core::mem::take(&mut self.notifications)
    }

    /// Bring the interface up (reset + open the transport connection).
    pub fn init(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::Init));
        self.run(actions)
    }

    /// Request the module descramble the services in `ca_pmt` (a serialized
    /// `ca_pmt` APDU body, e.g. from `dvb_ci::build_ca_pmt`).
    pub fn send_ca_pmt(&mut self, ca_pmt: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::SendCaPmt(ca_pmt)));
        self.run(actions)
    }

    /// Descramble the services in a PMT section: the stack filters the PMT's
    /// `CA_descriptor`s to the CAM's advertised CAIDs, sends a `ca_pmt` query,
    /// and auto-sends `ok_descrambling` once the `ca_pmt_reply` confirms it.
    /// Drive [`pump`](Self::pump) afterwards to exchange the reply; the outcome
    /// surfaces as [`Notification::CaPmtReply`]. Call after the CAM is ready and
    /// its `ca_info` has been received (otherwise no CAID filter is applied).
    pub fn descramble(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::Descramble(pmt_section)));
        self.run(actions)
    }

    /// One pump step: if the device is readable within `timeout`, read a frame
    /// and feed it; otherwise advance the stack's timers by `timeout` (driving
    /// the poll cadence). Returns whether a frame was processed.
    pub fn pump(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.device.poll(timeout)? {
            let n = self.device.read(&mut self.buf)?;
            if n > 0 {
                let frame = self.buf[..n].to_vec();
                let actions = self.stack.handle(Event::Readable(&frame));
                self.run(actions)?;
                return Ok(true);
            }
        }
        let actions = self.stack.handle(Event::Tick { elapsed: timeout });
        self.run(actions)?;
        Ok(false)
    }

    /// Execute the stack's actions against the device.
    fn run(&mut self, actions: Vec<Action>) -> io::Result<()> {
        for action in actions {
            match action {
                Action::Write(bytes) => self.device.write(&bytes)?,
                Action::Reset => self.device.reset()?,
                Action::QuerySlot => {
                    self.device.slot_info()?;
                }
                Action::SetTimer { after } => self.next_timer = Some(after),
                Action::Notify(n) => self.notifications.push(n),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceOp, MockCaDevice};
    use dvb_ci::tpdu::tags;

    #[test]
    fn init_drives_reset_slotinfo_and_create_tc_to_device() {
        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        let ops = &d.device().ops;
        assert_eq!(ops[0], DeviceOp::Reset);
        assert_eq!(ops[1], DeviceOp::SlotInfo);
        assert!(matches!(&ops[2], DeviceOp::Write(w) if w[0] == tags::CREATE_T_C));
    }

    #[test]
    fn reads_reply_then_polls_on_pump() {
        // Script the module accepting the connection.
        let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
        let mut d = Driver::new(dev);
        d.init().unwrap();
        // first pump reads the C_T_C_Reply (activates the connection)
        assert!(d.pump(Duration::from_millis(100)).unwrap());
        // next pump has nothing to read → ticks → emits a poll write
        assert!(!d.pump(Duration::from_millis(100)).unwrap());
        let last = d.device().ops.last().unwrap();
        assert!(matches!(last, DeviceOp::Write(w) if w.first() == Some(&tags::DATA_LAST)));
    }
}
