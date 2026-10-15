//! Linux `/dev/dvb/adapterN/caM` [`CaDevice`] implementation (the `linux`
//! feature).
//!
//! This is the one place the crate uses `unsafe` — the DVB CA ioctls
//! (`CA_RESET`, `CA_GET_SLOT_INFO`) via `rustix::ioctl`, confined to the
//! private `ioctl` submodule below, which exposes safe functions. The ioctl
//! request numbers are built by `rustix::ioctl::opcode` (the per-architecture
//! `_IOC` encoding — it differs between x86/arm and mips/powerpc/sparc) from
//! `include/uapi/linux/dvb/ca.h`'s group/number/type, not hard-coded magic.
//!
//! Runtime behaviour requires a real DVB card with a CI slot; it is
//! compile-checked in CI but exercised only on hardware.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsFd;
use std::path::Path;
use std::time::Duration;

use rustix::event::{Nsecs, PollFd, PollFlags, Timespec};

use crate::dataplane::{CiDataDevice, TS_PACKET_LEN};
use crate::device::{CaDevice, DeviceOpError, SlotInfo};

/// Poll a file descriptor for readability up to `timeout`.
///
/// `EINTR` surfaces as an `io::Error` of kind `Interrupted`, exactly as it did
/// with `libc::poll` on `main` (which also returned `last_os_error()` on `-1`
/// and did not retry). Nothing retries it further up either: the driver pump
/// does `self.device.poll(timeout)?`. A stray signal therefore still aborts
/// one pump call, unchanged by this migration. Only `POLLIN` counts as
/// readable; `POLLHUP`/`POLLERR` alone report `false`, as before. Unlike the
/// old millisecond truncation the timeout keeps its sub-millisecond part.
// `Nsecs` is `i64` on rustix's linux_raw backend (where `From<u32>` would do) but
// `c_long` (`i32`) under its libc backend on 32-bit targets, where only
// `TryFrom` exists; keep the portable form.
#[allow(clippy::unnecessary_fallible_conversions)]
fn poll_readable(fd: &impl AsFd, timeout: Duration) -> io::Result<bool> {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let ts = Timespec {
        tv_sec: i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: Nsecs::try_from(timeout.subsec_nanos()).unwrap_or(0),
    };
    rustix::event::poll(&mut fds, Some(&ts))?;
    Ok(fds[0].revents().contains(PollFlags::IN))
}

// --- DVB CA ioctl request numbers (uapi/linux/dvb/ca.h) --------------------
//
// The `_IOC` request encoding is NOT one layout on Linux: x86/arm/arm64/riscv/
// s390x/... use the asm-generic one (2 direction bits, 14 size bits), but
// mips/powerpc/sparc use 3 direction bits, 13 size bits and different
// direction values (`_IOC_NONE` = 1, `_IOC_READ` = 2, `_IOC_WRITE` = 4). A
// hand-rolled asm-generic encoding therefore issues the WRONG request on those
// targets (r10-W-1). `rustix::ioctl::opcode` carries the per-architecture
// table, so the request numbers are derived from it rather than re-encoded
// here.
use rustix::ioctl::{Opcode, opcode};

/// The DVB CA ioctl "magic" group, `'o'` (uapi `linux/dvb/ca.h`).
const DVB_CA_MAGIC: u8 = b'o';
/// `_IO('o', 128)` (uapi `linux/dvb/ca.h`).
const CA_RESET: Opcode = opcode::none(DVB_CA_MAGIC, 128);
/// `_IOR('o', 130, ca_slot_info_t)` (uapi `linux/dvb/ca.h`).
const CA_GET_SLOT_INFO: Opcode = opcode::read::<CaSlotInfo>(DVB_CA_MAGIC, 130);
/// `CA_CI_MODULE_PRESENT` — a module (or card) is physically inserted in the
/// slot (uapi `linux/dvb/ca.h` `ca_slot_info.flags`, bit 0).
const CA_CI_MODULE_PRESENT: u32 = 1;
/// `CA_CI_MODULE_READY` — the inserted module has completed its own init and
/// is usable (uapi `linux/dvb/ca.h` `ca_slot_info.flags`, bit 1). Distinct
/// from `CA_CI_MODULE_PRESENT`: a module can be present but not yet ready
/// briefly after insertion.
const CA_CI_MODULE_READY: u32 = 2;

/// Mirror of the kernel's `struct ca_slot_info` (uapi `linux/dvb/ca.h`):
/// `int num; int type; unsigned int flags;` — three 4-byte fields.
#[repr(C)]
struct CaSlotInfo {
    num: i32,
    typ: i32,
    flags: u32,
}

/// Size of the kernel's `struct ca_slot_info` (3 x 4 bytes).
#[cfg(test)]
const KERNEL_CA_SLOT_INFO_SIZE: usize = 12;
/// Alignment of the kernel's `struct ca_slot_info` (4-byte `int`s).
#[cfg(test)]
const KERNEL_CA_SLOT_INFO_ALIGN: usize = 4;

/// The only `unsafe` in the crate: the two DVB CA ioctls, behind safe fns.
#[allow(unsafe_code)]
mod ioctl {
    use super::{CA_GET_SLOT_INFO, CA_RESET, CaSlotInfo};
    use rustix::ioctl::{NoArg, Updater, ioctl};
    use std::io;
    use std::os::unix::io::BorrowedFd;

