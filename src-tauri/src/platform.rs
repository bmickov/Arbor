//! The few things that differ between Linux, the BSDs, macOS and Windows:
//! opening files and terminals, and running commands without a console
//! window popping up on Windows.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A command that, on Windows, runs without flashing a console window.
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Paths from libgit2 use forward slashes everywhere; Windows apps prefer
/// backslashes.
pub fn native_path(path: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.replace('/', "\\"))
    } else {
        PathBuf::from(path)
    }
}

/// A web or mail link rather than a file path.
fn is_link(s: &str) -> bool {
    let scheme = s.split_once(':').map(|(a, _)| a).unwrap_or("");
    (s.contains("://") && scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))) || s.starts_with("mailto:")
}

/// Open a file or folder with the desktop's default application, or a link
/// in the browser. Links are passed on as they are: turning their slashes
/// into Windows path separators would break them.
pub fn open_path(path: &str) -> Result<(), String> {
    let result = if is_link(path) { open::that_detached(path) } else { open::that_detached(native_path(path)) };
    result.map_err(|e| format!("Couldn't open {path}: {e}"))
}

/// Open a terminal window in `dir`.
pub fn open_terminal(dir: &str) -> Result<(), String> {
    let dir = native_path(dir);
    for mut cmd in terminal_candidates(&dir) {
        if cmd.spawn().is_ok() {
            return Ok(());
        }
    }
    Err("No terminal app was found.".into())
}

#[cfg(target_os = "macos")]
fn terminal_candidates(dir: &Path) -> Vec<Command> {
    // `open -a Terminal <dir>` opens a new Terminal window in that folder.
    let mut v = vec![];
    for app in ["iTerm", "Terminal"] {
        let mut c = Command::new("open");
        c.args(["-a", app]).arg(dir);
        v.push(c);
    }
    v
}

#[cfg(windows)]
fn terminal_candidates(dir: &Path) -> Vec<Command> {
    // Windows Terminal if installed, otherwise PowerShell or cmd in a new console.
    let mut wt = Command::new("wt.exe");
    wt.arg("-d").arg(dir);
    let mut ps = Command::new("cmd");
    ps.args(["/C", "start", "", "powershell", "-NoExit"]).current_dir(dir);
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "", "cmd"]).current_dir(dir);
    vec![wt, ps, cmd]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn terminal_candidates(dir: &Path) -> Vec<Command> {
    // Linux and the BSDs: try the common terminal emulators in turn.
    let candidates: [(&str, &[&str]); 11] = [
        ("x-terminal-emulator", &[]),
        ("gnome-terminal", &["--working-directory"]),
        ("konsole", &["--workdir"]),
        ("kgx", &["--working-directory"]),
        ("xfce4-terminal", &["--working-directory"]),
        ("mate-terminal", &["--working-directory"]),
        ("alacritty", &["--working-directory"]),
        ("kitty", &["--directory"]),
        ("wezterm", &["start", "--cwd"]),
        ("foot", &["--working-directory"]),
        ("xterm", &[]),
    ];
    candidates
        .iter()
        .map(|(bin, flags)| {
            let mut c = Command::new(bin);
            c.current_dir(dir);
            if !flags.is_empty() {
                c.args(*flags).arg(dir);
            }
            c
        })
        .collect()
}

/// How to run a Git hook script. Unix runs the file itself; Windows has no
/// shebang support, so hooks go through the `sh` that ships with Git for
/// Windows, like `git commit` does there.
pub fn hook_command(hook: &Path) -> Result<Command, String> {
    #[cfg(windows)]
    {
        let ext = hook.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "exe" | "bat" | "cmd") {
            return Ok(quiet_command(hook));
        }
        let sh = windows_sh().ok_or_else(|| {
            "This repository has Git hooks, which need Git for Windows (git-scm.com) to run. Install it, or remove the hook.".to_string()
        })?;
        let mut c = quiet_command(sh);
        c.arg(hook);
        Ok(c)
    }
    #[cfg(not(windows))]
    {
        Ok(Command::new(hook))
    }
}

/// `sh.exe` from Git for Windows: next to `git.exe` on the PATH
/// (`<Git>\cmd\git.exe` → `<Git>\bin\sh.exe`), or in the default install folders.
#[cfg(windows)]
fn windows_sh() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("sh.exe").is_file() {
                return Some(dir.join("sh.exe"));
            }
            if dir.join("git.exe").is_file() {
                if let Some(root) = dir.parent() {
                    roots.push(root.to_path_buf());
                }
            }
        }
    }
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(p) = std::env::var_os(var) {
            let base = PathBuf::from(p);
            roots.push(base.join("Git"));
            roots.push(base.join("Programs").join("Git"));
        }
    }
    roots.into_iter().map(|r| r.join("bin").join("sh.exe")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paths() {
        let p = native_path("C:/Users/me/code/repo");
        if cfg!(windows) {
            assert_eq!(p, PathBuf::from(r"C:\Users\me\code\repo"));
        } else {
            assert_eq!(p, PathBuf::from("C:/Users/me/code/repo"));
        }
    }

    #[test]
    fn links_are_not_paths() {
        assert!(is_link("https://github.com/o/r/pull/new/x"));
        assert!(is_link("mailto:someone@example.com"));
        assert!(!is_link("C:/Users/me/repo"));
        assert!(!is_link("/home/me/repo"));
        assert!(!is_link("C://odd"));
    }

    #[test]
    fn terminals_are_offered() {
        assert!(!terminal_candidates(Path::new(".")).is_empty());
    }
}
