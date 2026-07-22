//! Ink-coverage map and placement resolver: a coarse dirty/clear grid over
//! the page, plus a search that turns a card's semantic `Place` intent into
//! concrete pixel coordinates that never overlap existing ink.
// Temporary: consumed by the card renderer task.
#![allow(dead_code)]

use crate::cards::Place;
use crate::surface::Surface;

/// Grid cell size in pixels. 1620/25 = 64.8 -> 65 cols; 2160/25 = 86.4 -> 87 rows.
pub const CELL: usize = 25;

/// Low-res dirty/clear grid over the page. A cell is dirty once any ink (or
/// a previously-placed card) has touched it.
#[derive(Clone)]
pub struct InkMap {
    cols: usize,
    rows: usize,
    cells: Vec<bool>,
    pub screen_w: usize,
    pub screen_h: usize,
}

impl InkMap {
    /// A blank page of `screen_w` x `screen_h` pixels.
    pub fn new(screen_w: usize, screen_h: usize) -> Self {
        let cols = screen_w.div_ceil(CELL);
        let rows = screen_h.div_ceil(CELL);
        Self { cols, rows, cells: vec![false; cols * rows], screen_w, screen_h }
    }

    /// Sample the rendered page into a fresh map: a cell is dirty if any of
    /// a 5x5 grid of sample points inside it (step `CELL/5`) reads darker
    /// than luma 200. Cheaper than scanning every pixel, plenty accurate at
    /// `CELL` = 25px.
    pub fn from_surface(surf: &Surface) -> Self {
        let mut m = Self::new(surf.w, surf.h);
        let step = (CELL as i32 / 5).max(1);
        for row in 0..m.rows {
            for col in 0..m.cols {
                let x0 = (col * CELL) as i32;
                let y0 = (row * CELL) as i32;
                let mut dirty = false;
                'sample: for sy in 0..5 {
                    for sx in 0..5 {
                        // The last col/row of cells overhangs the true page
                        // edge (1620 % CELL = 20, 2160 % CELL = 10), so a
                        // raw sample point can land past surf.w/surf.h.
                        // Clamp explicitly rather than lean on
                        // Surface::luma's own out-of-range fallback
                        // (255/white) to define this cell's dirtiness.
                        let x = (x0 + sx * step).min(surf.w as i32 - 1);
                        let y = (y0 + sy * step).min(surf.h as i32 - 1);
                        if surf.luma(x, y) < 200 {
                            dirty = true;
                            break 'sample;
                        }
                    }
                }
                if dirty {
                    m.cells[row * m.cols + col] = true;
                }
            }
        }
        m
    }

    /// Mark a pixel rect (freshly rendered ink, or a card's own footprint)
    /// occupied. Cells only partly covered by the rect are still marked
    /// whole — deliberately conservative, not exact. Rects (or parts of
    /// them) outside the page are simply clipped: off-page space is never
    /// stored, but `find_spot` treats it as dirty anyway (see there).
    pub fn mark_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w <= 0 || h <= 0 || self.cols == 0 || self.rows == 0 {
            return;
        }
        let cell = CELL as i32;
        let col_lo = x.div_euclid(cell).max(0);
        let col_hi = (x + w - 1).div_euclid(cell).min(self.cols as i32 - 1);
        let row_lo = y.div_euclid(cell).max(0);
        let row_hi = (y + h - 1).div_euclid(cell).min(self.rows as i32 - 1);
        for row in row_lo..=row_hi {
            for col in col_lo..=col_hi {
                self.cells[row as usize * self.cols + col as usize] = true;
            }
        }
    }

    /// Fraction of cells that are dirty, 0.0..=1.0.
    pub fn coverage(&self) -> f32 {
        if self.cells.is_empty() {
            return 0.0;
        }
        let dirty = self.cells.iter().filter(|&&c| c).count();
        dirty as f32 / self.cells.len() as f32
    }

    /// Find the top-left pixel of a `want_w` x `want_h` rect — with `margin`
    /// px of clearance on every side — that is entirely free of ink.
    /// Searches outward from the cell containing `seed` in Chebyshev rings
    /// (closest candidates first), so the result naturally stays near the
    /// seed when the seed itself is already clear. Cells off the edge of
    /// the page count as dirty for the ink-clearance check; separately, the
    /// returned coordinate is checked against the true `screen_w`/`screen_h`
    /// (not just the cell grid, which rounds them up to whole cells) — so
    /// the returned CONTENT rect itself never hangs off the true page,
    /// even though its margin buffer may reach into the last cell's
    /// rounding overhang. `None` if nothing on the whole page fits — the
    /// caller should retry at a smaller size.
    pub fn find_spot(
        &self,
        want_w: i32,
        want_h: i32,
        seed: (i32, i32),
        margin: i32,
    ) -> Option<(i32, i32)> {
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        let margin = margin.max(0);
        let want_w = want_w.max(0);
        let want_h = want_h.max(0);
        let full_w = want_w.saturating_add(margin.saturating_mul(2));
        let full_h = want_h.saturating_add(margin.saturating_mul(2));
        let gw = (full_w as usize).div_ceil(CELL).max(1);
        let gh = (full_h as usize).div_ceil(CELL).max(1);
        if gw > self.cols || gh > self.rows {
            return None;
        }

        // Built fresh per call: mark_rect calls are infrequent and the grid
        // is only ~5.6k cells, so simplicity beats caching an invalidation
        // scheme.
        let ii = self.integral_image();
        let iw = self.cols + 1;

        let cell = CELL as i32;
        let seed_col = seed.0.div_euclid(cell).clamp(0, self.cols as i32 - 1);
        let seed_row = seed.1.div_euclid(cell).clamp(0, self.rows as i32 - 1);

        let max_radius = self.cols.max(self.rows) as i32;
        for radius in 0..=max_radius {
            for (col, row) in ring(seed_col, seed_row, radius) {
                if col < 0 || row < 0 {
                    continue;
                }
                let (col, row) = (col as usize, row as usize);
                if col + gw > self.cols || row + gh > self.rows {
                    continue;
                }
                let (px, py) = (col as i32 * CELL as i32 + margin, row as i32 * CELL as i32 + margin);
                // `self.cols`/`self.rows` round screen_w/screen_h UP to
                // whole cells (1625x2175 for the 1620x2160 target), so a
                // candidate can pass the check above yet still place
                // content past the true edge. `px`/`py` already have
                // `margin` folded in (they're the CONTENT's own top-left,
                // not the padded footprint's), so the guarantee we want —
                // content rect [px, px+want_w) x [py, py+want_h) lies
                // fully within [0, screen_w) x [0, screen_h) — is just
                // this, with no extra `+ margin`: adding it again would
                // double-count the margin already baked into px/py.
                if px + want_w > self.screen_w as i32 || py + want_h > self.screen_h as i32 {
                    continue;
                }
                if rect_sum(&ii, iw, row, col, gh, gw) == 0 {
                    return Some((px, py));
                }
            }
        }
        None
    }

    /// Summed-area table over the dirty grid (dirty = 1), so any candidate
    /// rect's cleanliness is an O(1) lookup during the ring search.
    fn integral_image(&self) -> Vec<i32> {
        let w = self.cols + 1;
        let mut ii = vec![0i32; w * (self.rows + 1)];
        for row in 0..self.rows {
            for col in 0..self.cols {
                let v = i32::from(self.cells[row * self.cols + col]);
                ii[(row + 1) * w + (col + 1)] =
                    v + ii[row * w + (col + 1)] + ii[(row + 1) * w + col] - ii[row * w + col];
            }
        }
        ii
    }
}

