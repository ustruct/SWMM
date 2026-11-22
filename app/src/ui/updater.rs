use std::{
    cmp::Ordering,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::AtomicU64,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use swmm_core::settings::UpdatePreference;
use swmm_i18n::string_ext::LocString;

use super::{App, Message};

pub const REPO: &str = "ustruct/SWMM";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const CHECKSUMS: &str = "SHA256SUMS.txt";
const OLD_SUFFIX: &str = "old";

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub package: Option<Asset>,
    checksums: Option<Asset>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    numbers: [u64; 3],
    stable: bool,
    pre: String,
}

fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match text.split_once(['-', '+']) {
        Some((core, pre)) => (core, pre.to_string()),
        None => (text, String::new()),
    };
    let mut numbers = [0u64; 3];
    let mut parts = core.split('.');
    for slot in numbers.iter_mut() {
        if let Some(part) = parts.next() {
            *slot = part.trim().parse().ok()?;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Version {
        numbers,
        stable: pre.is_empty(),
        pre,
    })
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a.cmp(&b) == Ordering::Greater,
        _ => false,
    }
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok().or(Some(exe)))
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".swmm-update-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn installed_by_msi(dir: &Path) -> bool {
    let program_files = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from);
    let dir = dir.to_string_lossy().to_lowercase();
    let dir = dir.trim_start_matches(r"\\?\");
    program_files
        .map(|p| p.to_string_lossy().to_lowercase())
        .any(|p| !p.is_empty() && dir.starts_with(&p))
        || !can_write(Path::new(dir))
}

fn package_name() -> Option<&'static str> {
    if cfg!(windows) {
        let msi = install_dir().is_some_and(|dir| installed_by_msi(&dir));
        Some(if msi { "swmm-x86_64-windows.msi" } else { "swmm-x86_64-windows.zip" })
    } else if cfg!(target_os = "macos") {
        Some("swmm-aarch64-macos.tar.xz")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("swmm-x86_64-linux.tar.xz")
    } else {
        None
    }
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let (status, body) = super::net::response_with_status(url)?;
    match status {
        200..=299 => serde_json::from_slice(&body).context("GitHub sent a reply SWMM couldn't read"),
        403 | 429 => anyhow::bail!("GitHub is limiting requests right now. Try again later."),
        404 => anyhow::bail!("No release of SWMM was found on GitHub"),
        _ => anyhow::bail!("GitHub answered with HTTP {status}"),
    }
}

fn latest(preference: UpdatePreference) -> Result<ApiRelease> {
    if preference == UpdatePreference::Beta {
        let releases: Vec<ApiRelease> =
            get_json(&format!("https://api.github.com/repos/{REPO}/releases?per_page=15"))?;
        releases
            .into_iter()
            .filter(|r| !r.draft && parse_version(&r.tag_name).is_some())
            .max_by(|a, b| {
                parse_version(&a.tag_name).cmp(&parse_version(&b.tag_name))
            })
            .context("No release of SWMM was found on GitHub")
    } else {
        let release: ApiRelease = get_json(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
        anyhow::ensure!(!release.draft && !release.prerelease, "No stable release of SWMM was found");
        Ok(release)
    }
}

pub fn check(preference: UpdatePreference) -> Result<Option<Release>> {
    let release = latest(preference)?;
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    log::info!("Newest SWMM release is {version}, this is {CURRENT}");
    if !is_newer(&version, CURRENT) {
        return Ok(None);
    }
    let find = |name: &str| release.assets.iter().find(|a| a.name.eq_ignore_ascii_case(name)).cloned();
    Ok(Some(Release {
        package: package_name().and_then(find),
        checksums: find(CHECKSUMS),
        page: release.html_url.clone(),
        version,
    }))
}

fn expected_hash(release: &Release, name: &str, dir: &Path) -> Result<Option<String>> {
    let Some(sums) = release.checksums.as_ref() else {
        return Ok(None);
    };
    let file = dir.join(CHECKSUMS);
    super::net::download_file(&sums.browser_download_url, &file, &AtomicU64::new(0))
        .context("Couldn't download the update's checksums")?;
    let text = std::fs::read_to_string(&file)?;
    Ok(text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        file.eq_ignore_ascii_case(name).then(|| hash.to_ascii_lowercase())
    }))
}

