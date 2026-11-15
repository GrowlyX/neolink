//! Neolink — a native, DisplayLink-compatible display manager for macOS.
//!
//! The window is a single GPUI view. Layout is intentionally split into three glass panes
//! (displays, arrangement canvas, inspector) so it scales from a 13" laptop to an ultrawide.

mod arrangement;
mod devices;
mod inspector;
mod state;
mod theme;
mod ui;

use std::borrow::Cow;

use gpui::AsyncApp;
use gpui::{
    div, point, prelude::*, px, size, svg, App, Application, AssetSource, Bounds, Context,
    SharedString, Window, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};
use state::AppState;

struct Neolink {
    state: AppState,
    settings_open: bool,
    light_theme: Option<bool>,
    /// Active drag on the arrangement canvas, if any.
    drag_anchor: Option<arrangement::Drag>,
    notify_rx: Option<std::sync::mpsc::Receiver<neolink_core::agent::Notification>>,
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
            settings_open: false,
            light_theme: load_theme_preference(),
            drag_anchor: None,
            notify_rx: subscribed.then_some(rx),
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
            .bg(theme::base_top())
            .text_color(theme::text())
            .font_family("Geist")
            .relative()
            .child(self.header(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(20.0))
                    .px(px(28.0))
                    .pb(px(20.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap(px(14.0))
                            .child(self.workspace_header(cx))
                            .child(self.canvas(cx))
                            .child(self.workspace_actions(cx)),
                    )
                    .child(
                        div()
                            .w(px(340.0))
                            .flex_none()
                            .h_full()
                            .child(self.inspector(cx)),
                    ),
            )
            .child(self.status_bar())
            .when(self.settings_open, |root| {
                root.child(self.settings_panel(cx))
            })
    }
}

