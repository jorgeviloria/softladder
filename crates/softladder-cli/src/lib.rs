//! Command line interface for SoftLadder.
//!
//! Four subcommands are available:
//!
//! * `run <project>` — loads a project and drives a [`Runtime`] for a number of
//!   scans, with the simulated clock taken from the real elapsed time and the
//!   loop paced by `--period-ms`.
//! * `lint <project>` — loads a project, runs one scan and reports every
//!   diagnostic it can find without executing hardware.
//! * `import <clprj> -o <slprj>` — ClassicLadder import, scheduled for M3.
//! * `export <slprj> -o <clprj>` — ClassicLadder export, scheduled for M3.
//!
//! Besides the codes documented in `softladder_core::diag`, `lint` reports the
//! structural codes `SL-E010` (duplicate id) and `SL-W010`/`SL-W011` (empty
//! project / empty rung).
//!
//! Exit codes: `0` success, `1` usage or I/O problem, `2` feature not
//! implemented yet, `3` the project has errors.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use softladder_core::{Diagnostic, ElementKind, Project, ScanEngine, Severity};
use softladder_io::{IoDriver, IoImage, SimDriver};
use softladder_project::{classicladder, native, ProjectError};
use softladder_runtime::Runtime;

/// Exit code for a successful command.
pub const EXIT_OK: i32 = 0;
/// Exit code for a usage or I/O error.
pub const EXIT_USAGE: i32 = 1;
/// Exit code for a feature that is not implemented yet.
pub const EXIT_NOT_IMPLEMENTED: i32 = 2;
/// Exit code for a project that contains errors.
pub const EXIT_DIAGNOSTICS: i32 = 3;

/// SoftLadder ladder-logic tooling.
#[derive(Debug, Parser)]
#[command(
    name = "softladder",
    version,
    about = "SoftLadder ladder-logic tooling",
    long_about = None
)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands of the SoftLadder CLI.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a project for a fixed number of scans.
    Run(RunArgs),
    /// Load a project and report its diagnostics without scanning hardware.
    Lint(LintArgs),
    /// Import a ClassicLadder `.clprj` project (planned for M3).
    Import(ImportArgs),
    /// Export a project to ClassicLadder `.clprj` format (planned for M3).
    Export(ExportArgs),
}

/// Arguments of `softladder run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Project file to run (`.slprj` or `.slprjz`).
    pub project: PathBuf,
    /// Number of scans to execute.
    #[arg(long, default_value_t = 100)]
    pub cycles: u64,
    /// Pause between two scans, in milliseconds.
    #[arg(long, default_value_t = 10)]
    pub period_ms: u64,
    /// Print a machine readable JSON summary instead of per-cycle lines.
    #[arg(long)]
    pub json: bool,
}

/// Arguments of `softladder lint`.
#[derive(Debug, Args)]
pub struct LintArgs {
    /// Project file to check (`.slprj` or `.slprjz`).
    pub project: PathBuf,
}

/// Arguments of `softladder import`.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// ClassicLadder container to read (`.clprj` or `.clprjz`).
    pub input: PathBuf,
    /// SoftLadder project to write.
    #[arg(short, long)]
    pub output: PathBuf,
}

/// Arguments of `softladder export`.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// SoftLadder project to read (`.slprj` or `.slprjz`).
    pub project: PathBuf,
    /// ClassicLadder container to write.
    #[arg(short, long)]
    pub output: PathBuf,
}

/// Parses command line arguments without touching the process environment.
pub fn parse_args<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args)
}

/// Runs a parsed command line and returns the process exit code.
pub fn execute(cli: Cli) -> i32 {
    match cli.command {
        Command::Run(args) => run_project(&args),
        Command::Lint(args) => lint_project(&args),
        Command::Import(args) => import_project(&args),
        Command::Export(args) => export_project(&args),
    }
}

/// Parses `args`, runs the CLI and returns the process exit code.
///
/// Help and version output are printed and reported as success.
pub fn main_with_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match parse_args(args) {
        Ok(cli) => execute(cli),
        Err(error) => {
            let exit_code = if error.use_stderr() {
                EXIT_USAGE
            } else {
                EXIT_OK
            };
            let _ = error.print();
            exit_code
        }
    }
}

