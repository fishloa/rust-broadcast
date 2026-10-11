//! Linux `/dev/dvb/adapterN/caM` [`CaDevice`] implementation (the `linux`
//! feature).
//!
//! This is the one place the crate uses `unsafe` — the DVB CA ioctls
//! (`CA_RESET`, `CA_GET_SLOT_INFO`) via `libc`, confined to the private
//! `ioctl` submodule below, which exposes safe functions. The ioctl request
//! numbers are
//! computed from the standard Linux `_IOC` encoding (Documentation/userspace-api
//! + `include/uapi/linux/dvb/ca.h`), not hard-coded magic.
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
use crate::device::{CaDevice, SlotInfo};

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

// --- Linux _IOC ioctl encoding (uapi/asm-generic/ioctl.h) ------------------
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_NONE: u32 = 0;
const IOC_READ: u32 = 2;

const fn ioc(dir: u32, typ: u32, nr: u32, size: u32) -> u64 {
    ((dir << IOC_DIRSHIFT) | (typ << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as u64
}

// DVB CA device (uapi/linux/dvb/ca.h): magic 'o', ca_slot_info, flags bit.
const DVB_CA_MAGIC: u32 = b'o' as u32;
const CA_RESET: u64 = ioc(IOC_NONE, DVB_CA_MAGIC, 128, 0);
const CA_GET_SLOT_INFO: u64 = ioc(
    IOC_READ,
    DVB_CA_MAGIC,
    130,
    core::mem::size_of::<CaSlotInfo>() as u32,
);
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
    use std::io;
    use std::os::unix::io::{AsRawFd, BorrowedFd};

    // r10-W-1: `libc::ioctl`'s request parameter is `libc::Ioctl`, which is
    // `c_ulong` on glibc but `c_int` on musl/uclibc/Android — a hard-coded
    // `as libc::c_ulong` fails to compile there. The encoded request always
    // fits in 32 bits, so the narrowing cast on musl/Android is lossless.

    /// `CA_RESET` on `fd`.
    pub(super) fn ca_reset(fd: BorrowedFd<'_>) -> io::Result<()> {
        // SAFETY: `fd` is a live borrowed descriptor for the whole call (the
        // `BorrowedFd` lifetime guarantees it is open); `CA_RESET` is an
        // `_IO` request that takes no argument, so no pointer is passed and
        // there is no memory for the kernel to read or write.
        let r = unsafe { libc::ioctl(fd.as_raw_fd(), CA_RESET as libc::Ioctl) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// `CA_GET_SLOT_INFO` on `fd`, filling `info` (its `num` selects the slot).
    pub(super) fn ca_get_slot_info(fd: BorrowedFd<'_>, info: &mut CaSlotInfo) -> io::Result<()> {
        // SAFETY: `fd` is live for the whole call (as above). The request is
        // `_IOR('o', 130, CaSlotInfo)`: the kernel reads `num` and writes the
        // whole `ca_slot_info`, whose size/alignment `CaSlotInfo` mirrors
        // (`#[repr(C)]`, asserted by the `ca_slot_info_matches_kernel_layout`
        // test). `info` is an exclusive `&mut` to a fully-initialised value that
        // outlives the call, so the pointer is valid, aligned, and unaliased
        // for the kernel's write.
        let r = unsafe {
            libc::ioctl(
                fd.as_raw_fd(),
                CA_GET_SLOT_INFO as libc::Ioctl,
                info as *mut CaSlotInfo,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
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
}

impl LinuxCaDevice {
    /// Open `/dev/dvb/adapter{adapter}/ca{ca}` (slot 0).
    pub fn open(adapter: u32, ca: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ca{ca}");
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file, slot: 0 })
    }

    /// Wrap an already-open CA device file for `slot`.
    #[must_use]
    pub fn from_file(file: File, slot: u8) -> Self {
        Self { file, slot }
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
            Err(e) => return Err(e),
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
        self.file.write_all(&frame)
    }

    fn reset(&mut self) -> io::Result<()> {
        ioctl::ca_reset(self.file.as_fd())?;
        // The module needs a moment to re-initialise before Create_T_C.
        std::thread::sleep(RESET_SETTLE);
        Ok(())
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
                _ => Err(err),
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
    }
}

/// A [`CiDataDevice`] backed by a Linux DVB CI TS data-plane device
/// (`/dev/dvb/adapterN/ciM`). The host writes scrambled TS and reads the
/// descrambled TS back; I/O is in whole 188-byte packets.
#[derive(Debug)]
pub struct LinuxCiDataDevice {
    file: File,
}

impl LinuxCiDataDevice {
    /// Open `path` **non-blocking** (`O_NONBLOCK`) as a CI data-plane device.
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
            .open(path)?;
        Ok(Self { file })
    }

    /// Open `/dev/dvb/adapter{adapter}/ci{ci}` — see `open_path`.
    pub fn open(adapter: u32, ci: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ci{ci}");
        Self::open_path(Path::new(&path))
    }

    /// Wrap an already-open CI data-plane device file. The caller is
    /// responsible for having opened it `O_NONBLOCK` — see
    /// `open_path` for why a blocking fd here hangs
    /// `feed_ts` (#1066).
    #[must_use]
    pub fn from_file(file: File) -> Self {
        Self { file }
    }
}

impl CiDataDevice for LinuxCiDataDevice {
    fn write(&mut self, ts: &[u8]) -> io::Result<()> {
        if !ts.len().is_multiple_of(TS_PACKET_LEN) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write not a multiple of 188 bytes",
            ));
        }
        self.file.write_all(ts)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !buf.len().is_multiple_of(TS_PACKET_LEN) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read buffer not a multiple of 188 bytes",
            ));
        }
        match self.file.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        poll_readable(&self.file, timeout)
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
}
