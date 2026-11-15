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
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
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
        Self { version: 1, displays: Vec::new() }
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
        let Some(d) = live.iter().find(|d| d.uuid == e.uuid) else { continue };
        let mirror = e.mirror_of.as_ref().and_then(|m| live.iter().find(|d| &d.uuid == m)).map(|m| m.id);
        changes.push(displays::Change {
            id: d.id,
            origin: Some((e.x, e.y)),
            mode_id: e.mode_id,
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
    let anchor = entries.iter().position(|e| e.main).or(if entries.is_empty() { None } else { Some(0) });
    if let Some(anchor) = anchor {
        let dx = entries[anchor].x;
        let dy = entries[anchor].y;
        for e in entries.iter_mut() {
            e.x -= dx;
            e.y -= dy;
        }
    }
}
