//! Pure filesystem helpers (std only, so they can be tested offline with `rustc --test`).
//! - safe_join: names from a foreign collection must not escape the destination folder.
//! - unique_name: never overwrite an existing file or folder on download.
//! - is_hidden_path: hidden/system files are not shared from folders by default.
//! - write_private / restrict_permissions: the node key is readable only by its owner.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// One path component from a foreign collection is acceptable on every OS.
fn component_ok(c: &str) -> bool {
    if c.is_empty() || c == "." || c == ".." {
        return false;
    }
    // ':' — drive prefix / NTFS alternate data stream; '\\' — Windows separator.
    if c.chars().any(|ch| ch == ':' || ch == '\\' || ch.is_control()) {
        return false;
    }
    if c.ends_with('.') || c.ends_with(' ') {
        return false;
    }
    let stem = c.split('.').next().unwrap_or(c).trim_end().to_ascii_uppercase();
    !WINDOWS_RESERVED.contains(&stem.as_str())
}

/// Join a '/'-separated relative name from a collection onto `base`, rejecting anything unsafe.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let err = || format!("недопустимое имя файла в раздаче: {rel}");
    if rel.is_empty() || rel.starts_with('/') {
        return Err(err());
    }
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        if !component_ok(part) {
            return Err(err());
        }
        // Second guard: the OS parser must also see exactly one normal component.
        let mut comps = Path::new(part).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(p)), None) => out.push(p),
            _ => return Err(err()),
        }
    }
    Ok(out)
}

fn taken(p: &Path) -> bool {
    // symlink_metadata: a broken symlink also counts as taken.
    std::fs::symlink_metadata(p).is_ok()
}

/// A name inside `dest` that does not exist yet: "a.txt" → "a (1).txt", folder "x" → "x (1)".
pub fn unique_name(dest: &Path, name: &str, is_dir: bool) -> String {
    if !taken(&dest.join(name)) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if !is_dir && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1u32..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|cand| !taken(&dest.join(cand)))
        .expect("u32 range exhausted")
}

/// Temporary hidden name used while exporting; renamed to `final_name` only when complete.
pub fn part_name(final_name: &str) -> String {
    format!(".{final_name}.tikdl-part")
}

/// Hidden or system file/folder name that should not leave the machine by accident.
pub fn is_hidden_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || name == "Icon\r"
        || lower == "$recycle.bin"
}

/// True if any component of a relative path is hidden (".git/config", "a/.env").
pub fn is_hidden_path(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        Component::Normal(s) => is_hidden_name(&s.to_string_lossy()),
        _ => false,
    })
}

/// Make an existing file owner-only (unix). No-op elsewhere: per-user app data on Windows is ACL-protected.
pub fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)?;
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Atomically write a secret: temp file created owner-only, fsync, rename over the target.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    restrict_permissions(path)
}

#[cfg(unix)]
type RawFileId = (u64, u64); // (device, inode)
#[cfg(not(unix))]
type RawFileId = String; // canonical path in lower case: Windows resolves links and junctions and keeps the real case

/// Identity of an existing file or folder. Two paths to the same object compare equal even when they
/// differ by a symlink, by letter case (case-insensitive disks) or by a macOS firmlink
/// (/System/Volumes/Data/Users/... is /Users/...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileId(RawFileId);

pub fn file_id(p: &Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(p).ok()?;
        Some(FileId((m.dev(), m.ino())))
    }
    #[cfg(not(unix))]
    {
        let c = std::fs::canonicalize(p).ok()?;
        Some(FileId(c.to_string_lossy().to_lowercase()))
    }
}

