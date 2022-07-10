//! Interactive tool to mess around with swap and physical memory on illumos

// TODO next ideas:
// - better describe what swap "used" and "available" are
// - add commands:
//   - hoover up memory for ARC
//     - manage file
//       - create zero-byte file on startup
//       - extend it as requested.  when we extend it, write one byte to each
//         page.
//       - for "hoover", just read the file? (optional offset, size?)
//   - hoover up memory for page cache
//     - same file management as ARC; manage an mmap mapping size and read it?
//   - hoover up memory for kmem (socket buffers?)
// - play around with some real examples to validate how I think this works
// - print out more kstats:
//   - swap allocation failures
//   - memory values: availrmem, freemem, etc.
//   - pageout activity?
// - spawn mdb up front and just write ::memstat and read output when we want to
//   get the stats.  This will avoid forking a child process while we have huge
//   mappings.

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use reedline_repl_rs::Result as ReplResult;
use std::fmt::Write;
use std::os::unix::process::ExitStatusExt;
use std::str::FromStr;
use std::sync::mpsc::RecvTimeoutError;

#[derive(Debug)]
struct SwappyError(anyhow::Error);

impl std::fmt::Display for SwappyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:#}", self.0))
    }
}

impl From<reedline_repl_rs::Error> for SwappyError {
    fn from(error: reedline_repl_rs::Error) -> Self {
        SwappyError(anyhow!("REPL error: {:#}", error))
    }
}

impl From<anyhow::Error> for SwappyError {
    fn from(error: anyhow::Error) -> Self {
        SwappyError(error)
    }
}

fn cmd_memstat(
    _args: ArgMatches,
    _swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    Ok(Some(Swappy::memstat().expect("memstat")))
}

fn cmd_swap_info(
    _args: ArgMatches,
    _swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    let swapinfo = Swappy::swap_info()?;
    Ok(Some(swapinfo.format()))
}

fn cmd_swap_mappings(
    _args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    Ok(Some(do_print_swap_mappings(swappy)))
}

fn do_print_swap_mappings(swappy: &Swappy) -> String {
    let mut s = String::new();
    writeln!(s, "SWAPPY-CREATED MAPPINGS").unwrap();
    writeln!(s, "{:18}  {:11}  {:9}", "ADDR", "SIZE (B)", "SIZE (GB)").unwrap();
    for m in &swappy.mappings {
        writeln!(
            s,
            "{:16p}  {:11}  {:9.1} {:9} {}",
            m.addr,
            m.size,
            (m.size as f64) / 1024.0 / 1024.0 / 1024.0,
            if m.reserved { "" } else { "NORESERVE" },
            if m.allocated { "ALLOCATED" } else { "" },
        )
        .unwrap();
    }
    s
}

fn cmd_swap_reserve(
    args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    do_swap_create_mapping(args, swappy, true)
}

fn cmd_swap_noreserve(
    args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    do_swap_create_mapping(args, swappy, false)
}

fn do_swap_create_mapping(
    args: ArgMatches,
    swappy: &mut Swappy,
    reserved: bool,
) -> Result<Option<String>, SwappyError> {
    let size_str: &String =
        args.get_one("size").context("\"size\" argument")?;
    let bytes = bytesize::ByteSize::from_str(size_str)
        .map_err(|e| anyhow!("parsing size: {}", e))?;
    let bytes_u64 = bytes.as_u64();
    let bytes_usize = usize::try_from(bytes_u64)
        .map_err(|e| anyhow!("value too large: {}", e))?;
    let addr = if reserved {
        swappy.swap_reserve(bytes_usize)?
    } else {
        swappy.swap_noreserve(bytes_usize)?
    };

    let mut s = String::new();
    write!(s, "new mapping: 0x{:x}\n\n", addr).unwrap();
    let swapinfo = Swappy::swap_info()?;
    s.push_str(&swapinfo.format());
    s.push_str("\n\n");
    s.push_str(&do_print_swap_mappings(swappy));
    Ok(Some(s))
}

fn cmd_swap_rm(
    args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    let addr_str: &String =
        args.get_one("addr").context("\"addr\" argument")?;
    let addr_usize: usize = parse_int::parse(addr_str)
        .map_err(|e| anyhow!("parsing addr: {}", e))?;

    swappy.swap_rm(addr_usize)?;

    let swapinfo = Swappy::swap_info()?;
    Ok(Some(swapinfo.format()))
}