    /// `CA_RESET` on `fd`.
    pub(super) fn ca_reset(fd: BorrowedFd<'_>) -> io::Result<()> {
        // SAFETY: `CA_RESET` is `_IO('o', 128)` (built by `rustix::ioctl::
        // opcode::none`, the correct request for this target architecture)
        // and takes no argument, so there is no memory for the kernel to read
        // or write. `fd` is a live borrowed descriptor for the whole call.
        unsafe { ioctl(fd, NoArg::<CA_RESET>::new()) }.map_err(io::Error::from)
    }

    /// `CA_GET_SLOT_INFO` on `fd`, filling `info` (its `num` selects the slot).
    pub(super) fn ca_get_slot_info(fd: BorrowedFd<'_>, info: &mut CaSlotInfo) -> io::Result<()> {
        // SAFETY: the request is `_IOR('o', 130, CaSlotInfo)` (built by
        // `rustix::ioctl::opcode::read::<CaSlotInfo>`, per-architecture): the
        // kernel reads `num` and writes the whole `ca_slot_info`, whose
        // size/alignment `CaSlotInfo` mirrors (`#[repr(C)]`, asserted by the
        // `ca_slot_info_matches_kernel_layout` test). `Updater` passes a
        // pointer to the exclusive, fully-initialised `info`, which outlives
        // the call. `fd` is live for the whole call.
        unsafe { ioctl(fd, Updater::<CA_GET_SLOT_INFO, CaSlotInfo>::new(info)) }
            .map_err(io::Error::from)
    }
}

/// Settle time after `CA_RESET` before the module is usable. The DD/cxd2099
/// (and others) only (re)initialise the slot a couple of seconds after reset;
/// `Create_T_C` sent too early is ignored. 3s is the value validated live
/// against a DD Octopus cxd2099 + AlphaCrypt module (2s intermittently raced the
/// module's resource-manager open).
const RESET_SETTLE: Duration = Duration::from_millis(3000);

/// A [`CaDevice`] backed by a Linux DVB CA character device.
///
/// The kernel `dvb_ca_en50221` character device carries a 2-byte link header on
/// every read/write — `[slot_id, connection_id, <TPDU>]`. This type adds/strips
/// that header, so the sans-IO transport deals in bare TPDUs. (Writing a raw
/// TPDU without the header is rejected `EINVAL` by the driver.)
#[derive(Debug)]
pub struct LinuxCaDevice {
    file: File,
    slot: u8,
    /// Device path for error context (see [`DeviceOpError`]).
    label: String,
}

impl LinuxCaDevice {
    /// Open `/dev/dvb/adapter{adapter}/ca{ca}` (slot 0).
    pub fn open(adapter: u32, ca: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ca{ca}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| DeviceOpError::wrap("open (O_RDWR)", &path, e))?;
        Ok(Self {
            file,
            slot: 0,
            label: path,
        })
    }

    /// Wrap an already-open CA device file for `slot`.
    #[must_use]
    pub fn from_file(file: File, slot: u8) -> Self {
        Self {
            file,
            slot,
            label: format!("CA device (slot {slot}, from_file)"),
        }
    }

    /// The `connection_id` for a TPDU = its `t_c_id`, which follows the tag +
    /// `length_field`. Falls back to 1 (the single connection) if unparseable.
    fn connection_id(tpdu: &[u8]) -> u8 {
        dvb_ci::length::decode(tpdu.get(1..).unwrap_or(&[]))
            .ok()
            .and_then(|(_, hdr)| tpdu.get(1 + hdr).copied())
            .unwrap_or(1)
    }
}

/// Largest legal kernel frame `[slot, connection_id, <TPDU>]` this device can
/// deliver: the 2-byte link header, plus the largest TPDU EN 50221 Table 1
/// can encode: `tpdu_tag`(1), `length_field`(up to 3 bytes total, its
/// maximum-value long form: size_indicator plus 2 length bytes), and the
/// `length_field`'s own max value, 65535, of `t_c_id` plus data (§7
/// semantics: "length_field codes the length of all following fields", i.e.
/// t_c_id + data — see `dvb-ci/docs/en_50221/apdu-coding.md`). r10-W-2: a
/// 4096-byte scratch buffer silently truncated any larger TPDU and returned
/// `Ok` with the truncated bytes as if they were the whole thing.
const MAX_CA_FRAME: usize = 2 + 1 + 3 + 65_535;

