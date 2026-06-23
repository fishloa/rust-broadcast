//! Drive the pure sans-IO core directly: `CiStack::handle(Event) -> Vec<Action>`.
//!
//! No device, threads, or clock — you feed events and execute the returned
//! actions yourself. This is what the `Driver` wraps; driving it by hand shows
//! the protocol with nothing in the way (and is how the state machines are
//! tested without hardware).
//!
//! Run: `cargo run -p dvb-ci-runtime --example sans_io_core`

use std::time::Duration;

use dvb_ci_runtime::dvb_ci::tpdu::tags;
use dvb_ci_runtime::{Action, CiStack, Event, HostRequest};

fn show(label: &str, actions: &[Action]) {
    println!("{label} -> {} action(s)", actions.len());
    for a in actions {
        match a {
            Action::Write(w) => println!("  Write {:02X?}", &w[..w.len().min(8)]),
            Action::SetTimer { after } => println!("  SetTimer after={after:?}"),
            other => println!("  {other:?}"),
        }
    }
}

fn main() {
    let mut stack = CiStack::new();

    // Host brings the interface up: reset + query slot + open transport.
    show("Init", &stack.handle(Event::Host(HostRequest::Init)));

    // Module accepts the transport connection.
    let reply = [tags::C_T_C_REPLY, 0x01, 0x01];
    show(
        "Readable(C_T_C_Reply)",
        &stack.handle(Event::Readable(&reply)),
    );

    // A timer tick with no inbound data drives the poll cadence
    // (an empty T_Data_Last down to the module).
    show(
        "Tick(100ms)",
        &stack.handle(Event::Tick {
            elapsed: Duration::from_millis(100),
        }),
    );
}
