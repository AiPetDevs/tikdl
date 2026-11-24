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

#[cfg(test)]
mod tests {
    use super::*;

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