impl Neolink {
    fn status_bar(&self) -> impl IntoElement {
        div()
            .h(px(38.0))
            .px(px(28.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(13.0))
            .text_color(theme::text_faint())
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .truncate()
                    .text_color(if self.state.status.contains("failed") {
                        theme::DANGER
                    } else {
                        theme::text_faint()
                    })
                    .child(self.state.status.clone()),
            )
            .child(format!(
                "{} display{}",
                self.state.items.len(),
                if self.state.items.len() == 1 { "" } else { "s" }
            ))
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agent = &self.state.agent;
        let status_color = match (agent.installed, agent.running) {
            (true, true) => theme::OK,
            (true, false) => theme::WARN,
            _ => theme::text_faint(),
        };
        let status_text = if agent.installed {
            match (&agent.version, agent.running) {
                (Some(v), true) => format!("DisplayLink {v} running"),
                (Some(v), false) => format!("DisplayLink {v} installed"),
                (None, true) => "DisplayLink running".into(),
                (None, false) => "DisplayLink installed".into(),
            }
        } else {
            "DisplayLink not installed".into()
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .h(px(72.0))
            .px(px(28.0))
            .child(
                svg()
                    .path("logo/neolink-lockup.svg")
                    .w(px(148.0))
                    .h(px(25.0)),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(13.0))
                    .py(px(7.0))
                    .rounded_full()
                    .bg(theme::glass_soft())
                    .border_1()
                    .border_color(theme::hairline_soft())
                    .child(ui::dot(status_color))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_dim())
                            .child(status_text),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        ui::pill("btn-refresh", "Refresh", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state.refresh();
                                this.state.status = "Refreshed".into();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        div()
                            .id("settings-button")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(px(9.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::glass_soft()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = !this.settings_open;
                                cx.notify();
                            }))
                            .child(svg().path("logo/settings.svg").size(px(18.0))),
                    ),
            )
    }

    fn workspace_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().items_end().h(px(62.0)).child(
            div().flex().flex_col().gap(px(3.0)).flex_1()
                .child(div().text_size(px(25.0)).font_weight(gpui::FontWeight::MEDIUM).child("Arrangement"))
                .child(div().text_size(px(14.0)).text_color(theme::text_dim()).child("Drag a display to where it sits on your desk. The bar marks the main display.")),
        ).children((self.state.agent.installed && !self.state.agent.running).then(|| {
            ui::pill("start-agent", "Start agent", false).on_click(cx.listener(|this, _, _, cx| {
                this.state.status = match neolink_core::agent::start() { Ok(()) => "DisplayLink agent started".into(), Err(e) => format!("{e}") };
                this.state.refresh();
                cx.notify();
            }))
        })).child(
            ui::pill("btn-install", if self.state.busy() { "Installing…" } else if self.state.agent.installed { "Reinstall DisplayLink" } else { "Get DisplayLink" }, !self.state.agent.installed)
                .on_click(cx.listener(|this, _, _, cx| { this.state.provision(); cx.notify(); })),
        )
    }

    fn workspace_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_save = !self.state.items.is_empty();
        div()
            .flex()
            .items_center()
            .h(px(36.0))
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(theme::text_dim())
                    .child(if self.state.dirty {
                        "Unsaved display changes"
                    } else {
                        "Everything matches your displays."
                    }),
            )
            .child(
                ui::pill("save-layout", "Save layout", can_save).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.state.status = match this.state.save() {
                            Ok(()) => "Layout saved".into(),
                            Err(e) => format!("Save failed: {e}"),
                        };
                        cx.notify();
                    },
                )),
            )
            .child(
                ui::pill("add-display", "+ Add virtual display", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        let cfg = neolink_core::virtual_display::VirtualDisplayConfig {
                            modes: state::suggested_modes(),
                            hidpi: true,
                            ..Default::default()
                        };
                        if let Err(e) = this.state.add_virtual(cfg) {
                            this.state.status = format!("Could not create display: {e}");
                        }
                        cx.notify();
                    },
                )),
            )
    }

    fn settings_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accessibility_ok = accessibility_allowed();
        div().absolute().top(px(62.0)).right(px(24.0)).w(px(420.0)).p(px(20.0))
            .flex().flex_col().gap(px(15.0)).rounded(px(14.0)).bg(theme::base_bottom())
            .border_1().border_color(theme::hairline()).shadow_lg()
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(18.0)).font_weight(gpui::FontWeight::MEDIUM).child("Settings"))
                .child(ui::pill("close-settings", "×", false).on_click(cx.listener(|this, _, _, cx| { this.settings_open = false; cx.notify(); }))))
            .child(ui::divider())
            .child(div().text_xs().text_color(theme::text_faint()).child("APPEARANCE"))
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap(px(3.0)).child(div().text_sm().child("Theme")).child(div().text_xs().text_color(theme::text_faint()).child("Choose the Neolink color theme.")))
                .child(div().flex().gap(px(4.0)).p(px(3.0)).rounded(px(9.0)).bg(theme::glass_soft())
                    .child(ui::pill("theme-system", "System", self.light_theme.is_none()).on_click(cx.listener(|this, _, _, cx| { this.light_theme = None; save_theme_preference(None); cx.notify(); })))
                    .child(ui::pill("theme-dark", "Dark", self.light_theme == Some(false)).on_click(cx.listener(|this, _, _, cx| { this.light_theme = Some(false); save_theme_preference(Some(false)); cx.notify(); })))
                    .child(ui::pill("theme-light", "Light", self.light_theme == Some(true)).on_click(cx.listener(|this, _, _, cx| { this.light_theme = Some(true); save_theme_preference(Some(true)); cx.notify(); })))
                )
            )
            .child(div().text_xs().text_color(theme::text_faint()).child("LOGIN"))
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap(px(3.0)).child(div().text_sm().child("Open on login")).child(div().text_xs().text_color(theme::text_faint()).child("Start Neolink when you sign in.")))
                .child(ui::toggle("toggle-login", self.state.launch_at_login).on_click(cx.listener(|this, _, _, cx| { this.state.toggle_login_item(); cx.notify(); }))))
            .child(ui::divider())
            .child(div().text_xs().text_color(theme::text_faint()).child("PERMISSIONS"))
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap(px(3.0)).child(div().text_sm().child("Screen Recording")).child(div().text_xs().text_color(theme::text_faint()).child("Used for future screen previews.")))
                .child(ui::pill("permission-screen", if self.state.capture_allowed { "Granted" } else { "Open settings" }, self.state.capture_allowed).on_click(cx.listener(|this, _, _, cx| {
                    if !this.state.capture_allowed { let _ = neolink_core::displays::request_screen_capture(); let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture").spawn(); }
                    cx.notify();
                }))))
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap(px(3.0)).child(div().text_sm().child("Accessibility")).child(div().text_xs().text_color(theme::text_faint()).child("Used for future display automation.")))
                .child(ui::pill("permission-accessibility", if accessibility_ok { "Granted" } else { "Open settings" }, accessibility_ok).on_click(cx.listener(move |_, _, _, _| {
                    if !accessibility_ok { let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility").spawn(); }
                }))))
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
        Ok(bytes.map(|bytes| Cow::Borrowed(bytes)))
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
        ]);
        let bounds = Bounds::centered(None, size(px(1180.0), px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::Blurred,
                window_min_size: Some(size(px(1020.0), px(640.0))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Neolink".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.0), px(16.0))),
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
                                    if this.notify_rx.is_none() || notified || this.settings_open {
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
                                let Some(created) = this.state.add_virtual(cfg).ok() else {
                                    cx.notify();
                                    return;
                                };
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
