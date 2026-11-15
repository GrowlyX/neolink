//! Damage tracking for the DL3 tile sender.
//!
//! The dock accepts a frame as a list of tile rectangles, and the vendor path
//! only ships rectangles that changed. This module keeps a compact fingerprint
//! of the previous frame so a new frame can be reduced to the smallest set of
//! 16x16 tile rectangles that actually differ.
//!
//! Fingerprints, not pixels: a 1920x1080 frame is 8160 tiles, so a `u64` per
//! tile is 65 KB of history rather than 8 MB of BGRA. Each tile is hashed with
//! two independent accumulators, so a collision would need two 64-bit lanes to
//! agree on two different 1 KiB blocks.
//!
//! Rectangles are emitted in tile units (multiples of [`TILE`]) and clipped to
//! the frame, which is what the DL3 header carries. Adjacent tiles are merged
//! greedily: horizontally into runs, then vertically when a run repeats exactly.
//! When damage covers most of the frame the tracker returns one full-frame rect
//! instead, since per-rect packet overhead and fragment restarts cost more than
//! the handful of unchanged tiles saved.

use crate::dl3::TILE;

/// A damage rectangle in pixels, tile aligned and clipped to the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
}

/// What a [`DirtyTracker::diff`] call found.
#[derive(Clone, Debug, Default)]
pub struct Damage {
    pub rects: Vec<Rect>,
    /// Tiles whose fingerprint changed.
    pub dirty_tiles: usize,
    /// True when the frame was sent whole (first frame, size change, or damage
    /// past the coalescing threshold).
    pub full: bool,
}

impl Damage {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Pixels the returned rects cover.
    pub fn covered_pixels(&self) -> u64 {
        self.rects.iter().map(Rect::area).sum()
    }

    /// Tiles the returned rects cover.
    pub fn covered_tiles(&self) -> usize {
        let tile = (TILE * TILE) as u64;
        self.rects
            .iter()
            .map(|r| (r.area().div_ceil(tile)) as usize)
            .sum()
    }
}

/// Above this fraction of the frame, one full-frame rect beats a rect list.
const FULL_FRAME_PERCENT: u64 = 60;

/// Frame-to-frame tile fingerprints.
pub struct DirtyTracker {
    width: u32,
    height: u32,
    tiles_x: u32,
    tiles_y: u32,
    /// `lo ^ rot(hi)` per tile; both lanes are compared on their own.
    lo: Vec<u64>,
    hi: Vec<u64>,
    primed: bool,
}

impl std::fmt::Debug for DirtyTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirtyTracker")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("tiles", &(self.tiles_x * self.tiles_y))
            .field("primed", &self.primed)
            .finish()
    }
}