/// JSON summary printed by `run --json`.
#[derive(Debug, Serialize)]
struct RunSummary<'a> {
    /// Project that was executed.
    project: String,
    /// Number of scans performed.
    cycles: u64,
    /// Longest scan duration in milliseconds.
    max_scan_ms: f64,
    /// Ticks that arrived later than the configured scan period.
    missed: u64,
    /// Digital output channels left set after the run, as mirrored through the
    /// simulation driver.
    active_outputs: Vec<usize>,
    /// Diagnostics collected over the whole run.
    diagnostics: &'a [Diagnostic],
}

fn run_project(args: &RunArgs) -> i32 {
    let mut runtime = match Runtime::from_file(&args.project) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: cannot load `{}`: {error}", args.project.display());
            return EXIT_USAGE;
        }
    };

    runtime.start_at(0);
    let started = Instant::now();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for _ in 0..args.cycles {
        // The simulated clock follows the real elapsed time of the run.
        let now_ms = started.elapsed().as_millis() as u64;
        let Some(report) = runtime.tick(now_ms) else {
            break;
        };
        if !args.json {
            println!(
                "cycle {:>6}  now={:>6}ms  state={:?}  diagnostics={}",
                report.cycles,
                now_ms,
                runtime.state,
                report.diagnostics.len()
            );
            for diagnostic in &report.diagnostics {
                println!("    {diagnostic}");
            }
        }
        diagnostics.extend(report.diagnostics);
        if args.period_ms > 0 {
            std::thread::sleep(Duration::from_millis(args.period_ms));
        }
    }

    let active_outputs = mirror_outputs(&runtime.engine);
    let stats = *runtime.stats();

    if args.json {
        let summary = RunSummary {
            project: args.project.display().to_string(),
            cycles: stats.cycles,
            max_scan_ms: stats.max_ms,
            missed: stats.missed,
            active_outputs,
            diagnostics: &diagnostics,
        };
        match serde_json::to_string_pretty(&summary) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("error: cannot serialize the run summary: {error}");
                return EXIT_USAGE;
            }
        }
    } else {
        println!(
            "summary: cycles={} max_scan_ms={:.3} missed={} diagnostics={}",
            stats.cycles,
            stats.max_ms,
            stats.missed,
            diagnostics.len()
        );
        for diagnostic in &diagnostics {
            println!("    {diagnostic}");
        }
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        EXIT_DIAGNOSTICS
    } else {
        EXIT_OK
    }
}

/// Copies the physical outputs of `engine` into a process image, echoes them
/// through the simulation driver and returns the channels that ended up set.
fn mirror_outputs(engine: &ScanEngine) -> Vec<usize> {
    let store = engine.store();
    let digital = store.digital_channels();
    let analog = store.analog_channels();
    let mut image = IoImage::new(digital, analog);
    for (channel, value) in store.phys_out.iter().enumerate() {
        if image.set_digital_output(channel, *value).is_err() {
            break;
        }
    }
    for (channel, value) in store.phys_out_words.iter().enumerate() {
        if image.set_analog_output(channel, f64::from(*value)).is_err() {
            break;
        }
    }

    let mut driver = SimDriver::new(digital, analog);
    if driver.write(&image).is_err() {
        return Vec::new();
    }
    (0..digital)
        .filter(|channel| driver.get_output(*channel) == Some(true))
        .collect()
}

fn lint_project(args: &LintArgs) -> i32 {
    let project = match native::load(&args.project) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("error: cannot load `{}`: {error}", args.project.display());
            return EXIT_USAGE;
        }
    };

    let mut diagnostics = structural_diagnostics(&project);
    let mut engine = ScanEngine::new(project);
    diagnostics.extend(engine.scan_once(0).diagnostics);
    diagnostics.sort_by_key(|diagnostic| std::cmp::Reverse(diagnostic.severity));

    for diagnostic in &diagnostics {
        println!("{diagnostic}");
    }
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    println!(
        "{}: {} diagnostic(s), {} error(s)",
        args.project.display(),
        diagnostics.len(),
        errors
    );

    if errors > 0 {
        EXIT_DIAGNOSTICS
    } else {
        EXIT_OK
    }
}

