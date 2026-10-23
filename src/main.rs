use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use ruim::cli::{self, Args, Command, Output};
use ruim::scan::{self, Progress};
use ruim::tree::Node;
use ruim::{format, print, ui};

fn main() -> ExitCode {
    match cli::parse(env::args_os().skip(1)) {
        Ok(Command::Help) => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("ruim {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(args)) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(problem) => {
                eprintln!("ruim: {problem}");
                ExitCode::FAILURE
            }
        },
        Err(problem) => {
            eprintln!("ruim: {problem}\nTry 'ruim --help'.");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let target = args
        .path
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or("could not find your home folder, so pass a path")?;
    let path = fs::canonicalize(&target).map_err(|err| format!("{}: {err}", target.display()))?;
    match args.output {
        Output::Auto if io::stdout().is_terminal() => {
            ui::run(path).map_err(|err| format!("the terminal failed: {err}"))
        }
        Output::Json => print(&path, args.depth, print::json),
        Output::Auto | Output::Text => print(&path, args.depth, print::text),
    }
}

fn print(
    path: &Path,
    depth: usize,
    render: fn(&Node, &Path, usize) -> String,
) -> Result<(), String> {
    let progress = Progress::default();
    let root =
        scan::scan(path, &progress).map_err(|err| format!("{}: {err}", format::path(path)))?;
    let written = io::stdout().write_all(render(&root, path, depth).as_bytes());
    if let Err(err) = written.and_then(|()| io::stdout().flush())
        && err.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(format!("could not write the output: {err}"));
    }
    warn_skipped(&root);
    Ok(())
}

fn warn_skipped(root: &Node) {
    let denied = root.count_where(|node| node.is_denied);
    if denied > 0 {
        let count = format::count(denied);
        eprintln!(
            "ruim: {count} entries could not be opened. {}",
            format::ACCESS_HINT
        );
    }
    let unreadable = root.count_where(|node| node.is_unreadable);
    if unreadable > 0 {
        eprintln!(
            "ruim: {} entries could not be read",
            format::count(unreadable)
        );
    }
}
