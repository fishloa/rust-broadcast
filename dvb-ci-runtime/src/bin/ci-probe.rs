//! `ci-probe` — discover and engage an installed CAM over a Linux DVB CI device.
//!
//! Built only with the `linux` feature on Linux. Wires the `dvb-ci-runtime`
//! `Driver` + `LinuxCaDevice` + `trace` decoder to a `clap` CLI (the workspace
//! CLI standard — see `docs/CLI-STANDARD.md`).
//!
//! ```text
//! ci-probe list
//! ci-probe info       --adapter 3 --ca 0
//! ci-probe descramble --adapter 3 --ca 0 --pmt service.bin
//! ci-probe mmi        --adapter 3 --ca 0
//! ```
//! `--trace` (any subcommand) dumps an annotated link trace on exit.

#[cfg(all(feature = "linux", target_os = "linux"))]
fn main() -> std::process::ExitCode {
    imp::run()
}

#[cfg(not(all(feature = "linux", target_os = "linux")))]
fn main() -> std::process::ExitCode {
    eprintln!("ci-probe requires the `linux` feature on a Linux host (DVB CA device access).");
    std::process::ExitCode::FAILURE
}

#[cfg(all(feature = "linux", target_os = "linux"))]
mod imp {
    use std::io::{self, Write};
    use std::path::Path;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use clap::{Args, Parser, Subcommand};
    use dvb_ci_runtime::device::RecordingCaDevice;
    use dvb_ci_runtime::event::{HotPlug, MmiEvent, MmiMenu};
    use dvb_ci_runtime::linux::LinuxCaDevice;
    use dvb_ci_runtime::{CaDevice, Driver, Notification, trace};

    const PUMP: Duration = Duration::from_millis(100);
    const READY_TIMEOUT: Duration = Duration::from_secs(10);

    type Dev = RecordingCaDevice<LinuxCaDevice>;

    /// Discover and engage a CAM over a Linux DVB CI device.
    #[derive(Parser)]
    #[command(name = "ci-probe", version, about, long_about = None)]
    struct Cli {
        #[command(subcommand)]
        command: Command,
        /// Dump an annotated link trace (TPDU → SPDU → APDU) to stderr on exit.
        #[arg(long, global = true)]
        trace: bool,
    }

    #[derive(Subcommand)]
    enum Command {
        /// List the CA devices present and each slot's status.
        List,
        /// Run the EN 50221 handshake; print application-info + the CAM's CAIDs.
        Info(DevArgs),
        /// Send a PMT to the CAM: query → reply → ok_descrambling.
        Descramble {
            #[command(flatten)]
            dev: DevArgs,
            /// Path to a raw PMT-section file (the service to descramble).
            #[arg(long)]
            pmt: String,
        },
        /// Interactive MMI: show the module's menus / enquiries and answer them.
        Mmi(DevArgs),
    }

    /// Which DVB CA device to talk to.
    #[derive(Args)]
    struct DevArgs {
        /// DVB adapter number (`/dev/dvb/adapterN`).
        #[arg(short, long, default_value_t = 0)]
        adapter: u32,
        /// CA slot device number (`caM`).
        #[arg(short, long, default_value_t = 0)]
        ca: u32,
    }

    pub fn run() -> ExitCode {
        let cli = Cli::parse();
        let trace = cli.trace;
        let result = match cli.command {
            Command::List => list(),
            Command::Info(d) => info(d.adapter, d.ca, trace),
            Command::Descramble { dev, pmt } => descramble(dev.adapter, dev.ca, &pmt, trace),
            Command::Mmi(d) => mmi(d.adapter, d.ca, trace),
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        }
    }

    /// Enumerate `/dev/dvb/adapterN/caM` and report each slot's status.
    fn list() -> io::Result<()> {
        let mut found = false;
        for adapter in 0..16 {
            let base = format!("/dev/dvb/adapter{adapter}");
            if !Path::new(&base).exists() {
                continue;
            }
            for ca in 0..4 {
                let path = format!("{base}/ca{ca}");
                if !Path::new(&path).exists() {
                    continue;
                }
                found = true;
                match LinuxCaDevice::open(adapter, ca) {
                    Ok(mut dev) => match dev.slot_info() {
                        Ok(si) => println!(
                            "{path}  slot {}  module_present={}  module_ready={}",
                            si.num, si.module_present, si.module_ready
                        ),
                        Err(e) => println!("{path}  (slot_info failed: {e})"),
                    },
                    Err(e) => println!("{path}  (open failed: {e})"),
                }
            }
        }
        if !found {
            println!("no /dev/dvb/adapterN/caM devices found");
        }
        Ok(())
    }

    /// Open a recording driver for `adapter`/`ca`.
    fn open(adapter: u32, ca: u32) -> io::Result<Driver<Dev>> {
        let dev = RecordingCaDevice::new(LinuxCaDevice::open(adapter, ca)?);
        Ok(Driver::new(dev))
    }

    fn dump_trace(driver: &Driver<Dev>, enabled: bool) {
        if enabled {
            eprintln!(
                "\n--- link trace ---\n{}",
                trace::decode_log(driver.device().log())
            );
        }
    }

