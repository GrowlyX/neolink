//! First-run setup assistant.
//!
//! A drag-and-drop install only puts the bundle in /Applications, so the first launch walks the
//! user through what a `.pkg` installer would otherwise do: fetch and install the DisplayLink
//! driver, grant the optional privacy permissions, and pick how Neolink starts. Every step can be
//! skipped and re-run later from Settings.

use crate::state::VendorPhase;
use gpui::prelude::*;
use gpui::{div, px, svg, Animation, AnimationExt, Context, Div, FontWeight, Rgba, SharedString};
use neolink_core::vendor::{self, Stage};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Welcome,
    Driver,
    Permissions,
    Preferences,
    Done,
}

const STEPS: [Step; 5] = [
    Step::Welcome,
    Step::Driver,
    Step::Permissions,
    Step::Preferences,
    Step::Done,
];

impl Step {
    fn index(self) -> usize {
        STEPS.iter().position(|s| *s == self).unwrap_or(0)
    }
    fn next(self) -> Step {
        STEPS[(self.index() + 1).min(STEPS.len() - 1)]
    }
    fn prev(self) -> Step {
        STEPS[self.index().saturating_sub(1)]
    }
}

fn marker_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home).join("Library/Application Support/Neolink/setup-complete")
    })
}

/// Show the assistant until it has been finished or skipped once.
pub fn needed() -> bool {
    if std::env::var_os("NEOLINK_DEMO").is_some()
        || std::env::var_os("NEOLINK_SKIP_SETUP").is_some()
    {
        return false;
    }
    marker_path().is_some_and(|p| !p.exists())
}

/// Debug builds can open on a given step (`NEOLINK_SETUP_STEP=2`) to iterate on its layout.
pub fn initial() -> Option<Step> {
    #[cfg(debug_assertions)]
    if let Some(i) = std::env::var("NEOLINK_SETUP_STEP")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return STEPS.get(i.saturating_sub(1)).copied();
    }
    needed().then_some(Step::Welcome)
}

pub fn mark_complete() {
    let Some(path) = marker_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, env!("CARGO_PKG_VERSION"));
}

fn open_settings_pane(anchor: &str) {
    let _ = std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{anchor}"
        ))
        .spawn();
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// The assistant is strictly black and white: ink on paper, with opacity as the only
/// hierarchy. No hues, gradients or shadows.
mod mono {
    use crate::theme;
    use gpui::prelude::*;
    use gpui::{div, px, Div, Hsla, Rgba, SharedString};

    fn tone(white: bool, a: f32) -> Rgba {
        let v = if white { 1.0 } else { 0.0 };
        Rgba {
            r: v,
            g: v,
            b: v,
            a,
        }
    }
    pub fn ink() -> Rgba {
        tone(!theme::is_light(), 1.0)
    }
    pub fn paper() -> Rgba {
        tone(theme::is_light(), 1.0)
    }
    fn ink_a(a: f32) -> Rgba {
        tone(!theme::is_light(), a)
    }
    pub fn dim() -> Rgba {
        ink_a(0.72)
    }
    pub fn faint() -> Rgba {
        ink_a(0.5)
    }
    pub fn line() -> Rgba {
        ink_a(0.2)
    }
    pub fn line_soft() -> Rgba {
        ink_a(0.11)
    }
    pub fn wash() -> Rgba {
        ink_a(0.05)
    }
    pub fn scrim() -> Rgba {
        tone(theme::is_light(), 0.9)
    }
    pub fn divider() -> impl IntoElement {
        div().h(px(1.0)).w_full().bg(line_soft())
    }
    pub fn dot(color: impl Into<Hsla>) -> impl IntoElement {
        div()
            .size(px(6.0))
            .flex_none()
            .rounded_full()
            .bg(color.into())
    }
    /// Outline pill; `active` fills it with ink.
    pub fn pill(
        id: impl Into<gpui::ElementId>,
        text: impl Into<SharedString>,
        active: bool,
    ) -> gpui::Stateful<Div> {
        let el = div()
            .id(id.into())
            .px(px(12.0))
            .py(px(5.0))
            .rounded_full()
            .border_1()
            .text_xs()
            .cursor_pointer()
            .child(text.into());
        if active {
            el.bg(ink()).border_color(ink()).text_color(paper())
        } else {
            el.border_color(line())
                .text_color(dim())
                .hover(|s| s.border_color(ink()).text_color(ink()))
        }
    }
    pub fn toggle(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
        div()
            .id(id.into())
            .relative()
            .w(px(34.0))
            .h(px(20.0))
            .flex_none()
            .rounded_full()
            .border_1()
            .border_color(if on { ink() } else { line() })
            .bg(if on { ink() } else { paper() })
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(if on { 16.0 } else { 2.0 }))
                    .size(px(14.0))
                    .rounded_full()
                    .bg(if on { paper() } else { faint() }),
            )
    }
    pub fn meter(ratio: f32, color: impl Into<Hsla>) -> impl IntoElement {
        div()
            .h(px(3.0))
            .w_full()
            .rounded_full()
            .bg(line_soft())
            .child(
                div()
                    .h_full()
                    .w(gpui::relative(ratio.clamp(0.0, 1.0)))
                    .rounded_full()
                    .bg(color.into()),
            )
    }
}

