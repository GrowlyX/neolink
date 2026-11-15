//! Right-hand inspector: everything about the selected display, plus virtual-display controls.

use crate::theme;
use crate::ui;
use gpui::prelude::*;
use gpui::{div, px, svg, Context};

impl crate::Neolink {
    pub(crate) fn inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(item) = self
            .state
            .selected
            .and_then(|i| self.state.items.get(i))
            .cloned()
        else {
            return div()
                .size_full()
                .child(
                    ui::glass(16.0)
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(ui::logo(30.0))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::text_dim())
                                .child("Select a display"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_faint())
                                .child("to configure it"),
                        ),
                )
                .into_any_element();
        };

        let index = self.state.selected.unwrap_or(0);
        let swatch = theme::swatch(index);
        let current_mode = item.cg.as_ref().and_then(|d| d.mode).map(|m| m.id);
        let modes = item.modes.clone();
        let rates = self.state.rates(index);
        let current_hz = self.state.refresh_hz(index);
        let is_virtual = item.is_virtual();
        let cfg = item.virtual_cfg.clone();

        let mut panel = ui::glass(16.0)
            .size_full()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .p_3()
            .gap_2()
            .child(ui::sheen())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .children(
                        ui::brand_logo(&item.name)
                            .map(|path| svg().path(path).w(px(30.0)).h(px(18.0)).flex_none()),
                    )
                    .child(ui::dot(swatch))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .truncate()
                            .child(item.name.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(ui::chip().flex_none().child(item.size_label()))
                    .child(ui::chip().flex_none().child(if is_virtual {
                        "Virtual"
                    } else {
                        "Physical"
                    }))
                    .when(item.main, |d| d.child(ui::chip().flex_none().child("Main"))),
            )
            .child(ui::divider())
            .child(ui::stat("Position", format!("{}, {}", item.x, item.y)))
            .child(ui::stat("Rotation", format!("{}°", item.rotation)))
            .child(ui::stat(
                "Refresh",
                match current_hz {
                    Some(hz) => format!("{hz} Hz"),
                    None => "-".to_string(),
                },
            ))
            .child(ui::stat(
                "Backing",
                if item.hidpi {
                    "2× HiDPI".to_string()
                } else {
                    "1×".to_string()
                },
            ));

        // Refresh picker. Only shown when the display really advertises more than one rate at
        // the size it is currently using, so we never invent a timing.
        if rates.len() > 1 {
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_dim())
                            .child("Refresh"),
                    )
                    .child(div().flex().gap_1().children(rates.into_iter().map(|hz| {
                        ui::pill(
                            gpui::ElementId::Name(format!("hz-{}-{hz}", item.uuid).into()),
                            format!("{hz} Hz"),
                            current_hz == Some(hz),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.status = match this.state.set_refresh(index, hz) {
                                Ok(()) => format!("Refresh set to {hz} Hz"),
                                Err(e) => format!("Refresh change failed: {e}"),
                            };
                            cx.notify();
                        }))
                    }))),
            );
        }

        if let Some(v) = &item.vendor {
            panel = panel
                .child(ui::divider())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_faint())
                        .child("DISPLAYLINK"),
                )
                .child(ui::stat(
                    "Brightness",
                    format!("{:.0}%", v.brightness * 100.0),
                ))
                .child(ui::meter(v.brightness as f32, theme::ACCENT))
                .child(ui::slider(
                    gpui::ElementId::Name(format!("bright-{}", item.uuid).into()),
                    v.brightness as f32,
                    theme::ACCENT,
                    cx.listener(move |this, set: &ui::SliderSet, _, cx| {
                        if let Err(e) =
                            this.state
                                .set_vendor_level(index, "brightness", set.step as f64)
                        {
                            this.state.status = format!("{e}");
                        }
                        cx.notify();
                    }),
                ))
                .child(ui::stat("Contrast", format!("{:.0}%", v.contrast * 100.0)))
                .child(ui::meter(v.contrast as f32, theme::ACCENT_SOFT))
                .child(ui::slider(
                    gpui::ElementId::Name(format!("contrast-{}", item.uuid).into()),
                    v.contrast as f32,
                    theme::ACCENT_SOFT,
                    cx.listener(move |this, set: &ui::SliderSet, _, cx| {
                        if let Err(e) =
                            this.state
                                .set_vendor_level(index, "contrast", set.step as f64)
                        {
                            this.state.status = format!("{e}");
                        }
                        cx.notify();
                    }),
                ))
                .child(ui::stat(
                    "USB",
                    format!("{:04x}:{:04x}", v.usb_vid, v.usb_pid),
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            ui::pill(
                                "btn-enabled",
                                if v.enabled { "Enabled" } else { "Disabled" },
                                v.enabled,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Err(e) = this.state.toggle_enabled(index) {
                                        this.state.status = format!("{e}");
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            ui::pill("btn-flip", "Flip", v.flipped).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Err(e) = this.state.toggle_flip(index) {
                                        this.state.status = format!("{e}");
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                );
        }

        // Scrollable mode list — the only part that can grow without bound.
        let mut mode_list = div()
            .id("mode-scroll")
            .flex()
            .flex_col()
            .gap_1p5()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme::text_faint())
                    .child("MODE"),
            );

        for m in modes.into_iter().take(24) {
            let active = current_mode == Some(m.id);
            let label = m.label();
            let id = gpui::ElementId::Name(format!("mode-{}-{}", item.uuid, m.id).into());
            mode_list = mode_list.child(
                div()
                    .id(id)
                    .w_full()
                    .px_2()
                    .py_1()
                    .flex_none()
                    .rounded(px(8.0))
                    .bg(if active {
                        theme::with_alpha(theme::ACCENT, 0.16)
                    } else {
                        theme::TRANSPARENT
                    })
                    .border_1()
                    .border_color(if active {
                        theme::with_alpha(theme::ACCENT, 0.5)
                    } else {
                        theme::TRANSPARENT
                    })
                    .text_xs()
                    .text_color(if active {
                        theme::ACCENT
                    } else {
                        theme::text_dim()
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::glass_soft()).text_color(theme::text()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(e) = this.state.set_mode(index, m.id) {
                            this.state.status = format!("Mode change failed: {e}");
                        }
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        panel = panel.child(mode_list);

        // Pinned footer.
        let mut footer = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(theme::hairline_soft())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        ui::pill("btn-main", "Make main", item.main).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Err(e) = this.state.set_main(index) {
                                    this.state.status = format!("{e}");
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        ui::pill("btn-remove", "Remove", false).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Err(e) = this.state.remove(index) {
                                    this.state.status = format!("{e}");
                                }
                                cx.notify();
                            },
                        )),
                    ),
            );

        if let Some(cfg) = cfg {
            let next = cfg.clone();
            let rot = cfg.rotation;
            footer = footer
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_dim())
                                .child("HiDPI backing"),
                        )
                        .child(ui::toggle("toggle-hidpi", cfg.hidpi).on_click(cx.listener(
                            move |this, _, _, cx| {
                                let mut updated = next.clone();
                                updated.hidpi = !updated.hidpi;
                                if let Err(e) = this.state.reconfigure_virtual(index, updated) {
                                    this.state.status = format!("{e}");
                                }
                                cx.notify();
                            },
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_dim())
                                .child("Rotation"),
                        )
                        .child(div().flex().gap_1().children(
                            [0u32, 90, 180, 270].into_iter().map(|deg| {
                                ui::pill(
                                    gpui::ElementId::Name(format!("rot-{deg}").into()),
                                    format!("{deg}°"),
                                    rot == deg,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Err(e) = this.state.rotate(index, deg) {
                                            this.state.status = format!("{e}");
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                        )),
                );
        }

        // Mirroring applies to any display, not just our own.
        let targets: Vec<(usize, String)> = self
            .state
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(i, it)| (i, it.name.clone()))
            .collect();
        if !targets.is_empty() {
            let mirrored = item.mirrored;
            footer =
                footer.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_dim())
                                .child("Mirror"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(ui::pill("mirror-off", "None", !mirrored).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        if let Err(e) = this.state.set_mirror(index, None) {
                                            this.state.status = format!("{e}");
                                        }
                                        cx.notify();
                                    }),
                                ))
                                .children(targets.into_iter().take(2).map(|(i, name)| {
                                    ui::pill(
                                        gpui::ElementId::Name(format!("mirror-{i}").into()),
                                        name,
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Err(e) = this.state.set_mirror(index, Some(i)) {
                                                this.state.status = format!("{e}");
                                            }
                                            cx.notify();
                                        },
                                    ))
                                })),
                        ),
                );
        }

        panel.child(footer).into_any_element()
    }
}