impl DirtyTracker {
    pub fn new(width: u32, height: u32) -> Self {
        let mut tracker = Self {
            width: 0,
            height: 0,
            tiles_x: 0,
            tiles_y: 0,
            lo: Vec::new(),
            hi: Vec::new(),
            primed: false,
        };
        tracker.reset(width, height);
        tracker
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_primed(&self) -> bool {
        self.primed
    }

    /// Forget the previous frame; the next `diff` reports everything.
    pub fn reset(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.tiles_x = width.div_ceil(TILE as u32);
        self.tiles_y = height.div_ceil(TILE as u32);
        let tiles = (self.tiles_x * self.tiles_y) as usize;
        self.lo.clear();
        self.lo.resize(tiles, 0);
        self.hi.clear();
        self.hi.resize(tiles, 0);
        self.primed = false;
    }

    /// Compare `pixels` (BGRA, `stride` bytes per row) against the previous
    /// frame, remember it, and return the rectangles that changed.
    ///
    /// An unchanged frame returns an empty [`Damage`]; the first frame and a
    /// resolution change return one full-frame rect.
    pub fn diff(&mut self, pixels: &[u8], stride: usize) -> Damage {
        let needed = (self.height as usize)
            .saturating_sub(1)
            .saturating_mul(stride)
            .saturating_add(self.width as usize * 4);
        if self.width == 0
            || self.height == 0
            || stride < self.width as usize * 4
            || pixels.len() < needed
        {
            return Damage::default();
        }

        let tx = self.tiles_x as usize;
        let ty = self.tiles_y as usize;
        let mut flags = vec![false; tx * ty];
        let mut changed = 0usize;

        for tile_y in 0..ty {
            let y0 = tile_y * TILE;
            let h = (self.height as usize - y0).min(TILE);
            for tile_x in 0..tx {
                let x0 = tile_x * TILE;
                let w = (self.width as usize - x0).min(TILE);
                let (lo, hi) = fingerprint(pixels, stride, x0, y0, w, h);
                let at = tile_y * tx + tile_x;
                if !self.primed || self.lo[at] != lo || self.hi[at] != hi {
                    flags[at] = true;
                    changed += 1;
                }
                self.lo[at] = lo;
                self.hi[at] = hi;
            }
        }

        let was_primed = self.primed;
        self.primed = true;
        if !was_primed {
            return Damage {
                rects: vec![self.full_rect()],
                dirty_tiles: tx * ty,
                full: true,
            };
        }

        if changed == 0 {
            return Damage::default();
        }

        let total_tiles = tx * ty;
        if changed * 100 >= total_tiles * FULL_FRAME_PERCENT as usize {
            return Damage {
                rects: vec![self.full_rect()],
                dirty_tiles: changed,
                full: true,
            };
        }

        Damage {
            rects: coalesce(&flags, tx, ty, self.width, self.height),
            dirty_tiles: changed,
            full: false,
        }
    }

    fn full_rect(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        }
    }
}

/// Two-lane fingerprint of one tile.
#[inline]
fn fingerprint(
    pixels: &[u8],
    stride: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
) -> (u64, u64) {
    const FNV: u64 = 0x100_0000_01b3;
    let mut lo = 0xcbf2_9ce4_8422_2325u64;
    let mut hi = 0x9e37_79b9_7f4a_7c15u64;
    for y in y0..y0 + h {
        let start = y * stride + x0 * 4;
        let row = &pixels[start..start + w * 4];
        let (words, tail) = row.as_chunks::<8>();
        for word in words {
            let v = u64::from_le_bytes(*word);
            lo = (lo ^ v).wrapping_mul(FNV);
            hi = (hi.rotate_left(27) ^ v).wrapping_mul(0xff51_afd7_ed55_8ccd);
        }
        if !tail.is_empty() {
            let mut v = 0u64;
            for (i, b) in tail.iter().enumerate() {
                v |= (*b as u64) << (8 * i);
            }
            lo = (lo ^ v).wrapping_mul(FNV);
            hi = (hi.rotate_left(27) ^ v).wrapping_mul(0xff51_afd7_ed55_8ccd);
        }
    }
    // Fold so neither lane needs the other to stay distinct.
    (lo ^ hi.rotate_left(31), hi)
}

