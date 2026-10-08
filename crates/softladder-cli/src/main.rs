//! SoftLadder command line binary.
//!
//! Thin wrapper around [`softladder_cli::main_with_args`]: it forwards the
//! process arguments and exits with the returned code.

#![forbid(unsafe_code)]

fn main() {
    std::process::exit(softladder_cli::main_with_args(std::env::args_os()));
}
