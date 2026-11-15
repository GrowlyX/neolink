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

use gpui::AsyncApp;
use gpui::{
    div, linear_color_stop, linear_gradient, point, prelude::*, px, size, App, Application, Bounds,
    Context, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use state::AppState;

struct Neolink {
    state: AppState,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(linear_gradient(
                160.0,
                linear_color_stop(theme::with_alpha(theme::BASE_TOP, 0.74), 0.0),
                linear_color_stop(theme::with_alpha(theme::BASE_BOTTOM, 0.80), 1.0),
            ))
            .text_color(theme::TEXT)
            .font_family(".SystemUIFont")
            .relative()
            .child(ambient_glow())
            .child(self.header(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap_3()
                    .px_3()
                    .pb_3()
                    .child(
                        div()
                            .w(px(268.0))
                            .flex_none()
                            .h_full()
                            .child(self.device_list(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .child(self.canvas(cx)),
                    )
                    .child(
                        div()
                            .w(px(300.0))
                            .flex_none()
                            .h_full()
                            .child(self.inspector(cx)),
                    ),
            )
            .child(self.status_bar())
    }
}

impl Neolink {
    fn status_bar(&self) -> impl IntoElement {
        div()
            .h(px(24.0))
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(theme::TEXT_FAINT)
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .truncate()
                    .text_color(if self.state.status.contains("failed") {
                        theme::DANGER
                    } else {
                        theme::TEXT_FAINT
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
            _ => theme::TEXT_FAINT,
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
            .gap_3()
            .h(px(58.0))
            .pt(px(10.0))
            .px_4()
            .child(div().w(px(56.0)))
            .child(ui::logo(26.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Neolink"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_FAINT)
                            .child("Display manager"),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .rounded_full()
                    .bg(theme::GLASS_SOFT)
                    .border_1()
                    .border_color(theme::HAIRLINE_SOFT)
                    .child(ui::dot(status_color))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_DIM)
                            .child(status_text),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
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
                        ui::pill("btn-login", "At login", self.state.launch_at_login).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.state.toggle_login_item();
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        ui::pill(
                            "btn-install",
                            if self.state.busy() {
                                "Installing…"
                            } else if agent.installed {
                                "Reinstall DisplayLink"
                            } else {
                                "Get DisplayLink"
                            },
                            !agent.installed,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.provision();
                            cx.notify();
                        })),
                    ),
            )
    }
}

/// Soft radial-ish accents painted behind the glass panes so the blur has something to pick up.
fn ambient_glow() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left(px(-120.0))
                .top(px(-160.0))
                .size(px(620.0))
                .rounded_full()
                .bg(linear_gradient(
                    135.0,
                    linear_color_stop(theme::with_alpha(theme::ACCENT, 0.13), 0.0),
                    linear_color_stop(theme::with_alpha(theme::ACCENT, 0.0), 1.0),
                )),
        )
        .child(
            div()
                .absolute()
                .right(px(-140.0))
                .bottom(px(-200.0))
                .size(px(620.0))
                .rounded_full()
                .bg(linear_gradient(
                    315.0,
                    linear_color_stop(theme::with_alpha(theme::ACCENT_SOFT, 0.10), 0.0),
                    linear_color_stop(theme::with_alpha(theme::ACCENT_SOFT, 0.0), 1.0),
                )),
        )
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
    Application::new().run(|cx: &mut App| {
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
                                    if this.notify_rx.is_none() || notified {
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