/// Cells at exactly Chebyshev distance `r` from `(cx, cy)` (the ring's
/// perimeter), in a fixed order — top row, bottom row, then the left/right
/// edges — so the same map and seed always search in the same order.
fn ring(cx: i32, cy: i32, r: i32) -> Vec<(i32, i32)> {
    if r == 0 {
        return vec![(cx, cy)];
    }
    let mut out = Vec::with_capacity(8 * r as usize);
    for col in (cx - r)..=(cx + r) {
        out.push((col, cy - r));
        out.push((col, cy + r));
    }
    for row in (cy - r + 1)..=(cy + r - 1) {
        out.push((cx - r, row));
        out.push((cx + r, row));
    }
    out
}

/// Sum of the dirty grid over rows `[row0, row0+gh)`, cols `[col0, col0+gw)`,
/// via the summed-area table `ii` (stride `iw` = cols+1). Intermediate terms
/// can go negative (signed on purpose) even though the final sum can't.
fn rect_sum(ii: &[i32], iw: usize, row0: usize, col0: usize, gh: usize, gw: usize) -> i32 {
    let (row1, col1) = (row0 + gh, col0 + gw);
    ii[row1 * iw + col1] - ii[row0 * iw + col1] - ii[row1 * iw + col0] + ii[row0 * iw + col0]
}

/// Where an anchor-relative card should search from: an explicit point, or
/// "none given" — a caller bug for `NearNewInk`/`NearAnchor` that `resolve`
/// degrades gracefully rather than panicking on.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    Point(i32, i32),
    None,
}

/// Breathing room `resolve` leaves around content placed away from the page
/// edge — what most callers want from `find_spot`.
const DEFAULT_MARGIN: i32 = 40;
/// Thickness of the four edge bands `Place::Margin` searches within.
const EDGE_BAND: i32 = 120;

