//! Splice commands — ANSI/SCTE 35 2023r1 §9.7, Tables 8-13.
//!
//! One module per `splice_command_type` (§9.6.1, Table 7), each mirroring the
//! dvb-si descriptor shape: spec-cited doc → `COMMAND_TYPE` const → struct →
//! symmetric [`Parse`](dvb_common::Parse)/[`Serialize`](dvb_common::Serialize)
//! → in-module round-trip tests. [`AnyCommand`] unifies them with a raw
//! fall-through for reserved types.

pub mod any;
pub mod bandwidth_reservation;
pub mod private_command;
pub mod splice_insert;
pub mod splice_null;
pub mod splice_schedule;
pub mod time_signal;

pub use any::AnyCommand;
pub use bandwidth_reservation::BandwidthReservation;
pub use private_command::PrivateCommand;
pub use splice_insert::{SpliceInsert, SpliceInsertComponent};
pub use splice_null::SpliceNull;
pub use splice_schedule::{SpliceSchedule, SpliceScheduleComponent, SpliceScheduleEvent};
pub use time_signal::TimeSignal;