impl CaDevice for LinuxCaDevice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Read one kernel frame `[slot, connection_id, <TPDU>]` into a scratch
        // buffer and hand the bare TPDU up. `poll` gates this, so it won't block;
        // `WouldBlock` is reported as "no data".
        let mut frame = [0u8; MAX_CA_FRAME];
        let n = match self.file.read(&mut frame) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(0),
            Err(e) => {
                return Err(DeviceOpError::wrap(
                    format!("read (<= {} byte frame)", frame.len()),
                    &self.label,
                    e,
                ));
            }
        };
        if n == frame.len() {
            // A `read()` reports only how many bytes it actually copied,
            // never "there was more" — a full buffer means we cannot tell
            // whether the real frame was exactly this size or the driver
            // handed us a larger one truncated to fit. Since `MAX_CA_FRAME`
            // already covers every legal TPDU, treat a full read as a
            // truncated (and therefore corrupt) frame rather than silently
            // returning it as complete.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CA device frame filled the maximum-size scratch buffer; \
                 it may have been truncated",
            ));
        }
        // Strip the 2-byte link header; anything shorter has no TPDU.
        let tpdu = frame.get(2..n).unwrap_or(&[]);
        let copy = tpdu.len().min(buf.len());
        buf[..copy].copy_from_slice(&tpdu[..copy]);
        Ok(copy)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        // Prepend the `[slot, connection_id]` link header the driver expects.
        let mut frame = Vec::with_capacity(buf.len() + 2);
        frame.push(self.slot);
        frame.push(Self::connection_id(buf));
        frame.extend_from_slice(buf);
        // The kernel's `dvb_ca_en50221_io_write` returns EINVAL while the slot
        // is not in its RUNNING state (e.g. the CAM is still initialising
        // after a CA_RESET), so name the frame in the error.
        self.file.write_all(&frame).map_err(|e| {
            DeviceOpError::wrap(
                format!(
                    "write ({} byte frame: slot {}, connection {})",
                    frame.len(),
                    frame[0],
                    frame[1]
                ),
                &self.label,
                e,
            )
        })
    }

    fn reset(&mut self) -> io::Result<()> {
        ioctl::ca_reset(self.file.as_fd())
            .map_err(|e| DeviceOpError::wrap("ioctl CA_RESET", &self.label, e))?;
        // The module needs a moment to re-initialise before Create_T_C; the
        // driver waits that out via `reset_settle` on its own clock rather
        // than this call blocking the pump (audit r10-O-10).
        Ok(())
    }

    fn reset_settle(&self) -> Duration {
        RESET_SETTLE
    }

    fn slot_info(&mut self) -> io::Result<SlotInfo> {
        let mut si = CaSlotInfo {
            num: i32::from(self.slot),
            typ: 0,
            flags: 0,
        };
        if let Err(err) = ioctl::ca_get_slot_info(self.file.as_fd(), &mut si) {
            // r10-W-3: only fall back for the specific errno the "doesn't
            // implement CA_GET_SLOT_INFO" drivers (DD/cxd2099) return —
            // EINVAL, or ENOTTY for a device node that doesn't support the
            // ioctl at all; presence then shows via the TPDU handshake
            // instead, so assume present+ready. Any OTHER error (EIO,
            // ENODEV, EBADF, …) is a real fault and must be propagated, not
            // masked as "the module is present and ready".
            return match err.raw_os_error() {
                Some(libc::EINVAL) | Some(libc::ENOTTY) => Ok(SlotInfo {
                    num: self.slot,
                    module_ready: true,
                    module_present: true,
                }),
                _ => Err(DeviceOpError::wrap(
                    format!("ioctl CA_GET_SLOT_INFO (slot {})", self.slot),
                    &self.label,
                    err,
                )),
            };
        }
        Ok(SlotInfo {
            num: si.num as u8,
            module_ready: si.flags & CA_CI_MODULE_READY != 0,
            module_present: si.flags & CA_CI_MODULE_PRESENT != 0,
        })
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        poll_readable(&self.file, timeout)
            .map_err(|e| DeviceOpError::wrap(format!("poll ({timeout:?})"), &self.label, e))
    }
}

/// The file descriptor(s) behind a [`LinuxCiDataDevice`].
#[derive(Debug)]
enum CiEndpoints {
    /// One `O_RDWR` fd used for both directions.
    Duplex(File),
    /// Two one-direction fds on the same node: an `O_WRONLY` fd for the
    /// scrambled TS going in and an `O_RDONLY | O_NONBLOCK` fd for the
    /// descrambled TS coming out.
    Split { write: File, read: File },
}

/// A [`CiDataDevice`] backed by a Linux DVB CI TS data-plane device
/// (`/dev/dvb/adapterN/ciM`). The host writes scrambled TS and reads the
/// descrambled TS back; I/O is in whole 188-byte packets.
///
/// # Duplex versus split (Digital Devices CXD2099)
/// Most drivers accept the node opened `O_RDWR`. The Digital Devices
/// CXD2099 `ci0` (dddvb, Ubuntu 5.15 kernel, verified on hardware) rejects
/// `O_RDWR` with `EINVAL` and only works as two one-direction descriptors.
/// [`open`](Self::open) therefore tries `O_RDWR` first and falls back to the
/// split open on `EINVAL` (and only `EINVAL`); [`open_split`](Self::open_split)
/// / [`from_split`](Self::from_split) pick it directly.
#[derive(Debug)]
pub struct LinuxCiDataDevice {
    io: CiEndpoints,
    /// Device path for error context (see [`DeviceOpError`]).
    label: String,
}

