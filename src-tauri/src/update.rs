//! Updates: finds the newest release on GitHub and installs it the way this
//! copy of Arbor was installed.
//!
//! The release is found without GitHub's API (which allows 60 calls an hour
//! per address): `releases/latest/download/SHA256SUMS.txt` redirects to the
//! newest release's checksum file, whose address names the release's tag. The
//! checksums then name every installer and are what each download is checked
//! against. The API is asked only for the release notes, and only if it can.
//!
//! How a release is installed:
//! - AppImage: the new AppImage replaces the old file, then Arbor restarts.
//! - .deb / .rpm: the package is installed with the system's package tool,
//!   which asks for an administrator's password (pkexec), then Arbor restarts.
//! - macOS: the app is copied out of the disk image over the installed one.
//! - Windows: the installer of the same kind runs and restarts Arbor.
//! - Anything else (FreeBSD, a build from source): the release page opens.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SITE: &str = "https://github.com/bmickov/Arbor";
const API: &str = "https://api.github.com/repos/bmickov/Arbor";
const SUMS: &str = "SHA256SUMS.txt";

type R<T> = std::result::Result<T, String>;

/// Where releases come from. Debug builds can be pointed at a test server.
fn site() -> String {
    #[cfg(debug_assertions)]
    if let Ok(s) = std::env::var("ARBOR_UPDATE_SITE") {
        return s;
    }
    SITE.into()
}

fn api() -> String {
    #[cfg(debug_assertions)]
    if let Ok(s) = std::env::var("ARBOR_UPDATE_API") {
        return s;
    }
    API.into()
}

/// This copy's version. Debug builds can pretend to be older, to try updating.
pub fn current_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("ARBOR_PRETEND_VERSION") {
        return v;
    }
    env!("CARGO_PKG_VERSION").into()
}

/// How this copy of Arbor was installed, which decides how it's updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    AppImage,
    Deb,
    Rpm,
    Dmg,
    Nsis,
    Msi,
    /// Updated by hand: from the release page.
    Manual,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub version: String,
    pub kind: Kind,
    /// "AppImage", "Debian package", … for Preferences.
    pub label: String,
    /// Arbor can install a release itself (else it opens the release page).
    pub installable: bool,
    /// Releases page, for "all versions".
    pub releases: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub current: String,
    /// Newer than this copy.
    pub newer: bool,
    pub name: String,
    /// Release notes (Markdown), when GitHub's API answered.
    pub notes: String,
    pub published: Option<String>,
    pub page: String,
    /// The download for this kind of install and processor, if there is one.
    pub asset: Option<Asset>,
}

pub fn context() -> Context {
    let kind = detect();
    let label = match kind {
        Kind::AppImage => "AppImage",
        Kind::Deb => "Debian package",
        Kind::Rpm => "RPM package",
        Kind::Dmg => "macOS app",
        Kind::Nsis => "Windows installer",
        Kind::Msi => "Windows Installer package",
        Kind::Manual => {
            if cfg!(debug_assertions) {
                "Development build"
            } else if cfg!(target_os = "freebsd") {
                "FreeBSD archive"
            } else {
                "Manual install"
            }
        }
    };
    Context {
        version: current_version(),
        kind,
        label: label.into(),
        installable: kind != Kind::Manual,
        releases: format!("{}/releases", site()),
    }
}

/// Work out how this copy was installed.
pub fn detect() -> Kind {
    #[cfg(debug_assertions)]
    if let Ok(k) = std::env::var("ARBOR_UPDATE_KIND") {
        return match k.as_str() {
            "appimage" => Kind::AppImage,
            "deb" => Kind::Deb,
            "rpm" => Kind::Rpm,
            "dmg" => Kind::Dmg,
            "nsis" => Kind::Nsis,
            "msi" => Kind::Msi,
            _ => Kind::Manual,
        };
    }
    let Ok(exe) = std::env::current_exe() else { return Kind::Manual };
    detect_for(&exe)
}

#[cfg(target_os = "linux")]
fn detect_for(exe: &Path) -> Kind {
    if std::env::var_os("APPIMAGE").is_some_and(|p| Path::new(&p).is_file()) {
        return Kind::AppImage;
    }
    if !exe.starts_with("/usr") {
        return Kind::Manual;
    }
    // dpkg lists each package's files; Tauri's package is named "arbor".
    let listed = std::fs::read_to_string("/var/lib/dpkg/info/arbor.list")
        .map(|l| l.lines().any(|f| Path::new(f) == exe))
        .unwrap_or(false);
    if listed {
        return Kind::Deb;
    }
    let owned = Command::new("rpm")
        .arg("-qf")
        .arg(exe)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if owned {
        Kind::Rpm
    } else {
        Kind::Manual
    }
}

