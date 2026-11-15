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

pub fn version_of(app: &Path) -> Option<String> {
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

/// How long a positive reachability answer is trusted. The UI ticks faster than the ping can
/// complete, so without this every refresh would burn 700 ms on the main thread.
const REACHABLE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

fn reachable_cache() -> &'static std::sync::Mutex<Option<(std::time::Instant, bool)>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(std::time::Instant, bool)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Forget the cached answer, e.g. after starting or stopping the agent.
pub fn invalidate() {
    if let Ok(mut c) = reachable_cache().lock() {
        *c = None;
    }
}

/// True only when the Mach service actually answers `getDisplays`. A `launchctl list` grep proves
/// the label exists, not that the service is up or that the reply decodes, so the shim pings it
/// with a 700 ms budget.
///
/// Positive answers are cached for `REACHABLE_TTL`; negative ones are not, so a service that
/// comes up is picked up on the next call. Callers on a hot path should still prefer caching the
/// result themselves.
pub fn reachable() -> bool {
    if !installed() {
        return false;
    }
    if let Ok(cache) = reachable_cache().lock() {
        if let Some((at, true)) = *cache {
            if at.elapsed() < REACHABLE_TTL {
                return true;
            }
        }
    }
    let answered = unsafe { neolink_xpc_ping() };
    if answered {
        if let Ok(mut cache) = reachable_cache().lock() {
            *cache = Some((std::time::Instant::now(), true));
        }
    }
    answered
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
    let app =
        path().ok_or_else(|| anyhow!("DisplayLink Manager is not installed at {APP_PATH}"))?;
    let uid = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    let status = Command::new("/bin/launchctl")
        .args(["asuser", &uid, "/usr/bin/open", "-g", "-a"])
        .arg(&app)
        .status()?;
    if !status.success() {
        return Err(anyhow!("launching {} failed", app.display()));
    }
    invalidate();
    Ok(())
}

pub fn stop() -> Result<()> {
    for name in [
        "DisplayLinkUserAgent",
        "DisplayLink Manager",
        "DisplayLinkXpcService",
    ] {
        let _ = Command::new("/usr/bin/pkill")
            .args(["-TERM", "-x", name])
            .status();
    }
    invalidate();
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
    fn neolink_xpc_ping() -> bool;
    fn neolink_xpc_start_notifications(cb: extern "C" fn(*const std::ffi::c_char)) -> bool;
    fn neolink_xpc_stop_notifications();
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

/// Snapshot cache for `vendor_displays`. The call blocks for up to 2 s, which is longer than the
/// UI's tick, so it must not run on every frame.
const DISPLAYS_TTL: std::time::Duration = std::time::Duration::from_millis(1000);

type DisplayCache = std::sync::Mutex<Option<(std::time::Instant, Vec<VendorDisplay>)>>;
type NotifySink = std::sync::Mutex<Option<Box<dyn FnMut(Notification) + Send>>>;

fn displays_cache() -> &'static DisplayCache {
    static CACHE: std::sync::OnceLock<DisplayCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Forget the cached display list, e.g. right after a setter or a notification.
pub fn invalidate_displays() {
    if let Ok(mut c) = displays_cache().lock() {
        *c = None;
    }
}

/// Displays the vendor agent currently manages. Empty when the agent is absent or idle.
///
/// Cached for `DISPLAYS_TTL`; `force` bypasses the cache.
pub fn vendor_displays_cached(force: bool) -> Vec<VendorDisplay> {
    if !force {
        if let Ok(cache) = displays_cache().lock() {
            if let Some((at, list)) = cache.as_ref() {
                if at.elapsed() < DISPLAYS_TTL {
                    return list.clone();
                }
            }
        }
    }
    let list = unsafe {
        let mut err: *mut NsError = std::ptr::null_mut();
        let arr = neolink_xpc_get_displays(&mut err);
        json_string(arr).unwrap_or_default()
    };
    if let Ok(mut cache) = displays_cache().lock() {
        *cache = Some((std::time::Instant::now(), list.clone()));
    }
    list
}

pub fn vendor_displays() -> Vec<VendorDisplay> {
    vendor_displays_cached(false)
}

/// Ask the agent to change brightness / contrast / enabled / flip for one persistent id.
pub fn vendor_set(op: &str, persistent_id: &str, value: f64) -> bool {
    let op = std::ffi::CString::new(op).unwrap_or_default();
    let pid = std::ffi::CString::new(persistent_id).unwrap_or_default();
    let ok = unsafe {
        let mut err: *mut NsError = std::ptr::null_mut();
        neolink_xpc_set(&mut err, op.as_ptr(), pid.as_ptr(), value)
    };
    invalidate_displays();
    ok
}

/// One agent-initiated change. Mirrors the four notification selectors.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Notification {
    Displays { displays: Vec<VendorDisplay> },
    Brightness { pid: String, value: f64 },
    Contrast { pid: String, value: f64 },
    Flip { pid: String, value: f64 },
}

fn handle_notification(json: &str) -> Option<Notification> {
    serde_json::from_str(json).ok()
}

extern "C" fn notification_trampoline(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    if let Some(n) = handle_notification(&text) {
        if let Ok(mut sink) = NOTIFY_SINK.lock() {
            if let Some(cb) = sink.as_mut() {
                cb(n);
            }
        }
    }
}

static NOTIFY_SINK: NotifySink = std::sync::Mutex::new(None);

/// Subscribe to vendor change notifications. The callback runs on an XPC queue, so it must be
/// cheap and non-blocking; hand work to a channel rather than calling into UI state.
///
/// Returns false when the service never acknowledged, in which case polling is still required.
pub fn subscribe(cb: impl FnMut(Notification) + Send + 'static) -> bool {
    if let Ok(mut sink) = NOTIFY_SINK.lock() {
        *sink = Some(Box::new(cb));
    }
    unsafe { neolink_xpc_start_notifications(notification_trampoline) }
}

pub fn unsubscribe() {
    unsafe { neolink_xpc_stop_notifications() }
    if let Ok(mut sink) = NOTIFY_SINK.lock() {
        *sink = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(pid: &str) -> VendorDisplay {
        VendorDisplay {
            cgid: 1,
            name: "Dock".into(),
            persistent_display_id: pid.into(),
            usb_serial: "S".into(),
            rotated: false,
            enabled: true,
            flipped: false,
            brightness: 0.5,
            contrast: 0.5,
            usb_vid: 0x17e9,
            usb_pid: 0x6006,
        }
    }

    #[test]
    fn notification_json_maps_to_variants() {
        let n = handle_notification(r#"{"event":"brightness","pid":"p1","value":0.25}"#).unwrap();
        assert_eq!(
            n,
            Notification::Brightness {
                pid: "p1".into(),
                value: 0.25
            }
        );
        let n = handle_notification(r#"{"event":"flip","pid":"p1","value":1}"#).unwrap();
        assert_eq!(
            n,
            Notification::Flip {
                pid: "p1".into(),
                value: 1.0
            }
        );
        assert!(handle_notification(r#"{"event":"displays","displays":[]}"#).is_some());
        assert!(handle_notification("nonsense").is_none());
    }

    #[test]
    fn display_rows_roundtrip_through_serde() {
        let row = display("abc");
        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(serde_json::from_str::<VendorDisplay>(&json).unwrap(), row);
    }

    #[test]
    fn invalidate_clears_the_display_cache() {
        // Seed the cache directly, then confirm the invalidation path empties it.
        if let Ok(mut c) = displays_cache().lock() {
            *c = Some((std::time::Instant::now(), vec![display("x")]));
        }
        invalidate_displays();
        assert!(displays_cache().lock().unwrap().is_none());
    }
}
