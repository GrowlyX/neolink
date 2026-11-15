//! Neolink components for GPUI: button, badge, card, segmented control, toggle.
//!
//! Each is a function that returns an element. Sizes, colors and type come from the brand
//! tokens (`neolink_brand::theme`, the Astryx "neolink" theme), and match the web kit's
//! components: pill buttons with bold uppercase labels, hairline cards, gray pills for
//! badges. Black and white only; no gradients, glows or shadows.

use crate::theme;
use crate::theme::{c, colors, radius, size, styled, text};
use gpui::prelude::*;
use gpui::{div, px, svg, Div, FontWeight, Hsla, IntoElement, SharedString, Stateful};

/// Resolve common monitor names to the official bundled brand marks.
pub(crate) fn brand_logo(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    [
        ("apple", "apple"),
        ("lg", "lg"),
        ("samsung", "samsung"),
        ("dell", "dell"),
        ("hp", "hp"),
        ("hewlett", "hp"),
        ("acer", "acer"),
        ("asus", "asus"),
        ("benq", "benq"),
        ("viewsonic", "viewsonic"),
        ("philips", "philips"),
        ("lenovo", "lenovo"),
        ("sony", "sony"),
        ("msi", "msi"),
        ("gigabyte", "gigabyte"),
        ("aoc", "aoc"),
        ("huawei", "huawei"),
        ("xiaomi", "xiaomi"),
        ("panasonic", "panasonic"),
        ("tcl", "tcl"),
        ("eizo", "eizo"),
        ("iiyama", "iiyama"),
        ("nec", "nec"),
        ("sharp", "sharp"),
        ("hisense", "hisense"),
        ("razer", "razer"),
        ("microsoft", "microsoft"),
    ]
    .iter()
    .find_map(|(needle, brand)| n.contains(needle).then_some(*brand))
    .map(|brand| match brand {
        "apple" => "brands/apple.svg",
        "lg" => "brands/lg.svg",
        "samsung" => "brands/samsung.svg",
        "dell" => "brands/dell.svg",
        "hp" => "brands/hp.svg",
        "acer" => "brands/acer.svg",
        "asus" => "brands/asus.svg",
        "benq" => "brands/benq.svg",
        "viewsonic" => "brands/viewsonic.svg",
        "philips" => "brands/philips.svg",
        "lenovo" => "brands/lenovo.svg",
        "sony" => "brands/sony.svg",
        "msi" => "brands/msi.svg",
        "gigabyte" => "brands/gigabyte.svg",
        "aoc" => "brands/aoc.svg",
        "huawei" => "brands/huawei.svg",
        "xiaomi" => "brands/xiaomi.svg",
        "panasonic" => "brands/panasonic.svg",
        "tcl" => "brands/tcl.svg",
        "eizo" => "brands/eizo.svg",
        "iiyama" => "brands/iiyama.svg",
        "nec" => "brands/nec.svg",
        "sharp" => "brands/sharp.svg",
        "hisense" => "brands/hisense.svg",
        "razer" => "brands/razer.svg",
        "microsoft" => "brands/microsoft.svg",
        _ => "brands/apple.svg",
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one action that matters: black on light, white on dark.
    Primary,
    Secondary,
    Ghost,
}

/// A pill button with a bold uppercase label. Attach `.on_click(...)` yourself when `enabled`.
pub fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    enabled: bool,
) -> Stateful<Div> {
    let k = colors();
    let (bg, fg) = match kind {
        ButtonKind::Primary => (k.accent, k.on_accent),
        ButtonKind::Secondary => (k.neutral, k.text_primary),
        ButtonKind::Ghost => (0x0000_0000, k.text_primary),
    };
    let pressed = k.overlay_pressed;
    let base = div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(px(size::MD))
        .px(px(14.0))
        .rounded(px(radius::FULL))
        .bg(c(bg))
        .text_color(c(fg))
        .child(styled(&text::BUTTON, label));
    if enabled {
        base.cursor_pointer()
            .hover(move |s| {
                if kind == ButtonKind::Primary {
                    s.opacity(0.82)
                } else {
                    s.bg(c(pressed))
                }
            })
            .active(|s| s.opacity(0.7))
    } else {
        base.opacity(0.35)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Neutral,
    /// Black on light, white on dark.
    Strong,
}

pub fn badge(label: impl Into<SharedString>, kind: BadgeKind) -> Div {
    let k = colors();
    let (bg, fg) = match kind {
        BadgeKind::Neutral => (k.background_gray, k.text_gray),
        BadgeKind::Strong => (k.info, k.on_info),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(20.0))
        .px(px(8.0))
        .rounded(px(radius::FULL))
        .bg(c(bg))
        .text_color(c(fg))
        .font_family(text::SUPPORTING.family)
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(label.into())
}

/// A hairline-bordered surface with the container radius.
pub fn card() -> Div {
    let k = colors();
    div()
        .bg(c(k.background_card))
        .border_1()
        .border_color(c(k.border))
        .rounded(px(radius::CONTAINER))
}

/// The track of a segmented control. Fill it with [`segment`]s.
pub fn segmented() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.0))
        .h(px(size::SM))
        .p(px(4.0))
        .rounded(px(radius::ELEMENT))
        .bg(c(colors().neutral))
}

