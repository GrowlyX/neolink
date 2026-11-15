//! The arrangement canvas.
//!
//! Displays are drawn to scale in a shared coordinate space and can be dragged with the mouse to
//! express their physical arrangement. Positions snap to a 10pt grid and are committed to
//! CoreGraphics on mouse-up, which is the same transaction the System Settings pane performs.

use crate::theme;
use crate::ui;
use gpui::prelude::*;
use gpui::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window, div, px};

/// Points of desktop space per canvas point, before zoom.
const CANVAS_PAD: f32 = 28.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    index: usize,
    start_x: i32,
    start_y: i32,
    mouse_x: f32,
    mouse_y: f32,
}

impl crate::Neolink {
    pub(crate) fn canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = &self.state.items;
        let zoom = self.state.canvas_zoom;
        let total_w: f32 = items.iter().map(|i| i.width as f32).sum::<f32>().max(1.0);
        let max_h: f32 = items.iter().map(|i| i.height as f32).fold(1.0f32, f32::max);
        // Fit the whole arrangement into the pane, then apply the user's zoom.
        let fit_w = 520.0 / total_w;
        let fit_h = 380.0 / max_h;
        let scale = fit_w.min(fit_h).min(0.22) * zoom;

        // Normalise so the leftmost/topmost display sits at CANVAS_PAD.
        let min_x = items.iter().map(|i| i.x).min().unwrap_or(0);
        let min_y = items.iter().map(|i| i.y).min().unwrap_or(0);

        let mut inner = ui::glass(16.0)
            .size_full()
            .relative()
            .overflow_hidden()
            .child(ui::sheen())
            .child(
                div()
                    .absolute()
                    .top(px(12.0))
                    .left(px(16.0))
                    .text_xs()
                    .text_color(theme::TEXT_FAINT)
                    .child("ARRANGEMENT"),
            )
            .child(
                div()
                    .absolute()
                    .top(px(10.0))
                    .right(px(12.0))
                    .flex()
                    .gap_1()
                    .child(ui::pill("zoom-out", "−", false).on_click(cx.listener(|this, _, _, cx| {
                        this.state.scale_step(-0.1);
                        cx.notify();
                    })))
                    .child(ui::pill("zoom-in", "+", false).on_click(cx.listener(|this, _, _, cx| {
                        this.state.scale_step(0.1);
                        cx.notify();
                    })))
                    .child(ui::pill("tidy", "Tidy", false).on_click(cx.listener(|this, _, _, cx| {
                        if let Err(e) = this.state.tidy() {
                            this.state.status = format!("Tidy failed: {e}");
                        }
                        cx.notify();
                    }))),
            );

        // Backdrop dot grid, drawn as a cheap repeating pattern of small squares.
        for (row, _) in (0..24).enumerate() {
            let top = 52.0 + row as f32 * 28.0;
            let mut line = div().absolute().top(px(top)).left(px(16.0)).flex().gap_7();
            for _ in 0..40 {
                line = line.child(div().size(px(1.0)).bg(theme::HAIRLINE_SOFT));
            }
            inner = inner.child(line);
        }

        for (index, item) in items.iter().enumerate() {
            let x = CANVAS_PAD + (item.x - min_x) as f32 * scale;
            let y = 72.0 + (item.y - min_y) as f32 * scale;
            let x = x.min(560.0);
            let y = y.min(360.0);
            let w = (item.width as f32 * scale).max(90.0);
            let h = (item.height as f32 * scale).max(56.0);
            let selected = self.state.selected == Some(index);
            let swatch = theme::swatch(index);
            let name = item.name.clone();
            let size_text = item.size_label();
            let is_main = item.main;

            inner = inner.child(
                div()
                    .id(("display-card", index))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(10.0))
                    .bg(if selected { theme::with_alpha(theme::ACCENT, 0.13) } else { theme::GLASS_SOFT })
                    .border_1()
                    .border_color(if selected { swatch } else { theme::HAIRLINE })
                    .cursor_move()
                    .hover(|s| s.border_color(theme::TEXT_FAINT))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            this.state.selected = Some(index);
                            this.state.dragging = Some(index);
                            this.drag_anchor = Some(Drag {
                                index,
                                start_x: this.state.items[index].x,
                                start_y: this.state.items[index].y,
                                mouse_x: e.position.x.into(),
                                mouse_y: e.position.y.into(),
                            });
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        if let Some(dr) = this.drag_anchor {
                            if e.pressed_button.is_none() {
                                return;
                            }
                                                    let ex: f32 = e.position.x.into();
                            let ey: f32 = e.position.y.into();
                            let dx = ex - dr.mouse_x;
                            let dy = ey - dr.mouse_y;
                            let scale = this.state.canvas_scale();
                            let nx = dr.start_x + (dx / scale).round() as i32;
                            let ny = dr.start_y + (dy / scale).round() as i32;
                            let nx = (nx / 10) * 10;
                            let ny = (ny / 10) * 10;
                            if let Some(item) = this.state.items.get_mut(dr.index) {
                                item.x = nx;
                                item.y = ny;
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            if let Some(dr) = this.drag_anchor.take() {
                                this.state.dragging = None;
                                let (x, y) = this
                                    .state
                                    .items
                                    .get(dr.index)
                                    .map(|i| (i.x, i.y))
                                    .unwrap_or((dr.start_x, dr.start_y));
                                if (x, y) != (dr.start_x, dr.start_y) {
                                    if let Err(e) = this.state.set_origin(dr.index, x, y) {
                                        this.state.status = format!("Move failed: {e}");
                                    }
                                }
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(ui::dot(swatch))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::TEXT)
                                            .whitespace_nowrap()
                                            .truncate()
                                            .child(name),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_FAINT)
                                    .whitespace_nowrap()
                                    .truncate()
                                    .child(size_text),
                            )
                            .when(is_main, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .child(ui::chip().flex_none().child("Main")),
                                )
                            }),
                    ),
            );
        }

        if items.is_empty() {
            inner = inner.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(div().text_sm().text_color(theme::TEXT_DIM).child("No displays"))
                    .child(div().text_xs().text_color(theme::TEXT_FAINT).child("Attach a dock or add a virtual display")),
            );
        }

        inner.into_any_element()
    }

    pub(crate) fn _canvas_anchor(_: &mut Window) {}
}
