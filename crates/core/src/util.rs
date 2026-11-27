use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, LazyLock},
};

use anyhow_ext::Context;
use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
pub use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

pub fn copy_dir<T: AsRef<Path>, U: AsRef<Path>>(src: T, dst: U) -> anyhow_ext::Result<()> {
    for p in jwalk::WalkDir::new(&src) {
        let from = p?.path();
        let to = dst.as_ref().join(from.strip_prefix(&src)?);
        if from.is_dir() && !to.exists() {
            std::fs::create_dir(to)?;
        } else if from.is_file() {
            std::fs::copy(from, to)?;
        }
    }
    Ok(())
}

pub fn hardlink_dir<T: AsRef<Path>, U: AsRef<Path>>(src: T, dst: U) -> anyhow_ext::Result<()> {
    for p in jwalk::WalkDir::new(&src) {
        let from = p?.path();
        let to = dst.as_ref().join(from.strip_prefix(&src)?);
        if from.is_dir() && !to.exists() {
            std::fs::create_dir(to)?;
        } else if from.is_file() {
            std::fs::hard_link(from, to)?;
        }
    }
    Ok(())
}

pub fn remove_dir_all(dir: impl AsRef<std::path::Path>) -> anyhow_ext::Result<()> {
    fn inner(dir: &Path) -> anyhow_ext::Result<()> {
        if std::fs::symlink_metadata(dir).is_err() {
            return Ok(());
        }
        #[cfg(windows)]
        {
            let mut result = remove_dir_all::remove_dir_all(dir);
            for attempt in 1..=6 {
                if result.is_ok() || std::fs::symlink_metadata(dir).is_err() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                result = remove_dir_all::remove_dir_all(dir);
            }
            if let Err(e) = result {
                if std::fs::symlink_metadata(dir).is_err() {
                    return Ok(());
                }
                if move_aside(dir) {
                    log::warn!(
                        "{} couldn't be deleted ({e}), moved it to the temporary folder instead",
                        dir.display()
                    );
                    return Ok(());
                }
                return Err(e).with_context(|| dir.to_string_lossy().to_string());
            }
        }
        #[cfg(not(windows))]
        fs_err::remove_dir_all(dir)?;
        Ok(())
    }
    inner(dir.as_ref())
}

#[cfg(windows)]
fn move_aside(dir: &Path) -> bool {
    let root = temp_root();
    if dir.starts_with(&root) {
        return false;
    }
    let Ok(target) = tempfile::Builder::new().prefix(TEMP_PREFIX).tempdir_in(&root) else {
        return false;
    };
    let target = target.keep();
    let dest = target.join("removed");
    if std::fs::rename(dir, &dest).is_err() {
        std::fs::remove_dir(&target).unwrap_or(());
        return false;
    }
    remove_dir_all::remove_dir_all(&target).unwrap_or(());
    true
}

#[inline(always)]
pub fn is_symlink(link: &Path) -> bool {
    #[cfg(windows)]
    {
        junction::exists(link).unwrap_or(false) || link.is_symlink()
    }
    #[cfg(unix)]
    {
        link.is_symlink()
    }
}

#[inline(always)]
pub fn is_symlink_to(link: &Path, dest: &Path) -> bool {
    #[cfg(windows)]
    {
        (junction::exists(link).unwrap_or(false)
            && junction::get_target(link).expect("Path does not exist") == dest)
            || (link.is_symlink() && link.read_link().expect("Path does not exist") == dest)
    }
    #[cfg(unix)]
    {
        link.is_symlink() && link.read_link().expect("Path does not exist") == dest
    }
}

#[inline(always)]
pub fn create_symlink(link: &Path, target: &Path) -> anyhow_ext::Result<()> {
    #[cfg(windows)]
    junction::create(target, link).or_else(|_| std::os::windows::fs::symlink_dir(target, link))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).with_context(|| {
        format!(
            "Failed to symlink {} to {}",
            target.display(),
            link.display()
        )
    })?;
    Ok(())
}

#[inline(always)]
pub fn remove_symlink(link: impl AsRef<std::path::Path>) -> anyhow_ext::Result<()> {
    fn inner(link: &Path) -> anyhow_ext::Result<()> {
        #[cfg(windows)]
        fs_err::remove_dir(link)?;
        #[cfg(not(windows))]
        fs_err::remove_file(link)?;
        Ok(())
    }
    inner(link.as_ref())
}

static TEMP_FS: LazyLock<RwLock<HashSet<PathBuf>>> =
    LazyLock::new(|| RwLock::new(HashSet::default()));