#[cfg(target_os = "macos")]
fn detect_for(exe: &Path) -> Kind {
    match bundle_of(exe) {
        // Run straight from a disk image or a quarantined download: macOS
        // runs it from a hidden read-only copy, which can't be replaced.
        Some(b) if !b.to_string_lossy().contains("/AppTranslocation/") && !b.starts_with("/Volumes") => Kind::Dmg,
        _ => Kind::Manual,
    }
}

#[cfg(windows)]
fn detect_for(exe: &Path) -> Kind {
    let Some(dir) = exe.parent() else { return Kind::Manual };
    // The NSIS installer puts its uninstaller next to the app; the MSI
    // package is uninstalled through Windows itself.
    if dir.join("uninstall.exe").is_file() {
        return Kind::Nsis;
    }
    let program_files = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|p| dir.starts_with(PathBuf::from(p)));
    if program_files {
        Kind::Msi
    } else {
        Kind::Manual
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn detect_for(_exe: &Path) -> Kind {
    Kind::Manual
}

/// The `.app` folder an executable is in.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

// ------------------------------------------------------------------ versions

/// Compare "1.2.10" with "1.2.9" by numbers. A pre-release ("1.3.0-beta.1")
/// comes before its release.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let v = v.split('+').next().unwrap_or(v);
        let (main, pre) = match v.split_once('-') {
            Some((m, p)) => (m, Some(p)),
            None => (v, None),
        };
        (main.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
    }
    let (an, ap) = split(a);
    let (bn, bp) = split(b);
    let len = an.len().max(bn.len());
    for i in 0..len {
        let (x, y) = (an.get(i).copied().unwrap_or(0), bn.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    match (ap, bp) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

/// `…/releases/download/v1.2.3/SHA256SUMS.txt` → "v1.2.3".
pub fn tag_from_location(location: &str) -> Option<String> {
    let rest = location.split("/releases/download/").nth(1)?;
    let (tag, file) = rest.split_once('/')?;
    (file.starts_with(SUMS) && safe_name(tag)).then(|| tag.to_string())
}

/// "hash  name" lines (sha256sum's format; a '*' marks binary mode).
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (hash, name) = l.trim().split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            let ok = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && safe_name(name);
            ok.then(|| (hash.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

/// A plain file name: nothing that could reach another path or URL.
fn safe_name(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

/// The installer for this kind of install on this processor (`arch` as in
/// `std::env::consts::ARCH`).
pub fn pick<'a>(kind: Kind, arch: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let x64 = arch == "x86_64";
    let want = |n: &str| -> bool {
        if !n.starts_with("Arbor") {
            return false;
        }
        match kind {
            Kind::AppImage => n.ends_with(if x64 { "_amd64.AppImage" } else { "_aarch64.AppImage" }),
            Kind::Deb => n.ends_with(if x64 { "_amd64.deb" } else { "_arm64.deb" }),
            Kind::Rpm => n.ends_with(if x64 { ".x86_64.rpm" } else { ".aarch64.rpm" }),
            Kind::Dmg => n.ends_with("_universal.dmg") || n.ends_with(if x64 { "_x64.dmg" } else { "_aarch64.dmg" }),
            Kind::Nsis => n.ends_with(if x64 { "_x64-setup.exe" } else { "_arm64-setup.exe" }),
            Kind::Msi => n.ends_with(".msi") && n.contains(if x64 { "_x64_" } else { "_arm64_" }),
            Kind::Manual => cfg!(target_os = "freebsd") && n.ends_with(if x64 { "_freebsd_amd64.tar.gz" } else { "_freebsd_arm64.tar.gz" }),
        }
    };
    names.into_iter().find(|n| want(n))
}

// ------------------------------------------------------------------ network

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .https_only(!cfg!(debug_assertions))
        .user_agent(format!("Arbor/{}", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(timeout)
        .build()
        .into()
}

fn net_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(404) => "GitHub has no release of Arbor there.".into(),
        ureq::Error::StatusCode(c) => format!("GitHub answered with error {c}."),
        ureq::Error::Timeout(_) => "GitHub didn't answer in time. Check your internet connection.".into(),
        ureq::Error::HostNotFound => "Couldn't reach GitHub. Check your internet connection.".into(),
        ureq::Error::Io(e) => format!("Couldn't reach GitHub: {e}"),
        e => format!("Couldn't reach GitHub: {e}"),
    }
}

/// The newest release's tag, from where `latest/download/SHA256SUMS.txt` leads.
fn latest_tag(agent: &ureq::Agent) -> R<String> {
    let url = format!("{}/releases/latest/download/{SUMS}", site());
    let resp = agent
        .get(&url)
        .config()
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(net_error)?;
    let status = resp.status().as_u16();
    if status == 404 {
        // No releases, or the newest is still being uploaded.
        return Err("Couldn't find the newest release's files on GitHub. Try again in a few minutes.".into());
    }
    let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("");
    tag_from_location(location).ok_or_else(|| format!("GitHub answered with something unexpected (status {status})."))
}

fn get_text(agent: &ureq::Agent, url: &str) -> R<String> {
    agent.get(url).call().map_err(net_error)?.body_mut().with_config().limit(1 << 20).read_to_string().map_err(net_error)
}

/// The release's title, notes and date from GitHub's API. Optional: the API
/// is rate-limited and blocked on some networks.
fn notes(agent: &ureq::Agent, tag: &str) -> Option<(String, String, Option<String>, String)> {
    let text = agent
        .get(&format!("{}/releases/tags/{tag}", api()))
        .header("Accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let page = s("html_url").filter(|u| u.starts_with(&format!("{}/", site())))?;
    Some((s("name").unwrap_or_default(), s("body").unwrap_or_default(), s("published_at"), page))
}

/// Ask GitHub for the newest release.
pub fn check() -> R<Release> {
    let agent = agent(Some(Duration::from_secs(30)));
    let tag = latest_tag(&agent)?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let sums = parse_sums(&get_text(&agent, &format!("{}/releases/download/{tag}/{SUMS}", site()))?);
    let kind = detect();
    let asset = pick(kind, std::env::consts::ARCH, sums.iter().map(|(_, n)| n.as_str())).map(|name| {
        let sha256 = sums.iter().find(|(_, n)| n == name).map(|(h, _)| h.clone()).unwrap_or_default();
        Asset { url: format!("{}/releases/download/{tag}/{name}", site()), name: name.to_string(), sha256 }
    });
    let (name, notes, published, page) =
        notes(&agent, &tag).unwrap_or_else(|| (String::new(), String::new(), None, format!("{}/releases/tag/{tag}", site())));
    let current = current_version();
    Ok(Release {
        newer: compare(&version, &current).is_gt(),
        name: if name.is_empty() { format!("Arbor {version}") } else { name },
        version,
        tag,
        current,
        notes,
        published,
        page,
        asset,
    })
}

/// Download `asset` to `dest`, checking it against its checksum. `progress`
/// gets (bytes so far, total if known) every so often.
pub fn download(asset: &Asset, dest: &Path, progress: &dyn Fn(u64, Option<u64>)) -> R<()> {
    if asset.sha256.len() != 64 {
        return Err(format!("{} has no checksum in the release, so Arbor won't install it.", asset.name));
    }
    let agent = agent(None);
    let mut resp = agent
        .get(&asset.url)
        .config()
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .call()
        .map_err(net_error)?;
    let total = resp.body().content_length();
    let mut reader = resp.body_mut().as_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("Couldn't save the download to {}: {e}", dest.display()))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now();
    progress(0, total);
    let result = (|| -> R<()> {
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| format!("Couldn't save the download: {e}"))?;
            done += n as u64;
            if last.elapsed() > Duration::from_millis(100) {
                progress(done, total);
                last = Instant::now();
            }
        }
        file.sync_all().map_err(|e| format!("Couldn't save the download: {e}"))?;
        progress(done, total);
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != asset.sha256 {
            return Err(format!("{} didn't match its checksum, so it wasn't installed. Try again later.", asset.name));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

// ------------------------------------------------------------------ install

/// Downloads are kept here until the next update.
fn download_dir() -> R<PathBuf> {
    let dir = dirs::cache_dir().ok_or("No cache folder to download to.")?.join("arbor").join("updates");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// What to do once the new version is in place.
pub enum Then {
    /// Start this program once Arbor has quit.
    Relaunch(PathBuf),
    /// An installer is running and will start Arbor itself.
    Nothing,
}

/// Download and install `release`. Arbor should quit right after, and
/// `relaunch` the returned program.
pub fn install(release: &Release, progress: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    let kind = detect();
    let asset = release.asset.as_ref().ok_or("This release has no download for this kind of install.")?;
    if !safe_name(&asset.name) || !asset.url.starts_with(&format!("{}/releases/download/", site())) {
        return Err("That download isn't from Arbor's releases.".into());
    }
    match kind {
        Kind::AppImage => install_appimage(asset, progress),
        Kind::Deb | Kind::Rpm => install_package(kind, asset, progress),
        Kind::Dmg => install_dmg(asset, progress),
        Kind::Nsis | Kind::Msi => install_windows(kind, asset, progress),
        Kind::Manual => Err("This copy of Arbor can't update itself. Download the new version from the release page.".into()),
    }
}

fn install_appimage(asset: &Asset, progress: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    let target = PathBuf::from(std::env::var_os("APPIMAGE").ok_or("Arbor isn't running from an AppImage.")?);
    // Replace the file in one step: download next to it, then rename over it.
    let dir = target.parent().ok_or("The AppImage has no folder.")?;
    if !writable(dir) {
        return Err(format!(
            "Arbor can't write to {}, so it can't replace the AppImage there. Download the new version from the release page instead.",
            dir.display()
        ));
    }
    let tmp = dir.join(format!(".{}.update-{}", asset.name, std::process::id()));
    download(asset, &tmp, progress)?;
    let done = (|| -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o755) | 0o111;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        }
        std::fs::rename(&tmp, &target)
    })();
    if let Err(e) = done {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't replace {}: {e}", target.display()));
    }
    Ok(Then::Relaunch(target))
}

fn writable(dir: &Path) -> bool {
    tempfile::Builder::new().prefix(".arbor-").tempfile_in(dir).is_ok()
}

fn install_package(kind: Kind, asset: &Asset, progress: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    // Before the package replaces it: Linux then reports "… (deleted)".
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let file = download_dir()?.join(&asset.name);
    download(asset, &file, progress)?;
    let exists = |p: &str| Path::new(p).exists();
    let mut cmd = Command::new("pkexec");
    if kind == Kind::Deb {
        if exists("/usr/bin/apt-get") {
            cmd.args(["apt-get", "install", "-y"]).arg(&file);
        } else {
            cmd.args(["dpkg", "-i"]).arg(&file);
        }
    } else if exists("/usr/bin/dnf") || exists("/usr/bin/dnf5") {
        cmd.args(["dnf", "install", "-y"]).arg(&file);
    } else if exists("/usr/bin/zypper") {
        cmd.args(["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]).arg(&file);
    } else {
        cmd.args(["rpm", "-U", "--replacepkgs"]).arg(&file);
    }
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("Installing needs pkexec, which isn't on this system. The package was saved as {}; install it with your package manager.", file.display())
        } else {
            format!("Couldn't start the installation: {e}")
        }
    })?;
    match out.status.code() {
        Some(0) => {}
        // pkexec: the password prompt was dismissed or refused.
        Some(126) | Some(127) => return Err("The update needs an administrator's permission to install.".into()),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            return Err(format!("Installing the package failed: {last}"));
        }
    }
    Ok(Then::Relaunch(exe))
}

#[cfg(target_os = "macos")]
fn install_dmg(asset: &Asset, progress: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    let bundle = bundle_of(&std::env::current_exe().map_err(|e| e.to_string())?).ok_or("Arbor isn't running from an app bundle.")?;
    let parent = bundle.parent().ok_or("The app has no folder.")?.to_path_buf();
    let dir = download_dir()?;
    let dmg = dir.join(&asset.name);
    download(asset, &dmg, progress)?;
    let mount = dir.join("mount");
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    let run = |c: &mut Command| -> R<()> {
        let out = c.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    run(Command::new("hdiutil").args(["attach", "-nobrowse", "-noautoopen", "-readonly", "-mountpoint"]).arg(&mount).arg(&dmg))
        .map_err(|e| format!("Couldn't open the disk image: {e}"))?;
    let result = (|| -> R<()> {
        let app = std::fs::read_dir(&mount)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("The disk image has no app in it.")?;
        let pid = std::process::id();
        let staged = parent.join(format!(".Arbor.app.update-{pid}"));
        let old = parent.join(format!(".Arbor.app.old-{pid}"));
        run(Command::new("ditto").arg(&app).arg(&staged)).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staged);
            format!("Couldn't copy the new version to {}: {e}", parent.display())
        })?;
        std::fs::rename(&bundle, &old).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staged);
            format!("Couldn't replace {}: {e}", bundle.display())
        })?;
        if let Err(e) = std::fs::rename(&staged, &bundle) {
            let _ = std::fs::rename(&old, &bundle);
            let _ = std::fs::remove_dir_all(&staged);
            return Err(format!("Couldn't replace {}: {e}", bundle.display()));
        }
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    })();
    let _ = run(Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount));
    result.map_err(|e| {
        if e.contains("ermission") {
            format!("{e}\nArbor needs to be in a folder you can change, such as Applications, to update itself.")
        } else {
            e
        }
    })?;
    Ok(Then::Relaunch(bundle))
}

