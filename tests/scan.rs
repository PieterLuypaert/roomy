use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use roomy::scan::{Progress, scan};
use roomy::tree::{Kind, Node};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("roomy-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Scratch(fs::canonicalize(&path).unwrap())
    }

    fn file(&self, relative: &str, bytes: usize) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![1u8; bytes]).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let locked = self.0.join("locked");
        if locked.exists() {
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
    node.children
        .iter()
        .find(|c| c.label() == name)
        .unwrap_or_else(|| panic!("no {name} in {}", node.label()))
}

fn scanned(path: &Path) -> (Node, Progress) {
    let progress = Progress::default();
    let root = scan(path, &progress).unwrap();
    (root, progress)
}

#[test]
fn sizes_add_up_and_the_biggest_comes_first() {
    let dir = Scratch::new("sizes");
    dir.file("big.bin", 200_000);
    dir.file("small.txt", 1_000);
    dir.file("sub/inner.bin", 50_000);

    let (root, progress) = scanned(&dir.0);

    let names: Vec<_> = root
        .children
        .iter()
        .map(|c| c.label().into_owned())
        .collect();
    assert_eq!(names, ["big.bin", "sub", "small.txt"]);
    assert!(child(&root, "big.bin").size >= 200_000);
    assert_eq!(child(&root, "sub").items, 1);
    let children: u64 = root.children.iter().map(|c| c.size).sum();
    assert!(root.size >= children);
    assert_eq!(root.items, 4);
    assert_eq!(progress.items(), 4);
}

#[test]
fn hard_links_count_once() {
    let dir = Scratch::new("links");
    let original = dir.file("a.bin", 100_000);
    fs::hard_link(&original, dir.0.join("b.bin")).unwrap();

    let (root, _) = scanned(&dir.0);

    let mut sizes = [child(&root, "a.bin").size, child(&root, "b.bin").size];
    sizes.sort();
    assert_eq!(sizes[0], 0);
    assert!(sizes[1] >= 100_000);
}

#[test]
fn symlinks_are_not_followed() {
    let dir = Scratch::new("symlinks");
    dir.file("data/movie.bin", 200_000);
    std::os::unix::fs::symlink(dir.0.join("data"), dir.0.join("shortcut")).unwrap();

    let (root, _) = scanned(&dir.0);

    let shortcut = child(&root, "shortcut");
    assert_eq!(shortcut.kind, Kind::Link);
    assert!(shortcut.children.is_empty());
    assert!(root.size < 400_000);
}

#[test]
fn unreadable_folders_are_flagged() {
    let dir = Scratch::new("denied");
    dir.file("locked/secret.bin", 10_000);
    let locked = dir.0.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        return;
    }

    let (root, _) = scanned(&dir.0);

    let node = child(&root, "locked");
    assert!(node.is_denied);
    assert!(!node.is_unreadable);
    assert!(node.children.is_empty());
    assert_eq!(root.count_where(|n| n.is_denied), 1);
}

#[test]
fn a_file_scans_as_a_single_entry() {
    let dir = Scratch::new("file");
    let path = dir.file("only.bin", 5_000);

    let (root, _) = scanned(&path);

    assert_eq!(root.kind, Kind::File);
    assert!(root.size >= 5_000);
}

#[test]
fn a_cancelled_scan_stops_early() {
    let dir = Scratch::new("cancel");
    dir.file("sub/deep.bin", 5_000);
    let progress = Progress::default();
    progress.cancel();

    let root = scan(&dir.0, &progress).unwrap();

    assert!(root.children.is_empty());
}
