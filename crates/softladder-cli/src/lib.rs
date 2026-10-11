//! Command line interface for SoftLadder.
//!
//! Six subcommands are available:
//!
//! * `run <project>` — loads a project and drives a [`Runtime`] for a number of
//!   scans. By default the scans use **simulated** time (`now_ms` advances by
//!   exactly `--period-ms` per cycle), which makes the run reproducible and
//!   fast; `--real-time` opts into wall-clock pacing with a sleep.
//! * `lint <project>` — loads a project, runs one scan and reports every
//!   diagnostic it can find without executing hardware.
//! * `import <clprj> -o <slprj>` — ClassicLadder import.
//! * `export <slprj> -o <clprj>` — ClassicLadder export.
//! * `test <project> <scenario>` — runs a `.sltest` input timeline against the
//!   project and checks its output assertions, for CI.
//! * `serve <project>` — scans on a wall-clock period while serving the monitor
//!   TCP protocol and the dashboard HTTP page, both against the same runtime.
//!
//! Besides the codes documented in `docs/ELEMENTS.md`, `lint` reports the
//! project-level codes `SL-E010` (duplicate id) and `SL-W010`/`SL-W011` (empty
//! project / empty rung).
//!
//! Exit codes: `0` success, `1` usage or I/O problem, `2` feature not
//! implemented yet, `3` the project has errors or a scenario failed.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
#[cfg(test)]
use softladder_core::ElementKind;
use softladder_core::{Diagnostic, Project, ScanEngine, Severity};
use softladder_io::{IoDriver, IoImage, SimDriver};
use softladder_monitor::{
    serve_http_stream, serve_stream, DashboardConfig, Role, ServerConfig, Token,
};
use softladder_project::{classicladder, native};
use softladder_runtime::{
    load_scenario, run_scenario, Clock, FailureKind, MonitoredRuntime, Runtime,
};

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
    /// Import a ClassicLadder `.clprj` project.
    Import(ImportArgs),
    /// Export a project to ClassicLadder `.clprj` format.
    Export(ExportArgs),
    /// Run a `.sltest` scenario against a project.
    Test(TestArgs),
    /// Scan while serving the monitor protocol and the dashboard.
    Serve(ServeArgs),
}

/// Arguments of `softladder run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Project file to run (`.slprj` or `.slprjz`).
    pub project: PathBuf,
    /// Number of scans to execute.
    #[arg(long, default_value_t = 100)]
    pub cycles: u64,
    /// Period between two scans, in milliseconds.
    ///
    /// Defaults to the project's `ScanConfig::period_ms`.
    #[arg(long)]
    pub period_ms: Option<u64>,
    /// Pace the run with the wall clock instead of deterministic simulated time.
    #[arg(long)]
    pub real_time: bool,
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

/// Arguments of `softladder test`.
#[derive(Debug, Args)]
pub struct TestArgs {
    /// SoftLadder project to run (`.slprj` or `.slprjz`).
    pub project: PathBuf,
    /// Scenario to check (`.sltest`).
    pub scenario: PathBuf,
}

