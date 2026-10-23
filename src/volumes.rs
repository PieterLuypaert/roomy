use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DATA_ROOT: &str = "/System/Volumes/Data";

const IMPORTANT_CAPACITY: &str = r#"ObjC.import("Foundation");
var value = Ref();
$.NSURL.fileURLWithPath("/").getResourceValueForKeyError(value, "NSURLVolumeAvailableCapacityForImportantUsageKey", null);
String(ObjC.unwrap(value[0]))"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub total: u64,
    pub free: u64,
    pub purgeable: Option<u64>,
    pub system: u64,
    pub data: u64,
    pub scan_root: PathBuf,
    pub is_apfs: bool,
}

impl Disk {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    pub fn other(&self) -> u64 {
        self.used().saturating_sub(self.system + self.data)
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .or_else(account_home)
        .unwrap_or_default()
}

fn account_home() -> Option<PathBuf> {
    let account = unsafe { libc::getpwuid(libc::getuid()) };
    if account.is_null() || unsafe { (*account).pw_dir }.is_null() {
        return None;
    }
    let dir = unsafe { CStr::from_ptr((*account).pw_dir) };
    Some(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
}

pub fn boot_disk() -> io::Result<Disk> {
    let root = space(Path::new("/"))?;
    let data_root = Path::new(DATA_ROOT);
    if !cfg!(target_os = "macos") || !data_root.is_dir() {
        return Ok(Disk {
            name: "/".to_string(),
            total: root.total,
            free: root.free,
            purgeable: None,
            system: 0,
            data: root.used,
            scan_root: PathBuf::from("/"),
            is_apfs: false,
        });
    }
    let data = space(data_root)?;
    Ok(Disk {
        name: volume_name().unwrap_or_else(|| "This Mac".to_string()),
        total: root.total,
        free: root.free,
        purgeable: None,
        system: root.used,
        data: data.used,
        scan_root: data_root.to_path_buf(),
        is_apfs: true,
    })
}

pub fn purgeable(free: u64) -> Option<u64> {
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", IMPORTANT_CAPACITY])
        .output()
        .ok()?;
    let important: u64 = String::from_utf8(output.stdout).ok()?.trim().parse().ok()?;
    important.checked_sub(free).filter(|&extra| extra > 0)
}

fn volume_name() -> Option<String> {
    fs::read_dir("/Volumes").ok()?.flatten().find_map(|entry| {
        let target = fs::read_link(entry.path()).ok()?;
        (target == Path::new("/")).then(|| entry.file_name().to_string_lossy().into_owned())
    })
}

pub fn usable(path: &Path) -> PathBuf {
    let logical = logical(path);
    if logical != path && logical.exists() {
        logical.to_path_buf()
    } else {
        path.to_path_buf()
    }
}

pub fn logical(path: &Path) -> &Path {
    match path.strip_prefix(DATA_ROOT) {
        Ok(rest) if rest.as_os_str().is_empty() => Path::new("/"),
        Ok(_) => Path::new(OsStr::from_bytes(
            &path.as_os_str().as_bytes()[DATA_ROOT.len()..],
        )),
        Err(_) => path,
    }
}

#[cfg(target_os = "macos")]
pub fn space(path: &Path) -> io::Result<Space> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = u64::from(stats.f_bsize);
    Ok(Space {
        total: stats.f_blocks * block,
        used: volume_used(&path)?,
        free: stats.f_bavail * block,
    })
}

#[cfg(target_os = "macos")]
#[repr(C, packed(4))]
struct UsedReply {
    length: u32,
    used: libc::off_t,
}

// APFS volumes share one container, so statfs reports the container's free blocks for
// every volume; the space each volume uses only comes from its volume attributes.
#[cfg(target_os = "macos")]
fn volume_used(path: &CString) -> io::Result<u64> {
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_SPACEUSED,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut reply = UsedReply { length: 0, used: 0 };
    let status = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            (&raw mut request).cast(),
            (&raw mut reply).cast(),
            std::mem::size_of::<UsedReply>(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    let used = reply.used;
    u64::try_from(used).map_err(io::Error::other)
}

#[cfg(not(target_os = "macos"))]
// The statvfs field types are u64 on 64-bit Linux but narrower on other targets.
#[allow(clippy::useless_conversion)]
pub fn space(path: &Path) -> io::Result<Space> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = u64::from(stats.f_frsize);
    let (blocks, free, available) = (
        u64::from(stats.f_blocks),
        u64::from(stats.f_bfree),
        u64::from(stats.f_bavail),
    );
    Ok(Space {
        total: blocks * block,
        used: (blocks - free) * block,
        free: available * block,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_paths_drop_the_data_volume_prefix() {
        assert_eq!(logical(Path::new("/System/Volumes/Data")), Path::new("/"));
        assert_eq!(
            logical(Path::new("/System/Volumes/Data/Users/ann")),
            Path::new("/Users/ann")
        );
        assert_eq!(logical(Path::new("/Users/ann")), Path::new("/Users/ann"));
        assert_eq!(
            logical(Path::new("/System/Volumes/DataX")),
            Path::new("/System/Volumes/DataX")
        );
    }

    #[test]
    fn the_home_folder_is_absolute() {
        assert!(home().is_absolute());
    }

    #[test]
    fn the_root_filesystem_reports_its_space() {
        let space = space(Path::new("/")).unwrap();

        assert!(space.total > 0);
        assert!(space.used <= space.total);
        assert!(space.free <= space.total);
    }

    #[test]
    fn other_volumes_take_what_system_and_data_leave() {
        let disk = Disk {
            name: "Disk".to_string(),
            total: 500,
            free: 200,
            purgeable: None,
            system: 20,
            data: 250,
            scan_root: PathBuf::from("/"),
            is_apfs: true,
        };

        assert_eq!(disk.used(), 300);
        assert_eq!(disk.other(), 30);
    }
}