impl LinuxCiDataDevice {
    /// Open `path` **non-blocking** (`O_NONBLOCK`) as a duplex (`O_RDWR`) CI
    /// data-plane device.
    ///
    /// # Why non-blocking (#1066)
    /// The kernel `ts_read()` on the real device sleeps
    /// (`wait_event_interruptible`) until >=188 bytes are available; it never
    /// returns `0` to signal "no more data right now" the way a mock's queue
    /// does. [`CaDescrambler::feed_ts`](crate::descrambler::CaDescrambler::feed_ts)
    /// loops `read` until it sees `0` to drain whatever the CAM has already
    /// produced — on a *blocking* fd, the first call that drains the queue
    /// blocks forever on the next read instead of returning, and the caller
    /// can never feed more TS to unblock it (a permanent hang). Opening
    /// `O_NONBLOCK` makes an empty read return `EWOULDBLOCK` immediately,
    /// which [`read`](Self::read) maps to `Ok(0)` — the same "no more data"
    /// signal the mock already gives, so the drain loop terminates. See
    /// `tests::read_on_empty_device_returns_immediately_not_blocking` below,
    /// which drives this exact function over a real blocking-capable FIFO.
    fn open_path(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| DeviceOpError::wrap("open (O_RDWR|O_NONBLOCK)", &path_label(path), e))?;
        Ok(Self {
            io: CiEndpoints::Duplex(file),
            label: path_label(path),
        })
    }

    /// Open `path` as two one-direction descriptors: `O_RDONLY | O_NONBLOCK`
    /// for reads (so an empty device returns `EWOULDBLOCK` — see
    /// [`open_path`](Self::open_path) for why that matters) and `O_WRONLY` for
    /// writes, left blocking exactly as the zenith host has always opened it
    /// (verified on hardware: 150 s, 111k `feed_ts` calls, max 249 us, no
    /// error). The read side is opened first so that on a FIFO stand-in the
    /// write open finds a reader.
    fn open_split_path(path: &Path) -> io::Result<Self> {
        let label = path_label(path);
        let read = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| DeviceOpError::wrap("open (O_RDONLY|O_NONBLOCK)", &label, e))?;
        let write = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| DeviceOpError::wrap("open (O_WRONLY)", &label, e))?;
        Ok(Self {
            io: CiEndpoints::Split { write, read },
            label,
        })
    }

    /// `O_RDWR` first, the split open only if that fails with `EINVAL`
    /// (`open_duplex` is a parameter so the fallback is testable without a
    /// device that rejects `O_RDWR`).
    fn open_with(
        path: &Path,
        open_duplex: impl FnOnce(&Path) -> io::Result<Self>,
    ) -> io::Result<Self> {
        match open_duplex(path) {
            Ok(dev) => Ok(dev),
            Err(e) if DeviceOpError::errno_of(&e) == Some(libc::EINVAL) => {
                Self::open_split_path(path)
            }
            Err(e) => Err(e),
        }
    }

    /// Open `/dev/dvb/adapter{adapter}/ci{ci}`: `O_RDWR | O_NONBLOCK`, and on
    /// `EINVAL` (the Digital Devices CXD2099 `ci0` — see the type docs) the
    /// [split open](Self::open_split). Any other error is returned as is.
    pub fn open(adapter: u32, ci: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ci{ci}");
        Self::open_with(Path::new(&path), Self::open_path)
    }

    /// Open `/dev/dvb/adapter{adapter}/ci{ci}` as two one-direction
    /// descriptors (write `O_WRONLY`, read `O_RDONLY | O_NONBLOCK`), for
    /// drivers that reject `O_RDWR` (Digital Devices CXD2099).
    pub fn open_split(adapter: u32, ci: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ci{ci}");
        Self::open_split_path(Path::new(&path))
    }

    /// Wrap an already-open CI data-plane device file. The caller is
    /// responsible for having opened it `O_NONBLOCK` — see
    /// `open_path` for why a blocking fd here hangs
    /// `feed_ts` (#1066).
    #[must_use]
    pub fn from_file(file: File) -> Self {
        Self {
            io: CiEndpoints::Duplex(file),
            label: "CI data device (from_file)".to_owned(),
        }
    }

    /// Wrap an already-open pair of one-direction descriptors: `write`
    /// (`O_WRONLY`) and `read` (`O_RDONLY`). The caller is responsible for
    /// having opened **`read` with `O_NONBLOCK`** — a blocking read fd makes
    /// `feed_ts`'s drain loop hang (#1066) — see `open_path`.
    #[must_use]
    pub fn from_split(write: File, read: File) -> Self {
        Self {
            io: CiEndpoints::Split { write, read },
            label: "CI data device (from_split)".to_owned(),
        }
    }

    fn write_file(&mut self) -> &mut File {
        match &mut self.io {
            CiEndpoints::Duplex(f) => f,
            CiEndpoints::Split { write, .. } => write,
        }
    }

    fn read_file(&mut self) -> &mut File {
        match &mut self.io {
            CiEndpoints::Duplex(f) => f,
            CiEndpoints::Split { read, .. } => read,
        }
    }
}

/// Printable device path for [`DeviceOpError`].
fn path_label(path: &Path) -> String {
    path.display().to_string()
}