/// Arguments of `softladder serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// SoftLadder project to run (`.slprj` or `.slprjz`).
    pub project: PathBuf,
    /// Scans to run before stopping and exiting.
    #[arg(long, default_value_t = 100)]
    pub cycles: u64,
    /// Period between two scans, in milliseconds.
    ///
    /// Defaults to the project's `ScanConfig::period_ms`.
    #[arg(long)]
    pub period_ms: Option<u64>,
    /// Address for the monitor TCP protocol.
    #[arg(long, default_value = "127.0.0.1:4840")]
    pub monitor_bind: std::net::SocketAddr,
    /// Address for the dashboard HTTP page.
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub http_bind: std::net::SocketAddr,
    /// Bearer tokens as `id=role` (`viewer`, `operator` or `engineer`),
    /// repeatable. With none, a default `local=engineer` bearer is used.
    #[arg(long)]
    pub token: Vec<String>,
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
        Command::Test(args) => test_project(&args),
        Command::Serve(args) => serve_project(&args),
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
///
/// Every field is a deterministic function of the project, the number of cycles
/// and the period: a simulated run never reports a wall-clock measurement, so
/// serializing the summary twice for the same command yields byte-identical
/// JSON.
#[derive(Debug, Serialize, PartialEq)]
struct RunSummary {
    /// Project that was executed.
    project: String,
    /// `true` when the run used deterministic simulated time.
    simulated: bool,
    /// Number of scans performed.
    cycles: u64,
    /// Period between two scans, in milliseconds.
    period_ms: u64,
    /// Longest scan duration in milliseconds; `0.0` for a simulated run.
    max_scan_ms: f64,
    /// Ticks that arrived later than the configured scan period.
    missed: u64,
    /// Digital output channels left set after the run, as mirrored through the
    /// simulation driver.
    active_outputs: Vec<usize>,
    /// Diagnostics collected over the whole run.
    diagnostics: Vec<Diagnostic>,
}

fn run_project(args: &RunArgs) -> i32 {
    let summary = match execute_run(args) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("error: {error}");
            return EXIT_USAGE;
        }
    };

    if args.json {
        match serde_json::to_string_pretty(&summary) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("error: cannot serialize the run summary: {error}");
                return EXIT_USAGE;
            }
        }
    } else {
        print_summary(&summary);
    }

    if summary
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        EXIT_DIAGNOSTICS
    } else {
        EXIT_OK
    }
}

/// Runs the project of `args` with a deterministic simulated clock (or a
/// realtime one with `--real-time`) and collects the run summary.
fn execute_run(args: &RunArgs) -> Result<RunSummary, String> {
    let mut runtime = Runtime::from_file(&args.project)
        .map_err(|error| format!("cannot load `{}`: {error}", args.project.display()))?;

    let period_ms = args
        .period_ms
        .unwrap_or(u64::from(runtime.project.scan.period_ms));
    let mut clock = if args.real_time {
        Clock::realtime(period_ms)
    } else {
        Clock::simulated(period_ms)
    };

    let batch = runtime.run_cycles(&mut clock, args.cycles);

    Ok(RunSummary {
        project: args.project.display().to_string(),
        simulated: batch.simulated,
        cycles: batch.cycles,
        period_ms: batch.period_ms,
        max_scan_ms: batch.max_scan_ms,
        missed: batch.missed,
        active_outputs: mirror_outputs(&runtime.engine),
        diagnostics: batch.diagnostics,
    })
}

/// Prints the human readable summary.
///
/// Simulated runs never print a wall-clock measurement, so the output of two
/// identical commands is identical as well.
fn print_summary(summary: &RunSummary) {
    let mode = if summary.simulated {
        "simulated"
    } else {
        "realtime"
    };
    println!(
        "summary: mode={mode} cycles={} period_ms={} missed={} diagnostics={}",
        summary.cycles,
        summary.period_ms,
        summary.missed,
        summary.diagnostics.len()
    );
    if !summary.simulated {
        println!("         max_scan_ms={:.3}", summary.max_scan_ms);
    }
    if !summary.active_outputs.is_empty() {
        let channels: Vec<String> = summary
            .active_outputs
            .iter()
            .map(usize::to_string)
            .collect();
        println!("         active_outputs={}", channels.join(","));
    }
    for diagnostic in &summary.diagnostics {
        println!("    {diagnostic}");
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

/// Runs the `.sltest` scenario of `args` against its project.
///
/// Prints one line per failure plus a summary line; the process exit code is
/// what CI branches on: `0` when every assertion held, `3` otherwise.
fn test_project(args: &TestArgs) -> i32 {
    let project = match native::load(&args.project) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("error: cannot load `{}`: {error}", args.project.display());
            return EXIT_USAGE;
        }
    };
    let scenario = match load_scenario(&args.scenario) {
        Ok(scenario) => scenario,
        Err(error) => {
            eprintln!("error: {error}");
            return EXIT_USAGE;
        }
    };

    let result = run_scenario(&project, &scenario);
    for failure in &result.failures {
        match &failure.kind {
            FailureKind::SetupError { message } => {
                println!(
                    "    cycle {} {}: setup error: {message}",
                    failure.cycle, failure.var,
                );
            }
            FailureKind::Mismatch { expected, actual } => {
                let actual = actual
                    .map(|value| format!("{value:?}"))
                    .unwrap_or_else(|| "unknown variable".to_owned());
                println!(
                    "    cycle {} {}: expected {expected:?}, actual {actual}",
                    failure.cycle, failure.var,
                );
            }
        }
    }
    println!(
        "test: scenario=\"{}\" project=\"{}\" cycles={} asserted={} failures={}",
        result.name,
        args.project.display(),
        result.cycles,
        result.asserted,
        result.failures.len(),
    );

    if result.passed() {
        EXIT_OK
    } else {
        EXIT_DIAGNOSTICS
    }
}

