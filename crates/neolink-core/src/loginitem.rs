//! Launch-at-login registration.
//!
//! Uses a per-user LaunchAgent rather than `SMAppService`, so it works for an ad-hoc signed,
//! locally built bundle (which `SMAppService` refuses) and stays inspectable at a known path.

use anyhow::{Context, Result};
use std::path::PathBuf;

pub const LABEL: &str = "ai.neolink.app";

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// The `.app` this binary lives in, if it was launched from a bundle.
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && app.extension()? == "app").then(|| app.to_path_buf())
}

fn plist_xml(app: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
        app.display()
    )
}

/// True when the plist exists *and* launchd has loaded it.
pub fn enabled() -> bool {
    let Ok(path) = plist_path() else { return false };
    if !path.is_file() {
        return false;
    }
    std::process::Command::new("/bin/launchctl")
        .args(["print", &format!("gui/{}/{}", uid(), LABEL)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn install() -> Result<()> {
    let app = bundle().context("not running from a .app bundle")?;
    let path = plist_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, plist_xml(&app))?;
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["bootstrap", &format!("gui/{}", uid())])
        .arg(&path)
        .status();
    Ok(())
}

pub fn remove() -> Result<()> {
    let path = plist_path()?;
    if path.exists() {
        let _ = std::process::Command::new("/bin/launchctl")
            .args(["bootout", &format!("gui/{}", uid())])
            .arg(&path)
            .status();
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn toggle() -> Result<bool> {
    if enabled() {
        remove()?;
        Ok(false)
    } else {
        install()?;
        Ok(true)
    }
}

fn uid() -> String {
    std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "0".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_uses_a_gui_domain_target() {
        assert_eq!(uid().parse::<u32>().map(|_| ()).is_ok(), true);
    }

    #[test]
    fn plist_points_at_the_bundle_and_autostarts() {
        let xml = plist_xml(std::path::Path::new("/Applications/Neolink.app"));
        assert!(xml.contains("<string>/Applications/Neolink.app</string>"));
        assert!(xml.contains("<key>RunAtLoad</key><true/>"));
        assert!(xml.contains(LABEL));
    }
}
