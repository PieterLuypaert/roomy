use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::volumes;

pub type Trasher = fn(&Path) -> io::Result<()>;

pub fn trash(path: &Path) -> io::Result<()> {
    run(&mut trash_command(path))
}

pub fn reveal(path: &Path) -> io::Result<()> {
    run(&mut reveal_command(&volumes::usable(path)))
}

pub fn copy(path: &Path) -> io::Result<()> {
    let text = volumes::usable(path).to_string_lossy().into_owned();
    let mut command = copy_command();
    let program = command.get_program().to_string_lossy().into_owned();
    // xclip stays in the background holding stderr, so only the exit status is awaited.
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| missing(&program, err))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} failed ({status})")))
    }
}

pub fn trash_command(path: &Path) -> Command {
    if !cfg!(target_os = "macos") {
        let mut command = Command::new("gio");
        command.arg("trash").arg("--").arg(path);
        return command;
    }
    if Path::new("/usr/bin/trash").exists() {
        let mut command = Command::new("/usr/bin/trash");
        command.arg(path);
        return command;
    }
    let mut command = Command::new("osascript");
    command.arg("-e").arg(finder_delete(path));
    command
}

pub fn reveal_command(path: &Path) -> Command {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    }
}

fn copy_command() -> Command {
    if cfg!(target_os = "macos") {
        Command::new("pbcopy")
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Command::new("wl-copy")
    } else {
        let mut command = Command::new("xclip");
        command.args(["-selection", "clipboard"]);
        command
    }
}

pub fn finder_delete(path: &Path) -> String {
    let quoted = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("tell application \"Finder\" to delete POSIX file \"{quoted}\"")
}

pub fn is_protected(path: &Path, home: &Path) -> bool {
    let logical = volumes::logical(path);
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let is_top = logical.components().count() <= 2;
    let is_a_home = logical
        .parent()
        .is_some_and(|parent| parent == Path::new("/Users") || parent == Path::new("/home"));
    let is_above_home = volumes::logical(&home).starts_with(logical);
    is_top || is_a_home || is_above_home || logical.starts_with("/System")
}

fn run(command: &mut Command) -> io::Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|err| missing(&program, err))?;
    finished(&program, output)
}

fn finished(program: &str, output: std::process::Output) -> io::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr.lines().find(|line| !line.trim().is_empty());
    Err(io::Error::other(match reason {
        Some(line) => line.trim().to_string(),
        None => format!("{program} failed ({})", output.status),
    }))
}

fn missing(program: &str, err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::NotFound {
        io::Error::other(format!("{program} is not installed"))
    } else {
        err
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn trash_uses_the_system_trash() {
        let command = trash_command(Path::new("/tmp/a b"));

        let words = words(&command);
        assert_eq!(words.last().map(String::as_str), Some("/tmp/a b"));
        if cfg!(target_os = "linux") {
            assert_eq!(words[..3], ["gio", "trash", "--"]);
        }
    }

    #[test]
    fn the_home_folder_and_its_parents_are_protected() {
        let home = Path::new("/Users/ann");

        assert!(is_protected(Path::new("/"), home));
        assert!(is_protected(Path::new("/System/Volumes/Data"), home));
        assert!(is_protected(Path::new("/Users"), home));
        assert!(is_protected(
            Path::new("/System/Volumes/Data/Users/ann"),
            home
        ));
        assert!(is_protected(Path::new("/Users/bob"), home));
        assert!(is_protected(Path::new("/private"), home));
        assert!(is_protected(Path::new("/System/Library/Fonts"), home));
        assert!(!is_protected(Path::new("/Users/ann/Documents"), home));
        assert!(!is_protected(Path::new("/opt/homebrew/Cellar"), home));
    }

    #[test]
    fn a_home_reached_through_a_symlink_is_protected() {
        let base = std::env::temp_dir().join(format!("ruim-home-{}", std::process::id()));
        let real = base.join("real");
        fs::create_dir_all(real.join("work")).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let real = fs::canonicalize(&real).unwrap();

        let is_home = is_protected(&real, &base.join("link"));
        let is_work = is_protected(&real.join("work"), &base.join("link"));
        fs::remove_dir_all(&base).unwrap();

        assert!(is_home);
        assert!(!is_work);
    }

    #[test]
    fn finder_scripts_escape_quotes() {
        let script = finder_delete(Path::new("/tmp/say \"hi\"\\"));

        assert_eq!(
            script,
            r#"tell application "Finder" to delete POSIX file "/tmp/say \"hi\"\\""#
        );
    }

    #[test]
    fn a_failing_command_reports_its_error_line() {
        let output = Command::new("sh")
            .args(["-c", "echo '' >&2; echo 'no such file' >&2; exit 3"])
            .output()
            .unwrap();

        let err = finished("sh", output).unwrap_err();

        assert_eq!(err.to_string(), "no such file");
    }
}