/// Outcome of [`run_serve`].
#[derive(Debug, PartialEq)]
struct ServeSummary {
    /// Scans performed.
    cycles: u64,
    /// Monitor TCP connections served.
    monitor_connections: u64,
    /// Dashboard HTTP requests served.
    http_requests: u64,
}

/// Scans `cycles` on a wall-clock period while serving both protocols.
///
/// The scan loop, the monitor listener and the dashboard listener run on
/// three threads sharing one [`MonitoredRuntime`]; every connection gets its
/// own audited session. Returns when the scans finish.
fn run_serve(
    target: Arc<Mutex<MonitoredRuntime>>,
    monitor: TcpListener,
    http: TcpListener,
    cycles: u64,
    period_ms: u64,
    monitor_config: ServerConfig,
    http_config: DashboardConfig,
) -> ServeSummary {
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor_connections = Arc::new(AtomicU64::new(0));
    let http_requests = Arc::new(AtomicU64::new(0));

    let scan_target = Arc::clone(&target);
    let scan_done = Arc::clone(&done);
    let scan = std::thread::spawn(move || {
        for cycle in 1..=cycles {
            let now_ms = cycle.saturating_mul(period_ms);
            scan_target.lock().expect("scan lock").tick(now_ms);
            std::thread::sleep(Duration::from_millis(period_ms));
        }
        scan_done.store(true, Ordering::SeqCst);
        cycles
    });

    let monitor_target = Arc::clone(&target);
    let monitor_done = Arc::clone(&done);
    let monitor_count = Arc::clone(&monitor_connections);
    let monitor_config = Arc::new(monitor_config);
    let monitor_thread = std::thread::spawn(move || {
        accept_loop(monitor, &monitor_done, move |stream| {
            let mut shared = Arc::clone(&monitor_target);
            match serve_stream(stream, &mut shared, &monitor_config) {
                Ok(_) => {
                    monitor_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(error) => eprintln!("monitor connection failed: {error}"),
            }
        });
    });

    let http_target = Arc::clone(&target);
    let http_done = Arc::clone(&done);
    let http_count = Arc::clone(&http_requests);
    let http_config = Arc::new(http_config);
    let http_thread = std::thread::spawn(move || {
        accept_loop(http, &http_done, move |stream| {
            let mut shared = Arc::clone(&http_target);
            match serve_http_stream(stream, &mut shared, &http_config) {
                Ok(()) => {
                    http_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(error) => eprintln!("dashboard connection failed: {error}"),
            }
        });
    });

    let cycles = scan.join().expect("scan thread joins");
    monitor_thread.join().expect("monitor thread joins");
    http_thread.join().expect("dashboard thread joins");
    ServeSummary {
        cycles,
        monitor_connections: monitor_connections.load(Ordering::SeqCst),
        http_requests: http_requests.load(Ordering::SeqCst),
    }
}

/// Accepts connections until `done` is set, handling each on its own thread.
///
/// The listener is non-blocking so the loop notices `done` within milliseconds.
fn accept_loop(
    listener: TcpListener,
    done: &std::sync::atomic::AtomicBool,
    handle: impl Fn(TcpStream) + Send + Sync + 'static,
) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let handle = Arc::new(handle);
    while !done.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit non-blocking mode; the session and
                // dashboard handlers drive blocking reads with timeouts.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let handle = Arc::clone(&handle);
                std::thread::spawn(move || handle(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                eprintln!("accept failed: {error}");
                break;
            }
        }
    }
}