    /// Run the handshake and print application-info + the CAM's CAIDs.
    ///
    /// Demonstrates the closure-callback delivery style (`Driver::pump_with`):
    /// this crate is sync/sans-IO, so there is no channel to receive on —
    /// `pump_with` invokes the handler in-line for each notification the pump
    /// cycle produced, instead of the caller poll-draining
    /// `Driver::take_notifications` itself.
    fn info(adapter: u32, ca: u32, trace: bool) -> io::Result<()> {
        let mut driver = open(adapter, ca)?;
        driver.init()?;
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut got_ca_info = false;
        while Instant::now() < deadline && !got_ca_info {
            let mut got = false;
            driver.pump_with(PUMP, |note| {
                got |= matches!(note, Notification::CaInfo { .. });
                print_note(note);
            })?;
            got_ca_info = got;
        }
        if !got_ca_info {
            eprintln!("timed out before ca_info (CAM may not have completed the handshake)");
        }
        dump_trace(&driver, trace);
        Ok(())
    }

    /// Feed a PMT-section file and run the query → reply → ok descramble sequence.
    fn descramble(adapter: u32, ca: u32, pmt_file: &str, trace: bool) -> io::Result<()> {
        let pmt = std::fs::read(pmt_file)?;
        let mut driver = open(adapter, ca)?;
        driver.init()?;
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut sent = false;
        let mut done = false;
        while Instant::now() < deadline && !done {
            driver.pump(PUMP)?;
            for note in driver.take_notifications() {
                if matches!(note, Notification::CaInfo { .. }) && !sent {
                    println!("ca_info received → sending descramble request");
                    driver.descramble(&pmt)?;
                    sent = true;
                }
                if let Notification::CaPmtReply {
                    program_number,
                    descrambling_ok,
                } = note
                {
                    println!(
                        "ca_pmt_reply: program {program_number} descrambling_ok={descrambling_ok}"
                    );
                    done = true;
                } else {
                    print_note(&note);
                }
            }
        }
        if !done {
            eprintln!("timed out before ca_pmt_reply");
        }
        dump_trace(&driver, trace);
        Ok(())
    }

    /// Interactive MMI: display module menus/enquiries and send the user's answer.
    fn mmi(adapter: u32, ca: u32, trace: bool) -> io::Result<()> {
        let mut driver = open(adapter, ca)?;
        driver.init()?;
        println!("MMI session — Ctrl-C to quit. Waiting for the module to present a menu…");
        let mut closed = false;
        while !closed {
            driver.pump(PUMP)?;
            for note in driver.take_notifications() {
                match note {
                    Notification::Mmi(MmiEvent::Menu(m)) => {
                        print_menu_header(&m);
                        for (i, choice) in m.choices.iter().enumerate() {
                            println!("  {}) {choice}", i + 1);
                        }
                        println!("  0) back");
                        let choice = prompt("select> ")?;
                        driver.mmi_menu_answer(choice.trim().parse().unwrap_or(0))?;
                    }
                    Notification::Mmi(MmiEvent::List(m)) => {
                        // A list is informational — show it, then dismiss.
                        print_menu_header(&m);
                        for item in &m.choices {
                            println!("  - {item}");
                        }
                        prompt("(press Enter)")?;
                        driver.mmi_menu_answer(0)?;
                    }
                    Notification::Mmi(MmiEvent::Enquiry {
                        prompt: p, blind, ..
                    }) => {
                        println!("\n{p}{}", if blind { " (hidden)" } else { "" });
                        let answer = prompt("answer> ")?;
                        driver.mmi_enquiry_answer(answer.trim().as_bytes())?;
                    }
                    Notification::Mmi(MmiEvent::Close) => {
                        println!("(module closed the MMI dialogue)");
                        closed = true;
                    }
                    other => print_note(&other),
                }
            }
        }
        dump_trace(&driver, trace);
        Ok(())
    }

    /// Print a menu/list's three header lines (skipping any that are blank).
    fn print_menu_header(m: &MmiMenu) {
        println!("\n== {} ==", m.title);
        for line in [&m.subtitle, &m.bottom] {
            if !line.trim().is_empty() {
                println!("{line}");
            }
        }
    }

    fn prompt(p: &str) -> io::Result<String> {
        print!("{p}");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(line)
    }

    fn print_note(note: &Notification) {
        match note {
            Notification::CamReady => println!("CAM ready (resource-manager handshake complete)"),
            Notification::ApplicationInfo {
                application_type,
                manufacturer,
                code,
                menu,
            } => println!(
                "application_info: type=0x{application_type:02X} manufacturer=0x{manufacturer:04X} \
                 code=0x{code:04X} menu={menu:?}"
            ),
            Notification::CaInfo { ca_system_ids } => {
                let ids: Vec<String> = ca_system_ids.iter().map(|c| format!("0x{c:04X}")).collect();
                println!("ca_info: {} CA_system_id(s): {}", ids.len(), ids.join(", "));
            }
            Notification::Mmi(ev) => println!("mmi: {ev:?}"),
            Notification::SessionOpened { resource } => {
                println!("session opened: {}", resource.name())
            }
            Notification::SessionClosed { session_nb } => {
                println!("session {session_nb} closed")
            }
            Notification::Error { detail } => eprintln!("stack error: {detail}"),
            Notification::HotPlug(hp) => match hp {
                HotPlug::CamPresent => println!("hot-plug: CAM inserted (slot present+ready)"),
                HotPlug::CamRemoved => println!("hot-plug: CAM removed"),
                HotPlug::CardInserted => println!("hot-plug: card inserted (inferred)"),
                HotPlug::CardRemoved => println!("hot-plug: card removed (inferred)"),
                HotPlug::CardChanged => println!("hot-plug: card changed (inferred)"),
                other => println!("hot-plug: {other}"),
            },
            other => println!("{other:?}"),
        }
    }
}
