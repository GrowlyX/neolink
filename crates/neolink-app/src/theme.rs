//! The seam between the Neolink design tokens and GPUI.
//!
//! `neolink_brand::theme` (generated from the Astryx "neolink" theme) holds the numbers and knows
//! nothing about GPUI. This module picks the colors for the current mode and turns tokens into
//! GPUI values. Nothing else in the app writes a hex color by hand.
//!
//! The palette is strictly black and white: hierarchy comes from the gray ramp, weight and size,
//! never from hue.

use gpui::{div, prelude::*, px, rgba, Div, FontWeight, Rgba, SharedString};
use neolink_brand::theme::{Colors, Mode, TextStyle};
use std::sync::atomic::{AtomicBool, Ordering};

pub use neolink_brand::theme::{radius, size, text};

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Set once per frame from the window appearance and the person's choice.
pub fn set_light_mode(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Every color role for the current mode.
pub fn colors() -> &'static Colors {
    neolink_brand::theme::colors(if is_light() { Mode::Light } else { Mode::Dark })
}

/// A token color (`0xRRGGBBAA`) as a GPUI color.
pub fn c(token: u32) -> Rgba {
    rgba(token)
}

/// Text set in one of the type styles. GPUI has no letter-spacing or text-transform, so
/// uppercase styles are uppercased here and tracking is dropped rather than faked.
pub fn styled(style: &TextStyle, content: impl Into<SharedString>) -> Div {
    let content: SharedString = content.into();
    let content: SharedString = if style.uppercase {
        content.to_uppercase().into()
    } else {
        content
    };
    div()
        .font_family(style.family)
        .text_size(px(style.size))
        .line_height(px((style.size * style.line_height).round()))
        .font_weight(FontWeight(style.weight as f32))
        .child(content)
}

// -- roles, named for where they are used --------------------------------------------------

/// Window background.
pub fn base_top() -> Rgba {
    c(colors().background_body)
}
/// Inset fills: wells, idle tiles, segmented tracks.
pub fn glass_soft() -> Rgba {
    c(colors().background_muted)
}
pub fn sheen() -> Rgba {
    c(colors().overlay_hover)
}
pub fn hairline() -> Rgba {
    c(colors().border)
}
pub fn text() -> Rgba {
    c(colors().text_primary)
}
pub fn text_dim() -> Rgba {
    c(colors().text_secondary)
}
pub fn text_faint() -> Rgba {
    c(colors().text_disabled)
}
/// Ink: black on light, white on dark. The one emphasis color.
pub fn accent() -> Rgba {
    c(colors().accent)
}
/// Status roles stay monochrome: state is carried by the label and the fill, not by hue.
pub fn ok() -> Rgba {
    accent()
}

pub const TRANSPARENT: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