// -- small pieces ---------------------------------------------------------------------------

fn icon(path: &'static str, size: f32, color: Rgba) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Rounded square holding an icon; the visual anchor at the top of each step.
fn badge(path: &'static str, tint: Rgba) -> impl IntoElement {
    div()
        .size(px(52.0))
        .flex_none()
        .rounded(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(mono::line())
        .child(icon(path, 24.0, tint))
}

fn primary(id: &'static str, text: impl Into<SharedString>, enabled: bool) -> gpui::Stateful<Div> {
    let el = div()
        .id(id)
        .h(px(36.0))
        .px(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into());
    if enabled {
        el.bg(mono::ink())
            .text_color(mono::paper())
            .cursor_pointer()
            .hover(|s| s.opacity(0.86))
            .active(|s| s.opacity(0.85))
    } else {
        el.bg(mono::wash())
            .text_color(mono::faint())
            .border_1()
            .border_color(mono::line_soft())
    }
}

fn ghost(id: &'static str, text: impl Into<SharedString>) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .h(px(36.0))
        .px(px(14.0))
        .flex()
        .items_center()
        .rounded(px(10.0))
        .text_size(px(13.0))
        .text_color(mono::dim())
        .cursor_pointer()
        .hover(|s| s.bg(mono::wash()).text_color(mono::ink()))
        .child(text.into())
}

/// Inset well that groups related rows inside the card.
fn well() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(mono::wash())
        .border_1()
        .border_color(mono::line_soft())
}

fn well_row(first: bool) -> Div {
    let row = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(11.0));
    if first {
        row
    } else {
        row.border_t_1().border_color(mono::line_soft())
    }
}

fn row_text(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_size(px(13.0)).child(title.into()))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(mono::faint())
                .child(detail.into()),
        )
}

fn kv(first: bool, key: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    well_row(first)
        .py(px(8.0))
        .child(
            div()
                .w(px(92.0))
                .flex_none()
                .text_size(px(12.0))
                .text_color(mono::faint())
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(12.0))
                .text_color(mono::dim())
                .truncate()
                .child(value.into()),
        )
}

fn granted() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_size(px(12.0))
        .text_color(mono::ink())
        .child(icon("icons/check.svg", 13.0, mono::ink()))
        .child("Granted")
}