fn cmd_swap_touch(
    args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    let addr_str: &String =
        args.get_one("addr").context("\"addr\" argument")?;
    let addr_usize: usize = parse_int::parse(addr_str)
        .map_err(|e| anyhow!("parsing addr: {}", e))?;

    let mut s = String::new();
    if !swappy.swap_touch(addr_usize)? {
        s.push_str("warning: pages were already touched\n");
    }

    let swapinfo = Swappy::swap_info()?;
    s.push_str(&swapinfo.format());
    Ok(Some(s))
}

fn cmd_kstat_dump(
    _args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    let physmem = swappy.kstat_read()?;
    let mut s = String::new();
    write!(s, "{:?}", physmem).unwrap();
    Ok(Some(s))
}

fn main() -> ReplResult<()> {
    let swappy = Swappy::new();
    let mut repl = Repl::new(swappy)
        .with_name("swappy")
        .with_description("mess around with swap and physical memory")
        .with_partial_completions(false)
        .with_command(
            Command::new("memstat").about("Show physical memory usage"),
            cmd_memstat,
        )
        .with_command(
            Command::new("swap-info").about("Show swap accounting information"),
            cmd_swap_info,
        )
        .with_command(
            Command::new("swap-mappings")
                .about("Show mappings created by swappy"),
            cmd_swap_mappings,
        )
        .with_command(
            Command::new("swap-reserve")
                .arg(Arg::new("size").required(true))
                .about("Create a new swap mapping"),
            cmd_swap_reserve,
        )
        .with_command(
            Command::new("swap-noreserve")
                .arg(Arg::new("size").required(true))
                .about("Create a new swap mapping with NORESERVE"),
            cmd_swap_noreserve,
        )
        .with_command(
            Command::new("swap-rm")
                .arg(Arg::new("addr").required(true))
                .about("Remove a swap mapping"),
            cmd_swap_rm,
        )
        .with_command(
            Command::new("swap-touch")
                .arg(Arg::new("addr").required(true))
                .about("Touch pages in a swap mapping to allocate them"),
            cmd_swap_touch,
        )
        .with_command(
            Command::new("kstat-dump")
                .about("Dump various kstats of potential interest"),
            cmd_kstat_dump,
        );

    repl.run()
}

struct Swappy {
    mappings: Vec<Mapping>,
    monitor_thread: std::thread::JoinHandle<Result<(), anyhow::Error>>,
    monitor_tx: std::sync::mpsc::SyncSender<MonitorMessage>,
}

struct Mapping {
    addr: *mut libc::c_void,
    size: usize,
    reserved: bool,
    allocated: bool,
}

impl Swappy {
    pub fn new() -> Swappy {
        let (monitor_tx, monitor_rx) = std::sync::mpsc::sync_channel(4);
        Swappy {
            mappings: Vec::new(),
            monitor_thread: std::thread::spawn(move || monitor_thread(monitor_rx)),
            monitor_tx,
        }
    }

