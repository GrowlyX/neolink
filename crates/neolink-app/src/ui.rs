//! Small building blocks that give the app its liquid-glass look.
//!
//! Everything is composed from GPUI primitives: a translucent fill, a 1px top sheen, a hairline
//! border and a soft shadow. Because the window itself is created with
//! `WindowBackgroundAppearance::Blurred`, the translucent layers pick up real desktop blur.

use crate::theme;
use gpui::prelude::*;
use gpui::{App, Div, Hsla, IntoElement, SharedString, Window, div, px, rgb};

/// A frosted panel with an optional highlight border.
pub fn glass(radius: f32) -> Div {
    div()
        .bg(theme::GLASS)
        .border_1()
        .border_color(theme::HAIRLINE)
        .rounded(px(radius))
        .shadow_lg()
}

/// A denser panel for content that must stay readable over busy wallpaper.
pub fn glass_strong(radius: f32) -> Div {
    div()
        .bg(theme::GLASS_STRONG)
        .border_1()
        .border_color(theme::HAIRLINE)
        .rounded(px(radius))
        .shadow_lg()
}

/// Inset chip, used for badges and small stats.
pub fn chip() -> Div {
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(theme::GLASS_SOFT)
        .border_1()
        .border_color(theme::HAIRLINE_SOFT)
        .text_xs()
        .text_color(theme::TEXT_DIM)
}

pub fn label(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_xs().text_color(theme::TEXT_FAINT).child(text.into())
}

pub fn value(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_sm().text_color(theme::TEXT).child(text.into())
}

/// Rounded pill button. `active` switches it to the accent treatment.
pub fn pill(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    active: bool,
) -> gpui::Stateful<Div> {
    let bg = if active { theme::with_alpha(theme::ACCENT, 0.18) } else { theme::GLASS_SOFT };
    let fg = if active { theme::ACCENT } else { theme::TEXT_DIM };
    let border = if active { theme::with_alpha(theme::ACCENT, 0.45) } else { theme::HAIRLINE_SOFT };
    div()
        .id(id.into())
        .px_3()
        .py_1p5()
        .rounded_full()
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_xs()
        .text_color(fg)
        .cursor_pointer()
        .hover(|s| s.bg(theme::SHEEN).text_color(theme::TEXT))
        .active(|s| s.opacity(0.8))
        .child(text.into())
}

/// A coloured status dot.
pub fn dot(color: impl Into<Hsla>) -> impl IntoElement {
    div().size_1p5().rounded_full().bg(color.into())
}

/// Top sheen: a 1px white gradient that sells the glass edge.
pub fn sheen() -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(theme::SHEEN)
}

pub fn divider() -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(theme::HAIRLINE_SOFT)
}

/// App glyph: two overlapping rounded rectangles suggesting a mirrored desktop.
pub fn logo(size: f32) -> impl IntoElement {
    div()
        .relative()
        .w(px(size))
        .h(px(size))
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(size * 0.18))
                .w(px(size * 0.62))
                .h(px(size * 0.62))
                .rounded(px(size * 0.16))
                .border_1()
                .border_color(theme::with_alpha(theme::ACCENT, 0.7)),
        )
        .child(
            div()
                .absolute()
                .left(px(size * 0.34))
                .top(px(0.0))
                .w(px(size * 0.62))
                .h(px(size * 0.62))
                .rounded(px(size * 0.16))
                .bg(theme::with_alpha(theme::ACCENT, 0.16))
                .border_1()
                .border_color(theme::ACCENT),
        )
}

/// A tiny toggle that reads as a physical switch.
pub fn toggle(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
    let (track, knob_left, knob_bg) = if on {
        (theme::with_alpha(theme::ACCENT, 0.35), px(17.0), theme::ACCENT)
    } else {
        (theme::SHEEN, px(2.0), theme::TEXT_DIM)
    };
    div()
        .id(id.into())
        .relative()
        .w(px(34.0))
        .h(px(20.0))
        .rounded_full()
        .bg(track)
        .border_1()
        .border_color(theme::HAIRLINE)
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .left(knob_left)
                .top(px(2.0))
                .size(px(15.0))
                .rounded_full()
                .bg(knob_bg),
        )
}

/// Slider track + filled portion, with the value shown as a percentage.
pub fn meter(ratio: f32, color: impl Into<Hsla>) -> impl IntoElement {
    let r = ratio.clamp(0.0, 1.0);
    div()
        .h(px(4.0))
        .w_full()
        .rounded_full()
        .bg(theme::SHEEN)
        .child(
            div()
                .h(px(4.0))
                .w(gpui::relative(r))
                .rounded_full()
                .bg(color.into()),
        )
}

/// Draws a small arrow/chevron using two rotated borders.
pub fn chevron(_up: bool) -> impl IntoElement {
    div()
        .size(px(6.0))
        .border_t_1()
        .border_r_1()
        .border_color(theme::TEXT_FAINT)
}

/// Convenience: a labelled stat row used in the inspector.
pub fn stat(name: &'static str, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_1()
        .child(label(name))
        .child(value(text))
}

pub fn accent_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_sm().text_color(theme::ACCENT).child(text.into())
}

pub fn warn_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_xs().text_color(theme::WARN).child(text.into())
}

pub fn _unused(_: &mut Window, _: &mut App) {
    let _ = rgb(0x000000);
}
