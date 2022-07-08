//! Interactive tool to mess around with swap and physical memory on illumos

use reedline_repl_rs::clap::{Arg, ArgMatches, Command};
use reedline_repl_rs::{Repl, Result};

fn cmd_mappings(_args: ArgMatches, swappy: &mut Swappy) -> Result<Option<String>> {
    Ok(Some(swappy.mappings.join(", ")))
}

fn cmd_add_mapping(args: ArgMatches, swappy: &mut Swappy) -> Result<Option<String>> {
    swappy.mappings.push(args.value_of("label").unwrap().to_string());
    Ok(None)
}

fn main() -> Result<()> {
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
}
