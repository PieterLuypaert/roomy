use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, DirEntry, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use rayon::prelude::*;

use crate::tree::{Kind, Node};

const SHARDS: usize = 64;
const STACK_SIZE: usize = 16 << 20;
const PATH_EVERY: u64 = 32;
// More threads than this mostly wait on filesystem locks and burn system time.
const MAX_THREADS: usize = 8;

#[derive(Default)]
pub struct Progress {
    items: AtomicU64,
    bytes: AtomicU64,
    dirs: AtomicU64,
    is_cancelled: AtomicBool,
    current: Mutex<PathBuf>,
}

impl Progress {
    pub fn items(&self) -> u64 {
        self.items.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn current(&self) -> PathBuf {
        self.current
            .lock()
            .map(|path| path.clone())
            .unwrap_or_default()
    }

    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Relaxed)
    }

    fn entered(&self, path: &Path) {
        if !self
            .dirs
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(PATH_EVERY)
        {
            return;
        }
        if let Ok(mut current) = self.current.try_lock() {
            path.clone_into(&mut current);
        }
    }

    fn counted(&self, items: u64, bytes: u64) {
        self.items.fetch_add(items, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub fn scan(root: &Path, progress: &Progress) -> io::Result<Node> {
    let meta = fs::metadata(root)?;
    let walker = Walker {
        dev: meta.dev(),
        progress,
        links: InodeSet::new(),
        dirs: InodeSet::new(),
    };
    if !meta.is_dir() {
        return Ok(walker.leaf(root.as_os_str(), &meta));
    }
    walker.dirs.first_seen(meta.ino());
    let threads = thread::available_parallelism().map_or(MAX_THREADS, |n| n.get().min(MAX_THREADS));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(STACK_SIZE)
        .build()
        .map_err(io::Error::other)?;
    Ok(pool.install(|| walker.dir(root, root.as_os_str(), &meta)))
}

struct Walker<'a> {
    dev: u64,
    progress: &'a Progress,
    links: InodeSet,
    dirs: InodeSet,
}

struct Subdir {
    path: PathBuf,
    name: OsString,
    meta: Metadata,
}

impl Walker<'_> {
    fn dir(&self, path: &Path, name: &OsStr, meta: &Metadata) -> Node {
        let mut node = Node::dir(name, allocated(meta));
        if self.progress.is_cancelled() {
            return node;
        }
        let subdirs = match self.read(path, &mut node) {
            Ok(subdirs) => subdirs,
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                node.is_denied = true;
                return node;
            }
            Err(_) => {
                node.is_unreadable = true;
                return node;
            }
        };
        let scanned: Vec<Node> = subdirs
            .into_par_iter()
            .map(|sub| self.dir(&sub.path, &sub.name, &sub.meta))
            .collect();
        node.children.extend(scanned);
        node.finish();
        node
    }

    fn read(&self, path: &Path, node: &mut Node) -> io::Result<Vec<Subdir>> {
        let entries = fs::read_dir(path)?;
        self.progress.entered(path);
        let mut subdirs = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => self.add(entry, node, &mut subdirs),
                Err(_) => node.is_unreadable = true,
            }
        }
        let bytes = node.children.iter().map(|child| child.size).sum();
        self.progress
            .counted((node.children.len() + subdirs.len()) as u64, bytes);
        Ok(subdirs)
    }

    fn add(&self, entry: DirEntry, node: &mut Node, subdirs: &mut Vec<Subdir>) {
        let name = entry.file_name();
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(err) => return node.children.push(blocked(&entry, &err)),
        };
        if !meta.is_dir() {
            node.children.push(self.leaf(&name, &meta));
        } else if meta.dev() != self.dev {
            node.children.push(mount(&name, &meta));
        } else if !self.dirs.first_seen(meta.ino()) {
            node.children.push(Node::leaf(&name, Kind::Link, 0));
        } else {
            let path = entry.path();
            subdirs.push(Subdir { path, name, meta });
        }
    }

    fn leaf(&self, name: &OsStr, meta: &Metadata) -> Node {
        let kind = match meta.file_type() {
            t if t.is_file() => Kind::File,
            t if t.is_symlink() => Kind::Link,
            _ => Kind::Other,
        };
        let is_extra_link = meta.nlink() > 1 && !self.links.first_seen(meta.ino());
        let size = if is_extra_link { 0 } else { allocated(meta) };
        Node::leaf(name, kind, size)
    }
}

fn blocked(entry: &DirEntry, err: &io::Error) -> Node {
    let kind = match entry.file_type() {
        Ok(t) if t.is_dir() => Kind::Dir,
        Ok(t) if t.is_symlink() => Kind::Link,
        Ok(t) if t.is_file() => Kind::File,
        _ => Kind::Other,
    };
    let mut node = Node::leaf(entry.file_name(), kind, 0);
    if err.kind() == io::ErrorKind::PermissionDenied {
        node.is_denied = true;
    } else {
        node.is_unreadable = true;
    }
    node
}

fn mount(name: &OsStr, meta: &Metadata) -> Node {
    let mut node = Node::dir(name, allocated(meta));
    node.is_mount = true;
    node
}

fn allocated(meta: &Metadata) -> u64 {
    meta.blocks() * 512
}

struct InodeSet {
    shards: Vec<Mutex<HashSet<u64>>>,
}

impl InodeSet {
    fn new() -> Self {
        let shards = (0..SHARDS).map(|_| Mutex::default()).collect();
        InodeSet { shards }
    }

    fn first_seen(&self, inode: u64) -> bool {
        let shard = &self.shards[(inode % SHARDS as u64) as usize];
        shard
            .lock()
            .map(|mut seen| seen.insert(inode))
            .unwrap_or(true)
    }
}
