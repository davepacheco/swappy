//! [`Swappy`] encapsulates the work kicked off by the REPL

use crate::kstat::kstat_read_physmem;
use crate::kstat::PhysicalMemoryStats;
use crate::monitor::Monitor;
use crate::swap::AnonInfo;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use bytesize::ByteSize;
use std::os::unix::process::ExitStatusExt;

pub struct Swappy {
    mappings: Vec<Mapping>,
    monitor: Monitor,
}

impl Swappy {
    pub fn new() -> Swappy {
        Swappy { mappings: Vec::new(), monitor: Monitor::new() }
    }

    // Summary swap stats (like `swap -s`)
    pub fn swap_info() -> Result<AnonInfo, anyhow::Error> {
        AnonInfo::fetch()
    }

    // Iterate mappings created by swappy
    pub fn mappings(&self) -> impl std::iter::Iterator<Item = &Mapping> {
        self.mappings.iter()
    }

    // Create a swap mapping (using mmap)
    pub fn swap_reserve(
        &mut self,
        bytes: usize,
    ) -> Result<usize, anyhow::Error> {
        self.do_swap_map(bytes, true)
    }

    // Create a NORESERVE swap mapping (using mmap)
    pub fn swap_noreserve(
        &mut self,
        bytes: usize,
    ) -> Result<usize, anyhow::Error> {
        self.do_swap_map(bytes, false)
    }

    fn do_swap_map(
        &mut self,
        size: usize,
        reserved: bool,
    ) -> Result<usize, anyhow::Error> {
        let nullptr = std::ptr::null_mut();
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let baseflags = libc::MAP_ANON | libc::MAP_PRIVATE;
        let flags =
            if reserved { baseflags } else { baseflags | libc::MAP_NORESERVE };
        let addr = unsafe { libc::mmap(nullptr, size, prot, flags, -1, 0) };
        if addr.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("mmap anon memory");
        }

        self.mappings.push(Mapping { addr, size, reserved, allocated: false });
        Ok(addr as usize)
    }

    pub fn swap_rm(&mut self, addr: usize) -> Result<(), anyhow::Error> {
        let mapping = self
            .mappings
            .iter_mut()
            .find(|m| m.addr as usize == addr)
            .ok_or_else(|| anyhow!("no mapping with address 0x{:x}", addr))?;

        let (addr, size, allocated) =
            (mapping.addr, mapping.size, mapping.allocated);
        if allocated {
            self.monitor.enable();
        }
        let rv = unsafe { libc::munmap(addr, size) };
        let error = std::io::Error::last_os_error();
        if allocated {
            self.monitor.disable();
        }

        if rv != 0 {
            return Err(error).context("munmap");
        }

        self.mappings.retain(|m| m.addr != addr);
        Ok(())
    }

    pub fn swap_touch(&mut self, addr: usize) -> Result<bool, anyhow::Error> {
        let mut mapping = self
            .mappings
            .iter_mut()
            .find(|m| m.addr as usize == addr)
            .ok_or_else(|| anyhow!("no mapping with address 0x{:x}", addr))?;

        let rv = !mapping.allocated;
        mapping.allocated = true;

        let start_addr = mapping.addr as usize;
        let end_addr = mapping.addr as usize + mapping.size;
        self.monitor.enable();

        for page_addr in (start_addr..end_addr).step_by(crate::PAGE_SIZE) {
            let page_ptr: *mut u8 = page_addr as *mut u8;
            unsafe { std::ptr::write(page_ptr, 1) };
        }

        self.monitor.disable();

        Ok(rv)
    }

    // Runs mdb's ::memstat
    pub fn memstat() -> Result<String, anyhow::Error> {
        let cmd_output = std::process::Command::new("pfexec")
            .arg("mdb")
            .arg("-ke")
            .arg("::memstat")
            .output()
            .expect("failed to run: `pfexec mdb -ke ::memstat`");
        let stdout = String::from_utf8_lossy(&cmd_output.stdout);
        let stderr = String::from_utf8_lossy(&cmd_output.stderr);
        if !cmd_output.status.success() {
            let (verb, noun, which) =
                if let Some(code) = cmd_output.status.code() {
                    ("exited", "status", code.to_string())
                } else if let Some(signal) = cmd_output.status.signal() {
                    ("terminated", "signal", signal.to_string())
                } else {
                    // This should not be possible.
                    ("terminated", "signal", String::from("unknown"))
                };

            bail!(
                "pfexec mdb -ke ::memstat: {} unexpectedly with {} {}: \
                stdout:\n{}stderr:\n{}",
                verb,
                noun,
                which,
                stdout,
                stderr,
            );
        }

        Ok(stdout.to_string())
    }

    // Fetches various memory-related kstats
    pub fn kstat_read(&mut self) -> Result<PhysicalMemoryStats, anyhow::Error> {
        // XXX How are you supposed to do this?  I want to hang this off of
        // `self.kstat` but I can't because update() consumes it.
        let kstat = kstat_rs::Ctl::new().expect("initializing kstat");
        kstat_read_physmem(&kstat)
    }
}

pub struct Mapping {
    pub addr: *mut libc::c_void,
    size: usize,
    pub reserved: bool,
    pub allocated: bool,
}

impl Mapping {
    pub fn size(&self) -> ByteSize {
        ByteSize::b(u64::try_from(self.size).unwrap())
    }
}
