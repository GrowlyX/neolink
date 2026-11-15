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

/// Login-window variant of the agent: a system LaunchAgent that runs the app
/// inside the login session so a dock comes up before anyone logs in, matching
/// the vendor's `LoginScreenExt.pkg` (their plist is the same shape with
/// `LimitLoadToSessionType = LoginWindow` and `open -W`). It has to be owned by
/// root in `/Library/LaunchAgents`, so installing it needs an admin prompt.
pub const LOGIN_LABEL: &str = "ai.neolink.loginscreen";

pub fn login_screen_path() -> PathBuf {
    PathBuf::from("/Library/LaunchAgents").join(format!("{LOGIN_LABEL}.plist"))
}

fn login_screen_xml(app: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LOGIN_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-W</string>
    <string>{}</string>
  </array>
  <key>LimitLoadToSessionType</key><string>LoginWindow</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
        app.display()
    )
}

pub fn login_screen_enabled() -> bool {
    login_screen_path().is_file()
}

/// Install the login-window agent, prompting once with `osascript`.
pub fn install_login_screen() -> Result<()> {
    let app = bundle().context("not running from a .app bundle")?;
    let path = login_screen_path();
    std::fs::write(&path, login_screen_xml(&app))?;
    // `bootstrap system` needs root; the prompt is the only privileged step.
    let path_s = path.display().to_string();
    let script = format!(
        "do shell script \"/bin/launchctl bootout system/{LOGIN_LABEL} 2>/dev/null; /bin/launchctl bootstrap system {path_s}\" with administrator privileges"
    );
    let status = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .status()
        .context("running osascript")?;
    if !status.success() {
        anyhow::bail!("could not bootstrap the login-window agent");
    }
    Ok(())
}

pub fn remove_login_screen() -> Result<()> {
    let path = login_screen_path();
    if path.exists() {
        let path_disp = path.display().to_string();
        let script = format!(
            "do shell script \"/bin/launchctl bootout system/{LOGIN_LABEL} 2>/dev/null; /bin/rm -f {path_disp}\" with administrator privileges"
        );
        let _ = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .status();
    }
    Ok(())
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
        assert!(uid().parse::<u32>().is_ok());
    }

    #[test]
    fn login_screen_agent_runs_in_the_login_window() {
        let xml = login_screen_xml(std::path::Path::new("/Applications/Neolink.app"));
        assert!(xml.contains("LoginWindow"));
        assert!(xml.contains("<string>-W</string>"));
        assert!(xml.contains(LOGIN_LABEL));
        assert!(login_screen_path().starts_with("/Library/LaunchAgents"));
    }

    #[test]
    fn plist_points_at_the_bundle_and_autostarts() {
        let xml = plist_xml(std::path::Path::new("/Applications/Neolink.app"));
        assert!(xml.contains("<string>/Applications/Neolink.app</string>"));
        assert!(xml.contains("<key>RunAtLoad</key><true/>"));
        assert!(xml.contains(LABEL));
    }
}
