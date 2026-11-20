//! Signing commits with GPG, SSH or X.509 keys, and checking signatures,
//! the way git does: the same config keys (`commit.gpgsign`, `gpg.format`,
//! `user.signingkey`, `gpg.program`, `gpg.ssh.program`,
//! `gpg.ssh.allowedSignersFile`) and the same external programs.

use super::{err, open, Error, Result};
use crate::platform::quiet_command;
use git2::{Commit, Oid, Repository, Signature, Tree};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Gpg,
    Ssh,
    X509,
}

impl Format {
    fn parse(s: &str) -> Format {
        match s {
            "ssh" => Format::Ssh,
            "x509" => Format::X509,
            _ => Format::Gpg,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Format::Gpg => "openpgp",
            Format::Ssh => "ssh",
            Format::X509 => "x509",
        }
    }
}

fn get(cfg: &git2::Config, key: &str) -> Option<String> {
    cfg.get_string(key).ok().filter(|s| !s.trim().is_empty())
}

/// `~/` at the start of a config path means the home folder, as in git.
fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

fn program(cfg: &git2::Config, f: Format) -> String {
    match f {
        Format::Gpg => get(cfg, "gpg.openpgp.program").or_else(|| get(cfg, "gpg.program")).unwrap_or_else(|| "gpg".into()),
        Format::X509 => get(cfg, "gpg.x509.program").unwrap_or_else(|| "gpgsm".into()),
        Format::Ssh => get(cfg, "gpg.ssh.program").unwrap_or_else(|| "ssh-keygen".into()),
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SigningSettings {
    /// commit.gpgsign
    pub enabled: bool,
    /// "openpgp", "ssh" or "x509"
    pub format: String,
    /// user.signingkey (empty: git's default for the format)
    pub key: String,
    /// Where commit.gpgsign is set: "local", "global" or "" (unset).
    pub scope: String,
    pub gpg_available: bool,
    pub ssh_available: bool,
    /// gpg.ssh.allowedSignersFile, which SSH verification needs.
    pub allowed_signers: String,
}

fn runs(program: &str, arg: &str) -> bool {
    quiet_command(program).arg(arg).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

pub fn settings(repo_path: &str) -> Result<SigningSettings> {
    let repo = open(repo_path)?;
    let cfg = repo.config()?.snapshot()?;
    let enabled = cfg.get_bool("commit.gpgsign").unwrap_or(false);
    let local = repo.config()?.open_level(git2::ConfigLevel::Local).ok();
    let scope = match local.as_ref().map(|l| l.get_bool("commit.gpgsign").is_ok()) {
        Some(true) => "local",
        _ if cfg.get_bool("commit.gpgsign").is_ok() => "global",
        _ => "",
    };
    let format = Format::parse(&get(&cfg, "gpg.format").unwrap_or_default());
    Ok(SigningSettings {
        enabled,
        format: format.name().into(),
        key: get(&cfg, "user.signingkey").unwrap_or_default(),
        scope: scope.into(),
        gpg_available: runs(&program(&cfg, Format::Gpg), "--version"),
        ssh_available: runs(&program(&cfg, Format::Ssh), "-?") ,
        allowed_signers: get(&cfg, "gpg.ssh.allowedSignersFile").unwrap_or_default(),
    })
}

/// Turn signing on or off, for this repository or for all of them (global).
pub fn set_settings(repo_path: &str, enabled: bool, format: &str, key: &str, global: bool) -> Result<()> {
    let repo = open(repo_path)?;
    let mut cfg = if global {
        git2::Config::open_default()?.open_level(git2::ConfigLevel::Global)?
    } else {
        repo.config()?.open_level(git2::ConfigLevel::Local)?
    };
    cfg.set_bool("commit.gpgsign", enabled)?;
    if enabled {
        let f = match format { "ssh" | "x509" => format, _ => "openpgp" };
        cfg.set_str("gpg.format", f)?;
        if key.trim().is_empty() {
            let _ = cfg.remove("user.signingkey");
        } else {
            cfg.set_str("user.signingkey", key.trim())?;
        }
    }
    // A repository setting overrides the global one; drop it when going global.
    if global {
        if let Ok(mut local) = repo.config()?.open_level(git2::ConfigLevel::Local) {
            let _ = local.remove("commit.gpgsign");
        }
    }
    Ok(())
}

/// Sign `data` per the repository's config; returns the armored signature.
fn sign_buffer(repo: &Repository, data: &str, committer: &Signature) -> Result<String> {
    let cfg = repo.config()?.snapshot()?;
    let format = Format::parse(&get(&cfg, "gpg.format").unwrap_or_default());
    let key = get(&cfg, "user.signingkey");
    let prog = program(&cfg, format);
    match format {
        Format::Gpg | Format::X509 => {
            // Like git: the signing key defaults to the committer's identity.
            let who = key.unwrap_or_else(|| format!("{} <{}>", committer.name().unwrap_or(""), committer.email().unwrap_or("")));
            let mut child = quiet_command(&prog)
                .args(["--status-fd=2", "-bsau", &who])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| Error::Msg(format!("Couldn't run {prog} to sign the commit: {e}. Install GnuPG or turn signing off in Preferences → Commits.")))?;
            child.stdin.take().unwrap().write_all(data.as_bytes())?;
            let out = child.wait_with_output()?;
            let status = String::from_utf8_lossy(&out.stderr);
            if !out.status.success() || !status.contains("SIG_CREATED ") {
                let why = status.lines().filter(|l| !l.starts_with("[GNUPG:]")).collect::<Vec<_>>().join(" ");
                return err(format!("Signing the commit with {prog} failed: {}", why.trim()));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Format::Ssh => {
            let Some(key) = key else {
                return err("Set the SSH key to sign with (user.signingkey) in Preferences → Commits.");
            };
            let dir = tempfile::tempdir()?;
            // A literal public key ("ssh-ed25519 AAAA…" or "key::…") signs through ssh-agent.
            let literal = key.strip_prefix("key::").map(str::to_string).or_else(|| key.starts_with("ssh-").then(|| key.clone()));
            let mut cmd = quiet_command(&prog);
            cmd.args(["-Y", "sign", "-n", "git", "-f"]);
            match literal {
                Some(k) => {
                    let f = dir.path().join("key.pub");
                    std::fs::write(&f, format!("{k}\n"))?;
                    cmd.arg(f).arg("-U");
                }
                None => {
                    cmd.arg(expand(&key));
                }
            }
            let buf = dir.path().join("commit");
            std::fs::write(&buf, data)?;
            let out = cmd
                .arg(&buf)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| Error::Msg(format!("Couldn't run {prog} to sign the commit: {e}.")))?;
            if !out.status.success() {
                let why = String::from_utf8_lossy(&out.stderr);
                let hint = if why.contains("passphrase") || why.contains("agent") {
                    " If the key has a passphrase, add it to ssh-agent first (ssh-add)."
                } else {
                    ""
                };
                return err(format!("Signing the commit with SSH failed: {}{hint}", why.trim()));
            }
            Ok(std::fs::read_to_string(dir.path().join("commit.sig"))?)
        }
    }
}

/// Whether new commits should be signed: an explicit choice, or commit.gpgsign.
pub fn wanted(repo: &Repository, choice: Option<bool>) -> bool {
    choice.unwrap_or_else(|| repo.config().and_then(|c| c.get_bool("commit.gpgsign")).unwrap_or(false))
}

/// Point HEAD (or the branch it's on) at `new`, with a reflog message.
pub fn update_head(repo: &Repository, new: Oid, reflog: &str) -> Result<()> {
    let head = repo.find_reference("HEAD")?;
    match head.symbolic_target()? {
        Some(target) => {
            let target = target.to_string();
            repo.reference(&target, new, true, reflog)?;
        }
        None => repo.set_head_detached(new)?,
    }
    Ok(())
}

/// Create a commit, signed when `sign` is true. Like `Repository::commit`,
/// `update_head` moves HEAD's branch to it.
#[allow(clippy::too_many_arguments)]
pub fn create(
    repo: &Repository,
    update_head_with: Option<&str>,
    author: &Signature,
    committer: &Signature,
    msg: &str,
    tree: &Tree,
    parents: &[&Commit],
    sign: bool,
) -> Result<Oid> {
    if !sign {
        return Ok(repo.commit(update_head_with.map(|_| "HEAD"), author, committer, msg, tree, parents)?);
    }
    let buf = repo.commit_create_buffer(author, committer, msg, tree, parents)?;
    let data = buf.as_str().map_err(|_| Error::Msg("The commit isn't valid UTF-8 and can't be signed.".into()))?.to_string();
    let signature = sign_buffer(repo, &data, committer)?;
    let id = repo.commit_signed(&data, &signature, Some("gpgsig"))?;
    if let Some(reflog) = update_head_with {
        update_head(repo, id, reflog)?;
    }
    Ok(id)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    /// "openpgp", "ssh" or "x509"
    pub format: String,
    /// "good": valid and the key is known and trusted;
    /// "unverified": valid, but the signer can't be confirmed (unknown or untrusted key, no allowed-signers file);
    /// "bad": the signature doesn't match the commit;
    /// "error": it couldn't be checked (program missing, for example).
    pub status: String,
    pub signer: Option<String>,
    pub key: Option<String>,
    pub detail: String,
}

/// Check a commit's signature; None when it isn't signed.
pub fn verify(repo_path: &str, id: &str) -> Result<Option<Verification>> {
    let own = git2::Config::open_default().and_then(|mut c| c.snapshot())?;
    verify_with(repo_path, id, &own)
}

/// `verify`, with `own` standing for your global and system git settings.
#[doc(hidden)]
pub fn verify_with(repo_path: &str, id: &str, own: &git2::Config) -> Result<Option<Verification>> {
    let repo = open(repo_path)?;
    let oid = super::oid(id)?;
    let Ok((sig, data)) = repo.extract_signature(&oid, None) else { return Ok(None) };
    let sig = String::from_utf8_lossy(&sig).into_owned();
    let cfg = repo.config()?.snapshot()?;
    let format = if sig.contains("BEGIN SSH SIGNATURE") {
        Format::Ssh
    } else if sig.contains("BEGIN SIGNED MESSAGE") {
        Format::X509
    } else {
        Format::Gpg
    };
    // Checking runs on its own whenever a signed commit is shown, so the
    // program comes from your own (global or system) settings only: a
    // repository's config can't make Arbor run something just by being
    // looked at. One that names a different program isn't checked.
    let prog = program(own, format);
    let wanted = program(&cfg, format);
    if wanted != prog {
        return Ok(Some(Verification {
            format: format.name().into(),
            status: "error".into(),
            signer: None,
            key: None,
            detail: format!(
                "Not checked: this repository's settings name their own program for checking signatures ({wanted}), and Arbor only runs the one from your own git settings ({prog}). Run `git verify-commit` to check it yourself."
            ),
        }));
    }
    let dir = tempfile::tempdir()?;
    let sig_file = dir.path().join("commit.sig");
    std::fs::write(&sig_file, &sig)?;
    let failed = |e: std::io::Error| Verification {
        format: format.name().into(),
        status: "error".into(),
        signer: None,
        key: None,
        detail: format!("Couldn't run {prog} to check the signature: {e}"),
    };

    let run = |cmd: &mut std::process::Command| -> std::io::Result<std::process::Output> {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        child.stdin.take().unwrap().write_all(&data)?;
        child.wait_with_output()
    };

    Ok(Some(match format {
        Format::Gpg | Format::X509 => {
            let out = match run(quiet_command(&prog).args(["--status-fd=1", "--verify"]).arg(&sig_file).arg("-")) {
                Ok(o) => o,
                Err(e) => return Ok(Some(failed(e))),
            };
            let status = String::from_utf8_lossy(&out.stdout);
            let field = |tag: &str| status.lines().find_map(|l| l.strip_prefix(&format!("[GNUPG:] {tag} ")).map(str::to_string));
            let (state, signer, key) = if let Some(g) = field("GOODSIG") {
                let (k, who) = g.split_once(' ').unwrap_or((&g, ""));
                let trusted = status.contains("TRUST_FULLY") || status.contains("TRUST_ULTIMATE");
                (if trusted { "good" } else { "unverified" }, Some(who.to_string()), Some(k.to_string()))
            } else if let Some(b) = field("BADSIG") {
                let (k, who) = b.split_once(' ').unwrap_or((&b, ""));
                ("bad", Some(who.to_string()), Some(k.to_string()))
            } else if let Some(e) = field("ERRSIG") {
                let k = e.split(' ').next().unwrap_or("").to_string();
                ("unverified", None, Some(k))
            } else {
                ("error", None, None)
            };
            let detail = match state {
                "good" => "Valid signature from a trusted key.".to_string(),
                "unverified" if signer.is_some() => "Valid signature, but the key isn't marked as trusted in your keyring.".into(),
                "unverified" => "Signed with a key that isn't in your keyring.".into(),
                "bad" => "The signature doesn't match this commit.".into(),
                _ => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            };
            Verification { format: format.name().into(), status: state.into(), signer, key, detail }
        }
        Format::Ssh => {
            let allowed = get(&cfg, "gpg.ssh.allowedSignersFile").map(|p| expand(&p)).filter(|p| p.exists());
            // Who signed it, according to the allowed-signers file.
            let principal = allowed.as_ref().and_then(|a| {
                quiet_command(&prog)
                    .args(["-Y", "find-principals", "-f"])
                    .arg(a)
                    .arg("-s")
                    .arg(&sig_file)
                    .stdin(Stdio::null())
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string))
            });
            let out = match (&allowed, &principal) {
                (Some(a), Some(p)) => run(quiet_command(&prog).args(["-Y", "verify", "-n", "git", "-f"]).arg(a).args(["-I", p, "-s"]).arg(&sig_file)),
                _ => run(quiet_command(&prog).args(["-Y", "check-novalidate", "-n", "git", "-s"]).arg(&sig_file)),
            };
            let out = match out {
                Ok(o) => o,
                Err(e) => return Ok(Some(failed(e))),
            };
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            let key = text.split_whitespace().find(|w| w.starts_with("SHA256:")).map(str::to_string);
            let (state, detail) = if !out.status.success() {
                let why = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                // ssh-keygen says so when the signature doesn't match; any
                // other failure (a signing-only program, a broken setup)
                // means it couldn't be checked, not that it's forged.
                let lower = text.to_lowercase();
                if lower.contains("incorrect signature") || lower.contains("verification failed") || lower.contains("could not verify") {
                    ("bad", format!("The signature doesn't match this commit. {why}").trim().to_string())
                } else {
                    ("error", format!("Couldn't check the signature with {prog}. {why}").trim().to_string())
                }
            } else if principal.is_some() {
                ("good", "Valid signature from a key in your allowed signers file.".into())
            } else if allowed.is_some() {
                ("unverified", "Valid signature, but the key isn't in your allowed signers file.".into())
            } else {
                ("unverified", "Valid signature. Set gpg.ssh.allowedSignersFile to check who made it.".into())
            };
            Verification { format: "ssh".into(), status: state.into(), signer: principal, key, detail }
        }
    }))
}
