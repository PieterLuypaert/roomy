use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::{env, fs};

use roomy::categories::Summary;
use roomy::cli::{self, Args, Command, Output};
use roomy::scan::{self, Progress};
use roomy::tree::Node;
use roomy::volumes::{self, Disk};
use roomy::{format, overview, print, ui};

fn main() -> ExitCode {
    match cli::parse(env::args_os().skip(1)) {
        Ok(Command::Help) => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("roomy {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(args)) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(problem) => {
                eprintln!("roomy: {problem}");
                ExitCode::FAILURE
            }
        },
        Err(problem) => {
            eprintln!("roomy: {problem}\nTry 'roomy --help'.");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let is_terminal = io::stdout().is_terminal();
    let Some(target) = args.path else {
        return match args.output {
            Output::Auto if is_terminal => {
                overview::run().map_err(|err| format!("the terminal failed: {err}"))
            }
            Output::Json => print_disk(args.depth, print::summary_json),
            Output::Auto | Output::Text => print_disk(args.depth, print::summary_text),
        };
    };
    let path = fs::canonicalize(&target).map_err(|err| format!("{}: {err}", target.display()))?;
    match args.output {
        Output::Auto if is_terminal => {
            ui::run(path).map_err(|err| format!("the terminal failed: {err}"))
        }
        Output::Json => print(&path, args.depth, print::json),
        Output::Auto | Output::Text => print(&path, args.depth, print::text),
    }
}

fn print_disk(depth: usize, render: fn(&Disk, &Summary, usize) -> String) -> Result<(), String> {
    let mut disk = volumes::boot_disk().map_err(|err| format!("could not read the disk: {err}"))?;
    let scanned = overview::scan_disk(&disk, &Progress::default())
        .map_err(|err| format!("{}: {err}", format::path(&disk.scan_root)))?;
    disk.purgeable = scanned.purgeable;
    write_out(&render(&disk, &scanned.summary, depth))?;
    warn_skipped(&scanned.root);
    Ok(())
}

fn print(
    path: &Path,
    depth: usize,
    render: fn(&Node, &Path, usize) -> String,
) -> Result<(), String> {
    let progress = Progress::default();
    let root =
        scan::scan(path, &progress).map_err(|err| format!("{}: {err}", format::path(path)))?;
    write_out(&render(&root, path, depth))?;
    warn_skipped(&root);
    Ok(())
}

fn write_out(text: &str) -> Result<(), String> {
    let written = io::stdout().write_all(text.as_bytes());
    match written.and_then(|()| io::stdout().flush()) {
        Err(err) if err.kind() != io::ErrorKind::BrokenPipe => {
            Err(format!("could not write the output: {err}"))
        }
        _ => Ok(()),
    }
}

fn warn_skipped(root: &Node) {
    let denied = root.count_where(|node| node.is_denied);
    if denied > 0 {
        let count = format::count(denied);
        eprintln!(
            "roomy: {count} entries could not be opened. {}",
            format::ACCESS_HINT
        );
    }
    let unreadable = root.count_where(|node| node.is_unreadable);
    if unreadable > 0 {
        eprintln!(
            "roomy: {} entries could not be read",
            format::count(unreadable)
        );
    }
}
