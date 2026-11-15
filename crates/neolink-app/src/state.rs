//! Application state: the live desktop plus every virtual display Neolink owns.
//!
//! All mutations funnel through this module so the UI never has to know whether a change touches
//! CoreGraphics, a virtual display we created, or the vendor agent over XPC.

use anyhow::{anyhow, Result};
use neolink_core::agent::{self, VendorDisplay};
use neolink_core::displays::{self, Display, Mode};
use neolink_core::layout::{self, Entry, Profile};
use neolink_core::virtual_display::{VirtualDisplay, VirtualDisplayConfig, VirtualMode};
use neolink_core::{loginitem, vendor};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A display as the UI sees it: CoreGraphics truth, plus our own provenance.
#[derive(Clone, Debug)]
pub struct Item {
    pub uuid: String,
    pub name: String,
    /// `None` when the display is virtual and has no CoreGraphics id yet.
    pub cg: Option<Display>,
    pub virtual_cfg: Option<VirtualDisplayConfig>,
    pub modes: Vec<Mode>,
    pub vendor: Option<VendorDisplay>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hidpi: bool,
    pub rotation: u32,
    pub main: bool,
    pub mirrored: bool,
    pub enabled: bool,
}

impl Item {
    fn from_display(d: &Display, vendor: Option<VendorDisplay>) -> Self {
        Self {
            uuid: d.uuid.clone(),
            name: d.name.clone(),
            cg: Some(d.clone()),
            virtual_cfg: None,
            modes: displays::modes(d.id),
            vendor,
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            hidpi: d.mode.map(|m| m.hidpi()).unwrap_or(false),
            rotation: d.rotation.round() as u32,
            main: d.main,
            mirrored: d.mirror_of.is_some(),
            enabled: d.active,
        }
    }

    pub fn is_virtual(&self) -> bool {
        self.virtual_cfg.is_some()
    }

    pub fn size_label(&self) -> String {
        let scale = if self.hidpi { " HiDPI" } else { "" };
        format!("{} × {}{}", self.width, self.height, scale)
    }
}

/// A virtual display we created, plus the config that produced it.
struct Owned {
    vd: VirtualDisplay,
    cfg: VirtualDisplayConfig,
}

pub struct AppState {
    pub items: Vec<Item>,
    /// Live virtual displays we created, keyed by UUID.
    owned: HashMap<String, Owned>,
    pub selected: Option<usize>,
    pub hovered: Option<usize>,
    pub status: String,
    pub agent: agent::Info,
    pub capture_allowed: bool,
    pub canvas_zoom: f32,
    /// Set while a display card is being dragged on the arrangement canvas.
    pub dragging: Option<usize>,
    pub dirty: bool,
    /// Cached view of the vendor package cache; refreshed with the desktop.
    pub vendor: vendor::Status,
    /// Live line from the background installer, if one is running.
    pub vendor_progress: Arc<Mutex<Option<String>>>,
    /// Shared with the worker so it can clear the flag when it finishes.
    pub vendor_busy: Arc<AtomicBool>,
    /// Mirrors the LaunchAgent on disk.
    pub launch_at_login: bool,
}

impl AppState {
    pub fn new() -> Self {
        let mut state = Self {
            items: Vec::new(),
            owned: HashMap::new(),
            selected: None,
            hovered: None,
            status: String::new(),
            agent: agent::info(),
            capture_allowed: displays::screen_capture_allowed(),
            canvas_zoom: 1.0,
            dragging: None,
            dirty: false,
            vendor: vendor::status(),
            vendor_progress: Arc::new(Mutex::new(None)),
            vendor_busy: Arc::new(AtomicBool::new(false)),
            launch_at_login: loginitem::enabled(),
        };
        state.refresh();
        // Start with the primary display selected so the inspector is never empty on launch.
        state.selected = state
            .items
            .iter()
            .position(|i| i.main)
            .or(if state.items.is_empty() {
                None
            } else {
                Some(0)
            });
        state
    }

