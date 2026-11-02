//! `hoist`: the Hoista command line.
//!
//! Commands: `providers`, `--version` and `--help`. Deploy, import and the MCP
//! server are added here as the engine gains each workflow.

use hoista_engine::{Provider, VERSION};
use std::process::ExitCode;

const USAGE: &str = "\
hoist: deploy apps built with AI, from your own machine

Usage:
  hoist providers    List the providers Hoista supports
  hoist --version    Show the version
  hoist --help       Show this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["providers"] => {
            for provider in Provider::ALL {
                println!("{:<12}{provider}", provider.slug());
            }
            ExitCode::SUCCESS
        }
        ["--version" | "-V"] => {
            println!("hoist {VERSION}");
            ExitCode::SUCCESS
        }
        [] | ["--help" | "-h"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
