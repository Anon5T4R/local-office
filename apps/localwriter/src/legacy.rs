//! What the rename from Local Word to Local Writer left behind.
//!
//! The product was called Local Word (the code, LocalOffice), and its per-user folders and
//! environment variables with it. This module does two things, both quietly and both safe to run on every start:
//!
//! * [`migrate_all`] moves each old-named folder (preferences, the AI connections, the updater's
//!   data and cache, the portable data folder) to its new name, once. It never overwrites: when the
//!   new folder exists the old one is left alone, and when the move has to be a copy (another
//!   volume, a file still open) the old folder stays as it was. Every move is logged. The API keys
//!   in the OS keychain are copied to the new service name too.
//! * [`env_var_os`] reads `LOCALWRITER_<NAME>` and falls back to the old `LOCALOFFICE_<NAME>`.
//!
//! The documents are not touched, and neither is the file format. The Windows file associations
//! are the installer's business (the MSI removes the old ProgIds and registers the new ones).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `(new, old)` for each folder-name convention: the app folder on Windows and macOS, the XDG
/// folder on Linux, the updater's per-user folder (the display name on Windows and macOS) and the
/// portable data folder beside the executable.
const NAMES: [(&str, &str); 4] =
    [("LocalWriter", "LocalOffice"), ("localwriter", "localoffice"), ("Local Writer", "Local Word"), ("LocalWriterData", "LocalWordData")];

/// How deep [`copy_tree`] follows folders: settings are a few levels deep at most.
const MAX_DEPTH: u32 = 16;

/// What [`migrate_dir`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// There is no old folder, or the new one exists already: nothing to do.
    Nothing,
    /// The old folder was renamed to the new one.
    Moved,
    /// The contents were copied; the old folder is still there.
    Copied,
    /// Neither worked; the old folder is untouched and the next start tries again.
    Failed(String),
}

/// An environment variable of this app: `LOCALWRITER_<name>`, else the old `LOCALOFFICE_<name>`.
pub fn env_var_os(name: &str) -> Option<OsString> {
    std::env::var_os(format!("LOCALWRITER_{name}")).or_else(|| std::env::var_os(format!("LOCALOFFICE_{name}")))
}

/// [`env_var_os`] as text; `None` when unset or not Unicode.
pub fn env_var(name: &str) -> Option<String> {
    env_var_os(name).and_then(|value| value.into_string().ok())
}

/// The folder the old name used in place of `new` (the last component swapped), when `new` carries
/// one of the app's folder names.
pub fn old_dir_of(new: &Path) -> Option<PathBuf> {
    let name = new.file_name()?.to_str()?;
    let (_, old) = NAMES.iter().find(|(current, _)| *current == name)?;
    Some(new.with_file_name(old))
}

/// [`migrate_dir`] from the folder the old name used in place of `new`, when there is one.
pub fn migrate_from_old_name(new: &Path) -> Outcome {
    old_dir_of(new).map_or(Outcome::Nothing, |old| migrate_dir(&old, new))
}

/// Move `old` to `new`, once: only when `old` is a folder and `new` doesn't exist. A rename on one
/// volume; otherwise the contents are copied to a staging folder beside `new` that is renamed into
/// place when complete, so `new` never exists half-copied. Nothing at `new` is ever replaced.
pub fn migrate_dir(old: &Path, new: &Path) -> Outcome {
    if !old.is_dir() || std::fs::symlink_metadata(new).is_ok() {
        return Outcome::Nothing;
    }
    if let Some(parent) = new.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        log::warn!("couldn't move {} to {}: {e}", old.display(), new.display());
        return Outcome::Failed(e.to_string());
    }
    if std::fs::rename(old, new).is_ok() {
        log::info!("moved the Local Writer folder {} to {}", old.display(), new.display());
        return Outcome::Moved;
    }
    let name = new.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stage = new.with_file_name(format!(".{name}.migrating-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let copied = copy_tree(old, &stage, 0).and_then(|()| {
        if std::fs::symlink_metadata(new).is_ok() {
            Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the new folder appeared meanwhile"))
        } else {
            std::fs::rename(&stage, new)
        }
    });
    match copied {
        Ok(()) => {
            log::info!("copied the Local Writer folder {} to {} (the old one stays)", old.display(), new.display());
            Outcome::Copied
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                return Outcome::Nothing;
            }
            log::warn!("couldn't copy {} to {}: {e}; the old settings stay where they are", old.display(), new.display());
            Outcome::Failed(e.to_string())
        }
    }
}

/// Copy the folder `from` to the new folder `to`: plain files and folders only (links are skipped).
fn copy_tree(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(std::io::Error::other(format!("{} is nested too deep", from.display())));
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, depth.saturating_add(1))?;
        } else if kind.is_file() {
            let mut src = std::fs::File::open(entry.path())?;
            let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
            std::io::copy(&mut src, &mut dst)?;
            dst.sync_all()?;
        }
    }
    Ok(())
}

