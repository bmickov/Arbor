//! How Arbor appears on the Linux/BSD desktop: its name, and the launcher
//! (`.desktop` file) and icon that docks, app menus and task switchers use.
//!
//! The desktop matches a window to its launcher by the window's application
//! id. GTK takes that from the program name, which is otherwise the binary's
//! file name, "arbor", so docks showed a lower-case "arbor" with a generic
//! icon. We name the program "Arbor" to match the packaged launcher,
//! `Arbor.desktop`.
//!
//! The .deb and .rpm packages install that launcher. An AppImage doesn't, so
//! when Arbor runs without one it adds its own to the user's
//! ~/.local/share/applications, with the icons beside it in
//! ~/.local/share/icons, pointing at the AppImage. The file is marked as
//! Arbor's (X-Arbor-Managed); one written by hand is never touched.

use serde::Serialize;

/// Must match the launcher's file name (Arbor.desktop) and StartupWMClass.
pub const APP_ID: &str = "Arbor";

/// Call before any window exists (before tauri::Builder).
pub fn set_names() {
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
    {
        gtk::glib::set_prgname(Some(APP_ID));
        gtk::glib::set_application_name("Arbor");
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Integration {
    /// Not a Linux/BSD desktop.
    Unsupported,
    /// A package (.deb, .rpm, Flatpak…) already provides the launcher.
    Packaged,
    /// A development build; left alone.
    Development,
    /// Our launcher is in place (written or updated now, or already right).
    Installed,
    /// Turned off: our launcher (if any) was removed.
    Removed,
}

#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd")))]
pub fn integrate(_enable: bool) -> Result<Integration, String> {
    Ok(Integration::Unsupported)
}

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
pub fn integrate(enable: bool) -> Result<Integration, String> {
    imp::integrate(enable).map_err(|e| format!("Couldn't update Arbor's launcher: {e}"))
}

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
mod imp {
    use super::{Integration, APP_ID};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    const MARK: &str = "X-Arbor-Managed=true";
    const ICONS: &[(&str, &[u8])] = &[
        ("32x32", include_bytes!("../icons/32x32.png")),
        ("64x64", include_bytes!("../icons/64x64.png")),
        ("128x128", include_bytes!("../icons/128x128.png")),
        ("256x256", include_bytes!("../icons/128x128@2x.png")),
        ("512x512", include_bytes!("../icons/icon.png")),
    ];

    fn data_home() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
    }

    /// A launcher installed system-wide, by a package.
    fn packaged() -> bool {
        let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut roots: Vec<PathBuf> = dirs.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        roots.push("/var/lib/flatpak/exports/share".into());
        if let Some(h) = dirs::home_dir() {
            roots.push(h.join(".local/share/flatpak/exports/share"));
        }
        let own = data_home();
        // An AppImage adds its own insides (which hold a copy of the launcher) to
        // XDG_DATA_DIRS; those don't count.
        let inside = std::env::var_os("APPDIR").map(PathBuf::from);
        roots
            .iter()
            .filter(|r| Some(*r) != own.as_ref())
            .filter(|r| !inside.as_ref().is_some_and(|a| r.starts_with(a)))
            .any(|r| r.join("applications").join(format!("{APP_ID}.desktop")).exists())
    }

    /// Quote a path for a desktop file's Exec key (Desktop Entry spec).
    fn exec_quote(p: &str) -> String {
        let mut s = String::from("\"");
        for c in p.chars() {
            match c {
                '"' | '`' | '$' | '\\' => {
                    s.push('\\');
                    s.push(c);
                }
                '%' => s.push_str("%%"),
                _ => s.push(c),
            }
        }
        s.push('"');
        s
    }

    fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if fs::read(path).ok().as_deref() == Some(bytes) {
            return Ok(());
        }
        if let Some(d) = path.parent() {
            fs::create_dir_all(d)?;
        }
        // Write then rename, so a desktop watching the folder never reads half a file.
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }

    fn ours(path: &Path) -> bool {
        fs::read_to_string(path).map(|s| s.lines().any(|l| l.trim() == MARK)).unwrap_or(false)
    }

    pub fn integrate(enable: bool) -> io::Result<Integration> {
        let Some(data) = data_home() else { return Ok(Integration::Unsupported) };
        let launcher = data.join("applications").join(format!("{APP_ID}.desktop"));
        let icon_path = |size: &str| data.join("icons/hicolor").join(size).join("apps/arbor.png");

        let remove = || -> io::Result<()> {
            if ours(&launcher) {
                fs::remove_file(&launcher)?;
                for (size, _) in ICONS {
                    let _ = fs::remove_file(icon_path(size));
                }
            }
            Ok(())
        };

        if !enable {
            remove()?;
            return Ok(Integration::Removed);
        }
        // A package's launcher already covers it; ours would shadow it.
        if packaged() {
            remove()?;
            return Ok(Integration::Packaged);
        }
        // Run from an AppImage: launch the AppImage itself, wherever it now is.
        let target = match std::env::var("APPIMAGE").ok().filter(|p| Path::new(p).is_file()) {
            Some(p) => p,
            None => {
                let exe = std::env::current_exe()?;
                let s = exe.to_string_lossy().to_string();
                if s.contains("/target/debug/") {
                    return Ok(Integration::Development);
                }
                s
            }
        };
        // Someone's own launcher by that name: leave it be.
        if launcher.exists() && !ours(&launcher) {
            return Ok(Integration::Installed);
        }

        // Desktops watch these folders, so new files show up without a nudge.
        for (size, bytes) in ICONS {
            write_if_changed(&icon_path(size), bytes)?;
        }
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Arbor\n\
             GenericName=Git client\n\
             Comment=A friendly Git client\n\
             Exec={} %F\n\
             Icon={}\n\
             Terminal=false\n\
             Categories=Development;RevisionControl;\n\
             Keywords=git;version control;vcs;\n\
             StartupWMClass={APP_ID}\n\
             StartupNotify=true\n\
             {MARK}\n",
            exec_quote(&target),
            // A full path: found at once, without waiting for an icon-theme rescan.
            icon_path("512x512").display(),
        );
        write_if_changed(&launcher, entry.as_bytes())?;
        Ok(Integration::Installed)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn quotes_exec_paths() {
            assert_eq!(exec_quote("/home/me/Apps/Arbor 1.AppImage"), "\"/home/me/Apps/Arbor 1.AppImage\"");
            assert_eq!(exec_quote("/a/$b`c\"d\\e%f"), "\"/a/\\$b\\`c\\\"d\\\\e%%f\"");
        }
    }
}
