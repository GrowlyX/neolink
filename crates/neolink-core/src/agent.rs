//! The official DisplayLink agent, as recovered from the 17.1.56 package.
//!
//! Neolink does not reimplement the USB transport or DL3 codec. Instead it can *supervise* the
//! vendor agent: detect it, start it, stop it, read its version, and talk to its Mach services.
//! This keeps behaviour bit-exact with what the hardware expects, while Neolink owns the UI,
//! persistence and layout logic (which the vendor app also exposes over XPC, see
//! `docs/RE-FINDINGS.md`).

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_PATH: &str = "/Applications/DisplayLink Manager.app";
pub const AGENT_BUNDLE_ID: &str = "com.displaylink.DisplayLinkUserAgent";
pub const XPC_V2: &str = "com.displaylink.XpcService.V2";

/// Mach services vended by `DisplayLinkXpcService`.
pub const XPC_SERVICES: [&str; 3] = [
    "com.displaylink.XpcService.V2",
    "com.displaylink.XpcService.internal",
    "com.displaylink.XpcService",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub installed: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub running: bool,
    pub xpc_running: bool,
}

fn version_of(app: &Path) -> Option<String> {
    let plist = app.join("Contents/Info.plist");
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

fn pgrep(pattern: &str) -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-q", pattern])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn path() -> Option<PathBuf> {
    let p = PathBuf::from(APP_PATH);
    p.exists().then_some(p)
}

pub fn installed() -> bool {
    path().is_some()
}

pub fn running() -> bool {
    pgrep("DisplayLinkUserAgent") || pgrep("DisplayLink Manager")
}

pub fn xpc_running() -> bool {
    pgrep("DisplayLinkXpcService")
}

/// True when the agent is installed *and* its Mach service answers. A `launchctl print` probe is
/// used because it works without entitlements, unlike `NSXPCConnection`.
pub fn reachable() -> bool {
    let out = Command::new("/bin/launchctl").arg("list").output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains("com.displaylink.XpcService")
        }
        Err(_) => false,
    }
}

pub fn info() -> Info {
    let p = path();
    Info {
        installed: p.is_some(),
        version: p.as_deref().and_then(version_of),
        path: p,
        running: running(),
        xpc_running: xpc_running(),
    }
}

/// Launch the vendor agent exactly the way its own postinstall does (`launchctl asuser` + `open`),
/// so it inherits the right session and Screen Recording identity.
pub fn start() -> Result<()> {
    if running() {
        return Ok(());
    }
    let app = path().ok_or_else(|| anyhow!("DisplayLink Manager is not installed at {APP_PATH}"))?;
    let uid = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    let status = Command::new("/bin/launchctl")
        .args(["asuser", &uid, "/usr/bin/open", "-g", "-a"])
        .arg(&app)
        .status()?;
    if !status.success() {
        return Err(anyhow!("launching {} failed", app.display()));
    }
    Ok(())
}

pub fn stop() -> Result<()> {
    for name in ["DisplayLinkUserAgent", "DisplayLink Manager", "DisplayLinkXpcService"] {
        let _ = Command::new("/usr/bin/pkill").args(["-TERM", "-x", name]).status();
    }
    Ok(())
}

/// Restart is only ever needed after an upgrade; it is here so the GUI has a single button.
pub fn restart() -> Result<()> {
    stop()?;
    std::thread::sleep(std::time::Duration::from_millis(600));
    start()
}

/// One display as reported by the vendor agent over XPC.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VendorDisplay {
    pub cgid: u32,
    pub name: String,
    pub persistent_display_id: String,
    pub usb_serial: String,
    pub rotated: bool,
    pub enabled: bool,
    pub flipped: bool,
    pub brightness: f64,
    pub contrast: f64,
    pub usb_vid: i32,
    pub usb_pid: i32,
}

#[repr(C)]
struct NsError {
    _priv: [u8; 0],
}

extern "C" {
    fn neolink_xpc_get_displays(err: *mut *mut NsError) -> *mut objc2::runtime::AnyObject;
    fn neolink_xpc_set(
        err: *mut *mut NsError,
        op: *const std::ffi::c_char,
        pid: *const std::ffi::c_char,
        value: f64,
    ) -> bool;
}

fn json_string(obj: *mut objc2::runtime::AnyObject) -> Option<Vec<VendorDisplay>> {
    if obj.is_null() {
        return None;
    }
    let bytes: *const u8 = unsafe { objc2::msg_send![obj, UTF8String] };
    let len: usize = unsafe { objc2::msg_send![obj, lengthOfBytesUsingEncoding: 4usize] };
    if bytes.is_null() {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    serde_json::from_slice(slice).ok()
}

/// Displays the vendor agent currently manages. Empty when the agent is absent or idle.
pub fn vendor_displays() -> Vec<VendorDisplay> {
    unsafe {
        let mut err: *mut NsError = std::ptr::null_mut();
        let arr = neolink_xpc_get_displays(&mut err);
        json_string(arr).unwrap_or_default()
    }
}

/// Ask the agent to change brightness / contrast / enabled / flip for one persistent id.
pub fn vendor_set(op: &str, persistent_id: &str, value: f64) -> bool {
    let op = std::ffi::CString::new(op).unwrap_or_default();
    let pid = std::ffi::CString::new(persistent_id).unwrap_or_default();
    unsafe {
        let mut err: *mut NsError = std::ptr::null_mut();
        neolink_xpc_set(&mut err, op.as_ptr(), pid.as_ptr(), value)
    }
}
