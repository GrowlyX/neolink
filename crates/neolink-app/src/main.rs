//! Neolink — a native, DisplayLink-compatible display manager for macOS.
//!
//! The window is a single GPUI view. Layout is intentionally split into three glass panes
//! (displays, arrangement canvas, inspector) so it scales from a 13" laptop to an ultrawide.

mod arrangement;
mod devices;
mod inspector;
mod setup;
mod state;
mod theme;
mod ui;

use std::borrow::Cow;

use gpui::AsyncApp;
use gpui::{
    div, point, prelude::*, px, size, svg, App, Application, AssetSource, Bounds, Context,
    SharedString, Window, WindowAppearance, WindowBounds, WindowOptions,
};
use state::AppState;

struct Neolink {
    state: AppState,
    settings_open: bool,
    light_theme: Option<bool>,
    /// Active drag on the arrangement canvas, if any.
    drag_anchor: Option<arrangement::Drag>,
    notify_rx: Option<std::sync::mpsc::Receiver<neolink_core::agent::Notification>>,
    /// First-run setup assistant, when it is on screen.
    setup: Option<setup::Step>,
}

impl Neolink {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        // The callback runs on an XPC queue; it only forwards, so the UI never blocks there.
        let subscribed = neolink_core::agent::subscribe(move |n| {
            let _ = tx.send(n);
        });
        Self {
            state: AppState::new(),
            // Debug builds can open with the panel showing, to iterate on its layout.
            settings_open: cfg!(debug_assertions)
                && std::env::var_os("NEOLINK_OPEN_SETTINGS").is_some(),
            light_theme: load_theme_preference(),
            drag_anchor: None,
            notify_rx: subscribed.then_some(rx),
            setup: setup::initial(),
        }
    }

    /// Drain queued agent notifications. Uses the event stream when it is live, otherwise only
    /// refreshes if a brightness/contrast/flip confirmation needs painting.
    fn drain_notifications(&mut self) -> bool {
        let Some(rx) = &self.notify_rx else {
            return false;
        };
        let mut dirty = false;
        while let Ok(n) = rx.try_recv() {
            if let neolink_core::agent::Notification::Displays { .. } = n {
                dirty = true;
            }
        }
        dirty
    }
}

// Layout, in logical pixels, shared with the reference design.
const TOP_BAR: f32 = 60.0;
const PAD: f32 = 24.0;
const GAP: f32 = 16.0;
const SIDE: f32 = 340.0;

impl Render for Neolink {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::set_light_mode(self.light_theme.unwrap_or(matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        )));
        div()
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(theme::base_top())
            .text_color(theme::text())
            .font_family(theme::text::BODY.family)
            .text_size(px(theme::text::BODY.size))
            .child(self.top_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(GAP))
                    .px(px(PAD))
                    .pb(px(PAD))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(GAP))
                            .child(self.workspace_header(cx))
                            .child(div().flex_1().min_h(px(0.0)).child(self.canvas(window, cx)))
                            .child(self.workspace_actions(cx)),
                    )
                    .child(
                        div()
                            .w(px(SIDE))
                            .flex_none()
                            .h_full()
                            .child(self.inspector(cx)),
                    ),
            )
            .when(self.settings_open, |root| {
                root.child(self.settings_panel(cx))
            })
            .when_some(self.setup, |root, step| {
                root.child(self.setup_overlay(step, cx))
            })
    }
}

