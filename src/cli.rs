use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: roomy [PATH] [options]

See where disk space goes. Plain roomy shows where the whole disk's space
goes; roomy PATH opens a browser of that folder. In a pipe it prints instead.

Options:
  -p, --print      print instead of opening the full-screen view
  -j, --json       print as JSON
  -d, --depth N    how many levels to print (default 1); without PATH,
                   2 also lists the places in each group
  -h, --help       show this help
  -V, --version    show the version

Keys:
  up/down or j/k   move          right/enter or l   open
  left or h, esc   go back       s                  change sort (folders)
  o                show in Finder                   y copy the path
  d                move to the Trash (asks first)   b browse the whole disk
  ?                all keys                         q quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Auto,
    Text,
    Json,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub path: Option<PathBuf>,
    pub output: Output,
    pub depth: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Args),
    Help,
    Version,
}

pub fn parse(raw: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = Args {
        path: None,
        output: Output::Auto,
        depth: 1,
    };
    let mut raw = raw.into_iter();
    while let Some(arg) = raw.next() {
        let flag = arg.to_str().unwrap_or_default();
        match flag {
            "--" => {
                for rest in raw.by_ref() {
                    add_path(&mut args, rest)?;
                }
            }
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-p" | "--print" => args.output = Output::Text,
            "-j" | "--json" => args.output = Output::Json,
            "-d" | "--depth" => args.depth = depth(raw.next().as_deref().and_then(OsStr::to_str))?,
            _ if flag.starts_with("--depth=") => args.depth = depth(flag.strip_prefix("--depth="))?,
            _ if flag.starts_with('-') && flag != "-" => {
                return Err(format!("unknown option {flag}"));
            }
            _ => add_path(&mut args, arg)?,
        }
    }
    Ok(Command::Run(args))
}

fn add_path(args: &mut Args, path: OsString) -> Result<(), String> {
    if args.path.is_some() {
        return Err("give at most one path".to_string());
    }
    args.path = Some(PathBuf::from(path));
    Ok(())
}

fn depth(value: Option<&str>) -> Result<usize, String> {
    value
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| "--depth needs a number, like --depth 2".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn defaults_to_the_browser_on_no_path() {
        let expected = Args {
            path: None,
            output: Output::Auto,
            depth: 1,
        };
        assert_eq!(run(&[]), Ok(Command::Run(expected)));
    }

    #[test]
    fn reads_path_output_and_depth() {
        let expected = Args {
            path: Some(PathBuf::from("/tmp")),
            output: Output::Json,
            depth: 3,
        };
        assert_eq!(
            run(&["--json", "/tmp", "-d", "3"]),
            Ok(Command::Run(expected))
        );
    }

    #[test]
    fn rejects_unknown_options_and_bad_depths() {
        assert_eq!(run(&["--nope"]), Err("unknown option --nope".to_string()));
        assert!(run(&["--depth", "lots"]).is_err());
        assert!(run(&["--depth"]).is_err());
        assert!(run(&["a", "b"]).is_err());
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        let expected = Args {
            path: Some(PathBuf::from("-odd")),
            output: Output::Auto,
            depth: 2,
        };
        assert_eq!(
            run(&["--depth=2", "--", "-odd"]),
            Ok(Command::Run(expected))
        );
    }

    #[test]
    fn help_and_version_win() {
        assert_eq!(run(&["/tmp", "-h"]), Ok(Command::Help));
        assert_eq!(run(&["-V"]), Ok(Command::Version));
    }
}