static TEMP_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

thread_local! {
    static TEMP_SCOPE: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

const TEMP_PREFIX: &str = ".swmm-";
const STALE_TEMP: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

pub fn set_temp_root(dir: &Path) {
    let dir = dir.join("tmp");
    if std::fs::create_dir_all(&dir).is_err() {
        log::warn!("Can't use {} for temporary files, using the system's", dir.display());
        *TEMP_ROOT.write() = None;
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_TEMP);
            if stale && entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
                let path = entry.path();
                if path.is_dir() {
                    remove_dir_all(&path).unwrap_or(());
                } else {
                    std::fs::remove_file(&path).unwrap_or(());
                }
            }
        }
    }
    *TEMP_ROOT.write() = Some(dir);
}

pub fn temp_root() -> PathBuf {
    TEMP_ROOT.read().clone().unwrap_or_else(std::env::temp_dir)
}

pub fn new_temp_dir() -> std::io::Result<PathBuf> {
    let builder = {
        let mut b = tempfile::Builder::new();
        b.prefix(TEMP_PREFIX);
        b
    };
    builder
        .tempdir_in(temp_root())
        .or_else(|_| builder.tempdir())
        .map(tempfile::TempDir::keep)
}

fn track(temp: PathBuf) -> MappedRwLockReadGuard<'static, PathBuf> {
    TEMP_FS.write().insert(temp.clone());
    TEMP_SCOPE.with(|s| {
        if let Some(scope) = s.borrow_mut().as_mut() {
            scope.push(temp.clone());
        }
    });
    RwLockReadGuard::map(TEMP_FS.read(), |tmps| unsafe {
        tmps.get(&temp).unwrap_unchecked()
    })
}

#[allow(clippy::unwrap_used)]
pub fn get_temp_folder() -> MappedRwLockReadGuard<'static, PathBuf> {
    track(new_temp_dir().unwrap())
}

#[allow(clippy::unwrap_used)]
pub fn get_temp_file() -> MappedRwLockReadGuard<'static, PathBuf> {
    let builder = {
        let mut b = tempfile::Builder::new();
        b.prefix(TEMP_PREFIX);
        b
    };
    let file = builder
        .tempfile_in(temp_root())
        .or_else(|_| builder.tempfile())
        .unwrap()
        .keep()
        .unwrap()
        .1;
    track(file)
}

fn remove_temp(tmp: &Path) {
    if tmp.is_file() {
        fs_err::remove_file(tmp).unwrap_or(());
    } else {
        remove_dir_all(tmp).unwrap_or(());
    }
}

pub fn begin_temp_scope() {
    TEMP_SCOPE.with(|s| *s.borrow_mut() = Some(Vec::new()));
}

pub fn end_temp_scope(keep: Option<&Path>) {
    let Some(scoped) = TEMP_SCOPE.with(|s| s.borrow_mut().take()) else { return };
    let mut all = TEMP_FS.write();
    for tmp in scoped {
        if keep.is_some_and(|k| k.starts_with(&tmp)) {
            continue;
        }
        all.remove(&tmp);
        remove_temp(&tmp);
    }
}

pub fn release_temp(path: &Path) {
    let mut all = TEMP_FS.write();
    let owners: Vec<PathBuf> = all.iter().filter(|t| path.starts_with(t)).cloned().collect();
    for tmp in owners {
        all.remove(&tmp);
        remove_temp(&tmp);
    }
}

pub fn clear_temp() {
    TEMP_FS.write().drain().for_each(|tmp| remove_temp(&tmp));
}

pub static USE_SZ: AtomicBool = AtomicBool::new(true);

pub fn extract_7z(file: &Path, folder: &Path) -> anyhow_ext::Result<()> {
    static SZ_EXISTS: LazyLock<bool> = LazyLock::new(|| {
        match std::process::Command::new("7z")
            .stdout(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                log::trace!("7z found");
                true
            }
            Err(_) => {
                log::trace!("7z not found");
                false
            }
        }
    });

    if *SZ_EXISTS && USE_SZ.load(std::sync::atomic::Ordering::Relaxed) {
        let output = std::process::Command::new("7z")
            .arg("x")
            .arg(file)
            .arg(format!("-o{}", folder.display()))
            .output()?;
        if !output.stderr.is_empty() {
            anyhow_ext::bail!("{}", std::string::String::from_utf8_lossy(&output.stderr))
        } else {
            Ok(())
        }
    } else {
        Ok(sevenz_rust::decompress_file(file, folder)?)
    }
}
