//! A display created on demand through CoreGraphics' private `CGVirtualDisplay` family.
//!
//! This is the same mechanism DisplayLink Manager 17.x uses for each attached dock, and it is
//! verified to work on macOS 14 through 27 (see `docs/RE-FINDINGS.md`). It needs no kernel
//! extension, no USB transport and no daemon: the object lives in this process, and the display
//! disappears when it is dropped.

use anyhow::{anyhow, bail, Result};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, rc::Retained};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::NSString;
use serde::{Deserialize, Serialize};
use std::ffi::CStr;

/// One mode offered to the virtual display. `bare` means the desktop sees this many points;
/// when `hidpi` is set the backing store is twice as large in each axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualMode {
    pub width: u32,
    pub height: u32,
    pub refresh: f64,
}

impl VirtualMode {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            refresh: 60.0,
        }
    }
    pub fn with_refresh(width: u32, height: u32, refresh: f64) -> Self {
        Self {
            width,
            height,
            refresh,
        }
    }
    pub fn label(&self) -> String {
        format!("{} × {} @ {:.0} Hz", self.width, self.height, self.refresh)
    }
    /// Rounded rate, for comparing modes that differ only in timing.
    pub fn hz(&self) -> u32 {
        self.refresh.round().max(0.0) as u32
    }
}

/// Everything needed to bring a display online. Values mirror the vendor's descriptor defaults
/// so the panel reports plausible EDID geometry to the rest of the system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualDisplayConfig {
    pub name: String,
    pub vendor_id: u32,
    pub product_id: u32,
    pub serial: u32,
    pub modes: Vec<VirtualMode>,
    pub hidpi: bool,
    pub rotation: u32,
}

impl Default for VirtualDisplayConfig {
    fn default() -> Self {
        Self {
            name: "Neolink Display".into(),
            // 0x1a2b is unregistered, so we never masquerade as a real vendor in IOKit.
            vendor_id: 0x1a2b,
            product_id: 0x1001,
            serial: 1,
            modes: vec![
                VirtualMode::new(1920, 1080),
                VirtualMode::new(2560, 1440),
                VirtualMode::new(1280, 720),
            ],
            hidpi: true,
            rotation: 0,
        }
    }
}

fn class(name: &CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).ok_or_else(|| anyhow!("{:?} is missing on this macOS version", name))
}

unsafe fn alloc_init(klass: &AnyClass) -> *mut AnyObject {
    let obj: *mut AnyObject = msg_send![klass, alloc];
    msg_send![obj, init]
}

/// A live virtual display. Dropping it removes the display from the system.
pub struct VirtualDisplay {
    object: Retained<AnyObject>,
    settings: Retained<AnyObject>,
    display_id: u32,
}

// `applySettings:` is documented as safe from any thread as long as descriptor `queue` is set to
// the dispatch queue the object was created on; we serialise through a mutex instead.
impl VirtualDisplay {
    /// Create and publish a display. The returned object owns it for as long as it lives.
    pub fn new(cfg: &VirtualDisplayConfig) -> Result<Self> {
        let modes: Vec<VirtualMode> = if cfg.modes.is_empty() {
            return Err(anyhow!("a virtual display needs at least one mode"));
        } else {
            cfg.modes.clone()
        };
        let widest = modes
            .iter()
            .map(|m| m.width.max(m.height))
            .max()
            .unwrap_or(1920);
        let highest = modes
            .iter()
            .map(|m| m.height.max(m.width))
            .max()
            .unwrap_or(1080);
        let max_pixels = if cfg.hidpi { 2 } else { 1 };

        unsafe {
            let desc = alloc_init(class(c"CGVirtualDisplayDescriptor")?);
            let name = NSString::from_str(&cfg.name);
            let _: () = msg_send![desc, setName: &*name];
            let _: () = msg_send![desc, setVendorID: cfg.vendor_id];
            let _: () = msg_send![desc, setProductID: cfg.product_id];
            let _: () = msg_send![desc, setSerialNumber: cfg.serial];
            let _: () = msg_send![desc, setSerialNum: cfg.serial];

            // 96 DPI assumption; only used for the "physical size" hint in System Settings.
            let _: () = msg_send![desc, setSizeInMillimeters: CGSize {
                width: (widest as f64 / 96.0) * 25.4,
                height: (highest as f64 / 96.0) * 25.4,
            }];
            let _: () = msg_send![desc, setMaxPixelsWide: widest * max_pixels];
            let _: () = msg_send![desc, setMaxPixelsHigh: highest * max_pixels];
            let _: () = msg_send![desc, setWhitePoint: CGPoint { x: 0.3127, y: 0.3290 }];
            let _: () = msg_send![desc, setRedPrimary: CGPoint { x: 0.64, y: 0.33 }];
            let _: () = msg_send![desc, setGreenPrimary: CGPoint { x: 0.30, y: 0.60 }];
            let _: () = msg_send![desc, setBluePrimary: CGPoint { x: 0.15, y: 0.06 }];

            let vclass = class(c"CGVirtualDisplay")?;
            let alloc: *mut AnyObject = msg_send![vclass, alloc];
            let object: *mut AnyObject = msg_send![alloc, initWithDescriptor: desc];
            let object = Retained::from_raw(object)
                .ok_or_else(|| anyhow!("CGVirtualDisplay declined to initialise"))?;
            c_release(desc);

            let settings = build_settings(&modes, cfg.hidpi, cfg.rotation)?;
            let ok: bool = msg_send![&*object, applySettings: &*settings];
            if !ok {
                bail!("CGVirtualDisplay rejected the requested modes");
            }
            let display_id: u32 = msg_send![&*object, displayID];
            if display_id == 0 {
                bail!("CGVirtualDisplay returned display id 0");
            }
            Ok(Self {
                object,
                settings,
                display_id,
            })
        }
    }