/// Checks the parts of a project that do not need a scan.
fn structural_diagnostics(project: &Project) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let mut seen_rungs: Vec<u32> = Vec::new();
    for rung in &project.rungs {
        if seen_rungs.contains(&rung.id) {
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "SL-E010",
                format!("duplicate rung id {}", rung.id),
            ));
        } else {
            seen_rungs.push(rung.id);
        }
    }

    let mut seen_sections: Vec<u32> = Vec::new();
    for section in &project.sections {
        if seen_sections.contains(&section.id) {
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "SL-E010",
                format!("duplicate section id {}", section.id),
            ));
        } else {
            seen_sections.push(section.id);
        }
    }

    if project.rungs.is_empty() {
        diagnostics.push(Diagnostic::new(
            Severity::Warning,
            "SL-W010",
            "the project has no rungs".to_owned(),
        ));
    }

    for (index, rung) in project.rungs.iter().enumerate() {
        if rung.elements.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    "SL-W011",
                    format!("rung {} is empty", rung.id),
                )
                .with_rung(index),
            );
        }
        for element in &rung.elements {
            let needs_variable = !matches!(
                element.kind,
                ElementKind::Compare | ElementKind::Operate | ElementKind::Connection
            );
            if needs_variable && element.var.is_none() {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Error,
                        "SL-E004",
                        format!(
                            "rung {} has a {:?} element without a variable",
                            rung.id, element.kind
                        ),
                    )
                    .with_rung(index),
                );
            }
        }
    }

    diagnostics
}

fn import_project(args: &ImportArgs) -> i32 {
    match classicladder::import_file(&args.input) {
        Ok(project) => match native::save(&project, &args.output) {
            Ok(()) => {
                println!(
                    "imported `{}` into `{}`",
                    args.input.display(),
                    args.output.display()
                );
                EXIT_OK
            }
            Err(error) => {
                eprintln!("error: cannot write `{}`: {error}", args.output.display());
                EXIT_USAGE
            }
        },
        Err(ProjectError::NotYetImplemented(milestone)) => {
            eprintln!(
                "error: importing ClassicLadder projects is not implemented yet \
                 (planned for {milestone}); the container of `{}` was read but its \
                 element mapping is still a stub",
                args.input.display()
            );
            EXIT_NOT_IMPLEMENTED
        }
        Err(error) => {
            eprintln!("error: cannot import `{}`: {error}", args.input.display());
            EXIT_USAGE
        }
    }
}