/// Parses `--token id=role` flags; with none, a default local bearer applies.
fn parse_tokens(flags: &[String]) -> Result<HashMap<String, Token>, String> {
    if flags.is_empty() {
        eprintln!("warning: no --token given; using default bearer `local` (engineer)");
        return Ok(HashMap::from([(
            "local".to_owned(),
            Token::new("local".to_owned(), Role::Engineer),
        )]));
    }
    flags
        .iter()
        .map(|flag| match flag.split_once('=') {
            Some((id, role)) if !id.is_empty() => match Role::parse(role) {
                Some(role) => Ok((id.to_owned(), Token::new(id.to_owned(), role))),
                None => Err(format!(
                    "bad --token `{flag}` (role is viewer, operator or engineer)"
                )),
            },
            _ => Err(format!("bad --token `{flag}` (id=role)")),
        })
        .collect()
}

fn serve_project(args: &ServeArgs) -> i32 {
    let project = match native::load(&args.project) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("error: cannot load `{}`: {error}", args.project.display());
            return EXIT_USAGE;
        }
    };
    let tokens = match parse_tokens(&args.token) {
        Ok(tokens) => tokens,
        Err(error) => {
            eprintln!("error: {error}");
            return EXIT_USAGE;
        }
    };
    let target = Arc::new(Mutex::new(MonitoredRuntime::new(project)));
    target.lock().expect("serve lock").start();
    let period_ms = args
        .period_ms
        .unwrap_or_else(|| target.lock().expect("serve lock").runtime().period_ms);

    let monitor = match TcpListener::bind(args.monitor_bind) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "error: cannot bind monitor to {}: {error}",
                args.monitor_bind
            );
            return EXIT_USAGE;
        }
    };
    let http = match TcpListener::bind(args.http_bind) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "error: cannot bind dashboard to {}: {error}",
                args.http_bind
            );
            return EXIT_USAGE;
        }
    };
    println!(
        "serve: monitor={} dashboard={} cycles={} period_ms={}",
        args.monitor_bind, args.http_bind, args.cycles, period_ms,
    );

    let summary = run_serve(
        target,
        monitor,
        http,
        args.cycles,
        period_ms,
        ServerConfig::new(tokens.clone()),
        DashboardConfig::new(tokens),
    );
    println!(
        "serve: done cycles={} monitor_connections={} http_requests={}",
        summary.cycles, summary.monitor_connections, summary.http_requests,
    );
    EXIT_OK
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
    // Element-level checks live in the core so that the editor, the CLI and the
    // monitor all report the same codes and messages.
    diagnostics.extend(softladder_core::lint(&project));
    let mut engine = ScanEngine::new(project);
    diagnostics.extend(engine.scan_once(0).diagnostics);
    diagnostics.dedup_by(|right, left| {
        right.code == left.code
            && right.message == left.message
            && right.section == left.section
            && right.rung == left.rung
    });
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
    }

    diagnostics
}

