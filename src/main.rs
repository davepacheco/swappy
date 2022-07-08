//! Interactive tool to mess around with swap and physical memory on illumos

use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::Repl;
use reedline_repl_rs::Result as ReplResult;

fn cmd_mappings(_args: ArgMatches, swappy: &mut Swappy) -> ReplResult<Option<String>> {
    Ok(Some(swappy.mappings.join(", ")))
}

fn cmd_add_mapping(args: ArgMatches, swappy: &mut Swappy) -> ReplResult<Option<String>> {
    swappy
        .mappings
        .push(args.value_of("label").unwrap().to_string());
    Ok(None)
}

fn cmd_swap_info(_args: ArgMatches, _swappy: &mut Swappy) -> ReplResult<Option<String>> {
    let swapinfo = Swappy::swap_info().unwrap();
    let allocated = (swapinfo.ani_max - swapinfo.ani_free) * PAGE_SIZE;
    let reserved = (swapinfo.ani_resv * PAGE_SIZE) - allocated;
    let available = (swapinfo.ani_max - swapinfo.ani_resv) * PAGE_SIZE;
    let total = swapinfo.ani_max * PAGE_SIZE;
    Ok(Some(format!(
        "allocated:  {:9} KiB\n\
         reserved:   {:9} KiB\n\
         used:       {:9} KiB\n\
         available:  {:9} KiB\n\
         total:      {:9} KiB",
        allocated / 1024,
        reserved / 1024,
        (allocated + reserved) / 1024,
        available / 1024,
        total / 1024
    )))
}

fn main() -> ReplResult<()> {
    let swappy = Swappy::new();
    let mut repl = Repl::new(swappy)
        .with_name("swappy")
        .with_description("mess around with swap and physical memory")
        .with_command(
            Command::new("mappings").about("List mappings created"),
            cmd_mappings,
        )
        .with_command(
            Command::new("add_mapping")
                .arg(Arg::new("label").required(true))
                .about("Add a new mapping"),
            cmd_add_mapping,
        )
        .with_command(
            Command::new("swap_info").about("Show swap accounting information"),
            cmd_swap_info,
        );
    repl.run()
}

struct Swappy {
    mappings: Vec<String>,
}

impl Swappy {
    fn new() -> Swappy {
        Swappy {
            mappings: Vec::new(),
        }
    }

    fn swap_info() -> Result<AnonInfo, std::io::Error> {
        let mut rv = AnonInfo {
            ani_max: 0,
            ani_free: 0,
            ani_resv: 0,
        };
        let ptr = &mut rv as *mut _ as *mut libc::c_void;
        let r = unsafe { swapctl(SC_AINFO, ptr) };
        match r {
            0 => Ok(rv),
            _ => Err(std::io::Error::last_os_error()),
        }
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