/// One option in a segmented control. Attach `.on_click(...)` yourself.
pub fn segment(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    let k = colors();
    let item = div()
        .id(id.into())
        .flex()
        .items_center()
        .h(px(20.0))
        .px(px(9.0))
        .rounded(px(12.0))
        .font_family(text::SUPPORTING.family)
        .text_size(px(12.0))
        .whitespace_nowrap()
        .cursor_pointer()
        .child(label.into());
    if selected {
        item.bg(c(k.background_surface))
            .text_color(c(k.text_primary))
            .font_weight(FontWeight::SEMIBOLD)
    } else {
        let hover = k.text_primary;
        item.text_color(c(k.text_secondary))
            .font_weight(FontWeight::MEDIUM)
            .hover(move |s| s.text_color(c(hover)))
    }
}

/// A switch: ink track when on, gray when off.
pub fn toggle(id: impl Into<gpui::ElementId>, on: bool) -> Stateful<Div> {
    let k = colors();
    div()
        .id(id.into())
        .w(px(36.0))
        .h(px(21.0))
        .flex_none()
        .rounded(px(radius::FULL))
        .p(px(2.0))
        .flex()
        .cursor_pointer()
        .when(on, |el| el.justify_end().bg(c(k.accent)))
        .when(!on, |el| el.bg(c(k.border_emphasized)))
        .child(
            div()
                .size(px(17.0))
                .rounded(px(radius::FULL))
                .bg(c(k.on_accent)),
        )
}

// -- compatibility names used across the views -----------------------------------------------

pub fn glass(_radius: f32) -> Div {
    card()
}

pub fn chip() -> Div {
    badge("", BadgeKind::Neutral)
}

/// A compact selectable chip for small option sets.
pub fn pill(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    segment(id, label, active)
}

pub fn label(text: impl Into<SharedString>) -> impl IntoElement {
    styled(&crate::theme::text::BODY, text).text_color(theme::text_dim())
}

/// A small status dot.
pub fn dot(color: impl Into<Hsla>) -> impl IntoElement {
    div()
        .size(px(6.0))
        .flex_none()
        .rounded_full()
        .bg(color.into())
}

/// Kept for call sites; the flat design has no sheen.
pub fn sheen() -> impl IntoElement {
    div()
}

pub fn divider() -> impl IntoElement {
    div().flex_none().h(px(1.0)).w_full().bg(theme::hairline())
}

/// The Neolink mark.
pub fn logo(size: f32) -> impl IntoElement {
    svg()
        .path("logo/neolink-mark.svg")
        .size(px(size))
        .flex_none()
        .text_color(theme::text())
}

/// Slider track + filled portion.
pub fn meter(ratio: f32, color: impl Into<Hsla>) -> impl IntoElement {
    let r = ratio.clamp(0.0, 1.0);
    div()
        .h(px(4.0))
        .w_full()
        .rounded_full()
        .bg(c(colors().track))
        .child(
            div()
                .h(px(4.0))
                .w(gpui::relative(r))
                .rounded_full()
                .bg(color.into()),
        )
}

/// Value a slider cell reports when clicked.
#[derive(Clone, Copy, Debug)]
pub struct SliderSet {
    pub step: f32,
}

/// An interactive slider. GPUI click events carry only window coordinates, so the track is split
/// into 20 hit cells; each cell reports its own step, which is exact for a click anywhere in it.
pub fn slider(
    id: impl Into<gpui::ElementId>,
    ratio: f32,
    color: impl Into<Hsla>,
    on_set: impl Fn(&SliderSet, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Div {
    const STEPS: usize = 20;
    let on_set = std::rc::Rc::new(on_set);
    let r = ratio.clamp(0.0, 1.0);
    let fill = color.into();
    let lit = (r * STEPS as f32).round() as usize;
    let name = id_hint(id);
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .h(px(16.0))
        .w_full();
    for i in 0..STEPS {
        let set = on_set.clone();
        row = row.child(
            div()
                .id(gpui::ElementId::Name(format!("{name}-{i}").into()))
                .flex_1()
                .h(px(4.0))
                .rounded_full()
                .bg(if i < lit {
                    fill
                } else {
                    c(colors().track).into()
                })
                .cursor_pointer()
                .hover(|s| s.h(px(7.0)))
                .on_click(move |_, window, cx| {
                    set(
                        &SliderSet {
                            step: (i + 1) as f32 / STEPS as f32,
                        },
                        window,
                        cx,
                    )
                }),
        );
    }
    div().w_full().py_0p5().child(row)
}

fn id_hint<T: Into<gpui::ElementId>>(id: T) -> String {
    match id.into() {
        gpui::ElementId::Name(n) => n.to_string(),
        gpui::ElementId::Integer(i) => i.to_string(),
        gpui::ElementId::Uuid(u) => format!("{u:?}"),
        _ => "slider".into(),
    }
}

/// A labelled fact row: label on the left, value on the right.
pub fn stat(name: &'static str, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(label(name))
        .child(styled(&crate::theme::text::BODY, text).text_size(px(13.0)))
}