fn file_hash(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn download(release: &Release) -> Result<(PathBuf, PathBuf)> {
    let asset = release
        .package
        .as_ref()
        .context("This release has no download for this system. Get it from the release page instead.")?;
    let dir = std::env::temp_dir().join("swmm-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(&asset.name);
    log::info!("Downloading SWMM {}", release.version);
    super::net::download_file(&asset.browser_download_url, &file, &AtomicU64::new(0))
        .context("Couldn't download the update")?;
    match expected_hash(release, &asset.name, &dir)? {
        Some(expected) => {
            let actual = file_hash(&file)?;
            anyhow::ensure!(
                actual == expected,
                "The downloaded update is damaged (its checksum doesn't match). Try again."
            );
            log::info!("The update's checksum matches");
        }
        None => log::warn!("This release has no checksum for {}", asset.name),
    }
    Ok((dir, file))
}

fn swap_in(staged: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = (|| -> Result<()> {
        for (new, target) in staged {
            let backup = target.with_extension(match target.extension() {
                Some(ext) => format!("{}.{OLD_SUFFIX}", ext.to_string_lossy()),
                None => OLD_SUFFIX.to_string(),
            });
            let _ = std::fs::remove_file(&backup);
            let moved = if target.exists() {
                std::fs::rename(target, &backup)
                    .with_context(|| format!("Couldn't move {} aside", target.display()))?;
                Some(backup)
            } else {
                None
            };
            done.push((target.clone(), moved));
            std::fs::copy(new, target).with_context(|| format!("Couldn't write {}", target.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        for (target, backup) in done.into_iter().rev() {
            let _ = std::fs::remove_file(&target);
            if let Some(backup) = backup {
                let _ = std::fs::rename(&backup, &target);
            }
        }
    }
    result
}

fn relaunch(exe: &Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args_os().skip(1).filter(|a| a == "--portable" || a == "--debug"));
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command.spawn().context("The update is installed, but SWMM couldn't restart. Open it again.")?;
    Ok(())
}

#[derive(Debug)]
pub enum Finish {
    Restart(PathBuf),
    #[cfg(windows)]
    RunInstaller { package: PathBuf, exe: PathBuf },
}

impl Finish {
    pub fn run(self) {
        let result = match self {
            Finish::Restart(exe) => relaunch(&exe),
            #[cfg(windows)]
            Finish::RunInstaller { package, exe } => install_msi(&package, &exe),
        };
        if let Err(e) = result {
            log::error!("Couldn't finish updating SWMM: {e:?}");
        }
    }
}

#[cfg(windows)]
fn install_msi(file: &Path, exe: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
    let script = format!(
        "\u{feff}$msi = {}\r\n$exe = {}\r\nStart-Process -FilePath msiexec.exe -ArgumentList \"/i `\"$msi`\" /passive /norestart\" -Wait\r\nStart-Process -FilePath $exe\r\n",
        quote(file),
        quote(exe),
    );
    let script_file = file.with_extension("ps1");
    std::fs::write(&script_file, script).context("Couldn't prepare the installer")?;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
        .arg(&script_file)
        .creation_flags(0x0800_0000)
        .spawn()
        .context("Couldn't start the installer")?;
    Ok(())
}

fn unpack_zip(file: &Path, dir: &Path, names: &[&str]) -> Result<Vec<PathBuf>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(file)?).context("The update archive is damaged")?;
    let mut out = Vec::new();
    for name in names {
        let index = (0..archive.len())
            .find(|i| {
                archive
                    .by_index(*i)
                    .ok()
                    .and_then(|e| e.enclosed_name())
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(name)))
                    .unwrap_or(false)
            })
            .with_context(|| format!("The update has no {name}"))?;
        let mut entry = archive.by_index(index)?;
        let dest = dir.join("staged").join(name);
        std::fs::create_dir_all(dest.parent().context("No folder")?)?;
        let mut writer = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut writer)?;
        out.push(dest);
    }
    Ok(out)
}

#[cfg(unix)]
fn unpack_tar(file: &Path, dir: &Path) -> Result<PathBuf> {
    let staged = dir.join("staged");
    std::fs::create_dir_all(&staged)?;
    let status = std::process::Command::new("tar")
        .arg("-xJf")
        .arg(file)
        .arg("-C")
        .arg(&staged)
        .status()
        .context("Couldn't run tar to unpack the update")?;
    anyhow::ensure!(status.success(), "Couldn't unpack the update");
    let exe = staged.join("swmm");
    anyhow::ensure!(exe.is_file(), "The update has no swmm program");
    Ok(exe)
}