#[cfg(not(target_os = "macos"))]
fn install_dmg(_: &Asset, _: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    Err("Disk images are for macOS.".into())
}

#[cfg(windows)]
fn install_windows(kind: Kind, asset: &Asset, progress: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let file = download_dir()?.join(&asset.name);
    download(asset, &file, progress)?;
    let mut cmd = if kind == Kind::Nsis {
        // Passive: a progress bar and no questions; /R starts Arbor after.
        let mut c = Command::new(&file);
        c.args(["/P", "/R", "/UPDATE"]);
        c
    } else {
        let mut c = Command::new("msiexec");
        c.arg("/i").arg(&file).args(["/passive", "AUTOLAUNCHAPP=True"]);
        c
    };
    cmd.creation_flags(DETACHED_PROCESS).spawn().map_err(|e| format!("Couldn't start the installer: {e}"))?;
    Ok(Then::Nothing)
}

#[cfg(not(windows))]
fn install_windows(_: Kind, _: &Asset, _: &dyn Fn(u64, Option<u64>)) -> R<Then> {
    Err("Windows installers are for Windows.".into())
}

/// Start `program` once this process has exited.
pub fn relaunch_after_exit(program: &Path) -> R<()> {
    #[cfg(unix)]
    {
        let pid = std::process::id().to_string();
        let mut cmd = Command::new("/bin/sh");
        // Wait for Arbor to quit (if it's asked to stay, the new version
        // starts whenever it does), then start the new one.
        let start = if cfg!(target_os = "macos") { "exec /usr/bin/open -n \"$2\"" } else { "exec \"$2\"" };
        cmd.arg("-c")
            .arg(format!("while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; {start}"))
            .arg("sh")
            .arg(&pid)
            .arg(program)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Settings an AppImage made for its own (now replaced) insides.
        if let Some(appdir) = std::env::var_os("APPDIR").map(PathBuf::from) {
            for (k, v) in clean_env(&appdir, std::env::vars_os()) {
                match v {
                    Some(v) => cmd.env(k, v),
                    None => cmd.env_remove(k),
                };
            }
        }
        cmd.spawn().map_err(|e| format!("Couldn't restart Arbor: {e}"))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = program;
        Ok(())
    }
}