/// System folders: neither the folder nor anything inside it is shared. Unix layout only (on Windows these
/// names would resolve against the current drive and hit ordinary folders like C:\\dev).
const SYSTEM_TREES: &[&str] = &[
    "/System", "/Library", "/usr", "/bin", "/sbin", "/etc", "/private/etc", "/dev", "/proc", "/sys", "/boot",
    "/lib", "/lib32", "/lib64", "/libx32", "/cores",
    // system data under /var (logs, package databases, root's home, spools, audit)
    "/private/var/log", "/private/var/db", "/private/var/root", "/private/var/spool", "/private/var/audit",
    "/private/var/run", "/private/var/vm", "/private/var/at", "/private/var/mail", "/private/var/backups",
    "/var/log", "/var/lib", "/var/db", "/var/root", "/var/spool", "/var/audit", "/var/run", "/var/backups",
    "/var/cache", "/var/mail", "/root",
];
/// Containers: the folder itself is not shared (all apps, all disks, all temp files), its subfolders are
/// (an app bundle, an external disk, /var/www on a server).
const SYSTEM_CONTAINERS: &[&str] = &[
    "/Applications", "/Users", "/home", "/Volumes", "/media", "/mnt", "/run", "/run/media", "/opt", "/srv",
    "/var", "/private/var", "/tmp", "/private/tmp", "/private", "/Network", "/var/tmp", "/private/var/tmp",
];

/// Root of the disk the OS runs from: "/" or "C:\\".
fn system_disk() -> PathBuf {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        PathBuf::from(format!("{drive}\\"))
    } else {
        PathBuf::from("/")
    }
}

/// Windows system folders, wherever the OS is installed.
fn windows_system_trees() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"]
        .iter()
        .filter_map(|k| std::env::var_os(k))
        .map(PathBuf::from)
        .collect()
}

/// Home folders the rules protect: the current user's and, on Unix, every user's under /Users and /home — so
/// running as root, under sudo or with HOME pointed elsewhere does not open someone's (or one's own) home.
fn homes(home: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = home.map(Path::to_path_buf).into_iter().collect();
    if cfg!(unix) {
        for base in ["/Users", "/home"] {
            for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
                let name = e.file_name();
                if name != "Shared" && e.path().is_dir() && !v.iter().any(|h| h == &e.path()) {
                    v.push(e.path());
                }
            }
        }
    }
    v
}

/// An entry right inside a home folder that holds keys and settings of programs.
fn settings_entry(name: &str, cloud_next: bool) -> bool {
    name.starts_with('.')
        || (cfg!(target_os = "macos") && name.eq_ignore_ascii_case("Library") && !cloud_next)
        || (cfg!(windows) && name.eq_ignore_ascii_case("AppData"))
}

const CLOUD: [&str; 2] = ["Mobile Documents", "CloudStorage"];

/// Identities of the places in home folders that are never shared, wherever a link to them comes from:
/// hidden entries (resolved, so ~/.ssh → ~/Dropbox/ssh protects the Dropbox folder too) and TikDL's folder.
/// The walk of a shared folder prunes these too.
pub fn forbidden_ids(home: Option<&Path>, app_data: &Path) -> Vec<FileId> {
    let mut v: Vec<FileId> = file_id(app_data).into_iter().collect();
    for h in homes(home) {
        for e in std::fs::read_dir(&h).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                v.extend(file_id(&e.path()));
            }
        }
    }
    v
}

const UNCHECKABLE: &str = "не удалось проверить, что это за папка — TikDL её не раздаёт";