fn import_project(args: &ImportArgs) -> i32 {
    let report = match classicladder::import_file(&args.input) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: cannot import `{}`: {error}", args.input.display());
            return EXIT_USAGE;
        }
    };
    print_diagnostics(&report.diagnostics);
    if let Err(error) = native::save(&report.project, &args.output) {
        eprintln!("error: cannot write `{}`: {error}", args.output.display());
        return EXIT_USAGE;
    }
    println!(
        "imported `{}` into `{}`",
        args.input.display(),
        args.output.display()
    );
    exit_for_diagnostics(&report.diagnostics)
}

fn export_project(args: &ExportArgs) -> i32 {
    // The template is what keeps every part SoftLadder does not model alive.
    // A `.clprj*` argument is imported first (and used as its own template);
    // a native project starts from an empty document.
    let (project, template, mut diagnostics) = match native::load(&args.project) {
        Ok(project) => (project, classicladder::Document::empty(), Vec::new()),
        Err(error) => {
            if !is_classicladder_path(&args.project) {
                eprintln!("error: cannot load `{}`: {error}", args.project.display());
                return EXIT_USAGE;
            }
            let document = match std::fs::read(&args.project)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    classicladder::Document::parse(&bytes).map_err(|error| error.to_string())
                }) {
                Ok(document) => document,
                Err(error) => {
                    eprintln!(
                        "error: cannot read the container of `{}`: {error}",
                        args.project.display()
                    );
                    return EXIT_USAGE;
                }
            };
            match classicladder::import(&document) {
                Ok(report) => (report.project, document, report.diagnostics),
                Err(error) => {
                    eprintln!("error: cannot import `{}`: {error}", args.project.display());
                    return EXIT_USAGE;
                }
            }
        }
    };
    let report = match classicladder::export_file(&project, &template, &args.output) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: cannot export `{}`: {error}", args.output.display());
            return EXIT_USAGE;
        }
    };
    diagnostics.extend(report.diagnostics);
    print_diagnostics(&diagnostics);
    println!(
        "exported `{}` into `{}`",
        args.project.display(),
        args.output.display()
    );
    exit_for_diagnostics(&diagnostics)
}

/// `true` for the path spellings of a ClassicLadder container.
fn is_classicladder_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["clp", "clprj", "clprjz"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// Prints every diagnostic the way the CLI always has.
fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        println!("{diagnostic}");
    }
}