    /// Re-read the desktop. Cheap (a handful of CoreGraphics calls) and called on every tick, so
    /// the UI is never stale.
    /// Re-read install state. Called from the same tick as `refresh`, but split out so the cheap
    /// CoreGraphics pass is not gated on the filesystem walk.
    pub fn refresh_vendor(&mut self) {
        self.vendor = vendor::status();
        if let Ok(slot) = self.vendor_progress.lock() {
            if let Some(line) = slot.as_ref() {
                self.status = line.clone();
            }
        }
    }

    /// Fetch, verify and install the pinned vendor package, off the UI thread. `installer` prompts
    /// for an admin password on the main queue, so the app must not block while it does.
    pub fn provision(&mut self) {
        if self.busy() {
            return;
        }
        self.vendor_busy.store(true, Ordering::SeqCst);
        self.status = format!("Preparing DisplayLink {}…", vendor::VERSION);
        let slot = self.vendor_progress.clone();
        let busy = self.vendor_busy.clone();
        std::thread::spawn(move || {
            let mut report = |stage: vendor::Stage, done: u64, total: u64| {
                let text = match stage {
                    vendor::Stage::Download if total > 0 => {
                        format!(
                            "Downloading DisplayLink: {:.1}/{:.1} MiB",
                            done as f64 / 1048576.0,
                            total as f64 / 1048576.0
                        )
                    }
                    vendor::Stage::Download => {
                        format!(
                            "Downloading DisplayLink: {:.1} MiB",
                            done as f64 / 1048576.0
                        )
                    }
                    vendor::Stage::Verify => "Verifying checksum…".into(),
                    vendor::Stage::Expand => "Expanding package…".into(),
                    vendor::Stage::Install => "Installing (admin password required)…".into(),
                };
                if let Ok(mut s) = slot.lock() {
                    *s = Some(text);
                }
            };
            let result = vendor::provision(&mut report);
            let text = match result {
                Ok(()) => {
                    let _ = neolink_core::agent::start();
                    format!("DisplayLink {} installed and started", vendor::VERSION)
                }
                Err(e) => format!("Install failed: {e}"),
            };
            if let Ok(mut s) = slot.lock() {
                *s = Some(text);
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn toggle_login_item(&mut self) {
        self.status = match loginitem::toggle() {
            Ok(true) => "Will open at login".into(),
            Ok(false) => "Removed from login items".into(),
            Err(e) => format!("Login item failed: {e}"),
        };
        self.launch_at_login = loginitem::enabled();
    }

    /// True while a background install is in flight.
    pub fn busy(&self) -> bool {
        self.vendor_busy.load(Ordering::SeqCst)
    }

    pub fn refresh(&mut self) {
        let vendors = if agent::reachable() {
            agent::vendor_displays()
        } else {
            Vec::new()
        };
        let live = displays::list();

        // Re-key owned displays to their current UUID; WindowServer may have published one after
        // creation, and the fallback key would otherwise linger forever.
        let owned_ids: Vec<(String, u32)> = self
            .owned
            .iter()
            .map(|(u, o)| (u.clone(), o.vd.display_id()))
            .collect();
        for (old_uuid, id) in owned_ids {
            if let Some(d) = live.iter().find(|d| d.id == id) {
                if d.uuid != old_uuid {
                    if let Some(o) = self.owned.remove(&old_uuid) {
                        self.owned.insert(d.uuid.clone(), o);
                    }
                }
            }
        }

        let mut items: Vec<Item> = live
            .iter()
            .map(|d| {
                let v = vendors.iter().find(|v| v.cgid == d.id).cloned();
                Item::from_display(d, v)
            })
            .collect();

        // Re-attach our provenance after the rebuild (it is not visible to CoreGraphics).
        for (uuid, o) in &self.owned {
            if let Some(item) = items.iter_mut().find(|i| &i.uuid == uuid) {
                item.virtual_cfg = Some(o.cfg.clone());
                item.hidpi = o.cfg.hidpi;
                item.rotation = o.cfg.rotation;
                item.modes = o
                    .cfg
                    .modes
                    .iter()
                    .enumerate()
                    .map(|(n, m)| Mode {
                        id: n as i32 + 1,
                        width: m.width,
                        height: m.height,
                        pixel_width: m.width * if o.cfg.hidpi { 2 } else { 1 },
                        pixel_height: m.height * if o.cfg.hidpi { 2 } else { 1 },
                        refresh: m.refresh,
                    })
                    .collect();
            }
        }

        let selected_uuid = self
            .selected
            .and_then(|i| self.items.get(i))
            .map(|i| i.uuid.clone());
        let hovered_uuid = self
            .hovered
            .and_then(|i| self.items.get(i))
            .map(|i| i.uuid.clone());

        items.sort_by_key(|i| (!i.main, i.x, i.y));
        self.selected = selected_uuid.and_then(|u| items.iter().position(|i| i.uuid == u));
        self.hovered = hovered_uuid.and_then(|u| items.iter().position(|i| i.uuid == u));
        self.items = items;
        self.agent = agent::info();
        self.capture_allowed = displays::screen_capture_allowed();
    }

    #[allow(dead_code)]
    pub fn selected_item(&self) -> Option<&Item> {
        self.selected.and_then(|i| self.items.get(i))
    }

    /// Create a virtual display and record that we own it.
    ///
    /// WindowServer publishes a freshly applied virtual display asynchronously, so we poll for its
    /// UUID instead of assuming it is already enumerable.
    pub fn add_virtual(&mut self, cfg: VirtualDisplayConfig) -> Result<usize> {
        let vd = VirtualDisplay::new(&cfg)?;
        let id = vd.display_id();
        let mut uuid = format!("virtual-{id}");
        for _ in 0..40 {
            if let Some(d) = displays::list().into_iter().find(|d| d.id == id) {
                uuid = d.uuid;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.owned.insert(
            uuid.clone(),
            Owned {
                vd,
                cfg: cfg.clone(),
            },
        );
        self.refresh();
        if let Some(pos) = self.items.iter().position(|i| i.uuid == uuid) {
            self.selected = Some(pos);
            // Offer exactly the modes we published rather than the generic list.
            self.items[pos].modes = cfg
                .modes
                .iter()
                .enumerate()
                .map(|(n, m)| Mode {
                    // Virtual modes have no CoreGraphics id; number them ourselves.
                    id: n as i32 + 1,
                    width: m.width,
                    height: m.height,
                    pixel_width: m.width * if cfg.hidpi { 2 } else { 1 },
                    pixel_height: m.height * if cfg.hidpi { 2 } else { 1 },
                    refresh: m.refresh,
                })
                .collect();
        }
        self.status = format!("Created “{}”", cfg.name);
        // Return the new display's own index; refresh may have re-sorted the list, so the
        // selected index is not necessarily the display we just made.
        let pos = self.items.iter().position(|i| i.uuid == uuid);
        if let Some(pos) = pos {
            self.selected = Some(pos);
        }
        pos.ok_or_else(|| anyhow!("virtual display was created but is not enumerable"))
    }

    /// Remove a display and release it back to the system.
    pub fn remove(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        if let Some(o) = self.owned.remove(&item.uuid) {
            let name = item.name.clone();
            drop(o);
            self.refresh();
            self.status = format!("Removed “{name}”");
        }
        Ok(())
    }

    /// Enable/disable a display. Vendor-managed docks go through the agent, which keeps the USB
    /// pipeline running; anything else is a CoreGraphics mode round trip.
    pub fn toggle_enabled(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        if let Some(v) = &item.vendor {
            let want = !v.enabled;
            if !agent::vendor_set("enabled", &v.persistent_display_id, want as u8 as f64) {
                self.refresh();
                return Err(anyhow!("DisplayLink agent did not apply the change"));
            }
            self.status = format!(
                "{} “{}”",
                if want { "Enabled" } else { "Disabled" },
                item.name
            );
        } else if let Some(d) = &item.cg {
            if let Some(mode_id) = d.mode.map(|m| m.id) {
                let changes = vec![displays::Change {
                    id: d.id,
                    mode_id: Some(mode_id),
                    ..Default::default()
                }];
                displays::apply(&changes, false)?;
            }
        }
        self.refresh();
        Ok(())
    }

    /// Set vendor brightness or contrast (0.0–1.0) for the selected dock.
    pub fn set_vendor_level(&mut self, index: usize, op: &str, value: f64) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        let Some(v) = item.vendor else { return Ok(()) };
        let value = value.clamp(0.0, 1.0);
        if !agent::vendor_set(op, &v.persistent_display_id, value) {
            self.refresh();
            return Err(anyhow!("DisplayLink agent did not apply {op}"));
        }
        if let Some(it) = self.items.get_mut(index) {
            if let Some(v) = it.vendor.as_mut() {
                match op {
                    "brightness" => v.brightness = value,
                    "contrast" => v.contrast = value,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Flip the vendor framebuffer, for docks mounted upside down.
    pub fn toggle_flip(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        let Some(v) = item.vendor else { return Ok(()) };
        if !agent::vendor_set("flip", &v.persistent_display_id, (!v.flipped) as u8 as f64) {
            self.refresh();
            return Err(anyhow!("DisplayLink agent did not apply the flip"));
        }
        self.refresh();
        Ok(())
    }

    pub fn set_origin(&mut self, index: usize, x: i32, y: i32) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        if let Some(d) = &item.cg {
            let changes = vec![displays::Change {
                id: d.id,
                origin: Some((x, y)),
                ..Default::default()
            }];
            displays::apply(&changes, false)?;
        }
        self.items[index].x = x;
        self.items[index].y = y;
        self.reassert_virtual(index)?;
        self.refresh();
        self.dirty = true;
        Ok(())
    }

    pub fn set_main(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        if let Some(d) = &item.cg {
            displays::set_main(d.id, true)?;
        }
        self.refresh();
        Ok(())
    }

    pub fn set_mode(&mut self, index: usize, mode_id: i32) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };

        // Virtual displays get their modes from us, so a change means rewriting the settings
        // object rather than picking a CoreGraphics mode id.
        if let (Some(cfg), Some(m)) = (
            item.virtual_cfg.clone(),
            item.modes.iter().find(|m| m.id == mode_id).cloned(),
        ) {
            let mut next = cfg.clone();
            next.modes = vec![VirtualMode::new(m.width, m.height)];
            return self.reconfigure_virtual(index, next);
        }

        if let Some(d) = &item.cg {
            let changes = vec![displays::Change {
                id: d.id,
                mode_id: Some(mode_id),
                ..Default::default()
            }];
            displays::apply(&changes, true)?;
        }
        self.reassert_virtual(index)?;
        self.refresh();
        Ok(())
    }

    /// Rotate by 90° steps on displays we own.
    pub fn rotate(&mut self, index: usize, degrees: u32) -> Result<()> {
        let Some(cfg) = self.items.get(index).and_then(|i| i.virtual_cfg.clone()) else {
            return Ok(());
        };
        let mut next = cfg;
        next.rotation = degrees % 360;
        self.reconfigure_virtual(index, next)
    }

    /// Rebuild a virtual display's settings in place (no flicker, same display id).
    pub fn reconfigure_virtual(&mut self, index: usize, cfg: VirtualDisplayConfig) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        let Some(o) = self.owned.get_mut(&item.uuid) else {
            return Ok(());
        };
        o.vd.reconfigure(&cfg)?;
        o.cfg = cfg.clone();
        self.refresh();
        Ok(())
    }

    /// Re-push a virtual display's own settings. CoreGraphics configuration calls (origin,
    /// mode, mirror) rebuild the display's mode list and drop anything we set through
    /// `CGVirtualDisplaySettings`, so every such call is followed by this.
    fn reassert_virtual(&mut self, index: usize) -> Result<()> {
        let Some(cfg) = self.items.get(index).and_then(|i| i.virtual_cfg.clone()) else {
            return Ok(());
        };
        self.reconfigure_virtual(index, cfg)
    }

    pub fn set_mirror(&mut self, index: usize, target: Option<usize>) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else {
            return Ok(());
        };
        let Some(d) = item.cg else { return Ok(()) };
        let master = target
            .and_then(|t| self.items.get(t))
            .and_then(|t| t.cg.as_ref())
            .map(|t| t.id);
        let changes = vec![displays::Change {
            id: d.id,
            mirror_of: Some(master),
            ..Default::default()
        }];
        displays::apply(&changes, true)?;
        self.reassert_virtual(index)?;
        self.refresh();
        Ok(())
    }

    /// One profile entry for an item, resolving the mirror master to its UUID so the record
    /// survives a reconnect (CoreGraphics ids do not).
    fn entry(&self, i: &Item) -> Entry {
        let mirror_of =
            i.cg.as_ref()
                .and_then(|d| d.mirror_of)
                .and_then(|id| {
                    self.items
                        .iter()
                        .find(|o| o.cg.as_ref().is_some_and(|c| c.id == id))
                })
                .map(|o| o.uuid.clone());
        Entry {
            uuid: i.uuid.clone(),
            name: i.name.clone(),
            x: i.x,
            y: i.y,
            mode_id: if i.virtual_cfg.is_some() {
                None
            } else {
                i.cg.as_ref().and_then(|d| d.mode.map(|m| m.id))
            },
            width: i.width,
            height: i.height,
            hidpi: i.hidpi,
            rotation: i.rotation,
            mirror_of,
            main: i.main,
        }
    }

    pub fn tidy(&mut self) -> Result<()> {
        let mut entries: Vec<Entry> = self.items.iter().map(|i| self.entry(i)).collect();
        layout::tidy(&mut entries);
        let profile = Profile {
            version: 1,
            displays: entries,
        };
        layout::apply(&profile)?;

        // Re-applying a CoreGraphics mode un-rotates a virtual display, so push our settings
        // back afterwards.
        let owned: Vec<(usize, VirtualDisplayConfig)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| it.virtual_cfg.clone().map(|c| (i, c)))
            .collect();
        for (i, cfg) in owned {
            if let Err(e) = self.reconfigure_virtual(i, cfg) {
                self.status = format!("Rotation restore failed: {e}");
            }
        }

        self.refresh();
        self.status = "Arrangement tidied".into();
        Ok(())
    }

    /// Persist the arrangement from our own view of the world, so rotation and HiDPI survive
    /// even though CoreGraphics does not report them for synthetic displays.
    pub fn save(&self) -> Result<()> {
        let profile = Profile {
            version: 1,
            displays: self.items.iter().map(|i| self.entry(i)).collect(),
        };
        layout::save(&profile)?;
        Ok(())
    }

    /// Points of desktop space per canvas point. Mirrors the fit logic in the canvas renderer.
    pub fn canvas_scale(&self) -> f32 {
        let total_w: f32 = self
            .items
            .iter()
            .map(|i| i.width as f32)
            .sum::<f32>()
            .max(1.0);
        let max_h: f32 = self
            .items
            .iter()
            .map(|i| i.height as f32)
            .fold(1.0f32, f32::max);
        (520.0 / total_w).min(380.0 / max_h).min(0.22) * self.canvas_zoom
    }

    pub fn scale_step(&mut self, delta: f32) {
        self.canvas_zoom = (self.canvas_zoom + delta).clamp(0.5, 2.0);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// The mode list a virtual display should advertise, derived from common panel sizes.
pub fn suggested_modes() -> Vec<VirtualMode> {
    vec![
        VirtualMode::new(3840, 2160),
        VirtualMode::new(2560, 1440),
        VirtualMode::new(1920, 1200),
        VirtualMode::new(1920, 1080),
        VirtualMode::new(1680, 1050),
        VirtualMode::new(1600, 900),
        VirtualMode::new(1440, 900),
        VirtualMode::new(1280, 1024),
        VirtualMode::new(1280, 720),
    ]
}
