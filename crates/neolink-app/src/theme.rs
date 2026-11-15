//! Neolink display manager palette. The settings appearance switch changes these tokens globally.

use gpui::Rgba;
use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn set_light_mode(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}
fn rgba(hex: u32, a: f32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}
fn mode(dark: u32, light: u32, alpha: f32) -> Rgba {
    rgba(
        if LIGHT.load(Ordering::Relaxed) {
            light
        } else {
            dark
        },
        alpha,
    )
}
pub fn base_top() -> Rgba {
    mode(0x0b0e14, 0xf5f5f5, 1.0)
}
pub fn base_bottom() -> Rgba {
    mode(0x151a24, 0xffffff, 1.0)
}
pub fn glass() -> Rgba {
    mode(0x1c2230, 0xffffff, 0.96)
}
pub fn glass_soft() -> Rgba {
    mode(0x1a2130, 0xf1f1f1, 0.94)
}
pub fn sheen() -> Rgba {
    mode(0xffffff, 0x000000, 0.07)
}
pub fn hairline() -> Rgba {
    mode(0xffffff, 0x000000, 0.12)
}
pub fn hairline_soft() -> Rgba {
    mode(0xffffff, 0x000000, 0.07)
}
pub fn text() -> Rgba {
    mode(0xf3f5f9, 0x171717, 1.0)
}
pub fn text_dim() -> Rgba {
    mode(0x9aa4b6, 0x666666, 1.0)
}
pub fn text_faint() -> Rgba {
    mode(0x66707f, 0x888888, 1.0)
}
pub const TRANSPARENT: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
pub const ACCENT: Rgba = Rgba {
    r: 0x5e as f32 / 255.0,
    g: 0xea as f32 / 255.0,
    b: 0xd4 as f32 / 255.0,
    a: 1.0,
};
pub const ACCENT_SOFT: Rgba = Rgba {
    r: 0x2d as f32 / 255.0,
    g: 0xd4 as f32 / 255.0,
    b: 0xbf as f32 / 255.0,
    a: 1.0,
};
pub const WARN: Rgba = Rgba {
    r: 0xf0 as f32 / 255.0,
    g: 0xb2 as f32 / 255.0,
    b: 0x52 as f32 / 255.0,
    a: 1.0,
};
pub const DANGER: Rgba = Rgba {
    r: 0xf8 as f32 / 255.0,
    g: 0x71 as f32 / 255.0,
    b: 0x71 as f32 / 255.0,
    a: 1.0,
};
pub const OK: Rgba = Rgba {
    r: 0x7d as f32 / 255.0,
    g: 0xd3 as f32 / 255.0,
    b: 0xa0 as f32 / 255.0,
    a: 1.0,
};
const SWATCHES: [Rgba; 6] = [
    ACCENT,
    Rgba {
        r: 0x93 as f32 / 255.0,
        g: 0xc5 as f32 / 255.0,
        b: 0xfd as f32 / 255.0,
        a: 1.0,
    },
    Rgba {
        r: 0xc4 as f32 / 255.0,
        g: 0xb5 as f32 / 255.0,
        b: 0xfd as f32 / 255.0,
        a: 1.0,
    },
    Rgba {
        r: 0xfc as f32 / 255.0,
        g: 0xa5 as f32 / 255.0,
        b: 0xa5 as f32 / 255.0,
        a: 1.0,
    },
    Rgba {
        r: 0xfc as f32 / 255.0,
        g: 0xd3 as f32 / 255.0,
        b: 0x4d as f32 / 255.0,
        a: 1.0,
    },
    Rgba {
        r: 0x86 as f32 / 255.0,
        g: 0xef as f32 / 255.0,
        b: 0xac as f32 / 255.0,
        a: 1.0,
    },
];
pub fn swatch(index: usize) -> Rgba {
    SWATCHES[index % SWATCHES.len()]
}
pub fn with_alpha(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}
