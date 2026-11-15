//! Display enumeration and reconfiguration on top of CoreGraphics (Quartz Display Services).
//!
//! Everything here works on any online display, regardless of what drives it: the built-in
//! panel, a native DisplayPort/HDMI output, or a virtual display backed by a USB graphics agent.

use anyhow::{anyhow, bail, Result};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;

pub type DisplayId = u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGSize {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type CGDisplayModeRef = *mut c_void;
type CGDisplayConfigRef = *mut c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFUUIDRef = *const c_void;
type CFStringRef = *const c_void;
type CGImageRef = *const c_void;
type CGDataProviderRef = *const c_void;
type CFDataRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetOnlineDisplayList(max: u32, displays: *mut DisplayId, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> DisplayId;
    fn CGDisplayBounds(d: DisplayId) -> CGRect;
    fn CGDisplayIsBuiltin(d: DisplayId) -> u32;
    fn CGDisplayIsActive(d: DisplayId) -> u32;
    fn CGDisplayMirrorsDisplay(d: DisplayId) -> DisplayId;
    fn CGDisplayVendorNumber(d: DisplayId) -> u32;
    fn CGDisplayModelNumber(d: DisplayId) -> u32;
    fn CGDisplaySerialNumber(d: DisplayId) -> u32;
    fn CGDisplayRotation(d: DisplayId) -> f64;
    fn CGDisplayScreenSize(d: DisplayId) -> CGSize;
    fn CGDisplayCopyDisplayMode(d: DisplayId) -> CGDisplayModeRef;
    fn CGDisplayCopyAllDisplayModes(d: DisplayId, options: CFDictionaryRef) -> CFArrayRef;
    fn CGDisplayModeRelease(m: CGDisplayModeRef);
    fn CGDisplayModeGetWidth(m: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetHeight(m: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetPixelWidth(m: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetPixelHeight(m: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetRefreshRate(m: CGDisplayModeRef) -> f64;
    fn CGDisplayModeGetIODisplayModeID(m: CGDisplayModeRef) -> i32;
    fn CGDisplayModeIsUsableForDesktopGUI(m: CGDisplayModeRef) -> bool;
    fn CGBeginDisplayConfiguration(cfg: *mut CGDisplayConfigRef) -> i32;
    fn CGConfigureDisplayOrigin(cfg: CGDisplayConfigRef, d: DisplayId, x: i32, y: i32) -> i32;
    fn CGConfigureDisplayWithDisplayMode(
        cfg: CGDisplayConfigRef,
        d: DisplayId,
        mode: CGDisplayModeRef,
        options: CFDictionaryRef,
    ) -> i32;
    fn CGConfigureDisplayMirrorOfDisplay(
        cfg: CGDisplayConfigRef,
        d: DisplayId,
        master: DisplayId,
    ) -> i32;
    fn CGCompleteDisplayConfiguration(cfg: CGDisplayConfigRef, option: u32) -> i32;
    fn CGCancelDisplayConfiguration(cfg: CGDisplayConfigRef) -> i32;
    fn CGDisplayRegisterReconfigurationCallback(
        cb: extern "C" fn(DisplayId, u32, *mut c_void),
        user: *mut c_void,
    ) -> i32;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGDisplayCreateImage(d: DisplayId) -> CGImageRef;
    fn CGImageGetWidth(img: CGImageRef) -> usize;
    fn CGImageGetHeight(img: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(img: CGImageRef) -> usize;
    fn CGImageGetDataProvider(img: CGImageRef) -> CGDataProviderRef;
    fn CGImageRelease(img: CGImageRef);
    fn CGDataProviderCopyData(p: CGDataProviderRef) -> CFDataRef;
    fn CFDataGetBytePtr(d: CFDataRef) -> *const u8;
    fn CFDataGetLength(d: CFDataRef) -> isize;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGDisplayCreateUUIDFromDisplayID(d: DisplayId) -> CFUUIDRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(a: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFArrayRef, i: isize) -> *const c_void;
    fn CFRelease(o: *const c_void);
    fn CFUUIDCreateString(alloc: *const c_void, uuid: CFUUIDRef) -> CFStringRef;
}

const K_CG_NULL_DISPLAY: DisplayId = 0;
const CONFIGURE_FOR_SESSION: u32 = 1;
const CONFIGURE_PERMANENTLY: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    /// IOKit display mode id; unique per display.
    pub id: i32,
    /// Size in points (what the desktop sees).
    pub width: u32,
    pub height: u32,
    /// Backing size in pixels.
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub refresh: f64,
}

impl Mode {
    pub fn hidpi(&self) -> bool {
        self.pixel_width > self.width
    }
    pub fn label(&self) -> String {
        let hz = if self.refresh > 0.0 {
            format!(" @ {:.0} Hz", self.refresh)
        } else {
            String::new()
        };
        let hi = if self.hidpi() { " HiDPI" } else { "" };
        format!("{} × {}{}{}", self.width, self.height, hz, hi)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    pub id: DisplayId,
    /// Stable across reconnects and reboots; the key used by saved profiles.
    pub uuid: String,
    pub name: String,
    pub vendor: u32,
    pub model: u32,
    pub serial: u32,
    pub builtin: bool,
    pub main: bool,
    pub active: bool,
    pub mirror_of: Option<DisplayId>,
    /// Position and size in the global desktop coordinate space (points, y down).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rotation: f64,
    pub size_mm: (f64, f64),
    pub mode: Option<Mode>,
}

unsafe fn mode_from_ref(m: CGDisplayModeRef) -> Mode {
    Mode {
        id: CGDisplayModeGetIODisplayModeID(m),
        width: CGDisplayModeGetWidth(m) as u32,
        height: CGDisplayModeGetHeight(m) as u32,
        pixel_width: CGDisplayModeGetPixelWidth(m) as u32,
        pixel_height: CGDisplayModeGetPixelHeight(m) as u32,
        refresh: CGDisplayModeGetRefreshRate(m),
    }
}

fn online_ids() -> Vec<DisplayId> {
    let mut ids = [0u32; 32];
    let mut n = 0u32;
    unsafe { CGGetOnlineDisplayList(32, ids.as_mut_ptr(), &mut n) };
    ids[..n as usize].to_vec()
}

fn uuid_of(id: DisplayId) -> String {
    unsafe {
        let u = CGDisplayCreateUUIDFromDisplayID(id);
        if u.is_null() {
            return format!("display-{id}");
        }
        let s = CFUUIDCreateString(std::ptr::null(), u);
        CFRelease(u);
        if s.is_null() {
            return format!("display-{id}");
        }
        CFString::wrap_under_create_rule(s as _).to_string()
    }
}

/// Localised product name. `CoreDisplay_DisplayCreateInfoDictionary` is the only source that also
/// covers mirrored and inactive displays; it is resolved at runtime so a future OS that drops it
/// degrades to a generic name instead of failing to launch.
fn name_of(id: DisplayId, builtin: bool) -> String {
    unsafe {
        type InfoFn = unsafe extern "C" fn(DisplayId) -> CFDictionaryRef;
        static LIB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let sym = *LIB.get_or_init(|| {
            let h = libc::dlopen(
                c"/System/Library/Frameworks/CoreDisplay.framework/CoreDisplay".as_ptr(),
                libc::RTLD_LAZY,
            );
            if h.is_null() {
                return 0;
            }
            libc::dlsym(h, c"CoreDisplay_DisplayCreateInfoDictionary".as_ptr()) as usize
        });
        if sym != 0 {
            let f: InfoFn = std::mem::transmute(sym);
            let d = f(id);
            if !d.is_null() {
                let dict: CFDictionary<CFString, CFType> =
                    CFDictionary::wrap_under_create_rule(d as _);
                if let Some(names) = dict.find(CFString::from_static_string("DisplayProductName")) {
                    if let Some(names) = names.downcast::<CFDictionary>() {
                        let (_, vals) = names.get_keys_and_values();
                        if let Some(v) = vals.first() {
                            let s = CFString::wrap_under_get_rule(*v as _).to_string();
                            if !s.is_empty() {
                                return s;
                            }
                        }
                    }
                }
            }
        }
    }
    if builtin {
        "Built-in Display".into()
    } else {
        format!("Display {id}")
    }
}

/// All online displays, main display first, then left-to-right.
pub fn list() -> Vec<Display> {
    let main = unsafe { CGMainDisplayID() };
    let mut out: Vec<Display> = online_ids()
        .into_iter()
        .map(|id| unsafe {
            let b = CGDisplayBounds(id);
            let m = CGDisplayCopyDisplayMode(id);
            let mode = if m.is_null() {
                None
            } else {
                let mm = mode_from_ref(m);
                CGDisplayModeRelease(m);
                Some(mm)
            };
            let mirror = CGDisplayMirrorsDisplay(id);
            let builtin = CGDisplayIsBuiltin(id) != 0;
            let sz = CGDisplayScreenSize(id);
            Display {
                id,
                uuid: uuid_of(id),
                name: name_of(id, builtin),
                vendor: CGDisplayVendorNumber(id),
                model: CGDisplayModelNumber(id),
                serial: CGDisplaySerialNumber(id),
                builtin,
                main: id == main,
                active: CGDisplayIsActive(id) != 0,
                mirror_of: (mirror != K_CG_NULL_DISPLAY).then_some(mirror),
                x: b.origin.x.round() as i32,
                y: b.origin.y.round() as i32,
                width: b.size.w.round() as u32,
                height: b.size.h.round() as u32,
                rotation: CGDisplayRotation(id),
                size_mm: (sz.w, sz.h),
                mode,
            }
        })
        .collect();
    out.sort_by_key(|d| (!d.main, d.x, d.y));
    out
}

fn with_modes<T>(id: DisplayId, f: impl FnOnce(&[CGDisplayModeRef]) -> T) -> T {
    unsafe {
        // Ask for the HiDPI variants too; without this only 1x modes are returned.
        let opts = CFDictionary::from_CFType_pairs(&[(
            CFString::from_static_string("kCGDisplayResolution"),
            core_foundation::boolean::CFBoolean::true_value().as_CFType(),
        )]);
        let arr = CGDisplayCopyAllDisplayModes(id, opts.as_concrete_TypeRef() as _);
        if arr.is_null() {
            return f(&[]);
        }
        let n = CFArrayGetCount(arr);
        let refs: Vec<CGDisplayModeRef> = (0..n)
            .map(|i| CFArrayGetValueAtIndex(arr, i) as CGDisplayModeRef)
            .collect();
        let r = f(&refs);
        CFRelease(arr);
        r
    }
}

/// Modes usable for the desktop, largest first, de-duplicated.
pub fn modes(id: DisplayId) -> Vec<Mode> {
    let mut v: Vec<Mode> = with_modes(id, |refs| {
        refs.iter()
            .filter(|m| unsafe { CGDisplayModeIsUsableForDesktopGUI(**m) })
            .map(|m| unsafe { mode_from_ref(*m) })
            .collect()
    });
    v.sort_by(|a, b| {
        (b.width, b.height, b.hidpi(), b.refresh.round() as i64).cmp(&(
            a.width,
            a.height,
            a.hidpi(),
            a.refresh.round() as i64,
        ))
    });
    v.dedup_by(|a, b| {
        a.width == b.width
            && a.height == b.height
            && a.hidpi() == b.hidpi()
            && a.refresh.round() == b.refresh.round()
    });
    v
}

/// A requested change to one display. Unset fields are left as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: DisplayId,
    pub origin: Option<(i32, i32)>,
    pub mode_id: Option<i32>,
    /// `Some(None)` stops mirroring; `Some(Some(d))` mirrors display `d`.
    pub mirror_of: Option<Option<DisplayId>>,
}

fn check(code: i32, what: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(anyhow!("{what} failed (CGError {code})"))
    }
}

/// Apply a set of changes as one atomic transaction. With `permanent`, WindowServer remembers the
/// result for this display set; otherwise it lasts until logout.
pub fn apply(changes: &[Change], permanent: bool) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let online = online_ids();
    unsafe {
        let mut cfg: CGDisplayConfigRef = std::ptr::null_mut();
        check(
            CGBeginDisplayConfiguration(&mut cfg),
            "CGBeginDisplayConfiguration",
        )?;
        let run = || -> Result<()> {
            for c in changes {
                if !online.contains(&c.id) {
                    bail!("display {} is not connected", c.id);
                }
                if let Some(mode_id) = c.mode_id {
                    let r = with_modes(c.id, |refs| {
                        refs.iter()
                            .find(|m| CGDisplayModeGetIODisplayModeID(**m) == mode_id)
                            .map(|m| {
                                CGConfigureDisplayWithDisplayMode(cfg, c.id, *m, std::ptr::null())
                            })
                    });
                    match r {
                        Some(code) => check(code, "setting display mode")?,
                        None => bail!("display {} has no mode {mode_id}", c.id),
                    }
                }
                if let Some(m) = c.mirror_of {
                    check(
                        CGConfigureDisplayMirrorOfDisplay(
                            cfg,
                            c.id,
                            m.unwrap_or(K_CG_NULL_DISPLAY),
                        ),
                        "configuring mirroring",
                    )?;
                }
                if let Some((x, y)) = c.origin {
                    check(
                        CGConfigureDisplayOrigin(cfg, c.id, x, y),
                        "setting display origin",
                    )?;
                }
            }
            Ok(())
        };
        if let Err(e) = run() {
            CGCancelDisplayConfiguration(cfg);
            return Err(e);
        }
        let opt = if permanent {
            CONFIGURE_PERMANENTLY
        } else {
            CONFIGURE_FOR_SESSION
        };
        check(
            CGCompleteDisplayConfiguration(cfg, opt),
            "CGCompleteDisplayConfiguration",
        )
    }
}

/// Make `id` the main display (menu bar, origin 0,0) keeping the relative arrangement.
pub fn set_main(id: DisplayId, permanent: bool) -> Result<()> {
    let all = list();
    let target = all
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| anyhow!("display {id} is not connected"))?;
    let (dx, dy) = (target.x, target.y);
    let changes: Vec<Change> = all
        .iter()
        .filter(|d| d.mirror_of.is_none())
        .map(|d| Change {
            id: d.id,
            origin: Some((d.x - dx, d.y - dy)),
            ..Default::default()
        })
        .collect();
    apply(&changes, permanent)
}

type Callback = Box<dyn Fn() + Send + Sync>;

extern "C" fn reconfigured(_d: DisplayId, flags: u32, user: *mut c_void) {
    // Bit 0 is "begin configuration"; only react once the change has landed.
    if flags & 1 == 0 {
        let cb = unsafe { &*(user as *const Callback) };
        cb();
    }
}

/// Call `f` whenever the display configuration changes (hot-plug, mode, arrangement).
/// Delivered on the main run loop; the callback is kept for the life of the process.
pub fn on_change(f: impl Fn() + Send + Sync + 'static) {
    let boxed: *mut Callback = Box::into_raw(Box::new(Box::new(f)));
    unsafe { CGDisplayRegisterReconfigurationCallback(reconfigured, boxed as *mut c_void) };
}

pub fn screen_capture_allowed() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn request_screen_capture() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

/// One captured frame of a display, already in the byte order the DL3 tile
/// sender wants (BGRA, 4 bytes per pixel).
pub struct Grab {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixels: Vec<u8>,
}

/// Capture a display with `CGDisplayCreateImage`.
///
/// BGRA is what CoreGraphics produces for a screen image on Apple silicon and
/// what `Transport::frame` ships, so the bytes are copied out unchanged. Returns
/// `None` when the display is asleep, has no image, or screen recording has not
/// been granted.
pub fn grab(id: DisplayId) -> Option<Grab> {
    unsafe {
        let img = CGDisplayCreateImage(id);
        if img.is_null() {
            return None;
        }
        let width = CGImageGetWidth(img);
        let height = CGImageGetHeight(img);
        let stride = CGImageGetBytesPerRow(img);
        let provider = CGImageGetDataProvider(img);
        let data = if provider.is_null() {
            std::ptr::null()
        } else {
            CGDataProviderCopyData(provider)
        };
        let mut pixels = Vec::new();
        if !data.is_null() {
            let len = CFDataGetLength(data).max(0) as usize;
            let bytes = CFDataGetBytePtr(data);
            if !bytes.is_null() && len > 0 {
                pixels.extend_from_slice(std::slice::from_raw_parts(bytes, len));
            }
            CFRelease(data);
        }
        CGImageRelease(img);
        (width > 0 && height > 0 && !pixels.is_empty()).then_some(Grab {
            width,
            height,
            stride,
            pixels,
        })
    }
}
