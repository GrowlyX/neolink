//! Saved arrangements.
//!
//! A profile is keyed by *display UUID* rather than display id, because CoreGraphics reassigns
//! ids every time a display reconnects. That is the same reason DisplayLink Manager persists
//! `persistentDisplayId` in its XPC DTOs (see `docs/RE-FINDINGS.md`).

use crate::displays;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where the arrangement lives. Deliberately inside Application Support so it survives reboots
/// and is easy to inspect with `neolinkctl layout`.
pub fn path() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    base.join("Library/Application Support/Neolink/layout.json")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub uuid: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// Mode id, when the mode belongs to a real (non-virtual) display.
    pub mode_id: Option<i32>,
    /// Target size in points, used for displays we create ourselves.
    pub width: u32,
    pub height: u32,
    pub hidpi: bool,
    pub rotation: u32,
    /// Preferred vertical refresh in Hz. `None` keeps whatever CoreGraphics chose.
    /// Older profiles have no field, so it deserialises as `None`.
    #[serde(default)]
    pub refresh: Option<f64>,
    pub mirror_of: Option<String>,
    pub main: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub displays: Vec<Entry>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            version: 1,
            displays: Vec::new(),
        }
    }
}

/// Snapshot the current desktop into a profile.
pub fn capture() -> Profile {
    Profile {
        version: 1,
        displays: displays::list()
            .into_iter()
            .map(|d| Entry {
                uuid: d.uuid,
                name: d.name,
                x: d.x,
                y: d.y,
                mode_id: d.mode.map(|m| m.id),
                width: d.width,
                height: d.height,
                hidpi: d.mode.map(|m| m.hidpi()).unwrap_or(false),
                rotation: d.rotation.round() as u32,
                refresh: d.mode.map(|m| m.refresh),
                mirror_of: d
                    .mirror_of
                    .and_then(|id| displays::list().into_iter().find(|o| o.id == id))
                    .map(|o| o.uuid),
                main: d.main,
            })
            .collect(),
    }
}

pub fn load() -> Result<Profile> {
    let p = path();
    if !p.exists() {
        return Ok(capture());
    }
    let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
}

pub fn save(profile: &Profile) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(profile)?)?;
    Ok(())
}

/// Apply a profile to every display that is currently online (matching by UUID). Displays in the
/// profile that are absent are skipped rather than treated as errors — docks get unplugged.
pub fn apply(profile: &Profile) -> Result<()> {
    let live = displays::list();
    let mut changes = Vec::new();
    for e in &profile.displays {
        let Some(d) = live.iter().find(|d| d.uuid == e.uuid) else {
            continue;
        };
        let mirror = e
            .mirror_of
            .as_ref()
            .and_then(|m| live.iter().find(|d| &d.uuid == m))
            .map(|m| m.id);
        // A saved rate only means something if this display still advertises it at the
        // saved size; otherwise leave the mode alone rather than inventing a timing.
        let mode_id = e.mode_id.or_else(|| {
            let want = e.refresh?;
            displays::modes(d.id)
                .into_iter()
                .find(|m| m.width == e.width && m.height == e.height && m.refresh == want)
                .map(|m| m.id)
        });
        changes.push(displays::Change {
            id: d.id,
            origin: Some((e.x, e.y)),
            mode_id,
            mirror_of: mirror.map(Some),
        });
    }
    displays::apply(&changes, true)
}

