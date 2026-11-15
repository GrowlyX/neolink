//! Liquid-glass design tokens.
//!
//! The palette is built from a dark, slightly blue-teal base so that translucent panels read as
//! glass rather than as flat grey, and every accent is tuned to stay legible against both the
//! window's vibrancy layer and an opaque fallback.

use gpui::{Hsla, Rgba};

/// `const`-friendly constructor so the palette can live in constants.
const fn rgba(hex: u32, a: f32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

/// Window chrome / base wash.
pub const BASE_TOP: Rgba = rgba(0x0b0e14, 1.0);
pub const BASE_BOTTOM: Rgba = rgba(0x151a24, 1.0);

/// Glass panel fills, from most to least opaque.
pub const GLASS_STRONG: Rgba = rgba(0x212633, 0.86);
pub const GLASS: Rgba = rgba(0x1c2230, 0.62);
pub const GLASS_SOFT: Rgba = rgba(0x1a2130, 0.38);
pub const SHEEN: Rgba = rgba(0xffffff, 0.07);
pub const TRANSPARENT: Rgba = rgba(0x000000, 0.0);

/// Hairlines.
pub const HAIRLINE: Rgba = rgba(0xffffff, 0.10);
pub const HAIRLINE_SOFT: Rgba = rgba(0xffffff, 0.05);

/// Type.
pub const TEXT: Rgba = rgba(0xf3f5f9, 1.0);
pub const TEXT_DIM: Rgba = rgba(0x9aa4b6, 1.0);
pub const TEXT_FAINT: Rgba = rgba(0x66707f, 1.0);

/// Accents.
pub const ACCENT: Rgba = rgba(0x5eead4, 1.0);
pub const ACCENT_SOFT: Rgba = rgba(0x2dd4bf, 1.0);
pub const ACCENT_DEEP: Rgba = rgba(0x0f766e, 1.0);
pub const WARN: Rgba = rgba(0xf0b252, 1.0);
pub const DANGER: Rgba = rgba(0xf87171, 1.0);
pub const OK: Rgba = rgba(0x7dd3a0, 1.0);

/// Per-display swatches, assigned by index so a display keeps its colour across restarts.
pub const SWATCHES: [Rgba; 6] = [
    rgba(0x5eead4, 1.0),
    rgba(0x93c5fd, 1.0),
    rgba(0xc4b5fd, 1.0),
    rgba(0xfca5a5, 1.0),
    rgba(0xfcd34d, 1.0),
    rgba(0x86efac, 1.0),
];

pub fn swatch(index: usize) -> Rgba {
    SWATCHES[index % SWATCHES.len()]
}

/// Same colour at a different alpha; used for hover fills and accent washes.
pub fn with_alpha(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}

pub fn text_hsla() -> Hsla {
    TEXT.into()
}
