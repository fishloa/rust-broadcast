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

    /// Mutably borrow the underlying device (e.g. to script a mock's inbound
    /// frames between pumps).
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
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
    /// `CA_descriptor`s to the CAM's advertised CAIDs and sends a `ca_pmt`
    /// (`list_management = only`, `cmd_id = ok_descrambling`). The outcome
    /// surfaces as [`Notification::CaPmtReply`]. Call after the CAM is ready and
    /// its `ca_info` has been received (otherwise no CAID filter is applied).
    pub fn descramble(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::Descramble(pmt_section)));
        self.run(actions)
    }

    /// Descramble a set of programmes in one CA-PMT list (`first`/`more`/`last`),
    /// replacing any previously selected set. Each element is a raw PMT section.
    pub fn descramble_programs(&mut self, pmt_sections: &[&[u8]]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::DescramblePrograms(pmt_sections)));
        self.run(actions)
    }

    /// Add one programme to the descrambled set (`list_management = add`) without
    /// re-listing the others — for a capacity manager adding a viewer's service.
    pub fn add_program(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::AddProgram(pmt_section)));
        self.run(actions)
    }

    /// Remove one programme from the descrambled set (`list_management = update`,
    /// `cmd_id = not_selected`) — tells the CAM to stop descrambling it.
    pub fn remove_program(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::RemoveProgram(pmt_section)));
        self.run(actions)
    }

    /// Answer an MMI menu/list by 1-based `choice_ref` (0 = back/cancel).
    pub fn mmi_menu_answer(&mut self, choice_ref: u8) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::MmiMenuAnswer(choice_ref)));
        self.run(actions)
    }

    /// Answer an MMI enquiry with the user's input (EN 300 468 Annex A bytes).
    pub fn mmi_enquiry_answer(&mut self, text: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::MmiEnquiryAnswer(text)));
        self.run(actions)
    }

    /// Abort the current MMI dialogue (`answ` with `answ_id = cancel`).
    pub fn mmi_cancel(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::MmiCancel));
        self.run(actions)
    }

    /// Ask the module to open its MMI menu (`enter_menu`) — e.g. to read card /
    /// entitlement info from the module's own menus.
    pub fn enter_menu(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::EnterMenu));
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
    use crate::event::{HostControlEvent, Notification};
    use broadcast_common::Serialize;
    use dvb_ci::tpdu::tags;

    fn ser<S: Serialize>(s: &S) -> Vec<u8> {
        let mut b = vec![0u8; s.serialized_len()];
        match s.serialize_into(&mut b) {
            Ok(n) => b.truncate(n),
            Err(_) => b.clear(),
        }
        b
    }

    /// Wrap an SPDU as a module→host `T_Data_Last` R_TPDU (+ trailing T_SB,
    /// data_available clear) on transport connection `tcid`.
    fn r_data(tcid: u8, spdu: &[u8]) -> Vec<u8> {
        use dvb_ci::tpdu::{SbValue, tags as tpdu_tags};
        let mut v = vec![tpdu_tags::DATA_LAST, (1 + spdu.len()) as u8, tcid];
        v.extend_from_slice(spdu);
        v.extend_from_slice(&[tpdu_tags::SB, 0x02, tcid, SbValue::new(false).0]);
        v
    }

    /// Wrap an APDU for delivery on `session_nb` (session_number prefix), then as
    /// a module→host R_TPDU on tcid 1.
    fn r_apdu(session_nb: u16, apdu: &[u8]) -> Vec<u8> {
        use dvb_ci::spdu::SessionNumber;
        let mut spdu = ser(&SessionNumber { session_nb });
        spdu.extend_from_slice(apdu);
        r_data(1, &spdu)
    }

    /// A standalone module→host `T_SB` (data_available clear) ack — flushes one
    /// queued host write per turn (#337).
    fn sb() -> Vec<u8> {
        use dvb_ci::tpdu::{SbValue, tags as tpdu_tags};
        vec![tpdu_tags::SB, 0x02, 0x01, SbValue::new(false).0]
    }

    /// Feed one scripted module frame into the mock and pump it, then pump a
    /// handful of SB acks so any queued host writes flush.
    fn feed(d: &mut Driver<MockCaDevice>, frame: Vec<u8>) {
        d.device_mut().inbound.push_back(frame);
        d.pump(Duration::from_millis(10)).unwrap();
        for _ in 0..8 {
            d.device_mut().inbound.push_back(sb());
            d.pump(Duration::from_millis(10)).unwrap();
        }
    }

    /// Drive the EN 50221 handshake through the `Driver` until host_control and
    /// the other module-provided sessions are open (mirrors the stack-level
    /// `stack_with_ca_session`, but exercises the real driver I/O path).
    fn driver_with_sessions() -> Driver<MockCaDevice> {
        use dvb_ci::objects::resource_manager::Profile;
        use dvb_ci::resource::{
            APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, HOST_CONTROL, MMI,
            RESOURCE_MANAGER,
        };
        use dvb_ci::spdu::{CreateSessionResponse, OpenSessionRequest, SessionStatus};

        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        // module accepts the transport connection
        feed(&mut d, vec![tags::C_T_C_REPLY, 0x01, 0x01]);
        // module opens the host's resource_manager → RM session 1
        feed(
            &mut d,
            r_data(
                1,
                &ser(&OpenSessionRequest {
                    resource: RESOURCE_MANAGER,
                }),
            ),
        );
        // module's profile → host: CamReady + profile_change + create_session for
        // each module-provided resource.
        feed(
            &mut d,
            r_apdu(
                1,
                &ser(&Profile {
                    resources: vec![
                        APPLICATION_INFORMATION,
                        CONDITIONAL_ACCESS_SUPPORT,
                        MMI,
                        HOST_CONTROL,
                    ],
                }),
            ),
        );
        // module accepts each create_session (session nbs 2..=5 in registration order)
        for (nb, res) in [
            (2u16, APPLICATION_INFORMATION),
            (3, CONDITIONAL_ACCESS_SUPPORT),
            (4, MMI),
            (5, HOST_CONTROL),
        ] {
            feed(
                &mut d,
                r_data(
                    1,
                    &ser(&CreateSessionResponse {
                        status: SessionStatus::Ok,
                        resource: res,
                        session_nb: nb,
                    }),
                ),
            );
        }
        d
    }

    // Session numbers the module allocates in `driver_with_sessions`, in
    // registration order: RM=1, app_info=2, conditional_access=3, mmi=4,
    // host_control=5. (Asserted by `handshake_opens_expected_sessions`.)
    const RM_SESSION: u16 = 1;
    const MMI_SESSION: u16 = 4;
    const HOST_CONTROL_SESSION: u16 = 5;

    #[test]
    fn host_control_tune_apdu_surfaces_notification_via_driver() {
        use dvb_ci::objects::host_control::Tune;

        let mut d = driver_with_sessions();
        let hc_nb = HOST_CONTROL_SESSION;
        d.take_notifications(); // drop handshake notifications

        // Module (CAM) sends a Tune request on its host_control session.
        let tune = Tune {
            network_id: 0x1122,
            original_network_id: 0x3344,
            transport_stream_id: 0x5566,
            service_id: 0x7788,
        };
        feed(&mut d, r_apdu(hc_nb, &ser(&tune)));

        // The runtime surfaces the decoded HostControl(Tune) notification.
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HostControl(HostControlEvent::Tune {
                network_id: 0x1122,
                original_network_id: 0x3344,
                transport_stream_id: 0x5566,
                service_id: 0x7788,
            })),
            "expected HostControl(Tune) notification, got {notes:?}"
        );
    }

    #[test]
    fn profile_reply_advertises_host_control() {
        use broadcast_common::Parse;
        use dvb_ci::objects::resource_manager::{Profile, ProfileEnq};
        use dvb_ci::resource::{HOST_CONTROL, RESOURCE_MANAGER};

        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        feed(&mut d, vec![tags::C_T_C_REPLY, 0x01, 0x01]);
        // Open RM, then the module enquires the host profile.
        feed(
            &mut d,
            r_data(
                1,
                &ser(&dvb_ci::spdu::OpenSessionRequest {
                    resource: RESOURCE_MANAGER,
                }),
            ),
        );
        // Module → profile_enq on the RM session → host replies with its profile.
        feed(&mut d, r_apdu(RM_SESSION, &ser(&ProfileEnq)));

        // Find the host's `profile` reply (tag 9F 80 11) in the written frames and
        // confirm it lists HOST_CONTROL.
        let want = dvb_ci::tag::PROFILE.to_bytes();
        let found = d.device().ops.iter().any(|op| {
            if let DeviceOp::Write(w) = op {
                if let Some(pos) = w.windows(3).position(|x| x == want) {
                    if let Ok(p) = Profile::parse(&w[pos..]) {
                        return p.resources.contains(&HOST_CONTROL);
                    }
                }
            }
            false
        });
        assert!(found, "profile reply must advertise HOST_CONTROL");
    }

    #[test]
    fn mmi_menu_answ_and_answ_are_byte_exact_on_the_mmi_session() {
        use dvb_ci::objects::mmi_high::{Answ, AnswId, MenuAnsw};

        let mut d = driver_with_sessions();
        let mmi_nb = MMI_SESSION;

        // menu_answ(choice_ref = 2): the driver method must put the exact dvb-ci
        // MenuAnsw serialization on the wire, on the MMI session.
        d.mmi_menu_answer(2).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        assert_apdu_on_session(&d, mmi_nb, &ser(&MenuAnsw { choice_ref: 2 }));

        // answ(answer, "1234"): byte-exact Answ serialization on the MMI session.
        d.mmi_enquiry_answer(b"1234").unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        assert_apdu_on_session(
            &d,
            mmi_nb,
            &ser(&Answ {
                answ_id: AnswId::Answer,
                text_chars: b"1234",
            }),
        );
    }

    /// Assert some host write carries `session_number(session_nb)` immediately
    /// followed by the exact `apdu` bytes (byte-exact APDU on the right session).
    fn assert_apdu_on_session(d: &Driver<MockCaDevice>, session_nb: u16, apdu: &[u8]) {
        use dvb_ci::spdu::SessionNumber;
        let mut want = ser(&SessionNumber { session_nb });
        want.extend_from_slice(apdu);
        let hit = d.device().ops.iter().any(|op| match op {
            DeviceOp::Write(w) => w.windows(want.len()).any(|x| x == want.as_slice()),
            _ => false,
        });
        assert!(
            hit,
            "expected APDU {apdu:02X?} on session {session_nb} (session-prefixed {want:02X?}) in writes"
        );
    }

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