impl Neolink {
    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(TOP_BAR))
            // clear the window's close / minimise / zoom buttons
            .pl(px(92.0))
            .pr(px(PAD))
            .gap(px(12.0))
            .child(
                svg()
                    .path("logo/neolink-lockup.svg")
                    .w(px(122.0))
                    .h(px(18.0))
                    .flex_none()
                    .text_color(theme::text()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("settings-button")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(40.0))
                    .rounded(px(theme::radius::INNER))
                    .cursor_pointer()
                    .when(self.settings_open, |el| {
                        el.bg(theme::c(theme::colors().overlay_pressed))
                    })
                    .hover(|s| s.bg(theme::sheen()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = !this.settings_open;
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path("logo/settings.svg")
                            .size(px(24.0))
                            .text_color(theme::text_dim()),
                    ),
            )
    }

    fn displaylink_label(&self) -> String {
        let agent = &self.state.agent;
        if agent.installed {
            match (&agent.version, agent.running) {
                (Some(v), true) => format!("DisplayLink {v}"),
                (Some(v), false) => format!("DisplayLink {v} stopped"),
                (None, true) => "DisplayLink".into(),
                (None, false) => "DisplayLink stopped".into(),
            }
        } else {
            "No DisplayLink driver".into()
        }
    }

    fn workspace_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agent = &self.state.agent;
        let busy = self.state.busy();
        div()
            .flex()
            .flex_none()
            .items_end()
            .gap(px(8.0))
            .h(px(56.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(theme::styled(&theme::text::HEADING_1, "Arrangement"))
                    .child(
                        theme::styled(
                            &theme::text::BODY,
                            "Drag a display to where it sits on your desk. The bar marks the main display.",
                        )
                        .text_color(theme::text_dim()),
                    ),
            )
            .child(
                ui::button("btn-refresh", "Rescan", ui::ButtonKind::Ghost, true).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.state.refresh();
                        this.state.status = "Rescanned displays.".into();
                        cx.notify();
                    }),
                ),
            )
            .children((agent.installed && !agent.running).then(|| {
                ui::button("start-agent", "Start agent", ui::ButtonKind::Secondary, true).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.state.status = match neolink_core::agent::start() {
                            Ok(()) => "DisplayLink agent started.".into(),
                            Err(e) => format!("{e}"),
                        };
                        this.state.refresh();
                        cx.notify();
                    }),
                )
            }))
            .child(
                ui::button(
                    "btn-install",
                    if busy {
                        "Installing…"
                    } else if agent.installed {
                        "Reinstall DisplayLink"
                    } else {
                        "Get DisplayLink"
                    },
                    if agent.installed {
                        ui::ButtonKind::Secondary
                    } else {
                        ui::ButtonKind::Primary
                    },
                    !busy,
                )
                .when(!busy, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.state.provision();
                        cx.notify();
                    }))
                }),
            )
    }

    /// Status on the left, the layout actions on the right.
    fn workspace_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_save = !self.state.items.is_empty();
        let failed = self.state.status.contains("failed") || self.state.status.contains("Failed");
        let status: String = if !self.state.status.is_empty() {
            self.state.status.clone()
        } else if self.state.dirty {
            "Unsaved display changes.".into()
        } else {
            "Everything matches your displays.".into()
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(36.0))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(8.0))
                    .child(
                        theme::styled(&theme::text::BODY, status)
                            .when(failed, |el| {
                                el.text_color(theme::text())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                            })
                            .when(!failed, |el| el.text_color(theme::text_dim()))
                            .whitespace_nowrap()
                            .truncate(),
                    )
                    .child(
                        theme::styled(&theme::text::BODY, self.displaylink_label())
                            .text_color(theme::text_faint())
                            .whitespace_nowrap(),
                    ),
            )
            .child(
                ui::button(
                    "save-layout",
                    "Save layout",
                    ui::ButtonKind::Ghost,
                    can_save,
                )
                .when(can_save, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| {
                        this.state.status = match this.state.save() {
                            Ok(()) => "Layout saved.".into(),
                            Err(e) => format!("Save failed: {e}"),
                        };
                        cx.notify();
                    }))
                }),
            )
            .child(
                ui::button(
                    "add-display",
                    "Add virtual display",
                    ui::ButtonKind::Secondary,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let cfg = neolink_core::virtual_display::VirtualDisplayConfig {
                        modes: state::suggested_modes(),
                        hidpi: true,
                        ..Default::default()
                    };
                    if let Err(e) = this.state.add_virtual(cfg) {
                        this.state.status = format!("Could not create display: {e}");
                    }
                    cx.notify();
                })),
            )
    }

    fn settings_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accessibility_ok = accessibility_allowed();
        let screen_ok = self.state.capture_allowed;
        let agent = &self.state.agent;
        let row = |title: &'static str, detail: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.0))
                .py(px(10.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(3.0))
                        .child(theme::styled(&theme::text::BODY, title))
                        .child(
                            theme::styled(&theme::text::SUPPORTING, detail)
                                .text_color(theme::text_dim()),
                        ),
                )
        };
        let granted = |ok: bool| {
            ui::badge(
                if ok { "Granted" } else { "Not granted" },
                if ok {
                    ui::BadgeKind::Strong
                } else {
                    ui::BadgeKind::Neutral
                },
            )
        };
        let theme_segment =
            |id: &'static str, label: &'static str, value: Option<bool>, cx: &mut Context<Self>| {
                ui::segment(id, label, self.light_theme == value).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.light_theme = value;
                        save_theme_preference(value);
                        cx.notify();
                    },
                ))
            };

        ui::card()
            .id("settings-panel")
            .occlude()
            .absolute()
            .top(px(TOP_BAR + 4.0))
            .right(px(PAD))
            .w(px(500.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(22.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(theme::styled(&theme::text::HEADING_1, "Settings"))
                    .child(
                        ui::button("close-settings", "Done", ui::ButtonKind::Ghost, true).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.settings_open = false;
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(theme::styled(&theme::text::LABEL, "Appearance"))
                    .child(
                        div().flex().child(
                            ui::segmented()
                                .child(theme_segment("theme-system", "Auto", None, cx))
                                .child(theme_segment("theme-light", "Light", Some(true), cx))
                                .child(theme_segment("theme-dark", "Dark", Some(false), cx)),
                        ),
                    ),
            )
            .child(ui::divider())
            .child(
                row("Open on login", "Start Neolink when you sign in.".into())
                    .id("toggle-login-row")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.toggle_login_item();
                        cx.notify();
                    }))
                    .child(ui::toggle("toggle-login", self.state.launch_at_login)),
            )
            .child(ui::divider())
            .child(theme::styled(&theme::text::LABEL, "DisplayLink"))
            .child(
                row(
                    "Driver",
                    if agent.installed {
                        format!(
                            "{} {}",
                            self.displaylink_label(),
                            if agent.running { "is running." } else { "is installed." }
                        )
                    } else {
                        "Needed for DisplayLink docks and adapters.".into()
                    },
                )
                .child(ui::badge(
                    if agent.installed { "Installed" } else { "Not installed" },
                    if agent.installed {
                        ui::BadgeKind::Strong
                    } else {
                        ui::BadgeKind::Neutral
                    },
                )),
            )
            .child(ui::divider())
            .child(theme::styled(&theme::text::LABEL, "Permissions"))
            .child(
                row("Screen Recording", "For future screen previews.".into()).child(if screen_ok {
                    granted(true).into_any_element()
                } else {
                    ui::button("grant-screen", "Allow", ui::ButtonKind::Secondary, true)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let _ = neolink_core::displays::request_screen_capture();
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
                                .spawn();
                            this.state.status =
                                "Allow Neolink in Privacy & Security › Screen Recording.".into();
                            cx.notify();
                        }))
                        .into_any_element()
                }),
            )
            .child(
                row("Accessibility", "For future display automation.".into()).child(
                    if accessibility_ok {
                        granted(true).into_any_element()
                    } else {
                        ui::button("grant-accessibility", "Allow", ui::ButtonKind::Secondary, true)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = std::process::Command::new("open")
                                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                                    .spawn();
                                this.state.status =
                                    "Allow Neolink in Privacy & Security › Accessibility.".into();
                                cx.notify();
                            }))
                            .into_any_element()
                    },
                ),
            )
            .child(ui::divider())
            .child(
                row("Setup assistant", "Driver, permissions and startup, step by step.".into()).child(
                    ui::button("rerun-setup", "Run again", ui::ButtonKind::Secondary, true).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.settings_open = false;
                            this.setup = Some(setup::Step::Welcome);
                            cx.notify();
                        }),
                    ),
                ),
            )
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

