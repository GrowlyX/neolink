//! The Neolink logo, computed.
//!
//! Every point comes from a handful of numbers (cap height, stroke weight,
//! angles, gaps); nothing is traced. This is a line-for-line port of
//! `web/src/brand/geometry.ts` and produces pixel-identical output.
//!
//! Coordinates are SVG user units with y pointing down. The wordmark is drawn
//! on a cap height of 100 units, the mark on a 100 x 100 square.
//!
//! Three ways to consume a [`Shape`]:
//!
//! * [`Shape::to_svg`] / [`Shape::to_path_data`]: text, for files and SVG renderers.
//! * [`Shape::segments`]: move / line / circular-arc / close commands.
//! * [`Shape::to_cubics`]: the same outline with every arc replaced by cubic
//!   Béziers, for path builders that have no arc command.

use std::f64::consts::PI;

const CAP: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// One drawing command. All arcs are circular and shorter than a half turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo(Point),
    LineTo(Point),
    /// Circular arc from the current point. `clockwise` is as seen on screen
    /// (y down), which is SVG's `sweep-flag = 1`.
    ArcTo {
        radius: f64,
        clockwise: bool,
        to: Point,
    },
    /// Cubic Bézier. Only produced by [`Shape::to_cubics`].
    CubicTo {
        c1: Point,
        c2: Point,
        to: Point,
    },
    Close,
}

/// A filled outline. Fill rule: nonzero. One color, always.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub segments: Vec<Segment>,
    /// `[min_x, min_y, width, height]`
    pub view_box: [f64; 4],
}

/// Where to draw: offset and uniform scale applied to every point.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
}

impl Default for Place {
    fn default() -> Self {
        Place {
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
        }
    }
}

impl Place {
    fn pt(&self, x: f64, y: f64) -> Point {
        Point {
            x: self.dx + x * self.scale,
            y: self.dy + y * self.scale,
        }
    }
}

/// 2 decimals, no trailing zeros: 12.50 -> "12.5", -0.001 -> "0".
fn n(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

impl Shape {
    pub fn width(&self) -> f64 {
        self.view_box[2]
    }

    pub fn height(&self) -> f64 {
        self.view_box[3]
    }

    /// Width divided by height. Size the logo by height and derive the width.
    pub fn aspect_ratio(&self) -> f64 {
        self.view_box[2] / self.view_box[3]
    }

    /// SVG path data (`d` attribute).
    pub fn to_path_data(&self) -> String {
        let mut d = String::new();
        for seg in &self.segments {
            match *seg {
                Segment::MoveTo(p) => d += &format!("M{} {}", n(p.x), n(p.y)),
                Segment::LineTo(p) => d += &format!("L{} {}", n(p.x), n(p.y)),
                Segment::ArcTo {
                    radius,
                    clockwise,
                    to,
                } => {
                    d += &format!(
                        "A{} {} 0 0 {} {} {}",
                        n(radius),
                        n(radius),
                        clockwise as u8,
                        n(to.x),
                        n(to.y)
                    )
                }
                Segment::CubicTo { c1, c2, to } => {
                    d += &format!(
                        "C{} {} {} {} {} {}",
                        n(c1.x),
                        n(c1.y),
                        n(c2.x),
                        n(c2.y),
                        n(to.x),
                        n(to.y)
                    )
                }
                Segment::Close => d.push('Z'),
            }
        }
        d
    }

    /// A complete standalone SVG document.
    ///
    /// `fill` and `background` are CSS colors. `padding` adds space around the
    /// shape in view-box units.
    pub fn to_svg(&self, fill: &str, background: Option<&str>, padding: f64) -> String {
        let [x, y, w, h] = self.view_box;
        let vb = [
            x - padding,
            y - padding,
            w + 2.0 * padding,
            h + 2.0 * padding,
        ]
        .map(n);
        let bg = match background {
            Some(color) => format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                vb[0], vb[1], vb[2], vb[3], color
            ),
            None => String::new(),
        };
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{}\" role=\"img\" aria-label=\"Neolink\">\
             <title>Neolink</title>{}<path fill=\"{}\" d=\"{}\"/></svg>\n",
            vb.join(" "),
            bg,
            fill,
            self.to_path_data()
        )
    }

    /// The same outline with every arc replaced by cubic Béziers (at most a
    /// quarter turn each; the error is far below a hundredth of a unit).
    pub fn to_cubics(&self) -> Shape {
        let mut out = Vec::with_capacity(self.segments.len() * 2);
        let mut current = Point { x: 0.0, y: 0.0 };
        let mut start = current;
        for seg in &self.segments {
            match *seg {
                Segment::MoveTo(p) => {
                    current = p;
                    start = p;
                    out.push(*seg);
                }
                Segment::LineTo(p) => {
                    current = p;
                    out.push(*seg);
                }
                Segment::CubicTo { to, .. } => {
                    current = to;
                    out.push(*seg);
                }
                Segment::ArcTo {
                    radius,
                    clockwise,
                    to,
                } => {
                    arc_to_cubics(current, radius, clockwise, to, &mut out);
                    current = to;
                }
                Segment::Close => {
                    current = start;
                    out.push(*seg);
                }
            }
        }
        Shape {
            segments: out,
            view_box: self.view_box,
        }
    }
}