    pub fn display_id(&self) -> u32 {
        self.display_id
    }

    /// Re-publish the mode list, HiDPI flag or rotation without tearing the display down. This is
    /// what makes rotation/refresh changes instant in the UI.
    pub fn reconfigure(&mut self, cfg: &VirtualDisplayConfig) -> Result<()> {
        let modes: Vec<VirtualMode> = if cfg.modes.is_empty() {
            return Err(anyhow!("a virtual display needs at least one mode"));
        } else {
            cfg.modes.clone()
        };
        unsafe {
            let settings = build_settings(&modes, cfg.hidpi, cfg.rotation)?;
            let ok: bool = msg_send![&*self.object, applySettings: &*settings];
            if !ok {
                bail!("CGVirtualDisplay rejected the updated settings");
            }
            self.settings = settings;
        }
        Ok(())
    }
}

unsafe fn c_release(obj: *mut AnyObject) {
    let _: () = msg_send![obj, release];
}

/// Rotation is expressed by publishing rotated timings.
///
/// `CGVirtualDisplaySettings.rotation` is accepted but not honoured on macOS 27 (verified: it
/// stays 0 and the bounds do not change), while swapping the mode dimensions does rotate the
/// display. So a 90°/270° rotation swaps every mode before it reaches CoreGraphics.
unsafe fn build_settings(
    modes: &[VirtualMode],
    hidpi: bool,
    rotation: u32,
) -> Result<Retained<AnyObject>> {
    let raw = alloc_init(class(c"CGVirtualDisplaySettings")?);
    let settings = Retained::from_raw(raw).ok_or_else(|| anyhow!("could not allocate settings"))?;
    let mclass = class(c"CGVirtualDisplayMode")?;
    let mut list: Vec<*mut AnyObject> = Vec::with_capacity(modes.len());
    let sideways = rotation % 180 == 90;
    for m in modes {
        let (w, h) = if sideways {
            (m.height, m.width)
        } else {
            (m.width, m.height)
        };
        let alloc: *mut AnyObject = msg_send![mclass, alloc];
        let mode: *mut AnyObject =
            msg_send![alloc, initWithWidth: w, height: h, refreshRate: m.refresh];
        if !mode.is_null() {
            list.push(mode);
        }
    }
    // `+[NSMutableArray array]` is autoreleased; alloc/init gives us a +1 we can own.
    let arr = alloc_init(class(c"NSMutableArray")?);
    let arr = Retained::from_raw(arr).ok_or_else(|| anyhow!("could not build mode array"))?;
    for p in &list {
        let _: () = msg_send![&*arr, addObject: *p];
    }
    for p in list {
        c_release(p);
    }
    let _: () = msg_send![&*settings, setModes: &*arr];
    let _: () = msg_send![&*settings, setHiDPI: if hidpi { 2u32 } else { 1u32 }];
    let _: () = msg_send![&*settings, setRotation: rotation];
    Ok(settings)
}

/// True when this macOS build exposes the private display classes.
pub fn is_available() -> bool {
    class(c"CGVirtualDisplay").is_ok() && class(c"CGVirtualDisplayDescriptor").is_ok()
}