fn accessibility_allowed() -> bool {
    unsafe { AXIsProcessTrusted() }
}

fn preference_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join("Library/Application Support/Neolink")
            .join("appearance")
    })
}

fn load_theme_preference() -> Option<bool> {
    let path = preference_path()?;
    match std::fs::read_to_string(path).ok()?.trim() {
        "light" => Some(true),
        "dark" => Some(false),
        _ => None,
    }
}

fn save_theme_preference(theme: Option<bool>) {
    let Some(path) = preference_path() else {
        return;
    };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_ok() {
        let value = match theme {
            Some(true) => "light",
            Some(false) => "dark",
            None => "system",
        };
        let _ = std::fs::write(path, value);
    }
}

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "logo/neolink-lockup.svg" => Some(include_bytes!("../assets/logo/neolink-lockup.svg")),
            "logo/settings.svg" => Some(include_bytes!("../assets/logo/settings.svg")),
            "logo/check.svg" => Some(include_bytes!("../assets/logo/check.svg")),
            "logo/neolink-mark.svg" => {
                Some(include_bytes!("../assets/brand/neolink-mark-white.svg"))
            }
            "icons/alert-circle.svg" => Some(include_bytes!("../assets/icons/alert-circle.svg")),
            "icons/check.svg" => Some(include_bytes!("../assets/icons/check.svg")),
            "icons/chevron-left.svg" => Some(include_bytes!("../assets/icons/chevron-left.svg")),
            "icons/download.svg" => Some(include_bytes!("../assets/icons/download.svg")),
            "icons/monitor.svg" => Some(include_bytes!("../assets/icons/monitor.svg")),
            "icons/power.svg" => Some(include_bytes!("../assets/icons/power.svg")),
            "icons/shield.svg" => Some(include_bytes!("../assets/icons/shield.svg")),
            "brands/acer.svg" => Some(include_bytes!("../assets/brands/acer.svg")),
            "brands/aoc.svg" => Some(include_bytes!("../assets/brands/aoc.svg")),
            "brands/apple.svg" => Some(include_bytes!("../assets/brands/apple.svg")),
            "brands/asus.svg" => Some(include_bytes!("../assets/brands/asus.svg")),
            "brands/benq.svg" => Some(include_bytes!("../assets/brands/benq.svg")),
            "brands/dell.svg" => Some(include_bytes!("../assets/brands/dell.svg")),
            "brands/eizo.svg" => Some(include_bytes!("../assets/brands/eizo.svg")),
            "brands/gigabyte.svg" => Some(include_bytes!("../assets/brands/gigabyte.svg")),
            "brands/hisense.svg" => Some(include_bytes!("../assets/brands/hisense.svg")),
            "brands/hp.svg" => Some(include_bytes!("../assets/brands/hp.svg")),
            "brands/huawei.svg" => Some(include_bytes!("../assets/brands/huawei.svg")),
            "brands/iiyama.svg" => Some(include_bytes!("../assets/brands/iiyama.svg")),
            "brands/lenovo.svg" => Some(include_bytes!("../assets/brands/lenovo.svg")),
            "brands/lg.svg" => Some(include_bytes!("../assets/brands/lg.svg")),
            "brands/microsoft.svg" => Some(include_bytes!("../assets/brands/microsoft.svg")),
            "brands/msi.svg" => Some(include_bytes!("../assets/brands/msi.svg")),
            "brands/nec.svg" => Some(include_bytes!("../assets/brands/nec.svg")),
            "brands/panasonic.svg" => Some(include_bytes!("../assets/brands/panasonic.svg")),
            "brands/philips.svg" => Some(include_bytes!("../assets/brands/philips.svg")),
            "brands/razer.svg" => Some(include_bytes!("../assets/brands/razer.svg")),
            "brands/samsung.svg" => Some(include_bytes!("../assets/brands/samsung.svg")),
            "brands/sharp.svg" => Some(include_bytes!("../assets/brands/sharp.svg")),
            "brands/sony.svg" => Some(include_bytes!("../assets/brands/sony.svg")),
            "brands/tcl.svg" => Some(include_bytes!("../assets/brands/tcl.svg")),
            "brands/viewsonic.svg" => Some(include_bytes!("../assets/brands/viewsonic.svg")),
            "brands/xiaomi.svg" => Some(include_bytes!("../assets/brands/xiaomi.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }
    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn cfg_clone(
    state: &AppState,
    index: usize,
) -> neolink_core::virtual_display::VirtualDisplayConfig {
    state
        .items
        .get(index)
        .and_then(|i| i.virtual_cfg.clone())
        .unwrap_or_default()
}

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        let _ = cx.text_system().add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Thin.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Bold.ttf")),
        ]);
        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(980.0), px(660.0))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Neolink".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(20.0), px(22.0))),
                }),
                ..Default::default()
            },
            |_, cx| {
                let view = cx.new(|_| Neolink::new());
                // Poll the desktop so displays that appear or vanish out-of-band show up.
                let weak = view.downgrade();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(500))
                            .await;
                        if cx
                            .update(|cx| {
                                weak.update(cx, |this, cx| {
                                    let notified = this.drain_notifications();
                                    // With a live notification stream a refresh is only needed for
                                    // structural changes; polling stays as the fallback.
                                    this.state.refresh_vendor();
                                    if this.notify_rx.is_none()
                                        || notified
                                        || this.settings_open
                                        || this.setup.is_some()
                                    {
                                        this.state.refresh();
                                        cx.notify();
                                    }
                                })
                                .is_err()
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                // `NEOLINK_DEMO=1` exercises the real action path on launch (handy without
                // Accessibility permission, which synthetic clicks need).
                if std::env::var_os("NEOLINK_DEMO").is_some() {
                    let weak = view.downgrade();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(900))
                            .await;
                        let _ = cx.update(|cx| {
                            weak.update(cx, |this, cx| {
                                let cfg = neolink_core::virtual_display::VirtualDisplayConfig {
                                    modes: state::suggested_modes(),
                                    name: "Demo Display".into(),
                                    ..Default::default()
                                };
                                let created = match this.state.add_virtual(cfg) {
                                    Ok(c) => c,
                                    Err(e) => {
                                        eprintln!("demo: add_virtual failed: {e}");
                                        cx.notify();
                                        return;
                                    }
                                };
                                eprintln!("demo: created index {created}");
                                if std::env::var("NEOLINK_DEMO").as_deref() == Ok("2") {
                                    // Exercise every action path so a headless run can be
                                    // verified without Accessibility permission.
                                    if let Some(m) = this.state.items.get(created).and_then(|i| {
                                        i.modes
                                            .iter()
                                            .find(|m| m.width == 1920 && m.height == 1080)
                                            .map(|m| m.id)
                                    }) {
                                        this.state.set_mode(created, m).ok();
                                    }
                                    this.state.set_origin(created, 1600, 40).ok();
                                    this.state.set_refresh(created, 75).ok();
                                    this.state.rotate(created, 90).ok();
                                    this.state
                                        .reconfigure_virtual(
                                            created,
                                            neolink_core::virtual_display::VirtualDisplayConfig {
                                                hidpi: false,
                                                ..cfg_clone(&this.state, created)
                                            },
                                        )
                                        .ok();
                                    this.state.tidy().ok();
                                    this.state.save().ok();
                                    this.state.status = "Demo: all actions ran".into();
                                }
                                cx.notify();
                            })
                            .ok();
                        });
                    })
                    .detach();
                }
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