/// Minor circular arc -> cubic Béziers.
fn arc_to_cubics(from: Point, r: f64, clockwise: bool, to: Point, out: &mut Vec<Segment>) {
    // Centre: on the perpendicular bisector of the chord, on the side that
    // makes the short way round match the requested direction.
    let (mx, my) = ((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let chord = dx.hypot(dy);
    let h = (r * r - chord * chord / 4.0).max(0.0).sqrt();
    let side = if clockwise { 1.0 } else { -1.0 };
    let cx = mx - side * h * dy / chord;
    let cy = my + side * h * dx / chord;

    let a0 = (from.y - cy).atan2(from.x - cx);
    let mut delta = (to.y - cy).atan2(to.x - cx) - a0;
    // on screen (y down) a clockwise turn is an increasing angle
    if clockwise && delta < 0.0 {
        delta += 2.0 * PI;
    }
    if !clockwise && delta > 0.0 {
        delta -= 2.0 * PI;
    }

    let pieces = (delta.abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let step = delta / pieces as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for i in 0..pieces {
        let (t0, t1) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (p0x, p0y) = (cx + r * t0.cos(), cy + r * t0.sin());
        let (p1x, p1y) = (cx + r * t1.cos(), cy + r * t1.sin());
        let end = if i + 1 == pieces {
            to
        } else {
            Point { x: p1x, y: p1y }
        };
        out.push(Segment::CubicTo {
            c1: Point {
                x: p0x - k * r * t0.sin(),
                y: p0y + k * r * t0.cos(),
            },
            c2: Point {
                x: p1x + k * r * t1.sin(),
                y: p1y - k * r * t1.cos(),
            },
            to: end,
        });
    }
}

fn polygon(points: &[(f64, f64)], place: Place, out: &mut Vec<Segment>) {
    for (i, &(x, y)) in points.iter().enumerate() {
        let p = place.pt(x, y);
        out.push(if i == 0 {
            Segment::MoveTo(p)
        } else {
            Segment::LineTo(p)
        });
    }
    out.push(Segment::Close);
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> [(f64, f64); 4] {
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// A diagonal stroke is a parallelogram with horizontal ends. For it to look
/// as heavy as the vertical stems its *perpendicular* thickness must equal
/// `weight`, which makes the horizontal end wider than `weight`. Solve for
/// that width by bisection.
///
/// `span` is the total horizontal distance the diagonal covers (ends
/// included) and `rise` the vertical distance.
pub fn diagonal_width(span: f64, rise: f64, weight: f64) -> f64 {
    let (mut lo, mut hi, mut d) = (0.0, span, 0.0);
    for _ in 0..80 {
        d = (lo + hi) / 2.0;
        if d * rise / (span - d).hypot(rise) < weight {
            lo = d;
        } else {
            hi = d;
        }
    }
    d
}

// ---------------------------------------------------------------------------
// The mark: a ring and a diagonal bar that together read as a round N.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct MarkOptions {
    /// Outer radius. The mark occupies a 2R x 2R square.
    pub radius: f64,
    /// Stroke weight of both the ring and the bar.
    pub weight: f64,
    /// Width of the two breaks in the ring, measured parallel to the bar.
    pub gap: f64,
    /// Angle of the bar from horizontal, in degrees. 90 is upright.
    pub angle: f64,
}

impl Default for MarkOptions {
    /// The official mark. Do not ship anything else.
    fn default() -> Self {
        MarkOptions {
            radius: 50.0,
            weight: 16.0,
            gap: 8.0,
            angle: 62.0,
        }
    }
}

/// Built with the bar vertical, then rotated into place.
///
/// The ring is broken twice, once on each side of the bar and at opposite
/// ends: right of the bar at the top, left of it at the bottom. What is left
/// is one continuous stroke: up the left arc, down the bar, up the right arc.
/// That is the stroke order of a capital N.
fn mark_segments(o: MarkOptions, place: Place, out: &mut Vec<Segment>) {
    let (big, small) = (o.radius, o.radius - o.weight);
    let a = o.weight / 2.0; // the bar occupies -a..a
    let b = a + o.gap; // each break occupies a..b
    let outer_y = |x: f64| (big * big - x * x).sqrt();
    let inner_y = |x: f64| (small * small - x * x).sqrt();

    let lean = (90.0 - o.angle).to_radians();
    let (sin, cos) = lean.sin_cos();
    // local (bar vertical, origin at centre) -> square, bar leaning left
    let p = |x: f64, y: f64| place.pt(big + x * cos + y * sin, big - x * sin + y * cos);
    let arc = |radius: f64, clockwise: bool, x: f64, y: f64| Segment::ArcTo {
        radius: radius * place.scale,
        clockwise,
        to: p(x, y),
    };

    out.push(Segment::MoveTo(p(a, -outer_y(a)))); // top-right corner of the bar
    out.push(arc(big, false, -b, outer_y(b))); // outer ring: over the top, down the left
    out.push(Segment::LineTo(p(-b, inner_y(b)))); // bottom-left break
    out.push(arc(small, true, -a, -inner_y(a))); // inner ring: back up the left
    out.push(Segment::LineTo(p(-a, outer_y(a)))); // down the left edge of the bar
    out.push(arc(big, false, b, -outer_y(b))); // outer ring: under the bottom, up the right
    out.push(Segment::LineTo(p(b, -inner_y(b)))); // top-right break
    out.push(arc(small, true, a, inner_y(a))); // inner ring: back down the right
    out.push(Segment::Close); // up the right edge of the bar
}

/// The official mark.
pub fn mark() -> Shape {
    mark_with(MarkOptions::default())
}

/// The mark with other numbers. For exploration and animation only.
pub fn mark_with(options: MarkOptions) -> Shape {
    let mut segments = Vec::new();
    mark_segments(options, Place::default(), &mut segments);
    Shape {
        segments,
        view_box: [0.0, 0.0, 2.0 * options.radius, 2.0 * options.radius],
    }
}

// ---------------------------------------------------------------------------
// The wordmark: N E O L I N K, drawn letter by letter.
// ---------------------------------------------------------------------------

/// Stroke weight of the letters on a cap height of 100.
const WEIGHT: f64 = 14.0;
/// Space between the K's stem and its chevron.
const K_GAP: f64 = 6.0;
/// Width of the diagonal cut through the O.
const O_GAP: f64 = 8.0;

// Letter widths. The O overshoots the cap height by 2 units top and bottom.
const W_N: f64 = 92.0;
const W_E: f64 = 70.0;
const W_O: f64 = 104.0;
const W_L: f64 = 66.0;
const W_K: f64 = 84.0;

/// Space after each letter, tuned by eye rather than fixed: round and open
/// letters sit closer than two straight stems.   N-E  E-O  O-L  L-I  I-N  N-K
const SPACING: [f64; 6] = [38.0, 32.0, 34.0, 26.0, 38.0, 38.0];

fn letter_n(w: f64, place: Place, out: &mut Vec<Segment>) {
    let d = diagonal_width(W_N, CAP, w);
    let run = W_N - d;
    polygon(
        &[
            (0.0, 0.0),
            (0.0, CAP),
            (w, CAP),
            (w, w * CAP / run),
            (run, CAP),
            (W_N, CAP),
            (W_N, 0.0),
            (W_N - w, 0.0),
            (W_N - w, (W_N - w - d) * CAP / run),
            (d, 0.0),
        ],
        place,
        out,
    );
}

/// Three bars and no stem.
fn letter_e(w: f64, place: Place, out: &mut Vec<Segment>) {
    polygon(&rect(0.0, 0.0, W_E, w), place, out);
    polygon(
        &rect(0.0, CAP / 2.0 - w / 2.0, W_E, CAP / 2.0 + w / 2.0),
        place,
        out,
    );
    polygon(&rect(0.0, CAP - w, W_E, CAP), place, out);
}

/// A ring split in two by a diagonal channel. The channel runs "/" at the
/// mirror image of the N's diagonal, so the two letters rhyme.
fn letter_o(w: f64, gap: f64, angle_deg: f64, place: Place, out: &mut Vec<Segment>) {
    let big = W_O / 2.0;
    let small = big - w;
    let (cx, cy) = (big, CAP / 2.0);
    let phi = angle_deg.to_radians();
    let (ux, uy) = (phi.cos(), -phi.sin()); // along the channel, up and right
    for side in [1.0, -1.0] {
        let (nx, ny) = (side * phi.sin(), side * phi.cos());
        let off = gap / 2.0;
        let on = |rad: f64, t: f64| {
            let along = (rad * rad - off * off).sqrt() * t;
            (cx + off * nx + along * ux, cy + off * ny + along * uy)
        };
        let (p1, p2, q2, q1) = (on(big, 1.0), on(big, -1.0), on(small, -1.0), on(small, 1.0));
        let clockwise = (p1.0 - cx) * ny - (p1.1 - cy) * nx > 0.0;
        out.push(Segment::MoveTo(place.pt(p1.0, p1.1)));
        out.push(Segment::ArcTo {
            radius: big * place.scale,
            clockwise,
            to: place.pt(p2.0, p2.1),
        });
        out.push(Segment::LineTo(place.pt(q2.0, q2.1)));
        out.push(Segment::ArcTo {
            radius: small * place.scale,
            clockwise: !clockwise,
            to: place.pt(q1.0, q1.1),
        });
        out.push(Segment::Close);
    }
}

fn letter_l(w: f64, place: Place, out: &mut Vec<Segment>) {
    polygon(
        &[
            (0.0, 0.0),
            (w, 0.0),
            (w, CAP - w),
            (W_L, CAP - w),
            (W_L, CAP),
            (0.0, CAP),
        ],
        place,
        out,
    );
}

fn letter_i(w: f64, place: Place, out: &mut Vec<Segment>) {
    polygon(&rect(0.0, 0.0, w, CAP), place, out);
}

/// A stem and a detached chevron.
fn letter_k(w: f64, gap: f64, place: Place, out: &mut Vec<Segment>) {
    let x0 = w + gap;
    let d = diagonal_width(W_K - x0, CAP / 2.0, w);
    polygon(&rect(0.0, 0.0, w, CAP), place, out);
    polygon(
        &[
            (x0, CAP / 2.0),
            (W_K - d, 0.0),
            (W_K, 0.0),
            (x0 + d, CAP / 2.0),
            (W_K, CAP),
            (W_K - d, CAP),
        ],
        place,
        out,
    );
}

/// Advance width of the wordmark at scale 1.
pub fn wordmark_width() -> f64 {
    W_N + W_E + W_O + W_L + WEIGHT + W_N + W_K + SPACING.iter().sum::<f64>()
}

fn wordmark_segments(place: Place, out: &mut Vec<Segment>) {
    let w = WEIGHT;
    // the O's cut mirrors the N's diagonal
    let o_angle = CAP.atan2(W_N - diagonal_width(W_N, CAP, w)).to_degrees();
    let widths = [W_N, W_E, W_O, W_L, w, W_N, W_K];
    let mut x = 0.0;
    for (i, width) in widths.iter().enumerate() {
        let at = Place {
            dx: place.dx + x * place.scale,
            dy: place.dy,
            scale: place.scale,
        };
        match i {
            0 | 5 => letter_n(w, at, out),
            1 => letter_e(w, at, out),
            2 => letter_o(w, O_GAP, o_angle, at, out),
            3 => letter_l(w, at, out),
            4 => letter_i(w, at, out),
            _ => letter_k(w, K_GAP, at, out),
        }
        x += width + SPACING.get(i).copied().unwrap_or(0.0);
    }
}

/// The NEOLINK lettering.
pub fn wordmark() -> Shape {
    let mut segments = Vec::new();
    wordmark_segments(Place::default(), &mut segments);
    // 2 units of overshoot above and below the cap height, for the O
    Shape {
        segments,
        view_box: [0.0, -2.0, wordmark_width(), CAP + 4.0],
    }
}

// ---------------------------------------------------------------------------
// The lockup: mark + wordmark on one line. The default logo.
// ---------------------------------------------------------------------------

/// Mark diameter in the lockup, on a cap height of 100.
pub const LOCKUP_MARK_SIZE: f64 = 136.0;
/// Space between mark and wordmark in the lockup.
pub const LOCKUP_GAP: f64 = 60.0;

pub fn lockup() -> Shape {
    let top = CAP / 2.0 - LOCKUP_MARK_SIZE / 2.0;
    let official = MarkOptions::default();
    let mut segments = Vec::new();
    mark_segments(
        official,
        Place {
            dx: 0.0,
            dy: top,
            scale: LOCKUP_MARK_SIZE / (2.0 * official.radius),
        },
        &mut segments,
    );
    wordmark_segments(
        Place {
            dx: LOCKUP_MARK_SIZE + LOCKUP_GAP,
            dy: 0.0,
            scale: 1.0,
        },
        &mut segments,
    );
    Shape {
        segments,
        view_box: [
            0.0,
            top,
            LOCKUP_MARK_SIZE + LOCKUP_GAP + wordmark_width(),
            LOCKUP_MARK_SIZE,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_mark_path_is_stable() {
        assert_eq!(
            mark().to_path_data(),
            "M33.89 2.67A50 50 0 0 0 58.11 99.34L49.96 84A34 34 0 0 1 27.42 24.58L66.11 97.33\
             A50 50 0 0 0 41.89 0.66L50.04 16A34 34 0 0 1 72.58 75.42Z"
        );
    }

    #[test]
    fn dimensions() {
        assert_eq!(mark().view_box, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(wordmark().view_box, [0.0, -2.0, 728.0, 104.0]);
        assert_eq!(lockup().view_box, [0.0, -18.0, 924.0, 136.0]);
    }

    #[test]
    fn cubics_keep_every_endpoint() {
        for shape in [mark(), wordmark(), lockup()] {
            let cubic = shape.to_cubics();
            assert!(cubic
                .segments
                .iter()
                .all(|s| !matches!(s, Segment::ArcTo { .. })));
            let arcs = shape
                .segments
                .iter()
                .filter(|s| matches!(s, Segment::ArcTo { .. }))
                .count();
            let curves = cubic
                .segments
                .iter()
                .filter(|s| matches!(s, Segment::CubicTo { .. }))
                .count();
            assert!(curves >= arcs);
            // every curve ends on the outline, so inside the view box
            // (control points may legitimately sit just outside it)
            let [x, y, w, h] = shape.view_box;
            for s in &cubic.segments {
                if let Segment::CubicTo { to, .. } = s {
                    assert!(to.x >= x - 0.01 && to.x <= x + w + 0.01, "{to:?}");
                    assert!(to.y >= y - 0.01 && to.y <= y + h + 0.01, "{to:?}");
                }
            }
        }
    }
}