/// The folders this version reads and writes, which an old-named twin may sit beside.
fn current_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // Preferences, logs, the user's fonts, templates, dictionaries and control key files.
    if let Some(dir) = localwriter_control_key::settings_dir() {
        dirs.push(dir);
    }
    // The AI connections.
    if let Some(dir) = localwriter_ai::data_dir() {
        dirs.push(dir);
    }
    // The updater's records, logs and downloads (the portable root when `portable.txt` is there).
    if let Ok(paths) = craft_update::Paths::for_app(&craft_update::suite::LOCALWRITER) {
        dirs.push(paths.data);
        dirs.push(paths.cache);
    }
    // The portable data folder beside the executable.
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("LocalWriterData"));
    }
    dirs
}

/// Move every old-named folder to its new name, then copy the keychain keys. Called first thing
/// at start; never fails (what can't be moved is logged and tried again next time).
pub fn migrate_all() {
    for new in current_dirs() {
        migrate_from_old_name(&new);
    }
    // The connections file moved with its folder; the keys live in the keychain, under the old
    // service name.
    localwriter_ai::migrate_legacy_keys();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("localwriter-legacy-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }

    fn old_folder(root: &Path, name: &str) -> PathBuf {
        let old = root.join(name);
        std::fs::create_dir_all(old.join("logs")).expect("old folder");
        std::fs::write(old.join("prefs.json"), b"{\"theme\":\"dark\"}").expect("prefs");
        std::fs::write(old.join("logs").join("update.log"), b"log").expect("log");
        old
    }

    #[test]
    fn the_old_name_of_each_folder_convention() {
        for (new, old) in NAMES {
            assert_eq!(old_dir_of(Path::new("/a").join(new).as_path()), Some(PathBuf::from("/a").join(old)));
        }
        assert_eq!(old_dir_of(Path::new("/a/Other")), None);
    }

    #[test]
    fn an_old_folder_is_moved_to_the_new_name_with_its_contents() {
        let root = temp("moved");
        let old = old_folder(&root, "LocalOffice");
        let new = root.join("LocalWriter");
        assert_eq!(migrate_from_old_name(&new), Outcome::Moved);
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("prefs.json")).expect("prefs"), b"{\"theme\":\"dark\"}");
        assert_eq!(std::fs::read(new.join("logs/update.log")).expect("log"), b"log");
        // One time: a second start finds the new folder and does nothing.
        assert_eq!(migrate_from_old_name(&new), Outcome::Nothing);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_name_convention_migrates() {
        for (n, (new, old)) in NAMES.into_iter().enumerate() {
            // A root of its own each time: Windows and macOS folders ignore case, so
            // `LocalWriter` and `localwriter` would be the same folder.
            let root = temp(&format!("all{n}"));
            old_folder(&root, old);
            assert_eq!(migrate_from_old_name(&root.join(new)), Outcome::Moved, "{old} -> {new}");
            assert!(root.join(new).join("prefs.json").is_file(), "{new}");
            assert!(!root.join(old).exists(), "{old}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn an_existing_new_folder_is_never_overwritten() {
        let root = temp("exists");
        let old = old_folder(&root, "LocalOffice");
        let new = root.join("LocalWriter");
        std::fs::create_dir_all(&new).expect("new folder");
        std::fs::write(new.join("prefs.json"), b"new").expect("prefs");
        assert_eq!(migrate_dir(&old, &new), Outcome::Nothing);
        assert_eq!(std::fs::read(new.join("prefs.json")).expect("prefs"), b"new");
        assert!(old.join("prefs.json").is_file(), "the old folder stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_old_folder_is_not_an_error_and_creates_nothing() {
        let root = temp("none");
        assert_eq!(migrate_from_old_name(&root.join("LocalWriter")), Outcome::Nothing);
        assert!(!root.join("LocalWriter").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plain_file_with_the_old_name_is_not_a_folder_to_move() {
        let root = temp("file");
        std::fs::write(root.join("LocalOffice"), b"x").expect("file");
        assert_eq!(migrate_from_old_name(&root.join("LocalWriter")), Outcome::Nothing);
        assert!(!root.join("LocalWriter").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_copy_path_leaves_the_old_folder_and_a_complete_new_one() {
        let root = temp("copy");
        let old = old_folder(&root, "LocalOffice");
        let new = root.join("LocalWriter");
        // The fallback taken when a rename isn't possible (another volume, a file held open).
        let stage = root.join("stage");
        copy_tree(&old, &stage, 0).expect("copy");
        std::fs::rename(&stage, &new).expect("rename into place");
        assert_eq!(std::fs::read(new.join("logs/update.log")).expect("log"), b"log");
        assert!(old.join("prefs.json").is_file(), "the copy keeps the original");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_nested_too_deep_tree_is_refused_not_followed() {
        let root = temp("deep");
        let mut deep = root.join("src");
        for _ in 0..(MAX_DEPTH + 3) {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).expect("deep");
        assert!(copy_tree(&root.join("src"), &root.join("dst"), 0).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
