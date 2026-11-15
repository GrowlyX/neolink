//! Left pane: every display, as a row you can select, toggle, or remove.

use crate::state::{suggested_modes, AppState};
use crate::theme;
use crate::ui;
use gpui::prelude::*;
use gpui::{div, px, Context};
use neolink_core::virtual_display::VirtualDisplayConfig;

impl crate::Neolink {
    // The display list pane is not mounted since the UI refresh; kept for the sidebar layout.
    #[allow(dead_code)]
    pub(crate) fn device_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.state.items.clone();

        let mut list = ui::glass(16.0)
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .child(ui::sheen())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_faint())
                            .child("DISPLAYS"),
                    )
                    .child(ui::chip().child(format!("{}", items.len()))),
            );

        for (index, item) in items.iter().enumerate() {
            let selected = self.state.selected == Some(index);
            let swatch = theme::accent();
            let name = item.name.clone();
            let sub = format!("{}  ·  {}, {}", item.size_label(), item.x, item.y);
            let enabled = item.enabled;
            list = list.child(
                div()
                    .id(("device-row", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded(px(10.0))
                    .bg(if selected {
                        theme::sheen()
                    } else {
                        theme::TRANSPARENT
                    })
                    .border_1()
                    .border_color(if selected {
                        theme::hairline()
                    } else {
                        theme::TRANSPARENT
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::glass_soft()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.selected = Some(index);
                        cx.notify();
                    }))
                    .child(ui::dot(swatch))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::text())
                                    .whitespace_nowrap()
                                    .truncate()
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::text_faint())
                                    .whitespace_nowrap()
                                    .truncate()
                                    .child(sub),
                            ),
                    )
                    .when(item.main, |d| d.child(ui::chip().child("MAIN")))
                    .child(ui::dot(if enabled {
                        theme::ok()
                    } else {
                        theme::text_faint()
                    })),
            );
        }

        list = list
            .child(
                div()
                    .id("device-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll(),
            )
            .child(ui::divider())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_2()
                    .child(
                        ui::pill("add-display", "+ Add virtual display", true).on_click(
                            cx.listener(|this, _, _, cx| {
                                let cfg = VirtualDisplayConfig {
                                    modes: suggested_modes(),
                                    hidpi: true,
                                    ..Default::default()
                                };
                                if let Err(e) = this.state.add_virtual(cfg) {
                                    this.state.status = format!("Could not create display: {e}");
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(ui::pill("save-layout", "Save layout", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.state.status = match this.state.save() {
                                        Ok(()) => "Layout saved".into(),
                                        Err(e) => format!("Save failed: {e}"),
                                    };
                                    cx.notify();
                                }),
                            ))
                            .child(ui::pill("start-agent", "Start agent", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.state.status = match neolink_core::agent::start() {
                                        Ok(()) => "DisplayLink agent started".into(),
                                        Err(e) => format!("{e}"),
                                    };
                                    this.state.refresh();
                                    cx.notify();
                                }),
                            )),
                    ),
            );

        list.into_any_element()
    }

    pub(crate) fn _state_hint(_: &AppState) {}
}
