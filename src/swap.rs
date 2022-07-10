//! Exposes the system's swap-related accounting stats

use crate::bytesize_display::ByteSizeDisplayGiB;
use crate::bytesize_display::ByteSizeDisplayKiB;
use crate::PAGE_SIZE;
use anyhow::Context;
use bytesize::ByteSize;

// See sys/swap.h
const SC_AINFO: libc::c_int = 5;

extern "C" {
    fn swapctl(cmd: libc::c_int, arg: *mut libc::c_void) -> libc::c_int;
}

/// Describes illumos swap-related accounting statistics
// See sys/swap.h
#[repr(C)]
#[derive(Debug)]
pub struct AnonInfo {
    ani_max: usize,
    ani_free: usize,
    ani_resv: usize,
}

impl AnonInfo {
    /// Amount of swap space for which physical pages have been allocated
    // See doswap() in usr/src/cmd/swap/swap.c.
    pub fn allocated(&self) -> ByteSize {
        ByteSize::b(((self.ani_max - self.ani_free) * PAGE_SIZE) as u64)
    }

    /// Amount of swap space that has been reserved but not allocated
    // See doswap() in usr/src/cmd/swap/swap.c.
    pub fn reserved(&self) -> ByteSize {
        ByteSize::b(
            (self.ani_resv * PAGE_SIZE) as u64 - self.allocated().as_u64(),
        )
    }

    /// Amount of swap space that is available for new reservations
    // See doswap() in usr/src/cmd/swap/swap.c.
    pub fn available(&self) -> ByteSize {
        ByteSize::b(((self.ani_max - self.ani_resv) * PAGE_SIZE) as u64)
    }

    /// Total swap space
    pub fn total(&self) -> ByteSize {
        ByteSize::b((self.ani_max * PAGE_SIZE) as u64)
    }
}

impl AnonInfo {
    /// Fetch the latest swap accounting stats
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

    /// Display the swap accounting stats in an expanded, detailed table
    pub fn display<'a>(&'a self) -> AnonInfoDisplay<'a> {
        AnonInfoDisplay(self)
    }
}

pub struct AnonInfoDisplay<'a>(&'a AnonInfo);

impl<'a> std::fmt::Display for AnonInfoDisplay<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let allocated = self.0.allocated();
        let reserved = self.0.reserved();
        let available = self.0.available();
        let total = self.0.total();

        f.write_str("SWAP ACCOUNTING\n")?;
        f.write_fmt(format_args!(
            "total (available + used):        {:9} KiB  {:5} GiB\n",
            ByteSizeDisplayKiB(total),
            ByteSizeDisplayGiB(total),
        ))?;
        f.write_fmt(format_args!(
            "    available:                   {:9} KiB  {:5} GiB\n",
            ByteSizeDisplayKiB(available),
            ByteSizeDisplayGiB(available),
        ))?;
        f.write_fmt(format_args!(
            "    used (reserved + allocated): {:9} KiB  {:5} GiB\n",
            ByteSizeDisplayKiB(allocated + reserved),
            ByteSizeDisplayGiB(allocated + reserved),
        ))?;
        f.write_fmt(format_args!(
            "        reserved, unallocated:   {:9} KiB  {:5} GiB\n",
            ByteSizeDisplayKiB(reserved),
            ByteSizeDisplayGiB(reserved),
        ))?;
        f.write_fmt(format_args!(
            "        allocated:               {:9} KiB  {:5} GiB\n",
            ByteSizeDisplayKiB(allocated),
            ByteSizeDisplayGiB(allocated),
        ))
    }
}
