//! Interactive tool to mess around with swap and physical memory on illumos

// TODO next ideas:
// - better describe what "used" and "available" are
// - play around with some real examples to validate how I think this works
// - print out more kstats:
//   - swap allocation failures
//   - memory values: availrmem, freemem, etc.
//   - pageout activity?

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use reedline_repl_rs::Result as ReplResult;
use std::fmt::Write;
use std::os::unix::process::ExitStatusExt;
use std::str::FromStr;

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
    let size_str: &String = args.get_one("size").unwrap();
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
    let swapinfo = Swappy::swap_info().unwrap();
    s.push_str(&swapinfo.format());
    s.push_str("\n\n");
    s.push_str(&do_print_swap_mappings(swappy));
    Ok(Some(s))
}

fn cmd_swap_touch(
    args: ArgMatches,
    swappy: &mut Swappy,
) -> Result<Option<String>, SwappyError> {
    let addr_str: &String = args.get_one("addr").unwrap();
    let addr_usize: usize = parse_int::parse(addr_str)
        .map_err(|e| anyhow!("parsing adr: {}", e))?;

    let mut s = String::new();
    if !swappy.swap_touch(addr_usize)? {
        s.push_str("warning: pages were already touched\n");
    }

    let swapinfo = Swappy::swap_info().unwrap();
    s.push_str(&swapinfo.format());
    Ok(Some(s))
}

fn main() -> ReplResult<()> {
    let swappy = Swappy::new();
    let mut repl = Repl::new(swappy)
        .with_name("swappy")
        .with_description("mess around with swap and physical memory")
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
            Command::new("swap-touch")
                .arg(Arg::new("addr").required(true))
                .about("Touch pages in a swap mapping to allocate them"),
            cmd_swap_touch,
        );

    repl.run()
}

struct Swappy {
    mappings: Vec<Mapping>,
}

struct Mapping {
    addr: *mut libc::c_void,
    size: usize,
    reserved: bool,
    allocated: bool,
}

impl Swappy {
    pub fn new() -> Swappy {
        Swappy { mappings: Vec::new() }
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
        for page_addr in (start_addr..end_addr).step_by(PAGE_SIZE) {
            let page_ptr: *mut u8 = page_addr as *mut u8;
            unsafe { std::ptr::write(page_ptr, 1) };
        }

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