    // Summary swap stats (like `swap -s`)
    pub fn swap_info() -> Result<AnonInfo, anyhow::Error> {
        let mut rv = AnonInfo { ani_max: 0, ani_free: 0, ani_resv: 0 };
        let ptr = &mut rv as *mut _ as *mut libc::c_void;
        let r = unsafe { swapctl(SC_AINFO, ptr) };
        match r {
            0 => Ok(rv),
            _ => Err(std::io::Error::last_os_error())
                .context("swapctl(SC_AINFO)"),
        }
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

        for page_addr in (start_addr..end_addr).step_by(PAGE_SIZE) {
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
        if let Err(error) = self.monitor_tx.send(MonitorMessage::StopStats(tx)) {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to disable monitor: {:#}", error);
        }
        if let Err(error) = rx.recv() {
            // This is likely that the other thread panicked.
            eprintln!("warning: failed to wait for monitor: {:#}", error);
        }
    }
}

enum MonitorMessage {
    StartStats,
    StopStats(std::sync::mpsc::SyncSender<()>),
}

const PAGE_SIZE: usize = 4096;

// See sys/swap.h
const SC_AINFO: libc::c_int = 5;

extern "C" {
    pub fn swapctl(cmd: libc::c_int, arg: *mut libc::c_void) -> libc::c_int;
}

// See sys/swap.h
#[repr(C)]
#[derive(Debug)]
struct AnonInfo {
    ani_max: usize,
    ani_free: usize,
    ani_resv: usize,
}

impl AnonInfo {
    fn format(&self) -> String {
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

fn kstat_value_u64<'a>(
    datum: &'a kstat_rs::Named<'a>,
) -> Result<u64, anyhow::Error> {
    if let kstat_rs::NamedData::UInt64(value) = datum.value {
        Ok(value)
    } else {
        Err(anyhow!(
            "kstat named {:?}: expected u64, found {:?}",
            datum.name,
            datum.value
        ))
    }
}

#[derive(Debug)]
struct PhysicalMemoryStats {
    physmem: u64,
    freemem: u64,
    availrmem: u64,
    lotsfree: u64,
    desfree: u64,
    minfree: u64,
}

impl PhysicalMemoryStats {
    fn from_kstat<'a>(
        kst: &'a kstat_rs::Data<'a>,
    ) -> Result<Self, anyhow::Error> {
        let mut physmem: Option<u64> = None;
        let mut freemem: Option<u64> = None;
        let mut availrmem: Option<u64> = None;
        let mut lotsfree: Option<u64> = None;
        let mut desfree: Option<u64> = None;
        let mut minfree: Option<u64> = None;

        let named = if let kstat_rs::Data::Named(named_stats) = kst {
            named_stats
        } else {
            bail!("expected named kstat for reading physical memory");
        };

        for nst in named {
            let which_value = match nst.name {
                "physmem" => &mut physmem,
                "freemem" => &mut freemem,
                "availrmem" => &mut availrmem,
                "lotsfree" => &mut lotsfree,
                "desfree" => &mut desfree,
                "minfree" => &mut minfree,
                _ => continue,
            };

            if which_value.is_some() {
                bail!("duplicate value for kstat named {:?}", nst.name);
            }

            let value = kstat_value_u64(nst)?;
            *which_value = Some(value);
        }

        Ok(PhysicalMemoryStats {
            physmem: physmem.ok_or_else(|| anyhow!("missing stat physmem"))?,
            freemem: freemem.ok_or_else(|| anyhow!("missing stat freemem"))?,
            availrmem: availrmem
                .ok_or_else(|| anyhow!("missing stat availrmem"))?,
            lotsfree: lotsfree
                .ok_or_else(|| anyhow!("missing stat lotsfree"))?,
            desfree: desfree.ok_or_else(|| anyhow!("missing stat desfree"))?,
            minfree: minfree.ok_or_else(|| anyhow!("missing stat minfree"))?,
        })
    }
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
                Ok(MonitorMessage::StartStats) => panic!("stats already started"),
                Ok(MonitorMessage::StopStats(tx)) => {
                    tx.send(()).context("confirming StopStats")?;
                    break;
                }
            }
        }
    }
}

fn kstat_read_physmem(
    kstat: &kstat_rs::Ctl,
) -> Result<PhysicalMemoryStats, anyhow::Error> {
    let mut filter = kstat.filter(Some("unix"), Some(0), Some("system_pages"));
    let mut kst =
        filter.next().ok_or_else(|| anyhow!("found no system_pages kstats"))?;
    if filter.next().is_some() {
        bail!("found too many system_pages kstats");
    }

    let data = kstat.read(&mut kst).context("reading kstat")?;
    PhysicalMemoryStats::from_kstat(&data)
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

    // TODO
    let free_gib = (physmem.freemem as usize * PAGE_SIZE) as f64
        / 1024.0
        / 1024.0
        / 1024.0;
    // TODO copied from above
    let swap_allocated = (swapinfo.ani_max - swapinfo.ani_free) * PAGE_SIZE;
    let swap_reserved = (swapinfo.ani_resv * PAGE_SIZE) - swap_allocated;
    let _swap_available = (swapinfo.ani_max - swapinfo.ani_resv) * PAGE_SIZE;
    let swap_total = swapinfo.ani_max * PAGE_SIZE;
    println!(
        "{:5.1} {:10.1} {:9.1} {:10.1}",
        free_gib,
        swap_allocated as f64 / 1024.0 / 1024.0 / 1024.0,
        swap_reserved as f64 / 1024.0 / 1024.0 / 1024.0,
        swap_total as f64 / 1024.0 / 1024.0 / 1024.0
    );

    Ok(())
}