impl CiDataDevice for LinuxCiDataDevice {
    fn write(&mut self, ts: &[u8]) -> io::Result<()> {
        if !ts.len().is_multiple_of(TS_PACKET_LEN) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write not a multiple of 188 bytes",
            ));
        }
        let label = self.label.clone();
        self.write_file()
            .write_all(ts)
            .map_err(|e| DeviceOpError::wrap(format!("write ({} bytes)", ts.len()), &label, e))
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !buf.len().is_multiple_of(TS_PACKET_LEN) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read buffer not a multiple of 188 bytes",
            ));
        }
        let cap = buf.len();
        match self.read_file().read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => {
                let label = self.label.clone();
                Err(DeviceOpError::wrap(
                    format!("read (<= {cap} bytes)"),
                    &label,
                    e,
                ))
            }
        }
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        let label = self.label.clone();
        poll_readable(self.read_file(), timeout)
            .map_err(|e| DeviceOpError::wrap(format!("poll ({timeout:?})"), &label, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Create a fresh FIFO (named pipe) under the OS temp dir and return its
    /// path. `mkfifo` + a FIFO's blocking-read semantics are POSIX/Linux —
    /// matching why this whole module compiles only under
    /// `target_os = "linux"` (crate doc) — and, unlike a Unix socketpair
    /// wrapped in a test-defined `CiDataDevice` (flagged in review as not
    /// biting), opening the FIFO through `LinuxCiDataDevice::open_path`
    /// drives the actual production `open`/`read` code under test.
    fn make_fifo() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dvb-ci-runtime-c1-1066-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_nanos()
        ));
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &path,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .expect("mkfifoat failed");
        path
    }

    /// #1066: `open_path` must open `O_NONBLOCK`, so an empty read
    /// returns `Ok(0)` promptly instead of blocking forever — the real bug
    /// was `CaDescrambler::feed_ts`'s drain loop hanging on its second
    /// `read`. A FIFO opened `O_RDWR` blocks on an empty read exactly like
    /// the real `ciM` character device unless `O_NONBLOCK` is set, so this
    /// is a faithful (if not identical) stand-in, unlike a socketpair with
    /// its own test-defined `CiDataDevice` impl, which never calls the
    /// production `open`/`read` at all.
    ///
    /// Removing `.custom_flags(libc::O_NONBLOCK)` from `open_path` makes the
    /// first `recv_timeout` below time out. Verified by hand: with that line
    /// removed, `cargo test -p dvb-ci-runtime --all-features --locked
    /// --target x86_64-unknown-linux-gnu` cannot be *run* on macOS at all
    /// (this module only exists under `target_os = "linux"`, and macOS
    /// cannot execute a Linux binary), so the removal was checked instead
    /// with `cargo test -p dvb-ci-runtime --all-features --locked --target
    /// x86_64-unknown-linux-gnu --no-run` (compiles either way) plus the
    /// cross-clippy command from CLAUDE.md (lints clean either way — clippy
    /// cannot see that the missing flag is a bug); real execution proof of
    /// this specific assertion is Linux CI, which runs this test on every
    /// push.
    #[test]
    fn read_on_empty_device_returns_immediately_not_blocking() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let mut writer = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open a second, writer-only handle on the same FIFO");

        let (tx, rx) = mpsc::channel();
        let (tx_go, rx_go) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut buf = [0u8; TS_PACKET_LEN];
            let r0 = dev.read(&mut buf).map(|n| (n, buf));
            if tx.send(r0).is_err() {
                return;
            }
            if rx_go.recv().is_err() {
                return;
            }

            let r1 = dev.read(&mut buf).map(|n| (n, buf));
            if tx.send(r1).is_err() {
                return;
            }
            if rx_go.recv().is_err() {
                return;
            }

            let r2 = dev.read(&mut buf).map(|n| (n, buf));
            let _ = tx.send(r2);
        });

        // 1: empty device (writer end open, no data written yet) — must
        // return `Ok(0)` promptly, not block.
        let first = rx.recv_timeout(Duration::from_secs(1));
        let Ok(Ok((n0, _))) = first else {
            // The reader thread may be stuck in a blocking `read`; write a
            // byte so it can still unblock and exit, then fail.
            let _ = writer.write_all(&[0u8; TS_PACKET_LEN]);
            panic!(
                "empty-device read did not return promptly (blocked past 1s) — \
                 got {first:?}; this is the #1066 hang O_NONBLOCK fixes"
            );
        };
        assert_eq!(n0, 0, "empty device must read 0 bytes");

        // 2: write one full packet BEFORE letting the reader proceed, so
        // there is no race between the write and the second `read`.
        let mut packet = [0xAAu8; TS_PACKET_LEN];
        packet[0] = 0x47; // MPEG-2 TS sync byte — realism only, not checked by read().
        writer.write_all(&packet).expect("write one packet");
        tx_go.send(()).expect("reader thread must still be alive");

        let (n1, buf1) = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader thread must respond promptly")
            .expect("read must not error");
        assert_eq!(n1, TS_PACKET_LEN, "must read exactly one packet's worth");
        assert_eq!(
            &buf1[..n1],
            &packet[..],
            "must read back the exact bytes written"
        );

        // 3: drained again — back to `Ok(0)`, not blocking.
        tx_go.send(()).expect("reader thread must still be alive");
        let (n2, _) = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("empty-again read did not return promptly (blocked past 1s)")
            .expect("read must not error");
        assert_eq!(n2, 0, "device must read 0 again once drained");

        let _ = std::fs::remove_file(&path);
    }

    /// A packet of recognisable bytes.
    fn packet(fill: u8) -> [u8; TS_PACKET_LEN] {
        let mut p = [fill; TS_PACKET_LEN];
        p[0] = 0x47;
        p
    }

    /// Run `f` on a thread and fail if it has not finished in `limit` - a
    /// read that blocks forever (the #1066 hang) must fail the test, not hang
    /// the suite. A stuck thread is unblocked through `unstick` so it can exit.
    fn within<T: Send + 'static>(
        limit: Duration,
        unstick: impl FnOnce(),
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(limit) {
            Ok(v) => v,
            Err(_) => {
                unstick();
                panic!("operation did not return within {limit:?} (blocked)");
            }
        }
    }

    /// CXD2099 `ci0` rejects `O_RDWR`; the device is two one-direction fds.
    /// The FIFO is blocking-faithful: a read on an empty FIFO whose write end
    /// is open never returns 0 (it blocks) unless the fd is `O_NONBLOCK`. A
    /// `feed_ts`-style drain loop (`read` until `0`) over the split read side
    /// must therefore terminate, deliver exactly what was written, and each
    /// call must come back promptly.
    #[test]
    fn split_read_side_is_nonblocking_and_a_drain_loop_terminates() {
        let path = make_fifo();
        // The open itself is bounded too: a blocking read-only open of a FIFO
        // with no writer yet sleeps, which is the same hang in another place.
        let open_path = path.clone();
        let mut dev = within(
            Duration::from_secs(2),
            || {},
            move || LinuxCiDataDevice::open_split_path(&open_path).expect("split open"),
        );
        // Loop the FIFO back: what the write fd writes, the read fd reads.
        dev.write(&packet(0xA1)).expect("write packet 1");
        dev.write(&packet(0xB2)).expect("write packet 2");

        let drained = within(
            Duration::from_secs(2),
            || {},
            move || {
                let mut got = Vec::new();
                let mut buf = [0u8; TS_PACKET_LEN];
                loop {
                    let n = dev.read(&mut buf).expect("read");
                    if n == 0 {
                        break;
                    }
                    assert_eq!(n, TS_PACKET_LEN);
                    got.push(buf);
                }
                // And once more on the now-empty device: still prompt, still 0.
                assert_eq!(dev.read(&mut buf).expect("read"), 0);
                got
            },
        );
        assert_eq!(drained, vec![packet(0xA1), packet(0xB2)]);
        let _ = std::fs::remove_file(&path);
    }

    /// The control that proves the mock is blocking-faithful and the test
    /// above bites: the same loopback with a read fd opened WITHOUT
    /// `O_NONBLOCK` (what `from_split` documents as the caller's mistake)
    /// blocks on the empty read instead of returning 0.
    #[test]
    fn split_with_a_blocking_read_fd_would_hang_the_drain_loop() {
        let path = make_fifo();
        // Open the write end first (O_RDWR on a FIFO never blocks) so the
        // blocking read-only open below finds a writer instead of waiting.
        let write = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("write open");
        let read = OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("blocking read-only open");
        // Keep a handle to unblock the stuck reader afterwards.
        let unblock = write.try_clone().expect("clone write fd");
        let mut dev = LinuxCiDataDevice::from_split(write, read);
        dev.write(&packet(0xC3)).expect("write");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; TS_PACKET_LEN];
            let first = dev.read(&mut buf); // the written packet
            let _ = tx.send(first);
            let second = dev.read(&mut buf); // empty: a blocking fd sleeps here
            let _ = tx.send(second);
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
            TS_PACKET_LEN
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(400)).is_err(),
            "a blocking read fd must NOT return on an empty FIFO - the mock is not blocking-faithful"
        );
        // Unblock the reader thread so the process can exit cleanly.
        let mut unblock = unblock;
        let _ = unblock.write_all(&packet(0xD4));
        let _ = std::fs::remove_file(&path);
    }

    /// `open` falls back to the split open when `O_RDWR` fails with `EINVAL` -
    /// and the fallback device really works (loopback through the FIFO).
    #[test]
    fn open_falls_back_to_split_on_einval_only() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_with(&path, |_| {
            Err(io::Error::from_raw_os_error(libc::EINVAL))
        })
        .expect("EINVAL on O_RDWR must fall back to the split open");
        assert!(matches!(dev.io, CiEndpoints::Split { .. }));
        dev.write(&packet(0xE5)).expect("write");
        let mut buf = [0u8; TS_PACKET_LEN];
        assert_eq!(dev.read(&mut buf).expect("read"), TS_PACKET_LEN);
        assert_eq!(buf, packet(0xE5));

        // A wrapped EINVAL (what `open_path` produces) triggers it too.
        let wrapped = LinuxCiDataDevice::open_with(&path, |p| {
            Err(DeviceOpError::wrap(
                "open (O_RDWR|O_NONBLOCK)",
                &path_label(p),
                io::Error::from_raw_os_error(libc::EINVAL),
            ))
        })
        .expect("wrapped EINVAL must fall back too");
        assert!(matches!(wrapped.io, CiEndpoints::Split { .. }));

        // Any other error is NOT papered over with a split open.
        for errno in [libc::ENOENT, libc::EACCES, libc::EBUSY, libc::ENODEV] {
            let err =
                LinuxCiDataDevice::open_with(&path, |_| Err(io::Error::from_raw_os_error(errno)))
                    .expect_err("a non-EINVAL open error must propagate");
            assert_eq!(DeviceOpError::errno_of(&err), Some(errno));
        }
        let _ = std::fs::remove_file(&path);
    }

    /// When `O_RDWR` works (the common case) `open_with` keeps the duplex fd.
    #[test]
    fn open_keeps_the_duplex_fd_when_rdwr_works() {
        let path = make_fifo();
        let dev =
            LinuxCiDataDevice::open_with(&path, LinuxCiDataDevice::open_path).expect("duplex open");
        assert!(matches!(dev.io, CiEndpoints::Duplex(_)));
        let _ = std::fs::remove_file(&path);
    }

    /// A failing device operation names the operation, the device and the
    /// errno, keeps the original `ErrorKind`, and still answers the errno.
    #[test]
    fn device_errors_carry_operation_device_and_errno() {
        let path = std::env::temp_dir().join(format!(
            "dvb-ci-runtime-h1-ctx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_nanos()
        ));
        // A read-only regular file: writing to it fails (EBADF on Linux).
        std::fs::write(&path, b"x").expect("stage file");
        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("open read-only");
        let mut dev = LinuxCaDevice::from_file(file, 2);
        let err = dev
            .write(&[0xA0, 0x01, 0x01])
            .expect_err("write to a read-only fd must fail");
        let text = err.to_string();
        assert!(
            text.contains("write (5 byte frame: slot 2, connection 1)"),
            "{text}"
        );
        assert!(text.contains("CA device (slot 2, from_file)"), "{text}");
        assert!(text.contains("os error"), "{text}");
        assert_eq!(DeviceOpError::errno_of(&err), Some(libc::EBADF));
        assert_eq!(err.kind(), io::Error::from_raw_os_error(libc::EBADF).kind());
        let _ = std::fs::remove_file(&path);
    }

    /// r10-W-2: a kernel frame that exactly fills [`MAX_CA_FRAME`] must be
    /// reported as an error (possible truncation), not silently accepted as
    /// a complete TPDU. A regular file lets us stage exactly that many bytes
    /// without needing real CI hardware.
    #[test]
    fn read_rejects_a_frame_that_fills_the_scratch_buffer() {
        let path = std::env::temp_dir().join(format!(
            "dvb-ci-runtime-w2-fullframe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_nanos()
        ));
        std::fs::write(&path, vec![0xAAu8; MAX_CA_FRAME]).expect("stage a MAX_CA_FRAME-byte file");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open the staged file");
        let mut dev = LinuxCaDevice::from_file(file, 0);
        let mut buf = [0u8; 8192];
        let err = dev
            .read(&mut buf)
            .expect_err("a full-buffer read must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// Sanity: an ordinary short frame still reads through unaffected.
    #[test]
    fn read_still_returns_short_frames_normally() {
        let path = std::env::temp_dir().join(format!(
            "dvb-ci-runtime-w2-shortframe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_nanos()
        ));
        // link header (slot, connection_id) + a 3-byte "TPDU".
        std::fs::write(&path, [0x00, 0x01, 0xDE, 0xAD, 0xBE]).expect("stage a short file");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open the staged file");
        let mut dev = LinuxCaDevice::from_file(file, 0);
        let mut buf = [0u8; 16];
        let n = dev.read(&mut buf).expect("short read must succeed");
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[0xDE, 0xAD, 0xBE]);
        let _ = std::fs::remove_file(&path);
    }

    /// r10-W-3: `CA_GET_SLOT_INFO` returning `ENOTTY` (the driver — or, here,
    /// a plain regular file standing in for one that doesn't support the
    /// ioctl at all — has no CI-slot ioctl handling) must still fall back to
    /// "present + ready", exactly as the documented EINVAL fallback did. A
    /// regular file is a faithful stand-in: `ioctl` on a non-device fd
    /// reliably returns `ENOTTY` on Linux, so this exercises the real
    /// `libc::ioctl` call and its errno branch, not a mocked one.
    #[test]
    fn slot_info_falls_back_on_enotty() {
        let path = std::env::temp_dir().join(format!(
            "dvb-ci-runtime-w3-slotinfo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_nanos()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("create a regular file to stand in for the CA device");
        let mut dev = LinuxCaDevice::from_file(file, 3);
        let info = dev.slot_info().expect("ENOTTY must fall back, not error");
        assert_eq!(info.num, 3);
        assert!(info.module_present);
        assert!(info.module_ready);
        let _ = std::fs::remove_file(&path);
    }
    fn poll_file(file: &File, timeout: Duration) -> io::Result<bool> {
        poll_readable(file, timeout)
    }

    /// `poll_readable` on an empty FIFO with a short timeout reports `false`
    /// after about that long; with data it reports `true` immediately.
    #[test]
    fn poll_readable_reports_data_and_honours_the_timeout() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let mut writer = OpenOptions::new().write(true).open(&path).expect("writer");

        let start = std::time::Instant::now();
        assert!(
            !dev.poll(Duration::from_millis(80)).unwrap(),
            "empty FIFO is not readable"
        );
        assert!(
            start.elapsed() >= Duration::from_millis(70),
            "poll returned before its timeout"
        );

        writer.write_all(&[0x47; TS_PACKET_LEN]).unwrap();
        let start = std::time::Instant::now();
        assert!(
            dev.poll(Duration::from_secs(5)).unwrap(),
            "data written: readable"
        );
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "must return as soon as readable"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A zero timeout never blocks.
    #[test]
    fn poll_readable_zero_timeout_does_not_block() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let _writer = OpenOptions::new().write(true).open(&path).expect("writer");
        let start = std::time::Instant::now();
        assert!(!dev.poll(Duration::ZERO).unwrap());
        assert!(start.elapsed() < Duration::from_millis(500));
        let _ = std::fs::remove_file(&path);
    }

    /// Sub-millisecond timeouts used to truncate to `poll(.., 0)` (returning
    /// at once); they must now sleep their true duration and never error.
    #[test]
    fn poll_readable_sub_millisecond_timeout_keeps_its_duration() {
        let path = make_fifo();
        let mut dev = LinuxCiDataDevice::open_path(&path).expect("open_path");
        let _writer = OpenOptions::new().write(true).open(&path).expect("writer");
        let start = std::time::Instant::now();
        assert!(!dev.poll(Duration::from_micros(300)).unwrap());
        assert!(
            start.elapsed() >= Duration::from_micros(250),
            "a 300 us timeout was truncated: returned after {:?}",
            start.elapsed()
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Hang-up without data (writer closed) is not "readable" for this API:
    /// the old code tested `revents & POLLIN` only. The device itself opens
    /// the FIFO `O_RDWR` (it is its own writer, so no hang-up ever shows), so
    /// this drives `poll_readable` on a separate read-only descriptor, and
    /// proves with a raw `rustix::event::poll` that the kernel really reports POLLHUP
    /// there (otherwise the assertion would be vacuous).
    #[test]
    fn poll_readable_ignores_hangup_without_data() {
        let path = make_fifo();
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .expect("read-only O_NONBLOCK open");
        let writer = OpenOptions::new().write(true).open(&path).expect("writer");
        drop(writer); // POLLHUP on the read end, no POLLIN, no data

        let mut raw = [PollFd::new(&reader, PollFlags::IN)];
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let r = rustix::event::poll(&mut raw, Some(&zero)).expect("poll");
        assert_eq!(r, 1, "kernel must report an event");
        let revents = raw[0].revents();
        assert!(
            revents.contains(PollFlags::HUP) && !revents.contains(PollFlags::IN),
            "precondition: POLLHUP without POLLIN, got revents={revents:?}"
        );

        assert!(!poll_file(&reader, Duration::from_millis(50)).unwrap());
        let _ = std::fs::remove_file(&path);
    }

    /// Our `CaSlotInfo` must match the kernel's `struct ca_slot_info`
    /// (`int num; int type; unsigned int flags;`) exactly, since the ioctl
    /// writes it through a raw pointer and the request number encodes its size.
    #[test]
    fn ca_slot_info_matches_kernel_layout() {
        assert_eq!(core::mem::size_of::<CaSlotInfo>(), KERNEL_CA_SLOT_INFO_SIZE);
        assert_eq!(
            core::mem::align_of::<CaSlotInfo>(),
            KERNEL_CA_SLOT_INFO_ALIGN
        );
    }

    // r10-W-1: the request numbers the kernel's `dvb_ca_en50221` `ioctl`
    // handler dispatches on, per target architecture. Linux does not have one
    // `_IOC` layout: asm-generic targets (x86, arm, aarch64, riscv, s390x, ...)
    // use `dir << 30 | size << 16 | type << 8 | nr` with `_IOC_READ = 2`;
    // mips/powerpc/sparc use `dir << 29` (3 direction bits, 13 size bits) with
    // `_IOC_NONE = 1`, `_IOC_READ = 2`. The expected values are the well-known
    // ones for `uapi/linux/dvb/ca.h`'s `_IO('o', 128)` and
    // `_IOR('o', 130, ca_slot_info_t)` (12-byte struct). A hand-rolled
    // asm-generic encoding fails the second block.
    //
    // Compile-time asserts, so `cargo check --tests --target <arch>` verifies
    // an architecture whose test binary cannot be run on the build host.
    #[cfg(not(any(
        target_arch = "mips",
        target_arch = "mips32r6",
        target_arch = "mips64",
        target_arch = "mips64r6",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "sparc",
        target_arch = "sparc64"
    )))]
    const _: () = {
        assert!(CA_RESET == 0x0000_6F80);
        assert!(CA_GET_SLOT_INFO == 0x800C_6F82);
    };
    #[cfg(any(
        target_arch = "mips",
        target_arch = "mips32r6",
        target_arch = "mips64",
        target_arch = "mips64r6",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))]
    const _: () = {
        assert!(CA_RESET == 0x2000_6F80);
        assert!(CA_GET_SLOT_INFO == 0x400C_6F82);
    };
}
