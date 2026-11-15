//! Small building blocks that give the app its liquid-glass look.
//!
//! Everything is composed from GPUI primitives: a translucent fill, a 1px top sheen, a hairline
//! border and a soft shadow. Because the window itself is created with
//! `WindowBackgroundAppearance::Blurred`, the translucent layers pick up real desktop blur.

use crate::theme;
use gpui::prelude::*;
use gpui::{div, px, rgb, App, Div, Hsla, IntoElement, SharedString, Window};

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

/// A frosted panel with an optional highlight border.
pub fn glass(radius: f32) -> Div {
    div()
        .bg(theme::glass())
        .border_1()
        .border_color(theme::hairline())
        .rounded(px(radius))
        .shadow_lg()
}

/// Inset chip, used for badges and small stats.
pub fn chip() -> Div {
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(theme::glass_soft())
        .border_1()
        .border_color(theme::hairline_soft())
        .text_xs()
        .text_color(theme::text_dim())
}

pub fn label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(theme::text_faint())
        .child(text.into())
}

pub fn value(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_sm().text_color(theme::text()).child(text.into())
}

/// Rounded pill button. `active` switches it to the accent treatment.
pub fn pill(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    active: bool,
) -> gpui::Stateful<Div> {
    let bg = if active {
        theme::with_alpha(theme::ACCENT, 0.18)
    } else {
        theme::glass_soft()
    };
    let fg = if active {
        theme::ACCENT
    } else {
        theme::text_dim()
    };
    let border = if active {
        theme::with_alpha(theme::ACCENT, 0.45)
    } else {
        theme::hairline_soft()
    };
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
        .hover(|s| s.bg(theme::sheen()).text_color(theme::text()))
        .active(|s| s.opacity(0.8))
        .child(text.into())
}

/// A coloured status dot.
pub fn dot(color: impl Into<Hsla>) -> impl IntoElement {
    div().size_1p5().rounded_full().bg(color.into())
}

/// Top sheen: a 1px white gradient that sells the glass edge.
pub fn sheen() -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(theme::sheen())
}

pub fn divider() -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(theme::hairline_soft())
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
        (
            theme::with_alpha(theme::ACCENT, 0.35),
            px(17.0),
            theme::ACCENT,
        )
    } else {
        (theme::sheen(), px(2.0), theme::text_dim())
    };
    div()
        .id(id.into())
        .relative()
        .w(px(34.0))
        .h(px(20.0))
        .rounded_full()
        .bg(track)
        .border_1()
        .border_color(theme::hairline())
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
        .bg(theme::sheen())
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
                .bg(if i < lit { fill } else { theme::sheen().into() })
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

/// Draws a small arrow/chevron using two rotated borders.
#[allow(dead_code)]
pub fn chevron(_up: bool) -> impl IntoElement {
    div()
        .size(px(6.0))
        .border_t_1()
        .border_r_1()
        .border_color(theme::text_faint())
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

#[allow(dead_code)]
pub fn accent_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_sm().text_color(theme::ACCENT).child(text.into())
}

#[allow(dead_code)]
pub fn warn_text(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_xs().text_color(theme::WARN).child(text.into())
}

pub fn _unused(_: &mut Window, _: &mut App) {
    let _ = rgb(0x000000);
}