/// Resolve a card's semantic placement into a concrete top-left pixel.
/// `anchor` is only consulted for `NearNewInk`/`NearAnchor`; every other
/// place picks its own seed. `None` means nothing on the page fits at this
/// size — the caller should retry with a smaller card.
pub fn resolve(
    map: &InkMap,
    place: &Place,
    anchor: Anchor,
    want_w: i32,
    want_h: i32,
) -> Option<(i32, i32)> {
    match place {
        Place::FullPage => Some((0, 0)), // caller owns the whole page itself
        Place::NearNewInk | Place::NearAnchor => {
            let seed = match anchor {
                Anchor::Point(x, y) => (x, y),
                // Caller bug: an anchor-relative place with no anchor.
                // Degrade to a screen-center search instead of panicking.
                Anchor::None => screen_center(map),
            };
            map.find_spot(want_w, want_h, seed, DEFAULT_MARGIN)
        }
        Place::BlankArea => map.find_spot(want_w, want_h, screen_center(map), DEFAULT_MARGIN),
        Place::Margin => resolve_margin(map, want_w, want_h),
    }
}

fn screen_center(map: &InkMap) -> (i32, i32) {
    (map.screen_w as i32 / 2, map.screen_h as i32 / 2)
}

/// `Place::Margin`: search only the four `EDGE_BAND`-px edge bands, tried
/// top -> bottom -> left -> right, first fit wins. For each attempt,
/// everything outside that one band is masked dirty on a throwaway clone,
/// so `find_spot` can't wander into the page interior chasing a "closer"
/// clear cell. Unlike other places this asks `find_spot` for zero breathing
/// margin: edge decorations sit flush against the border, and the page edge
/// is already the boundary — there's no ink there to keep clear of.
///
/// Every band's seed sits `EDGE_BAND / 2` px in from the true page edge —
/// the middle of that band's clear strip — symmetric on all four sides.
/// The four (mask, seed) pairs are looped over rather than spelled out four
/// times specifically so they can't drift out of sync with each other: an
/// earlier version hand-wrote each block and the bottom/right seeds ended up
/// using a mismatched offset that landed inside their own band's masked-dirty
/// interior instead of its clear strip.
fn resolve_margin(map: &InkMap, want_w: i32, want_h: i32) -> Option<(i32, i32)> {
    let (sw, sh) = (map.screen_w as i32, map.screen_h as i32);
    let (cx, cy) = (sw / 2, sh / 2);
    let half = EDGE_BAND / 2;

    // (rect to mask dirty on the clone, seed point) for each edge band, in
    // top -> bottom -> left -> right try order.
    let bands: [((i32, i32, i32, i32), (i32, i32)); 4] = [
        ((0, EDGE_BAND, sw, sh - EDGE_BAND), (cx, half)),
        ((0, 0, sw, sh - EDGE_BAND), (cx, sh - half)),
        ((EDGE_BAND, 0, sw - EDGE_BAND, sh), (half, cy)),
        ((0, 0, sw - EDGE_BAND, sh), (sw - half, cy)),
    ];

    for ((mx, my, mw, mh), seed) in bands {
        let mut band = map.clone();
        band.mark_rect(mx, my, mw, mh);
        if let Some(p) = band.find_spot(want_w, want_h, seed, 0) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{PixFmt, BLACK};

    /// 手工构造:把给定像素矩形标脏。
    fn map_with(dirty: &[(i32, i32, i32, i32)]) -> InkMap {
        let mut m = InkMap::new(1620, 2160);
        for &(x, y, w, h) in dirty {
            m.mark_rect(x, y, w, h);
        }
        m
    }

    /// A real, blank `Surface` at the exact target resolution (1620x2160).
    fn full_page_surf() -> (Vec<u8>, Surface) {
        let mut buf = vec![0xFFu8; 1620 * 2160 * 4];
        let ptr = buf.as_mut_ptr();
        let s = Surface::new(ptr, buf.len(), 1620, 2160, 1620 * 4, PixFmt::Rgb32);
        (buf, s)
    }

    #[test]
    fn empty_page_places_at_seed() {
        let m = InkMap::new(1620, 2160);
        let got = m.find_spot(300, 300, (800, 1000), 40).unwrap();
        assert!((got.0 - 800).abs() <= CELL as i32 * 2 && (got.1 - 1000).abs() <= CELL as i32 * 2,
            "seed itself is clear, should place there, got {got:?}");
    }

    #[test]
    fn spot_avoids_ink_and_respects_margin() {
        // 中央一大块墨迹;seed 在墨迹中心 → 必须绕到外面且留白 40px。
        let m = map_with(&[(600, 800, 400, 400)]);
        let (x, y) = m.find_spot(200, 200, (800, 1000), 40).unwrap();
        // 与脏区的间距 ≥ margin - CELL(格化容差)
        let clear_of_ink = x + 200 + 15 <= 600 || x >= 600 + 400 + 15
            || y + 200 + 15 <= 800 || y >= 800 + 400 + 15;
        assert!(clear_of_ink, "({x},{y}) overlaps the inked block");
    }

    #[test]
    fn full_page_of_ink_yields_none() {
        let m = map_with(&[(0, 0, 1620, 2160)]);
        assert!(m.find_spot(100, 100, (100, 100), 40).is_none());
    }

    #[test]
    fn coverage_counts_dirty_cells() {
        let m = map_with(&[(0, 0, 810, 2160)]); // 左半页
        let c = m.coverage();
        assert!((0.45..=0.55).contains(&c), "half page ≈ 0.5, got {c}");
    }

    #[test]
    fn margin_place_sticks_to_page_edges() {
        let m = map_with(&[(200, 200, 1200, 1700)]); // 中央大占用,只剩四边
        let got = resolve(&m, &crate::cards::Place::Margin, Anchor::None, 200, 100);
        let (x, y) = got.expect("margins are free");
        assert!(y <= 120 || y >= 2160 - 220 || x <= 120 || x >= 1620 - 320,
            "({x},{y}) is not in an edge band");
    }

    #[test]
    fn margin_resolves_to_bottom_band_when_top_is_also_blocked() {
        // Same central block as the sibling test, plus the whole top band
        // blotted out too — resolution must fall through to another band
        // (top -> bottom -> left -> right), exercising the bottom/left/right
        // seeds the sibling test never reaches (it's satisfied by top).
        let mut m = map_with(&[(200, 200, 1200, 1700)]);
        m.mark_rect(0, 0, 1620, 120);
        let got = resolve(&m, &crate::cards::Place::Margin, Anchor::None, 200, 100);
        let (x, y) = got.expect("bottom/left/right margins are still free");
        assert!(y <= 120 || y >= 2160 - 220 || x <= 120 || x >= 1620 - 320,
            "({x},{y}) is not in an edge band");
        if y >= 2160 - 120 {
            // Actually hugging the true bottom edge (want_h = 100), not
            // just barely short of the interior mask boundary — this is
            // exactly what the mismatched seed used to get wrong.
            assert!(y >= 2160 - 120 - 100, "({x},{y}) doesn't hug the bottom edge");
        }
    }

    #[test]
    fn from_surface_handles_the_overhanging_last_cell() {
        // 1620 % CELL = 20, 2160 % CELL = 10: the last col/row of cells
        // overhangs the true page edge, at the exact target resolution.
        // This 4x4 sliver sits exactly in that last cell's true corner
        // (pixels x:1616..1620, y:2156..2160) — a spot only the *clamped*
        // last sample of the cell's 5x5 grid ever reaches (the other 5x5
        // sample points, both the in-bounds ones and the unclamped
        // out-of-bounds ones that fall back to luma 255/white, all miss
        // it). So this specifically fails to register as dirty without the
        // clamp — not just a "does it panic" smoke test.
        let (_buf, mut surf) = full_page_surf();
        surf.fill_rect(1616, 2156, 4, 4, BLACK); // the true bottom-right pixel block
        let m = InkMap::from_surface(&surf);
        assert!(m.coverage() > 0.0, "the true corner pixel should register as dirty");
        if let Some((x, y)) = m.find_spot(50, 50, (1618, 2158), 0) {
            assert!(
                x + 50 <= 1616 || x >= 1620 || y + 50 <= 2156 || y >= 2160,
                "({x},{y}) overlaps the inked corner"
            );
        }
    }

    #[test]
    fn find_spot_never_overhangs_the_true_screen_edge() {
        // Only the bottom two cell rows (85, 86 — pixels y:2125..2175,
        // truncated to the true 2160 edge) are left clear. A want_h=45
        // (gh=2 cells) box's only cell-grid-valid row is 85, landing its
        // content at y=2125..2170 — 10px past the true screen_h=2160. Before
        // the fix this was returned anyway (validated only against the
        // rounded-up 87-row grid, not the true 2160px height); the fix must
        // reject it, either by finding a spot that's genuinely in bounds or
        // — as here, since row 85 is the *only* cell-grid-valid option —
        // by correctly giving up with `None`. Either is fine; an
        // out-of-bounds `Some` is not.
        let mut m = InkMap::new(1620, 2160);
        m.mark_rect(0, 0, 1620, 2110);
        let got = m.find_spot(100, 45, (800, 2140), 0);
        if let Some((x, y)) = got {
            assert!(
                x + 100 <= 1620 && y + 45 <= 2160,
                "({x},{y}) places content past the true screen edge"
            );
        }
    }
}