/// Merge dirty tiles into the fewest rectangles this simple pass can find:
/// horizontal runs first, then vertical merging of identical runs.
fn coalesce(flags: &[bool], tx: usize, ty: usize, width: u32, height: u32) -> Vec<Rect> {
    // (x, y, w_tiles, h_tiles)
    let mut rects: Vec<(usize, usize, usize, usize)> = Vec::new();
    // Open rects from the previous row, keyed by (x, w).
    let mut open: Vec<(usize, usize, usize, usize)> = Vec::new();

    for row in 0..ty {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut x = 0;
        while x < tx {
            if !flags[row * tx + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < tx && flags[row * tx + x] {
                x += 1;
            }
            runs.push((start, x - start));
        }

        let mut next_open = Vec::new();
        for (x, w) in runs {
            if let Some(found) = open.iter().position(|&(ox, _, ow, _)| ox == x && ow == w) {
                let (_, oy, ow, oh) = open.remove(found);
                next_open.push((x, oy, ow, oh + 1));
            } else {
                next_open.push((x, row, w, 1));
            }
        }
        // Rects that did not extend into this row are final.
        rects.append(&mut open);
        open = next_open;
    }
    rects.extend(open);

    rects
        .into_iter()
        .map(|(x, y, w, h)| {
            let px = (x * TILE) as u32;
            let py = (y * TILE) as u32;
            Rect {
                x: px,
                y: py,
                w: ((w * TILE) as u32).min(width - px),
                h: ((h * TILE) as u32).min(height - py),
            }
        })
        .filter(|r| r.w > 0 && r.h > 0)
        .collect()
}

/// Fast fill for a test/demo buffer.
#[cfg(test)]
fn paint(pixels: &mut [u8], stride: usize, rect: Rect, bgra: [u8; 4]) {
    for y in rect.y..rect.y + rect.h {
        for x in rect.x..rect.x + rect.w {
            let at = y as usize * stride + x as usize * 4;
            pixels[at..at + 4].copy_from_slice(&bgra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 128;
    const H: usize = 64;

    fn buffer() -> Vec<u8> {
        vec![0u8; W * H * 4]
    }

    #[test]
    fn first_frame_is_everything() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let d = t.diff(&buffer(), W * 4);
        assert!(d.full);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 0,
                y: 0,
                w: W as u32,
                h: H as u32
            }]
        );
        assert_eq!(d.dirty_tiles, (W / 16) * (H / 16));
    }

    #[test]
    fn unchanged_frame_is_empty() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let px = buffer();
        t.diff(&px, W * 4);
        let d = t.diff(&px, W * 4);
        assert!(!d.full);
        assert!(d.is_empty());
        assert_eq!(d.dirty_tiles, 0);
    }

    #[test]
    fn one_pixel_damages_one_tile() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        paint(
            &mut px,
            W * 4,
            Rect {
                x: 33,
                y: 21,
                w: 1,
                h: 1,
            },
            [1, 2, 3, 255],
        );
        let d = t.diff(&px, W * 4);
        assert_eq!(d.dirty_tiles, 1);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 32,
                y: 16,
                w: 16,
                h: 16
            }]
        );
    }

    #[test]
    fn a_changed_row_span_coalesces_into_one_rect() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        // Two tiles wide, one tile tall, non-adjacent to anything else.
        paint(
            &mut px,
            W * 4,
            Rect {
                x: 16,
                y: 32,
                w: 32,
                h: 16,
            },
            [9, 9, 9, 255],
        );
        let d = t.diff(&px, W * 4);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 16,
                y: 32,
                w: 32,
                h: 16
            }]
        );
    }

    #[test]
    fn identical_runs_merge_vertically() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        paint(
            &mut px,
            W * 4,
            Rect {
                x: 48,
                y: 0,
                w: 16,
                h: 48,
            },
            [7, 7, 7, 255],
        );
        let d = t.diff(&px, W * 4);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 48,
                y: 0,
                w: 16,
                h: 48
            }]
        );
    }

    #[test]
    fn disjoint_damage_stays_separate() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        paint(
            &mut px,
            W * 4,
            Rect {
                x: 0,
                y: 0,
                w: 16,
                h: 16,
            },
            [1, 1, 1, 255],
        );
        paint(
            &mut px,
            W * 4,
            Rect {
                x: 96,
                y: 48,
                w: 16,
                h: 16,
            },
            [2, 2, 2, 255],
        );
        let d = t.diff(&px, W * 4);
        assert_eq!(d.rects.len(), 2);
        assert_eq!(d.dirty_tiles, 2);
        assert_eq!(d.covered_pixels(), 2 * 16 * 16);
    }

    #[test]
    fn widespread_damage_ships_the_frame_whole() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        for (i, b) in px.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let d = t.diff(&px, W * 4);
        assert!(d.full);
        assert_eq!(d.rects.len(), 1);
        assert_eq!(d.rects[0].w, W as u32);
    }

    #[test]
    fn resize_reports_everything_again() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let px = buffer();
        t.diff(&px, W * 4);
        t.reset(64, 32);
        let small = vec![0u8; 64 * 32 * 4];
        let d = t.diff(&small, 64 * 4);
        assert!(d.full);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 0,
                y: 0,
                w: 64,
                h: 32
            }]
        );
    }

    #[test]
    fn non_multiple_of_tile_sizes_clip() {
        let mut t = DirtyTracker::new(40, 40);
        let mut px = vec![0u8; 40 * 40 * 4];
        t.diff(&px, 40 * 4);
        paint(
            &mut px,
            40 * 4,
            Rect {
                x: 39,
                y: 39,
                w: 1,
                h: 1,
            },
            [1, 1, 1, 255],
        );
        let d = t.diff(&px, 40 * 4);
        assert_eq!(
            d.rects,
            vec![Rect {
                x: 32,
                y: 32,
                w: 8,
                h: 8
            }]
        );
    }

    #[test]
    fn stride_padding_is_ignored() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let stride = W * 4 + 64;
        let mut px = vec![0u8; stride * H];
        t.diff(&px, stride);
        // Only padding changes: still a clean frame.
        for y in 0..H {
            for b in &mut px[y * stride + W * 4..(y + 1) * stride] {
                *b = 0xaa;
            }
        }
        assert!(t.diff(&px, stride).is_empty());
    }

    /// The invariant the transport depends on: every tile that changed is
    /// covered by at least one returned rect, and every rect is tile aligned
    /// and inside the frame. Checked over a spread of scattered damage.
    #[test]
    fn every_dirty_tile_is_covered_by_some_rect() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let mut px = buffer();
        t.diff(&px, W * 4);
        let damaged: Vec<Rect> = vec![
            Rect {
                x: 3,
                y: 5,
                w: 7,
                h: 9,
            },
            Rect {
                x: 60,
                y: 2,
                w: 21,
                h: 13,
            },
            Rect {
                x: 100,
                y: 40,
                w: 20,
                h: 20,
            },
            Rect {
                x: 17,
                y: 33,
                w: 16,
                h: 16,
            },
            Rect {
                x: 0,
                y: 63,
                w: 40,
                h: 1,
            },
        ];
        for (n, r) in damaged.iter().enumerate() {
            let v = [n as u8 + 1, 0x40, 0x80, 0xff];
            paint(&mut px, W * 4, *r, v);
        }
        let d = t.diff(&px, W * 4);
        assert!(!d.full, "this damage should stay partial");
        for r in &d.rects {
            assert_eq!(r.x % TILE as u32, 0, "rect not tile aligned: {r:?}");
            assert_eq!(r.y % TILE as u32, 0, "rect not tile aligned: {r:?}");
            assert!(r.x + r.w <= W as u32, "rect past frame: {r:?}");
            assert!(r.y + r.h <= H as u32, "rect past frame: {r:?}");
        }
        for row in 0..H as u32 / TILE as u32 {
            for col in 0..W as u32 / TILE as u32 {
                let tile = Rect {
                    x: col * TILE as u32,
                    y: row * TILE as u32,
                    w: TILE as u32,
                    h: TILE as u32,
                };
                let touches = damaged.iter().any(|d| {
                    d.x < tile.x + tile.w
                        && tile.x < d.x + d.w
                        && d.y < tile.y + tile.h
                        && tile.y < d.y + d.h
                });
                if !touches {
                    continue;
                }
                let covered = d.rects.iter().any(|r| {
                    r.x <= tile.x
                        && r.y <= tile.y
                        && r.x + r.w >= tile.x + tile.w
                        && r.y + r.h >= tile.y + tile.h
                });
                assert!(
                    covered,
                    "dirty tile at {tile:?} not covered by {:?}",
                    d.rects
                );
            }
        }
    }

    #[test]
    fn short_buffer_is_not_trusted() {
        let mut t = DirtyTracker::new(W as u32, H as u32);
        let d = t.diff(&[0u8; 4], W * 4);
        assert!(d.is_empty());
    }
}
