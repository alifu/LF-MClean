use std::io;
use std::process::ExitCode;

use clap::Parser;
use lfmclean::cli::{self, Cli};

fn main() -> ExitCode {
    match cli::run(&Cli::parse(), &mut io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lfmclean: {err:#}");
            ExitCode::FAILURE
        }
    }
}
