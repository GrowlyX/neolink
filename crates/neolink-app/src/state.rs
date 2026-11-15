//! Application state: the live desktop plus every virtual display Neolink owns.
//!
//! All mutations funnel through this module so the UI never has to know whether a change touches
//! CoreGraphics, a virtual display we created, or the vendor agent over XPC.

use anyhow::Result;
use neolink_core::agent::{self, VendorDisplay};
use neolink_core::displays::{self, Display, Mode};
use neolink_core::layout::{self, Entry, Profile};
use neolink_core::virtual_display::{VirtualDisplay, VirtualDisplayConfig, VirtualMode};
use std::collections::HashMap;

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
    pub brightness: f64,
    pub contrast: f64,
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
            brightness: 1.0,
            contrast: 1.0,
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

    pub fn aspect(&self) -> f32 {
        if self.height == 0 { 16.0 / 9.0 } else { self.width as f32 / self.height as f32 }
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
        };
        state.refresh();
        // Start with the primary display selected so the inspector is never empty on launch.
        state.selected = state
            .items
            .iter()
            .position(|i| i.main)
            .or(if state.items.is_empty() { None } else { Some(0) });
        state
    }

    /// Re-read the desktop. Cheap (a handful of CoreGraphics calls) and called on every tick, so
    /// the UI is never stale.
    pub fn refresh(&mut self) {
        let vendors = if agent::reachable() { agent::vendor_displays() } else { Vec::new() };
        let live = displays::list();

        // Re-key owned displays to their current UUID; WindowServer may have published one after
        // creation, and the fallback key would otherwise linger forever.
        let owned_ids: Vec<(String, u32)> = self.owned.iter().map(|(u, o)| (u.clone(), o.vd.display_id())).collect();
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

        let selected_uuid = self.selected.and_then(|i| self.items.get(i)).map(|i| i.uuid.clone());
        let hovered_uuid = self.hovered.and_then(|i| self.items.get(i)).map(|i| i.uuid.clone());

        items.sort_by_key(|i| (!i.main, i.x, i.y));
        self.selected = selected_uuid.and_then(|u| items.iter().position(|i| i.uuid == u));
        self.hovered = hovered_uuid.and_then(|u| items.iter().position(|i| i.uuid == u));
        self.items = items;
        self.agent = agent::info();
        self.capture_allowed = displays::screen_capture_allowed();
    }

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
        self.owned.insert(uuid.clone(), Owned { vd, cfg: cfg.clone() });
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
        Ok(self.selected.unwrap_or(0))
    }

    /// Remove a display and release it back to the system.
    pub fn remove(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        if let Some(o) = self.owned.remove(&item.uuid) {
            let name = item.name.clone();
            drop(o);
            self.refresh();
            self.status = format!("Removed “{name}”");
        }
        Ok(())
    }

    pub fn toggle_enabled(&mut self, index: usize) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        if let Some(v) = &item.vendor {
            let want = !v.enabled;
            agent::vendor_set("enabled", &v.persistent_display_id, want as u8 as f64);
        } else if let Some(d) = &item.cg {
            if let Some(mode_id) = d.mode.map(|m| m.id) {
                let changes = vec![displays::Change { id: d.id, mode_id: Some(mode_id), ..Default::default() }];
                displays::apply(&changes, false)?;
            }
        }
        self.refresh();
        Ok(())
    }

    pub fn set_origin(&mut self, index: usize, x: i32, y: i32) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        if let Some(d) = &item.cg {
            let changes = vec![displays::Change { id: d.id, origin: Some((x, y)), ..Default::default() }];
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
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        if let Some(d) = &item.cg {
            displays::set_main(d.id, true)?;
        }
        self.refresh();
        Ok(())
    }

    pub fn set_mode(&mut self, index: usize, mode_id: i32) -> Result<()> {
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };

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
            let changes = vec![displays::Change { id: d.id, mode_id: Some(mode_id), ..Default::default() }];
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
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        let Some(o) = self.owned.get_mut(&item.uuid) else { return Ok(()) };
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
        let Some(item) = self.items.get(index).cloned() else { return Ok(()) };
        let Some(d) = item.cg else { return Ok(()) };
        let master = target
            .and_then(|t| self.items.get(t))
            .and_then(|t| t.cg.as_ref())
            .map(|t| t.id);
        let changes = vec![displays::Change { id: d.id, mirror_of: Some(master), ..Default::default() }];
        displays::apply(&changes, true)?;
        self.reassert_virtual(index)?;
        self.refresh();
        Ok(())
    }

    pub fn tidy(&mut self) -> Result<()> {
        let mut entries: Vec<Entry> = self
            .items
            .iter()
            .map(|i| Entry {
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
                mirror_of: None,
                main: i.main,
            })
            .collect();
        layout::tidy(&mut entries);
        let profile = Profile { version: 1, displays: entries };
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
            displays: self
                .items
                .iter()
                .map(|i| Entry {
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
                    mirror_of: None,
                    main: i.main,
                })
                .collect(),
        };
        layout::save(&profile)?;
        Ok(())
    }

    /// Points of desktop space per canvas point. Mirrors the fit logic in the canvas renderer.
    pub fn canvas_scale(&self) -> f32 {
        let total_w: f32 = self.items.iter().map(|i| i.width as f32).sum::<f32>().max(1.0);
        let max_h: f32 = self.items.iter().map(|i| i.height as f32).fold(1.0f32, f32::max);
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