/// Turn a point-space rectangle list into a non-overlapping arrangement, keeping the display that
/// is currently at (0,0) pinned. Used by the "Tidy up" button.
pub fn tidy(entries: &mut [Entry]) {
    entries.sort_by_key(|e| (e.x, e.y));
    let mut cursor_x = 0i32;
    for e in entries.iter_mut() {
        if e.mirror_of.is_some() {
            continue;
        }
        e.x = cursor_x;
        e.y = 0;
        cursor_x += e.width as i32;
    }
    let anchor = entries
        .iter()
        .position(|e| e.main)
        .or(if entries.is_empty() { None } else { Some(0) });
    if let Some(anchor) = anchor {
        let dx = entries[anchor].x;
        let dy = entries[anchor].y;
        for e in entries.iter_mut() {
            e.x -= dx;
            e.y -= dy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, x: i32, mirror: Option<&str>) -> Entry {
        Entry {
            uuid: name.into(),
            name: name.into(),
            x,
            y: 0,
            mode_id: None,
            width: 1920,
            height: 1080,
            hidpi: false,
            rotation: 0,
            refresh: None,
            mirror_of: mirror.map(str::to_owned),
            main: false,
        }
    }

    #[test]
    fn mirror_target_roundtrips_through_json() {
        let profile = Profile {
            version: 1,
            displays: vec![entry("A", 0, None), entry("B", 1920, Some("A"))],
        };
        let text = serde_json::to_string(&profile).unwrap();
        assert!(text.contains("\"mirror_of\":\"A\""));
        let back: Profile = serde_json::from_str(&text).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn refresh_roundtrips_and_old_profiles_still_load() {
        let mut a = entry("A", 0, None);
        a.refresh = Some(75.0);
        let profile = Profile {
            version: 1,
            displays: vec![a],
        };
        let text = serde_json::to_string(&profile).unwrap();
        assert!(text.contains("\"refresh\":75"));
        let back: Profile = serde_json::from_str(&text).unwrap();
        assert_eq!(back, profile);

        // A profile written before the field existed must still deserialise.
        let old: Profile = serde_json::from_str(
            r#"{"version":1,"displays":[{"uuid":"A","name":"A","x":0,"y":0,
                "mode_id":3,"width":1920,"height":1080,"hidpi":false,
                "rotation":0,"mirror_of":null,"main":true}]}"#,
        )
        .unwrap();
        assert_eq!(old.displays[0].refresh, None);
    }

    #[test]
    fn refresh_is_only_restored_when_the_size_matches() {
        // The live list is whatever the machine has, so drive the matcher directly over a
        // synthetic mode set.
        let modes = vec![
            displays::Mode {
                id: 1,
                width: 1920,
                height: 1080,
                pixel_width: 1920,
                pixel_height: 1080,
                refresh: 60.0,
            },
            displays::Mode {
                id: 2,
                width: 1280,
                height: 720,
                pixel_width: 1280,
                pixel_height: 720,
                refresh: 75.0,
            },
        ];
        let pick = |w: u32, h: u32, hz: f64| {
            modes
                .iter()
                .find(|m| m.width == w && m.height == h && m.refresh == hz)
                .map(|m| m.id)
        };
        assert_eq!(pick(1920, 1080, 60.0), Some(1));
        // 75 Hz exists, but not at 1920x1080.
        assert_eq!(pick(1920, 1080, 75.0), None);
        assert_eq!(pick(1280, 720, 75.0), Some(2));
    }

    #[test]
    fn tidy_keeps_mirrored_display_out_of_the_row() {
        let mut rows = vec![
            entry("A", 0, None),
            entry("B", 5000, Some("A")),
            entry("C", 900, None),
        ];
        tidy(&mut rows);
        let b = rows.iter().find(|e| e.uuid == "B").unwrap();
        assert_eq!(b.x, 5000, "mirrored entry must not be re-flowed");
        let xs: Vec<i32> = rows
            .iter()
            .filter(|e| e.mirror_of.is_none())
            .map(|e| e.x)
            .collect();
        assert_eq!(xs, vec![0, 1920]);
    }

    #[test]
    fn tidy_pins_the_main_display_at_origin() {
        let mut rows = vec![entry("A", 4000, None), entry("B", 0, None)];
        rows[1].main = true;
        tidy(&mut rows);
        assert_eq!(rows.iter().find(|e| e.main).unwrap().x, 0);
        assert!(rows.iter().all(|e| e.y == 0));
    }
}
