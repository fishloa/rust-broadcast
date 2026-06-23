//! Linux `/dev/dvb/adapterN/caM` [`CaDevice`] implementation (the `linux`
//! feature).
//!
//! This is the one place the crate uses `unsafe` — the DVB CA ioctls
//! (`CA_RESET`, `CA_GET_SLOT_INFO`) via `libc`. The ioctl request numbers are
//! computed from the standard Linux `_IOC` encoding (Documentation/userspace-api
//! + `include/uapi/linux/dvb/ca.h`), not hard-coded magic.
//!
//! Runtime behaviour requires a real DVB card with a CI slot; it is
//! compile-checked in CI but exercised only on hardware.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::Duration;

use crate::dataplane::{CiDataDevice, TS_PACKET_LEN};
use crate::device::{CaDevice, SlotInfo};

/// Poll a file descriptor for readability up to `timeout`.
fn poll_readable(fd: libc::c_int, timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `pfd` points at one valid pollfd for the duration of the call.
    let r = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, ms) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(pfd.revents & libc::POLLIN != 0)
    }
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
/// `CA_CI_MODULE_READY` — the slot has a module that is ready.
const CA_CI_MODULE_READY: u32 = 1;

#[repr(C)]
struct CaSlotInfo {
    num: i32,
    typ: i32,
    flags: u32,
}

/// A [`CaDevice`] backed by a Linux DVB CA character device.
#[derive(Debug)]
pub struct LinuxCaDevice {
    file: File,
}

impl LinuxCaDevice {
    /// Open `/dev/dvb/adapter{adapter}/ca{ca}`.
    pub fn open(adapter: u32, ca: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ca{ca}");
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    /// Wrap an already-open CA device file.
    #[must_use]
    pub fn from_file(file: File) -> Self {
        Self { file }
    }
}

impl CaDevice for LinuxCaDevice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Driver calls this only after `poll` reports readable, so it will not
        // block. A `WouldBlock` is reported as "no data".
        match self.file.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)
    }

    fn reset(&mut self) -> io::Result<()> {
        // SAFETY: CA_RESET takes no argument; fd is a valid open CA device.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), CA_RESET as libc::c_ulong) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn slot_info(&mut self) -> io::Result<SlotInfo> {
        let mut si = CaSlotInfo {
            num: 0,
            typ: 0,
            flags: 0,
        };
        // SAFETY: CA_GET_SLOT_INFO writes a ca_slot_info; `si` is exactly that
        // struct and outlives the call; fd is a valid open CA device.
        let r = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                CA_GET_SLOT_INFO as libc::c_ulong,
                &mut si as *mut CaSlotInfo,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(SlotInfo {
            num: si.num as u8,
            module_ready: si.flags & CA_CI_MODULE_READY != 0,
        })
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        poll_readable(self.file.as_raw_fd(), timeout)
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
    /// Open `/dev/dvb/adapter{adapter}/ci{ci}`.
    pub fn open(adapter: u32, ci: u32) -> io::Result<Self> {
        let path = format!("/dev/dvb/adapter{adapter}/ci{ci}");
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    /// Wrap an already-open CI data-plane device file.
    #[must_use]
    pub fn from_file(file: File) -> Self {
        Self { file }
    }
}

impl CiDataDevice for LinuxCiDataDevice {
    fn write(&mut self, ts: &[u8]) -> io::Result<()> {
        if ts.len() % TS_PACKET_LEN != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write not a multiple of 188 bytes",
            ));
        }
        self.file.write_all(ts)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.len() % TS_PACKET_LEN != 0 {
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
        poll_readable(self.file.as_raw_fd(), timeout)
    }
}