fn export_project(args: &ExportArgs) -> i32 {
    let project = match native::load(&args.project) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("error: cannot load `{}`: {error}", args.project.display());
            return EXIT_USAGE;
        }
    };
    match classicladder::export_file(&project, &args.output) {
        Ok(()) => {
            println!(
                "exported `{}` into `{}`",
                args.project.display(),
                args.output.display()
            );
            EXIT_OK
        }
        Err(ProjectError::NotYetImplemented(milestone)) => {
            eprintln!(
                "error: exporting to ClassicLadder format is not implemented yet \
                 (planned for {milestone})"
            );
            EXIT_NOT_IMPLEMENTED
        }
        Err(error) => {
            eprintln!("error: cannot export `{}`: {error}", args.output.display());
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_project() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/traffic_light.slprj")
    }

    #[test]
    fn run_arguments_are_parsed() {
        let cli = Cli::try_parse_from([
            "softladder",
            "run",
            "project.slprj",
            "--cycles",
            "5",
            "--period-ms",
            "2",
            "--json",
        ])
        .expect("arguments parse");
        match cli.command {
            Command::Run(args) => {
                assert_eq!(args.project, PathBuf::from("project.slprj"));
                assert_eq!(args.cycles, 5);
                assert_eq!(args.period_ms, 2);
                assert!(args.json);
            }
            other => panic!("expected the run subcommand, got {other:?}"),
        }
    }

    #[test]
    fn run_arguments_have_defaults() {
        let cli = Cli::try_parse_from(["softladder", "run", "project.slprj"]).expect("parses");
        match cli.command {
            Command::Run(args) => {
                assert_eq!(args.cycles, 100);
                assert_eq!(args.period_ms, 10);
                assert!(!args.json);
            }
            other => panic!("expected the run subcommand, got {other:?}"),
        }
    }

    #[test]
    fn lint_import_and_export_arguments_are_parsed() {
        let cli = Cli::try_parse_from(["softladder", "lint", "project.slprj"]).expect("parses");
        assert!(matches!(cli.command, Command::Lint(_)));

        let cli = Cli::try_parse_from(["softladder", "import", "old.clprj", "-o", "new.slprj"])
            .expect("parses");
        match cli.command {
            Command::Import(args) => {
                assert_eq!(args.input, PathBuf::from("old.clprj"));
                assert_eq!(args.output, PathBuf::from("new.slprj"));
            }
            other => panic!("expected the import subcommand, got {other:?}"),
        }

        let cli = Cli::try_parse_from(["softladder", "export", "new.slprj", "-o", "old.clprj"])
            .expect("parses");
        match cli.command {
            Command::Export(args) => {
                assert_eq!(args.project, PathBuf::from("new.slprj"));
                assert_eq!(args.output, PathBuf::from("old.clprj"));
            }
            other => panic!("expected the export subcommand, got {other:?}"),
        }
    }

    #[test]
    fn missing_subcommands_and_unknown_flags_are_rejected() {
        assert!(Cli::try_parse_from(["softladder"]).is_err());
        assert!(Cli::try_parse_from(["softladder", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["softladder", "run"]).is_err());
        assert!(Cli::try_parse_from(["softladder", "run", "p.slprj", "--nope"]).is_err());
    }

    #[test]
    fn help_and_version_are_successes() {
        assert_eq!(main_with_args(["softladder", "--help"]), EXIT_OK);
        assert_eq!(main_with_args(["softladder", "--version"]), EXIT_OK);
    }

    #[test]
    fn a_bad_command_line_reports_usage() {
        assert_eq!(main_with_args(["softladder", "nope"]), EXIT_USAGE);
    }

    #[test]
    fn linting_the_example_project_succeeds() {
        let args = LintArgs {
            project: example_project(),
        };
        assert_eq!(lint_project(&args), EXIT_OK);
    }

    #[test]
    fn linting_a_missing_project_reports_usage() {
        let args = LintArgs {
            project: PathBuf::from("/definitely/not/here.slprj"),
        };
        assert_eq!(lint_project(&args), EXIT_USAGE);
    }

    #[test]
    fn structural_checks_find_duplicates_and_missing_variables() {
        let mut project = Project::new("lint");
        project.rungs.push(softladder_core::Rung::new(1));
        project.rungs.push(softladder_core::Rung::new(1));
        let mut rung = softladder_core::Rung::new(2);
        rung.elements.push(softladder_core::PlacedElement::new(
            ElementKind::CoilOut,
            0,
            0,
        ));
        project.rungs.push(rung);

        let diagnostics = structural_diagnostics(&project);
        assert!(diagnostics.iter().any(|d| d.code == "SL-E010"));
        assert!(diagnostics.iter().any(|d| d.code == "SL-E004"));
        assert!(diagnostics.iter().any(|d| d.code == "SL-W011"));
    }

    #[test]
    fn running_the_example_project_succeeds() {
        let args = RunArgs {
            project: example_project(),
            cycles: 3,
            period_ms: 0,
            json: true,
        };
        assert_eq!(run_project(&args), EXIT_OK);
    }

    #[test]
    fn import_exits_non_zero_while_it_is_a_stub() {
        let args = ImportArgs {
            input: PathBuf::from("/definitely/not/here.clprj"),
            output: PathBuf::from("/tmp/out.slprj"),
        };
        assert_eq!(import_project(&args), EXIT_USAGE);
    }
}
