//! On-demand acquisition of the official DisplayLink Manager package.
//!
//! Neolink ships no Synaptics code or data. Instead it downloads the vendor
//! `.pkg` from Synaptics' own CDN, pins it by SHA-256, expands it into a cache
//! under Application Support, and (only when the user asks) hands the original
//! package to `installer`. That keeps the redistributable surface to our own
//! Rust code plus a URL and a hash, while the user still gets a genuine,
//! signature-validated Synaptics install.
//!
//! The hash matters: it is the only thing standing between "we fetched a URL"
//! and "we are about to ask for an admin password to install it". A mismatch
//! aborts before extraction, let alone installation.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Version this build's transport work was reverse engineered against.
pub const VERSION: &str = "17.1.56";
/// Official Synaptics download. Not mirrored or vendored.
pub const URL: &str = "https://www.synaptics.com/sites/default/files/exe_files/2026-11/DisplayLink%20Manager%20Graphics%20Connectivity17.1-EXE.pkg";
/// SHA-256 of that exact file, measured 2026-11-14 (10,981,424 bytes).
pub const SHA256: &str = "e6e8c86f722d05fc4b5bca22d9627df00dfd11dadffb472278b075fb5b66067e";
/// Byte length of that file, so download progress has a real denominator even though curl
/// is silent and the CDN may not send a length.
pub const SIZE: u64 = 10_981_424;

/// The pinned version and digest, for display in the UI.
pub fn pinned() -> (&'static str, &'static str) {
    (VERSION, SHA256)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Download,
    Verify,
    Expand,
    Install,
}

/// Progress in bytes. `total` is 0 while the server has not told us the size.
pub type Progress<'a> = &'a mut dyn FnMut(Stage, u64, u64);

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Status {
    pub installed_version: Option<String>,
    pub package_cached: bool,
    pub expanded: bool,
    pub cache_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    pub dir: PathBuf,
    pub package: PathBuf,
    pub expanded: PathBuf,
}

pub fn cache_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join("Library/Application Support/Neolink/vendor"))
}