/// One line of the install timeline: done, active, or still to come.
fn stage_line(label: &'static str, state: i8, detail: Option<String>) -> impl IntoElement {
    // state: -1 pending, 0 active, 1 done
    let marker = div()
        .size(px(18.0))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center();
    let marker = match state {
        1 => marker.child(icon("icons/check.svg", 11.0, mono::ink())),
        0 => marker.border_1().border_color(mono::ink()).child(
            div()
                .size(px(6.0))
                .rounded_full()
                .bg(mono::ink())
                .with_animation(
                    "stage-pulse",
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(gpui::pulsating_between(0.35, 1.0)),
                    |el, delta| el.opacity(delta),
                ),
        ),
        _ => marker.border_1().border_color(mono::line()),
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(marker)
        .child(
            div()
                .flex_1()
                .text_size(px(13.0))
                .text_color(match state {
                    -1 => mono::faint(),
                    _ => mono::ink(),
                })
                .child(label),
        )
        .children(detail.map(|d| div().text_size(px(12.0)).text_color(mono::faint()).child(d)))
}

fn stage_rank(stage: Stage) -> i8 {
    match stage {
        Stage::Download => 0,
        Stage::Verify => 1,
        Stage::Expand => 2,
        Stage::Install => 3,
    }
}

// -- the assistant --------------------------------------------------------------------------

impl crate::Neolink {
    pub(crate) fn setup_overlay(&self, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let body: gpui::AnyElement = match step {
            Step::Welcome => self.setup_welcome().into_any_element(),
            Step::Driver => self.setup_driver(cx).into_any_element(),
            Step::Permissions => self.setup_permissions(cx).into_any_element(),
            Step::Preferences => self.setup_preferences(cx).into_any_element(),
            Step::Done => self.setup_done().into_any_element(),
        };

        // Progress: one segment per numbered step (the summary is not one), filled up to here.
        let mut segments = div().flex().gap(px(4.0)).w(px(124.0));
        for i in 0..STEPS.len() - 1 {
            segments = segments.child(div().flex_1().h(px(3.0)).rounded_full().bg(
                if i <= step.index() {
                    mono::ink()
                } else {
                    mono::line()
                },
            ));
        }

        div()
            .id("setup-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(mono::scrim())
            .child(
                div()
                    .id("setup-card")
                    .w(px(600.0))
                    .flex()
                    .flex_col()
                    .rounded(px(20.0))
                    .bg(mono::paper())
                    .border_1()
                    .border_color(mono::line())
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(32.0))
                            .pt(px(24.0))
                            .child(div().text_size(px(11.0)).text_color(mono::faint()).child(
                                if step == Step::Done {
                                    "SETUP COMPLETE".to_string()
                                } else {
                                    format!("STEP {} OF {}", step.index() + 1, STEPS.len() - 1)
                                },
                            ))
                            .child(segments),
                    )
                    .child(
                        div()
                            .px(px(32.0))
                            .pt(px(22.0))
                            .pb(px(26.0))
                            // Fixed, so the footer buttons stay put as the steps change.
                            .h(px(448.0))
                            .child(body),
                    )
                    .child(mono::divider())
                    .child(self.setup_footer(step, cx)),
            )
    }

    fn setup_footer(&self, step: Step, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.state.busy();
        let left: Option<gpui::AnyElement> = match step {
            Step::Welcome => Some(
                ghost("setup-skip", "Skip setup")
                    .on_click(cx.listener(|this, _, _, cx| {
                        mark_complete();
                        this.setup = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            Step::Done => None,
            _ if busy => None,
            _ => Some(
                ghost("setup-back", "")
                    .gap(px(4.0))
                    .child(icon("icons/chevron-left.svg", 14.0, mono::dim()))
                    .child("Back")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.setup = Some(step.prev());
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        };

        let phase = self.state.vendor_phase();
        let installed = self.state.agent.installed;
        let (label, enabled): (String, bool) = match step {
            Step::Welcome => ("Get started".into(), true),
            Step::Driver => match phase {
                VendorPhase::Working { .. } => ("Installing…".into(), false),
                VendorPhase::Failed(_) => ("Try again".into(), true),
                VendorPhase::Done => ("Continue".into(), true),
                _ if installed => ("Continue".into(), true),
                _ => ("Download and install".into(), true),
            },
            Step::Permissions | Step::Preferences => ("Continue".into(), true),
            Step::Done => ("Start using Neolink".into(), true),
        };
        let driver_action = step == Step::Driver
            && (matches!(phase, VendorPhase::Failed(_))
                || (!installed && phase == VendorPhase::Idle));

        let mut right = div().flex().items_center().gap(px(8.0));
        if step == Step::Driver && driver_action && !busy {
            right = right.child(
                ghost("setup-driver-skip", "Skip for now").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.setup = Some(Step::Permissions);
                        cx.notify();
                    },
                )),
            );
        }
        let mut button = primary("setup-primary", label, enabled);
        if enabled {
            button = button.on_click(cx.listener(move |this, _, _, cx| {
                if driver_action {
                    this.state.provision();
                } else if step == Step::Done {
                    mark_complete();
                    this.setup = None;
                    this.state.refresh();
                } else {
                    this.setup = Some(step.next());
                }
                cx.notify();
            }));
        }
        right = right.child(button);

        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(24.0))
            .py(px(16.0))
            .child(div().children(left))
            .child(right)
    }

    fn heading(
        &self,
        badge_icon: &'static str,
        tint: Rgba,
        title: &'static str,
        body: &'static str,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(badge(badge_icon, tint))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(24.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(px(21.0))
                            .text_color(mono::dim())
                            .child(body),
                    ),
            )
    }

    fn setup_welcome(&self) -> impl IntoElement {
        let items: [(&'static str, &'static str, &'static str); 3] = [
            (
                "icons/download.svg",
                "Install the DisplayLink driver",
                "Fetched from Synaptics on request and checked before it runs.",
            ),
            (
                "icons/shield.svg",
                "Review permissions",
                "Screen Recording and Accessibility, both optional for now.",
            ),
            (
                "icons/power.svg",
                "Choose how Neolink starts",
                "Open at login and pick a light or dark look.",
            ),
        ];
        let mut list = well();
        for (i, (path, title, detail)) in items.into_iter().enumerate() {
            list = list.child(
                well_row(i == 0)
                    .child(icon(path, 16.0, mono::ink()))
                    .child(row_text(title, detail)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .child(
                        svg()
                            .path("logo/neolink-mark.svg")
                            .size(px(52.0))
                            .text_color(mono::ink()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(26.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Welcome to Neolink"),
                            )
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .line_height(px(21.0))
                                    .text_color(mono::dim())
                                    .child(
                                        "Arrange displays, create virtual screens, and manage \
                                         DisplayLink docks. A few steps get everything ready.",
                                    ),
                            ),
                    ),
            )
            .child(list)
    }

    fn setup_driver(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = self.state.vendor_phase();
        let agent = &self.state.agent;
        let (version, digest) = vendor::pinned();

        let package = well()
            .child(kv(
                true,
                "Package",
                format!("DisplayLink Manager {version}"),
            ))
            .child(kv(false, "Source", "synaptics.com (official download)"))
            .child(kv(false, "Size", mib(vendor::SIZE)))
            .child(kv(
                false,
                "SHA-256",
                format!("{}…{}", &digest[..12], &digest[digest.len() - 12..]),
            ));

        let status: gpui::AnyElement =
            match &phase {
                VendorPhase::Working { stage, done, total } => {
                    let rank = stage_rank(*stage);
                    let line = |label, r: i8, detail| {
                        stage_line(
                            label,
                            if rank > r {
                                1
                            } else if rank == r {
                                0
                            } else {
                                -1
                            },
                            detail,
                        )
                    };
                    let ratio = if *total > 0 {
                        *done as f32 / *total as f32
                    } else {
                        0.0
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(line(
                            "Download",
                            0,
                            (rank == 0).then(|| format!("{} of {}", mib(*done), mib(*total))),
                        ))
                        .when(rank == 0, |d| {
                            d.child(div().pl(px(28.0)).child(mono::meter(ratio, mono::ink())))
                        })
                        .child(line("Verify checksum", 1, None))
                        .child(line("Unpack", 2, None))
                        .child(line(
                            "Install",
                            3,
                            (rank == 3).then(|| "Waiting for administrator approval".to_string()),
                        ))
                        .into_any_element()
                }
                VendorPhase::Failed(msg) => div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(mono::line())
                    .child(icon("icons/alert-circle.svg", 16.0, mono::ink()))
                    .child(row_text("Install did not finish", msg.clone()))
                    .into_any_element(),
                _ if agent.installed => {
                    let v = agent.version.clone().unwrap_or_else(|| version.to_string());
                    let fresh = phase == VendorPhase::Done;
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .px(px(14.0))
                                .py(px(12.0))
                                .rounded(px(12.0))
                                .border_1()
                                .border_color(mono::line())
                                .child(icon("icons/check.svg", 16.0, mono::ink()))
                                .child(row_text(
                                    format!("DisplayLink Manager {v} is installed"),
                                    if agent.running {
                                        "The DisplayLink agent is running."
                                    } else {
                                        "The DisplayLink agent is not running yet."
                                    },
                                ))
                                .when(!agent.running, |d| {
                                    d.child(
                                        mono::pill("setup-start-agent", "Start agent", true)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let _ = neolink_core::agent::start();
                                                neolink_core::agent::invalidate();
                                                this.state.refresh();
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .when(fresh, |d| {
                            d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.0))
                                        .line_height(px(18.0))
                                        .text_color(mono::faint())
                                        .child(
                                            "If macOS asks to allow DisplayLink, approve it in \
                                             System Settings › Privacy & Security.",
                                        ),
                                )
                                .child(mono::pill("setup-open-privacy", "Open", false).on_click(
                                    |_, _, _| open_settings_pane("General"),
                                )),
                        )
                        })
                        .into_any_element()
                }
                _ => div()
                    .text_size(px(12.0))
                    .line_height(px(18.0))
                    .text_color(mono::faint())
                    .child(
                        "Only needed for DisplayLink docks and adapters. macOS will ask for your \
                     administrator password before installing.",
                    )
                    .into_any_element(),
            };

        div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.heading(
                "icons/download.svg",
                mono::ink(),
                "DisplayLink driver",
                "DisplayLink docks need Synaptics' DisplayLink Manager. Neolink doesn't bundle \
                 it: it downloads the official package and checks it against a pinned \
                 fingerprint before anything is installed.",
            ))
            // While installing, the timeline replaces the package card.
            .when(!matches!(phase, VendorPhase::Working { .. }), |d| {
                d.child(package)
            })
            .child(status)
    }

    fn setup_permissions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.state.capture_allowed;
        let ax = crate::accessibility_allowed();
        let row = |first: bool,
                   id: &'static str,
                   title: &'static str,
                   detail: &'static str,
                   ok: bool,
                   cx: &mut Context<Self>,
                   act: fn()| {
            well_row(first).child(row_text(title, detail)).child(if ok {
                granted().into_any_element()
            } else {
                mono::pill(id, "Allow…", true)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        act();
                        this.state.status = "Waiting for permission in System Settings".into();
                        cx.notify();
                    }))
                    .into_any_element()
            })
        };
        div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.heading(
                "icons/shield.svg",
                mono::ink(),
                "Permissions",
                "Neolink manages displays without these today. They unlock screen previews and \
                 display automation as those features land. You can change them any time in \
                 Settings.",
            ))
            .child(
                well()
                    .child(row(
                        true,
                        "setup-screen",
                        "Screen Recording",
                        "Live thumbnails of each display.",
                        capture,
                        cx,
                        || {
                            let _ = neolink_core::displays::request_screen_capture();
                            open_settings_pane("Privacy_ScreenCapture");
                        },
                    ))
                    .child(row(
                        false,
                        "setup-ax",
                        "Accessibility",
                        "Automating display changes on your behalf.",
                        ax,
                        cx,
                        || open_settings_pane("Privacy_Accessibility"),
                    )),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .line_height(px(18.0))
                    .text_color(mono::faint())
                    .child(
                        "macOS may ask you to quit and reopen Neolink after granting Screen \
                         Recording.",
                    ),
            )
    }

    fn setup_preferences(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.heading(
                "icons/power.svg",
                mono::ink(),
                "Startup and appearance",
                "Keep Neolink ready from the moment you sign in, so saved layouts are restored \
                 as soon as your displays connect.",
            ))
            .child(
                well()
                    .child(
                        well_row(true)
                            .child(row_text("Open at login", "Start Neolink when you sign in."))
                            .child(
                                mono::toggle("setup-login", self.state.launch_at_login).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.state.toggle_login_item();
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .child(
                        well_row(false)
                            .child(row_text("Appearance", "Follow macOS, or pick one."))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.0))
                                    .p(px(3.0))
                                    .rounded(px(9.0))
                                    .bg(mono::paper())
                                    .child(self.theme_pill(
                                        "setup-theme-system",
                                        "System",
                                        None,
                                        cx,
                                    ))
                                    .child(self.theme_pill(
                                        "setup-theme-dark",
                                        "Dark",
                                        Some(false),
                                        cx,
                                    ))
                                    .child(self.theme_pill(
                                        "setup-theme-light",
                                        "Light",
                                        Some(true),
                                        cx,
                                    )),
                            ),
                    ),
            )
    }

    fn theme_pill(
        &self,
        id: &'static str,
        label: &'static str,
        value: Option<bool>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        mono::pill(id, label, self.light_theme == value).on_click(cx.listener(
            move |this, _, _, cx| {
                this.light_theme = value;
                crate::save_theme_preference(value);
                cx.notify();
            },
        ))
    }

    fn setup_done(&self) -> impl IntoElement {
        let agent = &self.state.agent;
        let rows: [(&'static str, String, bool); 4] = [
            (
                "DisplayLink driver",
                match (&agent.version, agent.installed) {
                    (Some(v), true) => format!("Installed ({v})"),
                    (None, true) => "Installed".into(),
                    _ => "Skipped. Install later with Get DisplayLink.".into(),
                },
                agent.installed,
            ),
            (
                "Screen Recording",
                if self.state.capture_allowed {
                    "Granted"
                } else {
                    "Not granted"
                }
                .into(),
                self.state.capture_allowed,
            ),
            (
                "Accessibility",
                if crate::accessibility_allowed() {
                    "Granted"
                } else {
                    "Not granted"
                }
                .into(),
                crate::accessibility_allowed(),
            ),
            (
                "Open at login",
                if self.state.launch_at_login {
                    "On"
                } else {
                    "Off"
                }
                .into(),
                self.state.launch_at_login,
            ),
        ];
        let mut list = well();
        for (i, (title, detail, ok)) in rows.into_iter().enumerate() {
            list = list.child(
                well_row(i == 0)
                    .py(px(9.0))
                    .child(mono::dot(if ok { mono::ink() } else { mono::faint() }))
                    .child(div().flex_1().text_size(px(13.0)).child(title))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(mono::faint())
                            .child(detail),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.heading(
                "icons/check.svg",
                mono::ink(),
                "You're all set",
                "Neolink is ready. Drag displays on the canvas to match your desk, and find \
                 every option from this setup again in Settings.",
            ))
            .child(list)
    }
}