/// `3` when any diagnostic is an error, `0` otherwise.
fn exit_for_diagnostics(diagnostics: &[Diagnostic]) -> i32 {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        EXIT_DIAGNOSTICS
    } else {
        EXIT_OK
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
            "--real-time",
            "--json",
        ])
        .expect("arguments parse");
        match cli.command {
            Command::Run(args) => {
                assert_eq!(args.project, PathBuf::from("project.slprj"));
                assert_eq!(args.cycles, 5);
                assert_eq!(args.period_ms, Some(2));
                assert!(args.real_time);
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
                // No `--period-ms`: the project's `ScanConfig` decides.
                assert_eq!(args.period_ms, None);
                assert!(!args.real_time);
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
    fn test_arguments_are_parsed() {
        let cli = Cli::try_parse_from(["softladder", "test", "project.slprj", "scenario.sltest"])
            .expect("parses");
        match cli.command {
            Command::Test(args) => {
                assert_eq!(args.project, PathBuf::from("project.slprj"));
                assert_eq!(args.scenario, PathBuf::from("scenario.sltest"));
            }
            other => panic!("expected the test subcommand, got {other:?}"),
        }
    }

    #[test]
    fn test_passes_on_the_traffic_light_example() {
        let scenario =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/traffic_light.sltest");
        let code = test_project(&TestArgs {
            project: example_project(),
            scenario,
        });
        assert_eq!(code, EXIT_OK);
    }

    #[test]
    fn test_reports_failures_and_missing_files() {
        let dir = std::env::temp_dir();
        let failing = dir.join("softladder-cli-failing.sltest");
        std::fs::write(
            &failing,
            r#"{"name":"bad","steps":[],"assertions":[{"cycle":0,"var":"%Q0","expected":true}]}"#,
        )
        .expect("writes");
        let code = test_project(&TestArgs {
            project: example_project(),
            scenario: failing.clone(),
        });
        assert_eq!(code, EXIT_DIAGNOSTICS);
        let _ = std::fs::remove_file(&failing);

        let code = test_project(&TestArgs {
            project: example_project(),
            scenario: PathBuf::from("/definitely/not/here.sltest"),
        });
        assert_eq!(code, EXIT_USAGE);
        let code = test_project(&TestArgs {
            project: PathBuf::from("/definitely/not/here.slprj"),
            scenario: PathBuf::from("/definitely/not/here.sltest"),
        });
        assert_eq!(code, EXIT_USAGE);
    }

    #[test]
    fn serve_arguments_are_parsed() {
        let cli = Cli::try_parse_from([
            "softladder",
            "serve",
            "project.slprj",
            "--cycles",
            "10",
            "--monitor-bind",
            "127.0.0.1:4840",
            "--token",
            "op-1=operator",
        ])
        .expect("parses");
        match cli.command {
            Command::Serve(args) => {
                assert_eq!(args.project, PathBuf::from("project.slprj"));
                assert_eq!(args.cycles, 10);
                assert_eq!(args.monitor_bind.port(), 4840);
                assert_eq!(args.token, vec!["op-1=operator".to_owned()]);
            }
            other => panic!("expected the serve subcommand, got {other:?}"),
        }
    }

    #[test]
    fn serve_scans_and_answers_both_protocols() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpStream;

        let monitor = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let http = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let monitor_addr = monitor.local_addr().expect("listener has an address");
        let http_addr = http.local_addr().expect("listener has an address");
        let tokens = HashMap::from([(
            "eng-1".to_owned(),
            Token::new("eng-1".to_owned(), Role::Engineer),
        )]);
        let project = native::load(&example_project()).expect("example loads");
        let target = Arc::new(Mutex::new(MonitoredRuntime::new(project)));
        target.lock().expect("test lock").start();
        let server = std::thread::spawn(move || {
            run_serve(
                target,
                monitor,
                http,
                500,
                1,
                ServerConfig::new(tokens.clone()),
                DashboardConfig::new(tokens),
            )
        });

        let stream = TcpStream::connect(monitor_addr).expect("client connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout sets");
        let mut reader = BufReader::new(stream.try_clone().expect("clone connects"));
        let mut writer = stream;
        let mut ask = |line: &str| {
            writer
                .write_all(format!("{line}\n").as_bytes())
                .expect("client writes");
            let mut answer = String::new();
            reader.read_line(&mut answer).expect("client reads");
            answer.trim_end().to_owned()
        };
        let auth: serde_json::Value =
            serde_json::from_str(&ask(r#"{"token":"eng-1"}"#)).expect("decodes");
        assert_eq!(auth, serde_json::json!({"type":"ok"}));
        let status: serde_json::Value =
            serde_json::from_str(&ask(r#"{"type":"get_status"}"#)).expect("decodes");
        assert_eq!(status["state"], "run");
        let forced: serde_json::Value = serde_json::from_str(&ask(
            r#"{"type":"force_var","var":{"kind":"PhysIn","index":0,"index_expr":null,"accessor":null},"value":{"Bit":true}}"#,
        ))
        .expect("decodes");
        assert_eq!(forced, serde_json::json!({"type":"ok"}));
        drop(reader);
        drop(writer);

        let mut http_stream = TcpStream::connect(http_addr).expect("client connects");
        http_stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout sets");
        write!(
            http_stream,
            "GET /api/vars?token=eng-1&var=%25I0 HTTP/1.1\r\nHost: x\r\n\r\n"
        )
        .expect("client writes");
        let mut response = String::new();
        BufReader::new(http_stream)
            .read_to_string(&mut response)
            .expect("client reads");
        assert!(response.starts_with("HTTP/1.0 200"), "{response}");
        assert!(response.contains(r#""var":"%I0""#), "{response}");
        assert!(response.contains(r#"{"Bit":true}"#), "{response}");

        let summary = server.join().expect("server joins");
        assert_eq!(summary.cycles, 500);
        assert_eq!(summary.monitor_connections, 1);
        assert_eq!(summary.http_requests, 1);
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
    fn project_level_checks_find_duplicate_ids_and_empty_rungs() {
        let mut project = Project::new("lint");
        project.rungs.push(softladder_core::Rung::new(1));
        project.rungs.push(softladder_core::Rung::new(1));
        project.rungs.push(softladder_core::Rung::new(2));

        let diagnostics = structural_diagnostics(&project);
        assert!(diagnostics.iter().any(|d| d.code == "SL-E010"));
        assert!(diagnostics.iter().any(|d| d.code == "SL-W011"));
    }

    #[test]
    fn element_level_checks_come_from_the_core_linter() {
        let mut project = Project::new("lint");
        let mut rung = softladder_core::Rung::new(1);
        rung.elements.push(softladder_core::PlacedElement::new(
            ElementKind::CoilOut,
            0,
            0,
        ));
        project.rungs.push(rung);
        let mut section = softladder_core::Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);

        let diagnostics = softladder_core::lint(&project);
        assert!(diagnostics.iter().any(|d| d.code == "SL-E004"));
    }

    #[test]
    fn running_the_example_project_succeeds() {
        let args = RunArgs {
            project: example_project(),
            cycles: 3,
            period_ms: Some(0),
            real_time: false,
            json: true,
        };
        assert_eq!(run_project(&args), EXIT_OK);
    }

    #[test]
    fn a_run_defaults_to_simulated_time() {
        let args = RunArgs {
            project: example_project(),
            cycles: 20,
            period_ms: None,
            real_time: false,
            json: false,
        };
        let summary = execute_run(&args).expect("the example project runs");
        assert!(summary.simulated);
        assert_eq!(summary.cycles, 20);
        assert_eq!(summary.missed, 0);
        // Without `--period-ms` the project's `ScanConfig` decides.
        assert_eq!(summary.period_ms, 10);
        assert_eq!(summary.max_scan_ms, 0.0);
    }

    #[test]
    fn a_simulated_run_does_not_sleep() {
        // 1000 cycles at one second each would take over 16 minutes if the run
        // were paced with the wall clock; simulated time must return at once.
        let args = RunArgs {
            project: example_project(),
            cycles: 1000,
            period_ms: Some(1000),
            real_time: false,
            json: false,
        };
        let started = std::time::Instant::now();
        let summary = execute_run(&args).expect("the example project runs");
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(summary.cycles, 1000);
        assert_eq!(summary.period_ms, 1000);
        assert_eq!(summary.missed, 0);
    }

    #[test]
    fn the_json_summary_is_byte_identical_between_runs() {
        let args = RunArgs {
            project: example_project(),
            cycles: 25,
            period_ms: None,
            real_time: false,
            json: true,
        };
        let first = execute_run(&args).expect("first run");
        let second = execute_run(&args).expect("second run");
        assert_eq!(first, second);

        let first_json = serde_json::to_string(&first).expect("the summary serializes");
        let second_json = serde_json::to_string(&second).expect("the summary serializes");
        assert_eq!(first_json, second_json);

        // The documented schema is present and stable.
        let value: serde_json::Value =
            serde_json::from_str(&first_json).expect("the summary is valid JSON");
        for key in [
            "project",
            "simulated",
            "cycles",
            "period_ms",
            "max_scan_ms",
            "missed",
            "active_outputs",
            "diagnostics",
        ] {
            assert!(value.get(key).is_some(), "the JSON summary has no `{key}`");
        }
        assert_eq!(value["simulated"], serde_json::Value::Bool(true));
        assert_eq!(value["cycles"], serde_json::json!(25));
        assert_eq!(value["missed"], serde_json::json!(0));
    }

    #[test]
    fn running_a_missing_project_reports_usage() {
        let args = RunArgs {
            project: PathBuf::from("/definitely/not/here.slprj"),
            cycles: 1,
            period_ms: None,
            real_time: false,
            json: true,
        };
        assert_eq!(run_project(&args), EXIT_USAGE);
    }

    #[test]
    fn importing_a_missing_container_reports_usage() {
        let args = ImportArgs {
            input: PathBuf::from("/definitely/not/here.clprj"),
            output: PathBuf::from("/tmp/out.slprj"),
        };
        assert_eq!(import_project(&args), EXIT_USAGE);
    }

    #[test]
    fn importing_a_corpus_project_writes_a_native_project() {
        let Some(input) = corpus_project("example.clprj") else {
            return;
        };
        let directory = TempDir::new();
        let output = directory.path().join("example.slprj");
        let args = ImportArgs {
            input: input.clone(),
            output: output.clone(),
        };
        assert_eq!(import_project(&args), EXIT_OK);
        let project = native::load(&output).expect("the imported project loads");
        assert_eq!(project.rungs.len(), 9);
        assert_eq!(project.name, "Example project");
    }

    #[test]
    fn importing_a_malformed_container_reports_an_error_diagnostic() {
        let directory = TempDir::new();
        let input = directory.path().join("bad.clprj");
        // A container whose rung file carries a cell that cannot be read.
        let part = "rung_0.csv";
        let text = format!(
            "_FILES_CLASSICLADDER\n_FILE-{part}\n#VER=3.0\nnot-a-cell\n_/FILE-{part}\n\
             _/FILES_CLASSICLADDER\n"
        );
        std::fs::write(&input, text).expect("the fixture is written");
        let args = ImportArgs {
            input,
            output: directory.path().join("bad.slprj"),
        };
        assert_eq!(import_project(&args), EXIT_DIAGNOSTICS);
    }

    #[test]
    fn exporting_an_authored_project_writes_a_container() {
        let directory = TempDir::new();
        let output = directory.path().join("traffic.clprj");
        let args = ExportArgs {
            project: example_project(),
            output: output.clone(),
        };
        assert_eq!(export_project(&args), EXIT_OK);
        let document = classicladder::Document::parse(&std::fs::read(&output).expect("readable"))
            .expect("the output is a container");
        assert!(document.part("rung_1.csv").is_some());
        // No template was given, so nothing is passed through.
        assert!(document.part("com_params.txt").is_none());
    }

    #[test]
    fn exporting_a_container_uses_it_as_its_own_template() {
        let Some(input) = corpus_project("example.clprj") else {
            return;
        };
        let directory = TempDir::new();
        let output = directory.path().join("example.clprj");
        let args = ExportArgs {
            project: input,
            output: output.clone(),
        };
        assert_eq!(export_project(&args), EXIT_OK);
        let document = classicladder::Document::parse(&std::fs::read(&output).expect("readable"))
            .expect("the output is a container");
        assert!(document.part("rung_0.csv").is_some());
        assert!(
            document.part("com_params.txt").is_some(),
            "the parts SoftLadder does not model are passed through"
        );
    }

    #[test]
    fn exporting_a_missing_project_reports_usage() {
        let args = ExportArgs {
            project: PathBuf::from("/definitely/not/here.slprj"),
            output: PathBuf::from("/tmp/out.clprj"),
        };
        assert_eq!(export_project(&args), EXIT_USAGE);
    }

    /// A self-cleaning temporary directory, so the tests need no extra crate.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "softladder-cli-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).expect("the temporary directory is created");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A corpus project, when the (git-ignored) corpus is present.
    fn corpus_project(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/classicladder-corpus/projects_examples")
            .join(name);
        if path.is_file() {
            Some(path)
        } else {
            println!(
                "note: `{}` is absent; run scripts/fetch_corpus.sh",
                path.display()
            );
            None
        }
    }
}