/// Environment changes for a program started from inside an AppImage: drop
/// the AppImage's own variables and its folders from path lists.
pub fn clean_env(
    appdir: &Path,
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> {
    let own = ["APPDIR", "APPIMAGE", "ARGV0", "OWD"];
    let mut out = Vec::new();
    for (k, v) in vars {
        if own.iter().any(|o| k == *o) {
            out.push((k, None));
            continue;
        }
        let Some(s) = v.to_str() else { continue };
        let inside = |p: &str| Path::new(p).starts_with(appdir);
        if !s.split(':').any(inside) {
            continue;
        }
        let kept: Vec<&str> = s.split(':').filter(|p| !p.is_empty() && !inside(p)).collect();
        out.push((k, if kept.is_empty() { None } else { Some(kept.join(":").into()) }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare("0.15.10", "0.15.9"), Greater);
        assert_eq!(compare("v0.16.0", "0.15.5"), Greater);
        assert_eq!(compare("0.15.5", "0.15.5"), Equal);
        assert_eq!(compare("0.15", "0.15.0"), Equal);
        assert_eq!(compare("0.15.4", "0.15.5"), Less);
        assert_eq!(compare("1.0.0-beta.1", "1.0.0"), Less);
        assert_eq!(compare("1.0.0", "0.99.99"), Greater);
    }

    #[test]
    fn tag_comes_from_the_redirect() {
        assert_eq!(tag_from_location("https://github.com/bmickov/Arbor/releases/download/v0.15.5/SHA256SUMS.txt").as_deref(), Some("v0.15.5"));
        assert_eq!(tag_from_location("https://github.com/bmickov/Arbor/releases/download/../x/SHA256SUMS.txt"), None);
        assert_eq!(tag_from_location("https://github.com/login"), None);
        assert_eq!(tag_from_location(""), None);
    }

    #[test]
    fn checksums_parse_and_reject_odd_names() {
        let text = "d12202578d65719aba88796647b4f1f1ed85d1936c9785f4c128f5b9eaa4af40  Arbor-0.15.5-1.aarch64.rpm\n\
                    1F7B2666E1C189E14F626F82D50932246504A7692DB0476790EAC5C833F9FF20 *Arbor_0.15.5_amd64.AppImage\n\
                    abc  short.txt\n\
                    1f7b2666e1c189e14f626f82d50932246504a7692db0476790eac5c833f9ff20  ../evil\n";
        let sums = parse_sums(text);
        assert_eq!(sums.len(), 2);
        assert_eq!(sums[1].1, "Arbor_0.15.5_amd64.AppImage");
        assert!(sums[1].0.starts_with("1f7b"));
    }

    #[test]
    fn picks_the_installer_for_each_kind() {
        let names = [
            "Arbor-0.15.5-1.aarch64.rpm",
            "Arbor-0.15.5-1.x86_64.rpm",
            "Arbor_0.15.5_aarch64.AppImage",
            "Arbor_0.15.5_amd64.AppImage",
            "Arbor_0.15.5_amd64.deb",
            "Arbor_0.15.5_arm64.deb",
            "Arbor_0.15.5_freebsd_amd64.tar.gz",
            "Arbor_0.15.5_universal.dmg",
            "Arbor_0.15.5_x64-setup.exe",
            "Arbor_0.15.5_x64_en-US.msi",
        ];
        let p = |k, a| pick(k, a, names.iter().copied());
        assert_eq!(p(Kind::AppImage, "x86_64"), Some("Arbor_0.15.5_amd64.AppImage"));
        assert_eq!(p(Kind::AppImage, "aarch64"), Some("Arbor_0.15.5_aarch64.AppImage"));
        assert_eq!(p(Kind::Deb, "x86_64"), Some("Arbor_0.15.5_amd64.deb"));
        assert_eq!(p(Kind::Deb, "aarch64"), Some("Arbor_0.15.5_arm64.deb"));
        assert_eq!(p(Kind::Rpm, "x86_64"), Some("Arbor-0.15.5-1.x86_64.rpm"));
        assert_eq!(p(Kind::Rpm, "aarch64"), Some("Arbor-0.15.5-1.aarch64.rpm"));
        assert_eq!(p(Kind::Dmg, "aarch64"), Some("Arbor_0.15.5_universal.dmg"));
        assert_eq!(p(Kind::Nsis, "x86_64"), Some("Arbor_0.15.5_x64-setup.exe"));
        assert_eq!(p(Kind::Msi, "x86_64"), Some("Arbor_0.15.5_x64_en-US.msi"));
        assert_eq!(p(Kind::Nsis, "aarch64"), None);
    }

    #[test]
    fn appimage_variables_are_dropped_for_the_new_one() {
        let appdir = Path::new("/tmp/.mount_ArborAbc");
        let vars = [
            ("APPDIR", "/tmp/.mount_ArborAbc"),
            ("APPIMAGE", "/home/a/Arbor.AppImage"),
            ("XDG_DATA_DIRS", "/tmp/.mount_ArborAbc/usr/share:/usr/local/share:/usr/share"),
            ("GTK_PATH", "/tmp/.mount_ArborAbc/usr/lib/gtk-3.0"),
            ("HOME", "/home/a"),
        ]
        .map(|(k, v)| (k.into(), v.into()));
        let out = clean_env(appdir, vars);
        let get = |k: &str| out.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("APPDIR"), Some(None));
        assert_eq!(get("APPIMAGE"), Some(None));
        assert_eq!(get("XDG_DATA_DIRS"), Some(Some("/usr/local/share:/usr/share".into())));
        assert_eq!(get("GTK_PATH"), Some(None));
        assert_eq!(get("HOME"), None);
    }
}

#[cfg(test)]
mod online {
    /// Asks GitHub for real: `cargo test -- --ignored online`.
    #[test]
    #[ignore]
    fn finds_the_latest_release() {
        let r = super::check().expect("check");
        eprintln!("{r:#?}");
        assert!(r.version.split('.').count() >= 2);
    }
}
