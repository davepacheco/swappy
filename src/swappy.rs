//! [`Swappy`] encapsulates the work kicked off by the REPL

use crate::bytesize_display::ByteSizeDisplayGiB;
use crate::kstat::kstat_read_physmem;
use crate::kstat::PhysicalMemoryStats;
use crate::swap::AnonInfo;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use bytesize::ByteSize;
use std::os::unix::process::ExitStatusExt;
use std::sync::mpsc::RecvTimeoutError;

pub struct Swappy {
    mappings: Vec<Mapping>,
    #[allow(dead_code)]
    monitor_thread: std::thread::JoinHandle<Result<(), anyhow::Error>>,
    monitor_tx: std::sync::mpsc::SyncSender<MonitorMessage>,
}

impl Swappy {
    pub fn new() -> Swappy {
        let (monitor_tx, monitor_rx) = std::sync::mpsc::sync_channel(4);
        Swappy {
            mappings: Vec::new(),
            monitor_thread: std::thread::spawn(move || {
                monitor_thread(monitor_rx)
            }),
            monitor_tx,
        }
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
            self.enable_monitor();
        }
        let rv = unsafe { libc::munmap(addr, size) };
        let error = std::io::Error::last_os_error();
        if allocated {
            self.disable_monitor();
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
        self.enable_monitor();

        for page_addr in (start_addr..end_addr).step_by(crate::PAGE_SIZE) {
            let page_ptr: *mut u8 = page_addr as *mut u8;
            unsafe { std::ptr::write(page_ptr, 1) };
        }

        self.disable_monitor();

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

    // Monitor subsystem
    //
    // Functions that expect to take a while and cause interesting effects on
    // the system can call enable_monitor() to print summary stats once per
    // second.  They call disable_monitor() to print one more stat and stop the
    // monitor.
    pub fn enable_monitor(&self) {
        if let Err(error) = self.monitor_tx.send(MonitorMessage::StartStats) {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to enable monitor: {:#}", error);
        }
    }

    pub fn disable_monitor(&self) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if let Err(error) = self.monitor_tx.send(MonitorMessage::StopStats(tx))
        {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to disable monitor: {:#}", error);
        }
        if let Err(error) = rx.recv() {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to wait for monitor: {:#}", error);
        }
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

enum MonitorMessage {
    StartStats,
    StopStats(std::sync::mpsc::SyncSender<()>),
}

fn monitor_thread(
    rx: std::sync::mpsc::Receiver<MonitorMessage>,
) -> Result<(), anyhow::Error> {
    loop {
        // Wait indefinitely to be told to start monitoring.
        match rx.recv().context("waiting for StartStats")? {
            MonitorMessage::StopStats(_) => panic!("stats already stopped"),
            MonitorMessage::StartStats => (),
        }

        // Now we're in monitor mode.  Print a header row.  Then we'll wait
        // again on the channel until we're told to stop.  The only difference
        // is that we wait with a timeout.  If we hit the timeout, we fetch and
        // print stats and then try again.

        println!(
            "{:5} {:10} {:9} {:10}",
            "FREE", "SWAP_ALLOC", "SWAP_RESV", "SWAP_TOTAL"
        );

        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Err(RecvTimeoutError::Timeout) => monitor_print(),
                Err(error) => {
                    return Err(error).context("waiting for StopStats")
                }
                Ok(MonitorMessage::StartStats) => {
                    panic!("stats already started")
                }
                Ok(MonitorMessage::StopStats(tx)) => {
                    tx.send(()).context("confirming StopStats")?;
                    break;
                }
            }
        }
    }
}

fn monitor_print() {
    if let Err(error) = monitor_print_stats().context("monitor_print()") {
        eprintln!("warning: {:#}", error);
    }
}

fn monitor_print_stats() -> Result<(), anyhow::Error> {
    let kstat = kstat_rs::Ctl::new().context("initializing kstat")?;
    let physmem = kstat_read_physmem(&kstat).context("kstat_read_physmem")?;
    // TODO refactor -- we use global funcs and associated funcs on Swappy.  We
    // should have one set of functions.  Also, we may just want to have all the
    // stat stuff happen in this background thread, changing the main thing to
    // just use channels to send requests for data.  It'd be cleaner in some
    // sense, but it's also not that bad to have multiple kstat readers.
    let swapinfo = Swappy::swap_info().context("swap_info")?;

    // TODO add kmem reap, arc reap, pageout activity

    println!(
        "{:5} {:10} {:9} {:10}",
        ByteSizeDisplayGiB(physmem.freemem),
        ByteSizeDisplayGiB(swapinfo.allocated()),
        ByteSizeDisplayGiB(swapinfo.reserved()),
        ByteSizeDisplayGiB(swapinfo.total()),
    );

    Ok(())
}
