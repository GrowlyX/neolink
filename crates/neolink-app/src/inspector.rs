//! Right-hand inspector: everything about the selected display, plus virtual-display controls.

use crate::theme::{self, c, colors, radius, styled, text};
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
                        .child(ui::logo(28.0))
                        .child(
                            styled(&text::BODY, "Select a display.").text_color(theme::text_dim()),
                        ),
                )
                .into_any_element();
        };

        let index = self.state.selected.unwrap_or(0);
        let current_mode = item.cg.as_ref().and_then(|d| d.mode).map(|m| m.id);
        let modes = item.modes.clone();
        let rates = self.state.rates(index);
        let current_hz = self.state.refresh_hz(index);
        let is_virtual = item.is_virtual();
        let cfg = item.virtual_cfg.clone();

        let mut panel = ui::card()
            .size_full()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .p(px(20.0))
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .children(ui::brand_logo(&item.name).map(|path| {
                                svg()
                                    .path(path)
                                    .w(px(34.0))
                                    .h(px(20.0))
                                    .flex_none()
                                    .text_color(theme::text())
                            }))
                            .child(
                                styled(&text::HEADING_2, item.name.clone())
                                    .min_w(px(0.0))
                                    .whitespace_nowrap()
                                    .truncate(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .when(item.main, |d| {
                                d.child(ui::badge("Main", ui::BadgeKind::Strong))
                            })
                            .child(ui::badge(
                                if is_virtual { "Virtual" } else { "Physical" },
                                ui::BadgeKind::Neutral,
                            ))
                            .when(item.vendor.is_some(), |d| {
                                d.child(ui::badge("DisplayLink", ui::BadgeKind::Neutral))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(6.0))
                    .child(ui::stat("Looks like", item.size_label()))
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
                    )),
            )
            .child(ui::divider());

        // Refresh picker. Only shown when the display really advertises more than one rate at
        // the size it is currently using, so we never invent a timing.
        if rates.len() > 1 {
            panel = panel.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .child(styled(&text::LABEL, "Refresh rate"))
                    .child(ui::segmented().children(rates.into_iter().map(|hz| {
                        ui::segment(
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
                .child(styled(&text::LABEL, "DisplayLink"))
                .child(ui::stat(
                    "Brightness",
                    format!("{:.0}%", v.brightness * 100.0),
                ))
                .child(ui::meter(v.brightness as f32, theme::accent()))
                .child(ui::slider(
                    gpui::ElementId::Name(format!("bright-{}", item.uuid).into()),
                    v.brightness as f32,
                    theme::accent(),
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
                .child(ui::meter(v.contrast as f32, theme::accent()))
                .child(ui::slider(
                    gpui::ElementId::Name(format!("contrast-{}", item.uuid).into()),
                    v.contrast as f32,
                    theme::accent(),
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
                            ui::button(
                                "btn-enabled",
                                if v.enabled { "Disable" } else { "Enable" },
                                ui::ButtonKind::Secondary,
                                true,
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
                            ui::button(
                                "btn-flip",
                                if v.flipped { "Unflip" } else { "Flip" },
                                ui::ButtonKind::Secondary,
                                true,
                            )
                            .on_click(cx.listener(
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
            .gap(px(2.0))
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll();

        for m in modes.into_iter().take(24) {
            let active = current_mode == Some(m.id);
            let label = m.label();
            let id = gpui::ElementId::Name(format!("mode-{}-{}", item.uuid, m.id).into());
            let k = colors();
            let (fg, sub) = if active {
                (k.on_accent, k.on_accent)
            } else {
                (k.text_primary, k.text_secondary)
            };
            let hover = k.overlay_hover;
            let (size_part, rest) = match label.split_once(" @ ") {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (label.clone(), String::new()),
            };
            mode_list = mode_list.child(
                div()
                    .id(id)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(34.0))
                    .px(px(12.0))
                    .rounded(px(radius::INNER))
                    .text_color(c(fg))
                    .when(active, |el| el.bg(c(k.accent)))
                    .when(!active, |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.bg(c(hover)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(e) = this.state.set_mode(index, m.id) {
                                    this.state.status = format!("Mode change failed: {e}");
                                }
                                cx.notify();
                            }))
                    })
                    .child(styled(&text::CODE, size_part).text_size(px(13.0)).flex_1())
                    .child(
                        styled(&text::SUPPORTING, rest)
                            .text_color(c(sub))
                            .opacity(0.8),
                    )
                    .when(active, |el| {
                        el.child(
                            svg()
                                .path("logo/check.svg")
                                .size(px(14.0))
                                .flex_none()
                                .text_color(c(fg)),
                        )
                    }),
            );
        }
        panel = panel
            .child(styled(&text::LABEL, "Resolution").flex_none())
            .child(mode_list);

        // Pinned footer.
        let mut footer = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(10.0))
            .pt(px(14.0))
            .border_t_1()
            .border_color(theme::hairline())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        ui::button(
                            "btn-main",
                            if item.main {
                                "Main display"
                            } else {
                                "Make main display"
                            },
                            ui::ButtonKind::Secondary,
                            !item.main,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Err(e) = this.state.set_main(index) {
                                this.state.status = format!("{e}");
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ui::button("btn-remove", "Remove", ui::ButtonKind::Ghost, true).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Err(e) = this.state.remove(index) {
                                    this.state.status = format!("{e}");
                                }
                                cx.notify();
                            }),
                        ),
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
                        .child(ui::label("HiDPI backing"))
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
                        .child(ui::label("Rotation"))
                        .child(
                            ui::segmented().children([0u32, 90, 180, 270].into_iter().map(|deg| {
                                ui::segment(
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
                            })),
                        ),
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
            footer = footer.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(ui::label("Mirror"))
                    .child(
                        ui::segmented()
                            .child(ui::segment("mirror-off", "None", !mirrored).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Err(e) = this.state.set_mirror(index, None) {
                                        this.state.status = format!("{e}");
                                    }
                                    cx.notify();
                                }),
                            ))
                            .children(targets.into_iter().take(2).map(|(i, name)| {
                                ui::segment(
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