pub fn install(release: &Release) -> Result<Finish> {
    let exe = std::env::current_exe().context("Couldn't find SWMM's program file")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let exe_dir = exe.parent().context("SWMM's program has no folder")?.to_path_buf();
    let (dir, file) = download(release)?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();

    #[cfg(windows)]
    if name.ends_with(".msi") {
        return Ok(Finish::RunInstaller { package: file, exe });
    }

    let staged: Vec<(PathBuf, PathBuf)> = if name.ends_with(".zip") {
        let files = unpack_zip(&file, &dir, &["swmm.exe", "swmm.dll"])?;
        let exe_name = exe.file_name().context("SWMM's program has no name")?.to_os_string();
        vec![
            (files[0].clone(), exe_dir.join(exe_name)),
            (files[1].clone(), exe_dir.join("swmm.dll")),
        ]
    } else {
        #[cfg(unix)]
        {
            vec![(unpack_tar(&file, &dir)?, exe.clone())]
        }
        #[cfg(not(unix))]
        anyhow::bail!("SWMM can't install {name} on this system")
    };
    anyhow::ensure!(
        can_write(&exe_dir),
        "SWMM can't write to {}. Download the update from the release page instead.",
        exe_dir.display()
    );
    swap_in(&staged)?;
    log::info!("Installed SWMM {}", release.version);
    Ok(Finish::Restart(exe))
}

pub fn clean_old_files() {
    let Some(dir) = install_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.starts_with("swmm") && name.ends_with(&format!(".{OLD_SUFFIX}")) {
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("swmm-update"));
}

pub fn migrate_preference(core: &swmm_core::core::Manager) {
    let mut settings = core.settings_mut();
    let older = settings
        .last_version
        .as_deref()
        .is_none_or(|v| is_newer("0.1.6", v));
    if older && settings.check_updates == UpdatePreference::None {
        settings.check_updates = UpdatePreference::Stable;
        if let Err(e) = settings.save() {
            log::warn!("Couldn't save the update setting: {e:?}");
        }
    }
}

pub fn check_in_background(core: &swmm_core::core::Manager, sender: flume::Sender<Message>) {
    let preference = core.settings().check_updates;
    if preference == UpdatePreference::None || cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || match check(preference) {
        Ok(Some(release)) => {
            sender.send(Message::UpdateAvailable(Box::new(release), false)).unwrap_or(());
        }
        Ok(None) => {}
        Err(e) => log::warn!("Couldn't check for SWMM updates: {e:?}"),
    });
}

impl App {
    pub(super) fn ask_to_update(&mut self, release: Box<Release>) {
        if release.package.is_none() {
            let text = localized("Update_Manual", &release.version);
            self.do_update(Message::Toast(text));
            open::that(&release.page).unwrap_or(());
            return;
        }
        let prompt = localized("Update_Available", &release.version);
        self.do_update(Message::Confirm(Box::new(Message::InstallUpdate(release)), prompt));
    }

    pub(super) fn handle_update_message(&mut self, ctx: &eframe::egui::Context, msg: Message) {
        match msg {
            Message::CheckForUpdates => self.do_task(|core| {
                let preference = match core.settings().check_updates {
                    UpdatePreference::None => UpdatePreference::Stable,
                    other => other,
                };
                Ok(match check(preference)? {
                    Some(release) => Message::UpdateAvailable(Box::new(release), true),
                    None => Message::UpToDate,
                })
            }),
            Message::UpdateAvailable(release, manual) => {
                if manual {
                    self.busy.set(false);
                    self.ask_to_update(release);
                } else {
                    self.pending_update = Some(release);
                }
            }
            Message::UpToDate => {
                self.busy.set(false);
                self.do_update(Message::Toast("Update_None".localize().into_owned()));
            }
            Message::InstallUpdate(release) => {
                self.do_task(move |_| Ok(Message::UpdateReady(Box::new(install(&release)?))));
            }
            Message::UpdateReady(finish) => {
                self.busy.set(false);
                self.after_exit = Some(*finish);
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }
}

fn localized(key: &'static str, version: &str) -> String {
    let message = key.localize();
    let vars = std::collections::HashMap::from([
        ("version".to_string(), version.to_string()),
        ("current".to_string(), CURRENT.to_string()),
    ]);
    strfmt::strfmt(&message, &vars).unwrap_or_else(|_| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("0.1.6", "0.1.5"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.5", "0.1.5"));
        assert!(!is_newer("0.1.4", "0.1.5"));
        assert!(is_newer("0.1.6", "0.1.6-beta.1"));
        assert!(!is_newer("0.1.6-beta.1", "0.1.6"));
        assert!(is_newer("0.1.6-beta.2", "0.1.6-beta.1"));
        assert!(is_newer("0.2", "0.1.9"));
        assert!(!is_newer("nonsense", "0.1.5"));
    }
}