pub fn paths() -> Result<Paths> {
    let dir = cache_root()?.join(VERSION);
    Ok(Paths {
        package: dir.join("DisplayLinkManager.pkg"),
        expanded: dir.join("expanded"),
        dir,
    })
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

pub fn status() -> Status {
    let p = match paths() {
        Ok(p) => p,
        Err(_) => return Status::default(),
    };
    let installed_version = super::agent::path().and_then(|app| super::agent::version_of(&app));
    Status {
        installed_version,
        package_cached: p.package.is_file(),
        expanded: p.expanded.join("Distribution").is_file(),
        cache_bytes: dir_size(&p.dir),
    }
}

/// Streaming SHA-256, so a multi-megabyte package never lands in memory whole.
fn sha256_file(path: &Path) -> Result<String> {
    let out = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .context("running shasum")?;
    if !out.status.success() {
        bail!(
            "shasum failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let digest = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if digest.len() != 64 {
        bail!("unexpected shasum output");
    }
    Ok(digest)
}

/// Download the pinned package and verify it. Re-uses a cached copy that already matches.
pub fn fetch(progress: Progress) -> Result<PathBuf> {
    let p = paths()?;
    std::fs::create_dir_all(&p.dir).context("creating vendor cache")?;

    if p.package.is_file() {
        progress(Stage::Verify, 0, 0);
        if sha256_file(&p.package)?.eq_ignore_ascii_case(SHA256) {
            return Ok(p.package);
        }
        // Corrupt or half-written from an earlier run; start over rather than guess.
        let _ = std::fs::remove_file(&p.package);
    }

    let partial = p.dir.join("download.part");
    let _ = std::fs::remove_file(&partial);
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--retry-delay",
            "2",
            "--connect-timeout",
            "20",
            "--output",
        ])
        .arg(&partial)
        .arg(URL)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning curl")?;

    // Poll the growing file so the UI can move; curl itself stays silent.
    let mut last = 0u64;
    loop {
        match child.try_wait()? {
            Some(status) => {
                if !status.success() {
                    let mut msg = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut msg);
                    }
                    let _ = std::fs::remove_file(&partial);
                    bail!("download failed: {}", msg.trim());
                }
                break;
            }
            None => {
                let got = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
                if got != last {
                    last = got;
                    progress(Stage::Download, got, SIZE.max(got));
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }

    let got = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    progress(Stage::Download, got, got);

    progress(Stage::Verify, got, got);
    let digest = sha256_file(&partial)?;
    if !digest.eq_ignore_ascii_case(SHA256) {
        let _ = std::fs::remove_file(&partial);
        bail!("package checksum mismatch\n  expected {SHA256}\n  got      {digest}");
    }

    std::fs::rename(&partial, &p.package).context("moving download into cache")?;
    Ok(p.package)
}

/// Expand the package so its inner bundles and `.spkg` blobs can be inspected.
pub fn expand(progress: Progress) -> Result<PathBuf> {
    let p = paths()?;
    let package = fetch(progress)?;
    if p.expanded.join("Distribution").is_file() {
        return Ok(p.expanded);
    }
    let _ = std::fs::remove_dir_all(&p.expanded);
    progress(Stage::Expand, 0, 0);
    let out = Command::new("/usr/sbin/pkgutil")
        .arg("--expand-full")
        .arg(&package)
        .arg(&p.expanded)
        .output()
        .context("running pkgutil")?;
    if !out.status.success() {
        bail!(
            "pkgutil failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(p.expanded)
}

/// Where the vendor app lives inside the expanded payload, if it was expanded.
pub fn expanded_app() -> Option<PathBuf> {
    let p = paths().ok()?;
    let app = p
        .expanded
        .join("DisplayLinkManagerApp.pkg/Payload/DisplayLink Manager.app");
    app.is_dir().then_some(app)
}

/// The per-dock firmware/EDID packs shipped inside the vendor app.
pub fn resource_packs() -> Result<Vec<PathBuf>> {
    let app = expanded_app().ok_or_else(|| anyhow!("package is not expanded yet"))?;
    let dir = app.join("Contents/Resources");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "spkg"))
        .collect();
    out.sort();
    Ok(out)
}

/// Install the verified package with the system installer, in one admin prompt.
///
/// `installer` is the same tool the vendor's own distribution script ends up in,
/// so signature checks, receipts and the login-screen extension are all handled
/// by macOS rather than by us.
pub fn install() -> Result<()> {
    let p = paths()?;
    if !p.package.is_file() {
        bail!("package is not cached yet");
    }
    let cmd = format!(
        "/usr/sbin/installer -pkg {} -target /",
        shell_quote(&p.package)
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .context("running osascript")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "installer failed: {}\nrun manually: sudo {}",
            err.trim(),
            cmd
        );
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Fetch, expand, install. Expansion is not required for installation but keeps
/// the firmware packs handy for the transport layer.
pub fn provision(progress: Progress) -> Result<()> {
    expand(progress)?;
    progress(Stage::Install, 0, 0);
    install()
}

/// Drop the cache (the installed app itself is left alone).
pub fn purge() -> Result<u64> {
    let root = cache_root()?;
    let freed = dir_size(&root);
    if root.exists() {
        std::fs::remove_dir_all(&root).context("removing vendor cache")?;
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_a_sha256_and_a_version() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(SHA256.to_ascii_lowercase(), SHA256);
        assert!(VERSION.split('.').count() == 3);
        assert!(URL.starts_with("https://"));
    }

    #[test]
    fn cache_lives_under_application_support() {
        let p = paths().unwrap();
        assert!(p.dir.ends_with("Neolink/vendor/17.1.56"));
        assert!(p.package.ends_with("DisplayLinkManager.pkg"));
        assert!(p.expanded.ends_with("expanded"));
    }

    #[test]
    fn quoting_survives_apostrophes() {
        assert_eq!(shell_quote(Path::new("/tmp/a b/c.pkg")), "'/tmp/a b/c.pkg'");
        assert_eq!(
            shell_quote(Path::new("/tmp/it's.pkg")),
            "'/tmp/it'\\''s.pkg'"
        );
    }
}
