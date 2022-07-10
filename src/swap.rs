//! Exposes the system's swap-related accounting stats

use crate::PAGE_SIZE;
use anyhow::Context;
use bytesize::ByteSize;

// See sys/swap.h
const SC_AINFO: libc::c_int = 5;

extern "C" {
    fn swapctl(cmd: libc::c_int, arg: *mut libc::c_void) -> libc::c_int;
}

// See sys/swap.h
#[repr(C)]
#[derive(Debug)]
pub struct AnonInfo {
    ani_max: usize,
    ani_free: usize,
    ani_resv: usize,
}

impl AnonInfo {
    pub fn allocated(&self) -> ByteSize {
        ByteSize::b(((self.ani_max - self.ani_free) * PAGE_SIZE) as u64)
    }

    pub fn reserved(&self) -> ByteSize {
        ByteSize::b(
            (self.ani_resv * PAGE_SIZE) as u64 - self.allocated().as_u64(),
        )
    }

    pub fn available(&self) -> ByteSize {
        ByteSize::b(((self.ani_max - self.ani_resv) * PAGE_SIZE) as u64)
    }

    pub fn total(&self) -> ByteSize {
        ByteSize::b((self.ani_max * PAGE_SIZE) as u64)
    }
}

impl AnonInfo {
    pub fn fetch() -> Result<AnonInfo, anyhow::Error> {
        let mut rv = AnonInfo { ani_max: 0, ani_free: 0, ani_resv: 0 };
        let ptr = &mut rv as *mut _ as *mut libc::c_void;
        let r = unsafe { swapctl(SC_AINFO, ptr) };
        match r {
            0 => Ok(rv),
            _ => Err(std::io::Error::last_os_error())
                .context("swapctl(SC_AINFO)"),
        }
    }

    pub fn format(&self) -> String {
        // See doswap() in usr/src/cmd/swap/swap.c.
        let allocated = (self.ani_max - self.ani_free) * PAGE_SIZE;
        let reserved = (self.ani_resv * PAGE_SIZE) - allocated;
        let available = (self.ani_max - self.ani_resv) * PAGE_SIZE;
        let total = self.ani_max * PAGE_SIZE;

        format!(
            "SWAP ACCOUNTING\n\
         allocated:                  {:9} KiB  {:5.1} GiB\n\
         reserved (not allocated):   {:9} KiB  {:5.1} GiB\n\
         used:                       {:9} KiB  {:5.1} GiB\n\
         available:                  {:9} KiB  {:5.1} GiB\n\
         total:                      {:9} KiB  {:5.1} GiB",
            allocated / 1024,
            allocated as f64 / 1024.0 / 1024.0 / 1024.0,
            reserved / 1024,
            reserved as f64 / 1024.0 / 1024.0 / 1024.0,
            (allocated + reserved) / 1024,
            (allocated + reserved) as f64 / 1024.0 / 1024.0 / 1024.0,
            available / 1024,
            available as f64 / 1024.0 / 1024.0 / 1024.0,
            total / 1024,
            total as f64 / 1024.0 / 1024.0 / 1024.0,
        )
    }
}
