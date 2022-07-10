//! Interactive tool to mess around with swap and physical memory on illumos

// TODO next ideas:
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

use anyhow::anyhow;
use anyhow::Context;
use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use std::fmt::Write;
use std::str::FromStr;
use swappy::bytesize_display::ByteSizeDisplayGiB;
use swappy::swappy::Swappy;

fn main() -> reedline_repl_rs::Result<()> {
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
    Ok(Some(swapinfo.display().to_string()))
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
    writeln!(s, "{:18}  {:11}  {:9}", "ADDR", "SIZE (B)", "SIZE (GiB)")
        .unwrap();
    for m in swappy.mappings() {
        let size = m.size();
        writeln!(
            s,
            "{:16p}  {:11}  {:10} {:9} {}",
            m.addr,
            size.as_u64(),
            ByteSizeDisplayGiB(size),
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
    write!(s, "{}", swapinfo.display()).unwrap();
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
    Ok(Some(swapinfo.display().to_string()))
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
    write!(s, "{}", swapinfo.display()).unwrap();
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