/// Why TikDL never shares `path`, or None if it may be shared. Never shared: the system disk, system
/// folders, a whole home folder and folders that contain it, hidden files and folders in a home
/// folder (keys and settings of programs: .ssh, .aws, .config…, also when they are links elsewhere), ~/Library on
/// macOS (except iCloud Drive and cloud folders) and AppData on Windows, and TikDL's own folder (node key).
/// Folders are compared by identity (see FileId), not by the text of the path. Fails closed: a path that exists but
/// cannot be checked (macOS /.resolve/…, /.vol/…) is refused.
pub fn forbidden_share(path: &Path, home: Option<&Path>, app_data: &Path) -> Option<&'static str> {
    // Symlinks resolved first: the name rules below look at the real folder names.
    let real = match std::fs::canonicalize(path) {
        Ok(r) => r,
        Err(_) if std::fs::symlink_metadata(path).is_ok() => return Some(UNCHECKABLE),
        Err(_) => return None, // does not exist: sharing fails with its own error
    };
    let chain: Vec<(&Path, Option<FileId>)> = real.ancestors().map(|a| (a, file_id(a))).collect();
    let Some(me) = chain[0].1.clone() else { return Some(UNCHECKABLE) };
    // the object the path opens must be the one the resolved name names (/.nofollow/ opens "/")
    if file_id(path).as_ref() != Some(&me) {
        return Some(UNCHECKABLE);
    }
    let is = |target: &Path| file_id(target).as_ref() == Some(&me);
    let inside_id = |t: &FileId| chain.iter().any(|(_, id)| id.as_ref() == Some(t));
    let inside = |target: &Path| file_id(target).is_some_and(|t| inside_id(&t));
    let file = std::fs::metadata(&real).is_ok_and(|m| m.is_file());

    if inside(app_data) {
        return Some(if file {
            "это файл TikDL (ключ узла или хранилище) — его раздавать нельзя"
        } else {
            "это папка TikDL с ключом узла — её раздавать нельзя"
        });
    }
    if is(system_disk().as_path()) {
        return Some("это весь системный диск — TikDL его не раздаёт");
    }
    let lexical = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    for h in homes(home) {
        if is(&h) {
            return Some("это вся домашняя папка — раздайте нужную папку внутри неё");
        }
        if let Ok(h_real) = std::fs::canonicalize(&h) {
            if h_real.ancestors().skip(1).any(|a| file_id(a).as_ref() == Some(&me)) {
                return Some("в этой папке лежит домашняя папка — TikDL её не раздаёт");
            }
        }
        // by the resolved chain (real folder names) …
        let h_id = file_id(&h);
        if let Some(k) = chain.iter().position(|(_, id)| id.is_some() && *id == h_id) {
            // chain[k] is home, chain[k - 1] is the entry right inside it, chain[k - 2] one level deeper.
            let name = |i: usize| chain[i].0.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if k >= 1 {
                let cloud = k >= 2 && CLOUD.iter().any(|c| name(k - 2).eq_ignore_ascii_case(c));
                if settings_entry(&name(k - 1), cloud) {
                    return Some("здесь хранятся ключи и настройки программ — TikDL это не раздаёт");
                }
            }
        }
        // … and by the path as given (a hidden entry that is itself a link: ~/.ssh → ~/Dropbox/ssh)
        if let Ok(rest) = lexical.strip_prefix(&h) {
            let mut parts = rest.components().map(|c| c.as_os_str().to_string_lossy().to_string());
            if let Some(top) = parts.next() {
                let cloud = parts.next().is_some_and(|n| CLOUD.iter().any(|c| n.eq_ignore_ascii_case(c)));
                if settings_entry(&top, cloud) {
                    return Some("здесь хранятся ключи и настройки программ — TikDL это не раздаёт");
                }
            }
        }
    }
    // the target of a hidden entry's link, wherever it lives
    if forbidden_ids(home, app_data).iter().any(inside_id) {
        return Some("здесь хранятся ключи и настройки программ — TikDL это не раздаёт");
    }
    let unix = !cfg!(windows);
    let trees = SYSTEM_TREES.iter().filter(|_| unix).map(PathBuf::from).chain(windows_system_trees());
    let containers = SYSTEM_CONTAINERS.iter().filter(|_| unix);
    if trees.into_iter().any(|t| inside(t.as_path())) || containers.into_iter().any(|c| is(Path::new(c))) {
        return Some(if file {
            "это файл из системной папки — TikDL его не раздаёт"
        } else {
            "это системная папка — TikDL её не раздаёт"
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake home with the usual dangerous and normal places; TikDL's folder inside it as on macOS.
    #[test]
    fn forbidden_share_home_rules() {
        let root = tmpdir("forbid");
        let home = root.join("home");
        let data = home.join("Library/Application Support/tikdl");
        for d in [
            data.join("store"),
            home.join(".ssh"),
            home.join(".config/gh"),
            home.join("Documents/trip"),
            home.join("Library/Mobile Documents/com~apple~CloudDocs/Photos"),
            home.join("Library/CloudStorage/Drive"),
            home.join("AppData/Roaming"),
            root.join("other"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(home.join(".netrc"), b"x").unwrap();
        std::fs::write(home.join("notes.txt"), b"x").unwrap();
        let f = |p: &Path| forbidden_share(p, Some(&home), &data);

        // TikDL's own folder and everything in it.
        assert!(f(&data).is_some());
        assert!(f(&data.join("store")).is_some());
        // The whole home and any folder that contains it.
        assert!(f(&home).is_some());
        assert!(f(&root).is_some());
        // Hidden files and folders right in home: keys and settings of programs.
        assert!(f(&home.join(".ssh")).is_some());
        assert!(f(&home.join(".config/gh")).is_some());
        assert!(f(&home.join(".netrc")).is_some());
        // Normal places are shared.
        assert_eq!(f(&home.join("Documents/trip")), None);
        assert_eq!(f(&home.join("notes.txt")), None);
        assert_eq!(f(&root.join("other")), None);
        // ~/Library is closed on macOS, except iCloud Drive and cloud folders; AppData only on Windows.
        assert_eq!(f(&home.join("Library")).is_some(), cfg!(target_os = "macos"));
        assert_eq!(f(&home.join("Library/Mobile Documents/com~apple~CloudDocs/Photos")), None);
        assert_eq!(f(&home.join("Library/CloudStorage/Drive")), None);
        assert_eq!(f(&home.join("AppData/Roaming")).is_some(), cfg!(windows));
        // A path that does not exist is not judged here: sharing fails with its own error.
        assert_eq!(f(&home.join("missing")), None);

        // A symlink does not hide where it points.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(home.join(".ssh"), root.join("other/keys")).unwrap();
            std::os::unix::fs::symlink(&home, root.join("other/me")).unwrap();
            assert!(f(&root.join("other/keys")).is_some());
            assert!(f(&root.join("other/me")).is_some());
            assert!(f(&root.join("other/me/.ssh")).is_some());
        }
        // A hidden entry that is itself a link elsewhere (stow, Dropbox, iCloud): judged by its own name, and its
        // target is protected too.
        #[cfg(unix)]
        {
            std::fs::create_dir_all(home.join("Dropbox/ssh")).unwrap();
            std::fs::write(home.join("Dropbox/ssh/id_ed25519"), b"k").unwrap();
            std::os::unix::fs::symlink(home.join("Dropbox/ssh"), home.join(".ssh2")).unwrap();
            assert!(f(&home.join(".ssh2")).is_some());
            assert!(f(&home.join("Dropbox/ssh")).is_some());
            assert!(f(&home.join("Dropbox/ssh/id_ed25519")).is_some());
            assert!(forbidden_ids(Some(&home), &data).contains(&file_id(&home.join("Dropbox/ssh")).unwrap()));
        }
        // Another letter case on a case-insensitive disk (the default on macOS and Windows).
        let variant = root.join("HOME");
        if variant.join(".SSH").exists() {
            assert!(f(&variant).is_some());
            assert!(f(&variant.join(".SSH")).is_some());
            assert_eq!(f(&variant.join("LIBRARY")).is_some(), cfg!(target_os = "macos"));
            assert!(f(&variant.join("LIBRARY/APPLICATION SUPPORT/TIKDL")).is_some());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Real system folders of the machine the tests run on.
    #[test]
    fn forbidden_share_system_rules() {
        let data = Path::new("/nonexistent-tikdl-data");
        let f = |p: &str| forbidden_share(Path::new(p), None, data);
        if cfg!(unix) {
            assert!(f("/").is_some());
            assert!(f("/usr").is_some());
            assert!(f("/usr/bin").is_some());
            assert!(f("/etc").is_some());
            assert!(f("/tmp").is_some());
            for p in ["/var/log", "/private/var/db", "/var/db/receipts"] {
                if Path::new(p).exists() {
                    assert!(f(p).is_some(), "{p} must be forbidden");
                }
            }
            // fails closed: macOS path aliases that canonicalize cannot resolve
            if Path::new("/.resolve/1/usr").exists() {
                assert!(f("/.resolve/1/usr").is_some());
                assert!(f("/.resolve/1/tmp").is_some());
            }
            if Path::new("/.nofollow/usr").exists() {
                assert!(f("/.nofollow/").is_some(), "/.nofollow/ opens the system disk");
            }
            // Inside a container folder sharing is allowed: the test's own temp folder lives there.
            let t = tmpdir("sys");
            assert_eq!(forbidden_share(&t, None, data), None);
            let _ = std::fs::remove_dir_all(&t);
        }
        for p in ["/Applications", "/Library", "/System/Volumes/Data", "/Volumes"] {
            if Path::new(p).exists() {
                assert!(f(p).is_some(), "{p} must be forbidden");
            }
        }
        // Inside /Applications an app bundle may be shared (a real one: Safari.app is a link into /System).
        let app = std::fs::read_dir("/Applications").into_iter().flatten().flatten().map(|e| e.path()).find(|p| {
            p.extension().is_some_and(|x| x == "app")
                && std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
        });
        if let Some(app) = app {
            assert_eq!(forbidden_share(&app, None, data), None, "{} must be shareable", app.display());
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tikdl-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn safe_join_rejects_bad_names() {
        let b = Path::new("/base");
        for bad in [
            "", "../x", "a/../b", "/etc/x", "C:x", "a\\b", "..\\..\\x", "f:ads", "dir/file:ads",
            "CON", "nul.txt", "a/Com1.log", "x.", "x ", "a//b", "./a", "a/\u{0007}",
        ] {
            assert!(safe_join(b, bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn safe_join_accepts_normal_names() {
        let b = Path::new("/base");
        assert_eq!(safe_join(b, "a.txt").unwrap(), Path::new("/base/a.txt"));
        assert_eq!(safe_join(b, "Фото/2026/x.jpg").unwrap(), Path::new("/base/Фото/2026/x.jpg"));
        assert_eq!(safe_join(b, "console.txt").unwrap(), Path::new("/base/console.txt"));
        assert_eq!(safe_join(b, "a..b").unwrap(), Path::new("/base/a..b"));
    }

    #[test]
    fn unique_name_never_overwrites() {
        let d = tmpdir("uniq");
        assert_eq!(unique_name(&d, "a.txt", false), "a.txt");
        std::fs::write(d.join("a.txt"), b"old").unwrap();
        assert_eq!(unique_name(&d, "a.txt", false), "a (1).txt");
        std::fs::write(d.join("a (1).txt"), b"old").unwrap();
        assert_eq!(unique_name(&d, "a.txt", false), "a (2).txt");
        std::fs::create_dir(d.join("my.folder")).unwrap();
        assert_eq!(unique_name(&d, "my.folder", true), "my.folder (1)");
        assert_eq!(unique_name(&d, "noext", false), "noext");
        std::fs::write(d.join(".env"), b"x").unwrap();
        assert_eq!(unique_name(&d, ".env", false), ".env (1)");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("missing"), d.join("broken")).unwrap();
            assert_eq!(unique_name(&d, "broken", false), "broken (1)");
        }
        assert_eq!(std::fs::read(d.join("a.txt")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hidden_files_detected() {
        for h in [".git/config", ".env", "a/.DS_Store", "x/Thumbs.db", "desktop.ini", "a/.ssh/id_rsa"] {
            assert!(is_hidden_path(Path::new(h)), "{h} must be hidden");
        }
        for v in ["a.txt", "docs/readme.md", "photo.2026.jpg"] {
            assert!(!is_hidden_path(Path::new(v)), "{v} must be shared");
        }
    }

    #[cfg(unix)]
    #[test]
    fn key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("key");
        let p = d.join("node.key");
        write_private(&p, &[7u8; 32]).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&p).unwrap(), vec![7u8; 32]);
        // An old key written with default 0644 gets tightened on load.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        restrict_permissions(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // Overwrite works (atomic rename over existing file).
        write_private(&p, &[9u8; 32]).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![9u8; 32]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
