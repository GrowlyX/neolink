//! The arrangement canvas.
//!
//! Displays are drawn to scale, fitted and centred in the pane, and can be dragged to express
//! their physical arrangement. Positions snap to a 10pt grid and are committed to CoreGraphics
//! on mouse-up, the same transaction the System Settings pane performs.
//!
//! The scale is frozen for the length of a drag so the canvas never rescales under the cursor.

use crate::theme::{self, c, colors, radius, styled, text};
use crate::ui;
use gpui::prelude::*;
use gpui::{
    div, px, svg, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};

/// Breathing room between the arrangement and the canvas edge.
const MARGIN: f32 = 56.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    index: usize,
    start_x: i32,
    start_y: i32,
    mouse_x: f32,
    mouse_y: f32,
    fit: Fit,
}

/// Desktop points to canvas pixels: `canvas = (desktop - origin) * scale + offset`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fit {
    scale: f32,
    origin_x: f32,
    origin_y: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Fit {
    fn new(items: &[crate::state::Item], w: f32, h: f32, zoom: f32) -> Fit {
        if items.is_empty() {
            return Fit {
                scale: 1.0,
                origin_x: 0.0,
                origin_y: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
            };
        }
        let min_x = items.iter().map(|i| i.x).min().unwrap_or(0) as f32;
        let min_y = items.iter().map(|i| i.y).min().unwrap_or(0) as f32;
        let max_x = items
            .iter()
            .map(|i| i.x + i.width as i32)
            .max()
            .unwrap_or(1) as f32;
        let max_y = items
            .iter()
            .map(|i| i.y + i.height as i32)
            .max()
            .unwrap_or(1) as f32;
        let (bw, bh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
        let scale = ((w - 2.0 * MARGIN) / bw)
            .min((h - 2.0 * MARGIN) / bh)
            .clamp(0.02, 0.2)
            * zoom;
        Fit {
            scale,
            origin_x: min_x,
            origin_y: min_y,
            offset_x: (w - bw * scale) / 2.0,
            offset_y: (h - bh * scale) / 2.0,
        }
    }

    fn place(&self, x: i32, y: i32) -> (f32, f32) {
        (
            (x as f32 - self.origin_x) * self.scale + self.offset_x,
            (y as f32 - self.origin_y) * self.scale + self.offset_y,
        )
    }
}

impl crate::Neolink {
    /// The canvas fills what the header, actions row and inspector leave over.
    fn canvas_size(window: &Window) -> (f32, f32) {
        let v = window.viewport_size();
        let w = f32::from(v.width) - 2.0 * 24.0 - 16.0 - 340.0;
        let h = f32::from(v.height) - 60.0 - 56.0 - 36.0 - 2.0 * 16.0 - 24.0;
        (w.max(200.0), h.max(160.0))
    }

    pub(crate) fn canvas(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = &self.state.items;
        let (w, h) = Self::canvas_size(window);
        let fit = match self.drag_anchor {
            Some(drag) => drag.fit,
            None => Fit::new(items, w, h, self.state.canvas_zoom),
        };
        let k = colors();

        let mut inner = ui::card()
            .id("canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                let Some(dr) = this.drag_anchor else { return };
                if e.pressed_button != Some(MouseButton::Left) {
                    this.finish_drag(cx);
                    return;
                }
                let dx = f32::from(e.position.x) - dr.mouse_x;
                let dy = f32::from(e.position.y) - dr.mouse_y;
                let nx = dr.start_x + (dx / dr.fit.scale).round() as i32;
                let ny = dr.start_y + (dy / dr.fit.scale).round() as i32;
                if let Some(item) = this.state.items.get_mut(dr.index) {
                    item.x = (nx / 10) * 10;
                    item.y = (ny / 10) * 10;
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
            )
            // Zoom and tidy: small, quiet, top right.
            .child(
                div().absolute().top(px(12.0)).right(px(12.0)).child(
                    ui::segmented()
                        .child(ui::segment("zoom-out", "−", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state.scale_step(-0.1);
                                cx.notify();
                            },
                        )))
                        .child(ui::segment("zoom-in", "+", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state.scale_step(0.1);
                                cx.notify();
                            },
                        )))
                        .child(ui::segment("tidy", "Tidy", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                if let Err(e) = this.state.tidy() {
                                    this.state.status = format!("Tidy failed: {e}");
                                }
                                cx.notify();
                            },
                        ))),
                ),
            );

        // Draw the selected display last so it sits above its neighbours.
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|i| self.state.selected == Some(*i));
        for index in order {
            let item = &items[index];
            let (x, y) = fit.place(item.x, item.y);
            let tw = item.width as f32 * fit.scale;
            let th = item.height as f32 * fit.scale;
            let selected = self.state.selected == Some(index);
            let dragging = self.drag_anchor.is_some_and(|d| d.index == index);
            let (bg, fg, border) = if selected {
                (k.accent, k.on_accent, k.accent)
            } else {
                (k.background_muted, k.text_primary, k.border_emphasized)
            };
            let roomy = tw > 96.0 && th > 54.0;
            let brand = ui::brand_logo(&item.name);

            inner = inner.child(
                div()
                    .id(("display-card", index))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(tw))
                    .h(px(th))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .overflow_hidden()
                    .rounded(px(radius::INNER))
                    .border_1()
                    .border_color(c(border))
                    .bg(c(bg))
                    .text_color(c(fg))
                    .when(dragging, |el| el.opacity(0.88))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.state.selected = Some(index);
                            this.state.dragging = Some(index);
                            let (w, h) = Self::canvas_size(window);
                            this.drag_anchor = Some(Drag {
                                index,
                                start_x: this.state.items[index].x,
                                start_y: this.state.items[index].y,
                                mouse_x: e.position.x.into(),
                                mouse_y: e.position.y.into(),
                                fit: Fit::new(&this.state.items, w, h, this.state.canvas_zoom),
                            });
                            cx.notify();
                        }),
                    )
                    // The main display carries the menu bar.
                    .when(item.main, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(6.0))
                                .left(px(8.0))
                                .right(px(8.0))
                                .h(px(4.0))
                                .rounded(px(radius::FULL))
                                .bg(c(fg))
                                .opacity(0.45),
                        )
                    })
                    .when(roomy, |el| {
                        el.children(brand.map(|path| {
                            svg()
                                .path(path)
                                .w(px(36.0))
                                .h(px(14.0))
                                .flex_none()
                                .text_color(c(fg))
                        }))
                        .child(
                            styled(&text::LABEL, item.name.clone())
                                .max_w(px(tw - 16.0))
                                .overflow_hidden()
                                .whitespace_nowrap(),
                        )
                        .child(styled(&text::CODE, item.size_label()).opacity(0.7))
                    }),
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
                    .gap(px(4.0))
                    .child(styled(&text::BODY, "No displays found."))
                    .child(
                        styled(&text::SUPPORTING, "Attach a dock or add a virtual display.")
                            .text_color(theme::text_dim()),
                    ),
            );
        } else if items.len() == 1 {
            inner = inner.child(
                div()
                    .absolute()
                    .bottom(px(20.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        styled(
                            &text::SUPPORTING,
                            "One display connected. Plug in another, or add a virtual display.",
                        )
                        .text_color(theme::text_dim()),
                    ),
            );
        }

        inner
    }

    /// Commit a drag: write the new origin through CoreGraphics if it moved.
    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(dr) = self.drag_anchor.take() {
            self.state.dragging = None;
            let (x, y) = self
                .state
                .items
                .get(dr.index)
                .map(|i| (i.x, i.y))
                .unwrap_or((dr.start_x, dr.start_y));
            if (x, y) != (dr.start_x, dr.start_y) {
                if let Err(e) = self.state.set_origin(dr.index, x, y) {
                    self.state.status = format!("Move failed: {e}");
                }
            }
        }
        cx.notify();
    }
}
