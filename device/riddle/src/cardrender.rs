//! Card -> render plan: turns a parsed `cards::Card` into concrete
//! screen-space stroke plans the animation loop can draw. The integration
//! point between the paper_cards data model (Task 4), the built-in stamp
//! library (Task 5), the ink-coverage placement resolver (Task 6), and the
//! existing handwriting rasterizer (script.rs) — nothing upstream of this
//! module knows about pixels, and nothing downstream of it knows about
//! `Card`.

use crate::cards;
use crate::fb;
use crate::layout;
use crate::script;
use crate::stamps;
use crate::surface;

/// One card's worth of screen-space drawing: ordered strokes, ink color, the
/// animation speed (points drawn per frame), and the dirty region the
/// caller must mark occupied (already `map.mark_rect`'d by `plan_card`
/// itself, but marking is idempotent so the caller may do it again).
// Temporary: consumed by the cards-test harness task.
#[allow(dead_code)]
pub struct RenderPlan {
    pub strokes: Vec<Vec<(i32, i32)>>,
    pub color: u16,
    pub points_per_frame: i32,
    pub region: fb::BBox,
}

/// Square placement-box side, in pixels, for each `Size` tier (relative to
/// the 1620px page width). `Text` ignores this for its own box (content +
/// wrap decide that) but still uses it to cap wrap width — see [`text_px`].
fn size_box(size: cards::Size) -> i32 {
    match size {
        cards::Size::S => 243,
        cards::Size::M => 486,
        cards::Size::L => 810,
    }
}

/// Font size, in pixels, for a `Text` card at each `Size` tier.
fn text_px(size: cards::Size) -> f32 {
    match size {
        cards::Size::S => 44.0,
        cards::Size::M => 56.0,
        cards::Size::L => 84.0,
    }
}

/// Points per frame for the animation loop, by pace.
fn ppf(pace: cards::Pace) -> i32 {
    match pace {
        cards::Pace::Normal => 26,
        cards::Pace::Slow => 8,
    }
}

/// The shrink ladder a card's box steps down when the larger size does not
/// fit anywhere on the page: L -> M -> S, starting at whatever size the
/// card asked for.
fn size_ladder(size: cards::Size) -> Vec<cards::Size> {
    match size {
        cards::Size::L => vec![cards::Size::L, cards::Size::M, cards::Size::S],
        cards::Size::M => vec![cards::Size::M, cards::Size::S],
        cards::Size::S => vec![cards::Size::S],
    }
}

/// Where a card's placement search should seed from: `new_ink_anchor` for
/// `NearNewInk`, the card's own `anchor_norm` (converted to pixels) for
/// `NearAnchor`, or nothing for the placements that pick their own seed.
fn seed_anchor(common: &cards::CardCommon, new_ink_anchor: (i32, i32)) -> layout::Anchor {
    match common.place {
        cards::Place::NearNewInk => layout::Anchor::Point(new_ink_anchor.0, new_ink_anchor.1),
        cards::Place::NearAnchor => common
            .anchor_norm
            .map(|(nx, ny)| {
                layout::Anchor::Point(
                    (nx * fb::SCREEN_W as f32).round() as i32,
                    (ny * fb::SCREEN_H as f32).round() as i32,
                )
            })
            .unwrap_or(layout::Anchor::None),
        _ => layout::Anchor::None,
    }
}

/// Resolve a square box for `common`, shrinking down [`size_ladder`] until
/// one fits. `None` if nothing in the ladder fits anywhere on the page.
fn resolve_boxed(
    map: &layout::InkMap,
    common: &cards::CardCommon,
    anchor: layout::Anchor,
) -> Option<(cards::Size, i32, i32)> {
    for size in size_ladder(common.size) {
        let box_px = size_box(size);
        if let Some((x, y)) = layout::resolve(map, &common.place, anchor, box_px, box_px) {
            return Some((size, x, y));
        }
    }
    None
}

/// Build a plan from finished screen-space strokes, or `None` if they carry
/// no ink at all (an empty sketch, an unknown stamp, a shape name that
/// didn't match) — callers must drop the card quietly rather than push a
/// plan with a degenerate region.
fn make_plan(strokes: Vec<Vec<(i32, i32)>>, color: u16, points_per_frame: i32) -> Option<RenderPlan> {
    let mut region = fb::BBox::empty();
    let mut any = false;
    for s in &strokes {
        for &(x, y) in s {
            region.add(x, y, 4);
            any = true;
        }
    }
    if !any {
        return None;
    }
    Some(RenderPlan { strokes, color, points_per_frame, region })
}

/// Push `plan` (if any) onto `out`, marking its region on `map` immediately
/// — self-stacking prevention for cards that produce several plans (a
/// stamp row, count dots): each instance is marked as soon as it is placed,
/// so instance #2 never lands on top of instance #1 even though they were
/// all resolved from the same outer block. The caller (once wired up) may
/// `mark_rect` again on top of this; marking is idempotent.
fn commit(plan: Option<RenderPlan>, map: &mut layout::InkMap, out: &mut Vec<RenderPlan>) {
    if let Some(p) = plan {
        let (x, y, w, h) = p.region.rect();
        map.mark_rect(x, y, w, h);
        out.push(p);
    }
}

/// Points around a closed circle, integer screen coordinates.
fn circle_pts(cx: i32, cy: i32, r: i32, n: usize) -> Vec<(i32, i32)> {
    (0..=n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            ((cx as f32 + r as f32 * a.cos()).round() as i32, (cy as f32 + r as f32 * a.sin()).round() as i32)
        })
        .collect()
}

/// Normalized (unit-box, y-down) template strokes for a `TraceKind::Shape`
/// content name. Parametric like `stamps.rs`, but these are guide outlines
/// meant to be traced over in black, not stamped, so they favor simple
/// closed silhouettes over decorative detail. `None` for an unrecognized
/// name (the trace card then falls back to just its grid, if any).
fn shape_strokes(kind: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    use std::f32::consts::{PI, TAU};
    let strokes = match kind {
        "circle" => vec![(0..=40)
            .map(|i| {
                let a = i as f32 / 40.0 * TAU;
                (0.5 + 0.45 * a.cos(), 0.5 + 0.45 * a.sin())
            })
            .collect()],
        "square" => vec![vec![(0.06, 0.06), (0.94, 0.06), (0.94, 0.94), (0.06, 0.94), (0.06, 0.06)]],
        "triangle" => vec![vec![(0.5, 0.05), (0.95, 0.9), (0.05, 0.9), (0.5, 0.05)]],
        "star" => {
            let verts: Vec<(f32, f32)> = (0..=5)
                .map(|i| {
                    let a = (-90.0 + i as f32 * 144.0).to_radians();
                    (0.5 + 0.48 * a.cos(), 0.5 + 0.48 * a.sin())
                })
                .collect();
            vec![verts]
        }
        "heart" => {
            let half: Vec<(f32, f32)> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0 * PI;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                    (0.5 + x / 34.0, 0.42 - y / 34.0)
                })
                .collect();
            let mut whole = half.clone();
            whole.extend(half.iter().rev().skip(1).map(|&(x, y)| (1.0 - x, y)));
            vec![whole]
        }
        // 2.5 sine periods across the box, mid-height.
        "wave" => vec![(0..=48)
            .map(|i| {
                let t = i as f32 / 48.0;
                (t, 0.5 + 0.28 * (t * 2.5 * TAU).sin())
            })
            .collect()],
        _ => return None,
    };
    Some(strokes)
}

// ---------------------------------------------------------------------
// Text: wrap -> rasterize -> thin -> trace, left-aligned at the resolved
// point (unlike main.rs::plan_reply, which centers a reply line).
// ---------------------------------------------------------------------

/// Wrap, rasterize, thin and trace `content` at `px`/`max_px`. Returns the
/// block's pixel size, the line pitch used to stack lines, and each line's
/// raster-local strokes (not yet placed on the page).
fn prepare_text(
    font: &script::FontStack<'_>,
    content: &str,
    px: f32,
    max_px: f32,
) -> (i32, i32, i32, Vec<Vec<Vec<(i32, i32)>>>) {
    let lines = script::wrap(font, content, px, max_px);
    let mut line_h = (px * 1.45) as i32;
    let mut w = 0i32;
    let mut prepared = Vec::with_capacity(lines.len());
    for line_text in &lines {
        let mut raster = script::rasterize_line(font, line_text, px);
        script::thin(&mut raster);
        let strokes = script::trace(&raster);
        line_h = line_h.max(raster.height as i32 + 10);
        w = w.max(raster.width as i32);
        prepared.push(strokes);
    }
    let h = line_h * prepared.len() as i32;
    (w, h, line_h, prepared)
}

/// Place already-prepared line strokes left-aligned at `(x0, y0)`, stacked
/// downward by `line_h`.
fn place_text(prepared: Vec<Vec<Vec<(i32, i32)>>>, line_h: i32, x0: i32, y0: i32) -> Vec<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    let mut y = y0;
    for strokes in prepared {
        for s in strokes {
            out.push(s.iter().map(|&(sx, sy)| (x0 + sx, y + sy)).collect());
        }
        y += line_h;
    }
    out
}

fn plan_text(
    common: &cards::CardCommon,
    content: &str,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    let mut any_lines = false;
    for size in size_ladder(common.size) {
        let px = text_px(size);
        let max_w = (2.0 * size_box(size) as f32).min(1380.0);
        let (w, h, line_h, prepared) = prepare_text(font, content, px, max_w);
        if prepared.is_empty() {
            continue;
        }
        any_lines = true;
        if let Some((x0, y0)) = layout::resolve(map, &common.place, anchor, w, h) {
            let strokes = place_text(prepared, line_h, x0, y0);
            commit(make_plan(strokes, surface::BLACK, ppf(common.pace)), map, &mut out);
            if !out.is_empty() {
                return out;
            }
        }
    }
    // Distinguish "there was nothing to draw" from "there was nowhere to
    // draw it" — the latter message would be misleading for empty content.
    if any_lines {
        eprintln!("riddle: card dropped (no room)");
    } else {
        eprintln!("riddle: empty text card dropped");
    }
    out
}

/// Place text into an explicit Page-child rect. Unlike the top-level path
/// there is no placement search (the rect is server-given), but the text
/// still must not spill past the rect's own height: try the card's own
/// size first, then the same L->M->S ladder `plan_text` uses, until some
/// size's wrapped block fits `h`. If nothing in the ladder fits vertically
/// even at the smallest size, keep that size's lines but drop whatever
/// doesn't fit within `h` — never emit a stroke point below `y0 + h`.
fn text_plan_in_rect(
    common: &cards::CardCommon,
    content: &str,
    font: &script::FontStack<'_>,
    rect: (i32, i32, i32, i32),
) -> Option<RenderPlan> {
    let (x0, y0, w, h) = rect;
    let mut any_lines = false;
    let mut smallest: Option<(i32, Vec<Vec<Vec<(i32, i32)>>>)> = None;
    for size in size_ladder(common.size) {
        let px = text_px(size);
        let (_w, block_h, line_h, prepared) = prepare_text(font, content, px, w as f32);
        if prepared.is_empty() {
            continue;
        }
        any_lines = true;
        if block_h <= h {
            let strokes = place_text(prepared, line_h, x0, y0);
            return make_plan(strokes, surface::BLACK, ppf(common.pace));
        }
        smallest = Some((line_h, prepared));
    }
    if !any_lines {
        eprintln!("riddle: empty text card dropped");
        return None;
    }
    // Nothing in the ladder fit the rect's height: clip the smallest
    // size's lines to however many whole lines fit, rather than letting
    // ink spill past y0 + h.
    let (line_h, mut prepared) = smallest?;
    let max_lines = (h / line_h).max(0) as usize;
    if max_lines < prepared.len() {
        eprintln!("riddle: page text clipped to rect");
        prepared.truncate(max_lines);
    }
    if prepared.is_empty() {
        return None;
    }
    let strokes = place_text(prepared, line_h, x0, y0);
    make_plan(strokes, surface::BLACK, ppf(common.pace))
}

// ---------------------------------------------------------------------
// Sketch: aspect-fit the normalized point cloud into its box, centered.
// ---------------------------------------------------------------------

/// Scale `strokes` (normalized 0..1 points) to fit uniformly, centered,
/// inside a `bw` x `bh` pixel box whose top-left is `(x0, y0)`. Empty input
/// (or a degenerate all-empty stroke list) yields an empty result rather
/// than dividing by zero or panicking.
fn sketch_geometry(strokes: &[Vec<(f32, f32)>], x0: i32, y0: i32, bw: i32, bh: i32) -> Vec<Vec<(i32, i32)>> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for s in strokes {
        for &(x, y) in s {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !min_x.is_finite() {
        return Vec::new();
    }
    let (span_w, span_h) = (max_x - min_x, max_y - min_y);
    let scale = if span_w > 1e-6 || span_h > 1e-6 {
        (bw as f32 / span_w.max(1e-6)).min(bh as f32 / span_h.max(1e-6))
    } else {
        1.0
    };
    let pad_x = (bw as f32 - span_w * scale) / 2.0;
    let pad_y = (bh as f32 - span_h * scale) / 2.0;
    strokes
        .iter()
        .map(|s| {
            s.iter()
                .map(|&(x, y)| {
                    (
                        (x0 as f32 + pad_x + (x - min_x) * scale).round() as i32,
                        (y0 as f32 + pad_y + (y - min_y) * scale).round() as i32,
                    )
                })
                .collect::<Vec<(i32, i32)>>()
        })
        .filter(|s: &Vec<(i32, i32)>| !s.is_empty())
        .collect()
}

fn plan_sketch(
    common: &cards::CardCommon,
    strokes: &[Vec<(f32, f32)>],
    map: &mut layout::InkMap,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    if strokes.iter().all(|s| s.is_empty()) {
        eprintln!("riddle: card dropped (empty sketch)");
        return out;
    }
    let Some((size, x0, y0)) = resolve_boxed(map, common, anchor) else {
        eprintln!("riddle: card dropped (no room)");
        return out;
    };
    let box_px = size_box(size);
    let mapped = sketch_geometry(strokes, x0, y0, box_px, box_px);
    commit(make_plan(mapped, surface::BLACK, ppf(common.pace)), map, &mut out);
    out
}

fn sketch_plan_in_rect(
    common: &cards::CardCommon,
    strokes: &[Vec<(f32, f32)>],
    rect: (i32, i32, i32, i32),
) -> Option<RenderPlan> {
    if strokes.iter().all(|s| s.is_empty()) {
        return None;
    }
    let (x0, y0, w, h) = rect;
    let mapped = sketch_geometry(strokes, x0, y0, w, h);
    make_plan(mapped, surface::BLACK, ppf(common.pace))
}

// ---------------------------------------------------------------------
// Stamp: one fixed-size (S, 243px) instance per copy, stepping right and
// wrapping after 3 per row, regardless of the card's own `size`.
// ---------------------------------------------------------------------

const STAMP_BOX: i32 = 243;
const STAMP_GAP: i32 = 24;
const STAMP_PER_ROW: u32 = 3;

fn stamp_block_dims(count: u32) -> (i32, i32) {
    let cols = count.min(STAMP_PER_ROW);
    let rows = count.div_ceil(STAMP_PER_ROW);
    (
        cols as i32 * (STAMP_BOX + STAMP_GAP) - STAMP_GAP,
        rows as i32 * (STAMP_BOX + STAMP_GAP) - STAMP_GAP,
    )
}

/// Lay out `count` copies of `unit` (normalized 0..1 strokes) starting at
/// block top-left `(bx, by)`, one instance's strokes per output entry.
fn stamp_instances(unit: &[Vec<(f32, f32)>], count: u32, bx: i32, by: i32) -> Vec<Vec<Vec<(i32, i32)>>> {
    (0..count)
        .map(|i| {
            let col = i % STAMP_PER_ROW;
            let row = i / STAMP_PER_ROW;
            let ox = bx + col as i32 * (STAMP_BOX + STAMP_GAP);
            let oy = by + row as i32 * (STAMP_BOX + STAMP_GAP);
            unit.iter()
                .map(|s| {
                    s.iter()
                        .map(|&(x, y)| {
                            (ox + (x * STAMP_BOX as f32).round() as i32, oy + (y * STAMP_BOX as f32).round() as i32)
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

fn plan_stamp(
    common: &cards::CardCommon,
    name: &str,
    count: u32,
    map: &mut layout::InkMap,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    let Some(unit) = stamps::strokes_for(name) else {
        eprintln!("riddle: card dropped (unknown stamp {name:?})");
        return out;
    };
    // Defensive: `cards::convert_card` already clamps to `MAX_STAMP_COUNT`,
    // but a hand-built `Card` (tests, or a future caller) skips that — and
    // `stamp_block_dims` multiplies count-derived rows/cols into an i32, so
    // an unclamped huge count would overflow and panic in a debug build.
    let count = count.clamp(1, cards::MAX_STAMP_COUNT);
    let (bw, bh) = stamp_block_dims(count);
    let Some((bx, by)) = layout::resolve(map, &common.place, anchor, bw, bh) else {
        eprintln!("riddle: card dropped (no room)");
        return out;
    };
    for instance in stamp_instances(&unit, count, bx, by) {
        commit(make_plan(instance, surface::BLACK, ppf(common.pace)), map, &mut out);
    }
    out
}

fn stamp_plans_in_rect(common: &cards::CardCommon, name: &str, count: u32, rect: (i32, i32, i32, i32)) -> Vec<RenderPlan> {
    let Some(unit) = stamps::strokes_for(name) else {
        eprintln!("riddle: card dropped (unknown stamp {name:?})");
        return Vec::new();
    };
    let count = count.clamp(1, cards::MAX_STAMP_COUNT);
    let (x0, y0, ..) = rect;
    stamp_instances(&unit, count, x0, y0)
        .into_iter()
        .filter_map(|s| make_plan(s, surface::BLACK, ppf(common.pace)))
        .collect()
}

// ---------------------------------------------------------------------
// Count: Dots (circle + numeral per instance), Tally (groups of 4 verticals
// + 1 diagonal), Numbers (delegates to Text).
// ---------------------------------------------------------------------

/// Matches `cards::convert_card`'s own `n.clamp(1, 20)`. Defensive here too:
/// a hand-built `Card::Count` (tests, or a future caller) skips that
/// parse-time clamp, and both Dots/Tally do count-scaled i32 arithmetic
/// (block width/height) that must never overflow.
const MAX_COUNT_N: u32 = 20;

const DOT_D: i32 = 60;
const DOT_R: i32 = DOT_D / 2;
const DOT_COL_STEP: i32 = 84; // DOT_D + 24 gap, matching the stamp row style
const DOT_PER_ROW: u32 = 5;
const DOT_NUM_PX: f32 = 24.0;
const DOT_NUM_GAP: i32 = 6;
const DOT_ROW_GAP: i32 = 24;

/// Rasterize numerals `1..=n` once, up front, so the row height (which
/// depends on numeral glyph height) is known before the block is resolved.
fn numerals(font: &script::FontStack<'_>, n: u32) -> Vec<(i32, i32, Vec<Vec<(i32, i32)>>)> {
    (1..=n.max(1))
        .map(|i| {
            let mut raster = script::rasterize_line(font, &i.to_string(), DOT_NUM_PX);
            script::thin(&mut raster);
            let strokes = script::trace(&raster);
            (raster.width as i32, raster.height as i32, strokes)
        })
        .collect()
}

fn dots_block_dims(n: u32, numeral_h: i32) -> (i32, i32) {
    let n = n.max(1);
    let cols = n.min(DOT_PER_ROW);
    let rows = n.div_ceil(DOT_PER_ROW);
    let row_h = DOT_D + DOT_NUM_GAP + numeral_h;
    (cols as i32 * DOT_COL_STEP - 24, rows as i32 * (row_h + DOT_ROW_GAP) - DOT_ROW_GAP)
}

fn dot_instances(
    nums: &[(i32, i32, Vec<Vec<(i32, i32)>>)],
    numeral_h: i32,
    bx: i32,
    by: i32,
) -> Vec<Vec<Vec<(i32, i32)>>> {
    let row_h = DOT_D + DOT_NUM_GAP + numeral_h;
    nums.iter()
        .enumerate()
        .map(|(i, (nw, _, nstrokes))| {
            let col = (i as u32) % DOT_PER_ROW;
            let row = (i as u32) / DOT_PER_ROW;
            let ox = bx + col as i32 * DOT_COL_STEP;
            let oy = by + row as i32 * (row_h + DOT_ROW_GAP);
            let (cx, cy) = (ox + DOT_R, oy + DOT_R);
            let mut strokes = vec![circle_pts(cx, cy, DOT_R, 28)];
            let nx0 = ox + DOT_R - nw / 2;
            let ny0 = oy + DOT_D + DOT_NUM_GAP;
            for s in nstrokes {
                strokes.push(s.iter().map(|&(sx, sy)| (nx0 + sx, ny0 + sy)).collect());
            }
            strokes
        })
        .collect()
}

fn plan_count_dots(
    common: &cards::CardCommon,
    n: u32,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    let nums = numerals(font, n);
    let numeral_h = nums.iter().map(|&(_, h, _)| h).max().unwrap_or(0);
    let (bw, bh) = dots_block_dims(n, numeral_h);
    let Some((bx, by)) = layout::resolve(map, &common.place, anchor, bw, bh) else {
        eprintln!("riddle: card dropped (no room)");
        return out;
    };
    for instance in dot_instances(&nums, numeral_h, bx, by) {
        commit(make_plan(instance, surface::BLACK, ppf(common.pace)), map, &mut out);
    }
    out
}

fn dots_plans_in_rect(common: &cards::CardCommon, n: u32, font: &script::FontStack<'_>, rect: (i32, i32, i32, i32)) -> Vec<RenderPlan> {
    let nums = numerals(font, n);
    let numeral_h = nums.iter().map(|&(_, h, _)| h).max().unwrap_or(0);
    let (x0, y0, ..) = rect;
    dot_instances(&nums, numeral_h, x0, y0)
        .into_iter()
        .filter_map(|s| make_plan(s, surface::BLACK, ppf(common.pace)))
        .collect()
}

const TALLY_H: i32 = 90;
const TALLY_STROKE_GAP: i32 = 18;
const TALLY_GROUP_W: i32 = TALLY_STROKE_GAP * 3;
const TALLY_GROUP_PITCH: i32 = TALLY_GROUP_W + 36;
const TALLY_GROUPS_PER_ROW: u32 = 4;
const TALLY_ROW_PITCH: i32 = TALLY_H + 30;
/// A full group's diagonal slash overshoots the group's nominal
/// `0..TALLY_GROUP_W` span by this many px on each side (see
/// `tally_groups`) — folded into `tally_block_dims` so the resolved block
/// actually contains the diagonal's ink, not just the four verticals.
const TALLY_OVERSHOOT: i32 = 6;

fn tally_block_dims(n: u32) -> (i32, i32) {
    let groups = n.max(1).div_ceil(5);
    let cols = groups.min(TALLY_GROUPS_PER_ROW);
    let rows = groups.div_ceil(TALLY_GROUPS_PER_ROW);
    (
        cols as i32 * TALLY_GROUP_PITCH - 36 + 2 * TALLY_OVERSHOOT,
        rows as i32 * TALLY_ROW_PITCH - 30,
    )
}

/// One output entry per group of up to 5: up to 4 verticals, plus a
/// diagonal slash across them once a group actually reaches 5. Groups are
/// inset by `TALLY_OVERSHOOT` from `bx` so the first group's diagonal
/// (which reaches `TALLY_OVERSHOOT` px left of its own verticals) lands
/// exactly at `bx`, matching the padding `tally_block_dims` added.
fn tally_groups(n: u32, bx: i32, by: i32) -> Vec<Vec<Vec<(i32, i32)>>> {
    let n = n.max(1);
    let groups = n.div_ceil(5);
    let mut remaining = n;
    let bx = bx + TALLY_OVERSHOOT;
    (0..groups)
        .map(|g| {
            let col = g % TALLY_GROUPS_PER_ROW;
            let row = g / TALLY_GROUPS_PER_ROW;
            let gx = bx + col as i32 * TALLY_GROUP_PITCH;
            let gy = by + row as i32 * TALLY_ROW_PITCH;
            let in_group = remaining.min(5);
            remaining -= in_group;
            let verticals = in_group.min(4);
            let mut strokes = Vec::new();
            for k in 0..verticals {
                let x = gx + k as i32 * TALLY_STROKE_GAP;
                strokes.push(vec![(x, gy), (x, gy + TALLY_H)]);
            }
            if in_group == 5 {
                strokes.push(vec![
                    (gx - TALLY_OVERSHOOT, gy + TALLY_H - 10),
                    (gx + TALLY_GROUP_W + TALLY_OVERSHOOT, gy + 10),
                ]);
            }
            strokes
        })
        .collect()
}

fn plan_count_tally(common: &cards::CardCommon, n: u32, map: &mut layout::InkMap, anchor: layout::Anchor) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    let (bw, bh) = tally_block_dims(n);
    let Some((bx, by)) = layout::resolve(map, &common.place, anchor, bw, bh) else {
        eprintln!("riddle: card dropped (no room)");
        return out;
    };
    for group in tally_groups(n, bx, by) {
        commit(make_plan(group, surface::BLACK, ppf(common.pace)), map, &mut out);
    }
    out
}

fn tally_plans_in_rect(common: &cards::CardCommon, n: u32, rect: (i32, i32, i32, i32)) -> Vec<RenderPlan> {
    let (x0, y0, ..) = rect;
    tally_groups(n, x0, y0)
        .into_iter()
        .filter_map(|s| make_plan(s, surface::BLACK, ppf(common.pace)))
        .collect()
}

fn count_numbers_text(n: u32) -> String {
    (1..=n.max(1)).map(|i| i.to_string()).collect::<Vec<_>>().join(" ")
}

fn plan_count_numbers(
    common: &cards::CardCommon,
    n: u32,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    plan_text(common, &count_numbers_text(n), map, font, anchor)
}

fn numbers_plan_in_rect(
    common: &cards::CardCommon,
    n: u32,
    font: &script::FontStack<'_>,
    rect: (i32, i32, i32, i32),
) -> Option<RenderPlan> {
    text_plan_in_rect(common, &count_numbers_text(n), font, rect)
}

fn plan_count(
    common: &cards::CardCommon,
    n: u32,
    style: cards::CountStyle,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let n = n.clamp(1, MAX_COUNT_N);
    match style {
        cards::CountStyle::Dots => plan_count_dots(common, n, map, font, anchor),
        cards::CountStyle::Tally => plan_count_tally(common, n, map, anchor),
        cards::CountStyle::Numbers => plan_count_numbers(common, n, map, font, anchor),
    }
}

fn count_plans_in_rect(
    common: &cards::CardCommon,
    n: u32,
    style: cards::CountStyle,
    font: &script::FontStack<'_>,
    rect: (i32, i32, i32, i32),
) -> Vec<RenderPlan> {
    let n = n.clamp(1, MAX_COUNT_N);
    match style {
        cards::CountStyle::Dots => dots_plans_in_rect(common, n, font, rect),
        cards::CountStyle::Tally => tally_plans_in_rect(common, n, rect),
        cards::CountStyle::Numbers => numbers_plan_in_rect(common, n, font, rect).into_iter().collect(),
    }
}

// ---------------------------------------------------------------------
// Trace: grid first (if requested), then the template to trace over —
// both FADED, so the child's own black pen stands out on top.
// ---------------------------------------------------------------------

/// Outer rect (4 edges, one polyline each) + horizontal/vertical midlines:
/// 6 strokes exactly, the classic 田字格 "tian" character-writing grid.
fn grid_strokes(x0: i32, y0: i32, box_px: i32) -> Vec<Vec<(i32, i32)>> {
    let (x1, y1) = (x0 + box_px, y0 + box_px);
    let (mx, my) = (x0 + box_px / 2, y0 + box_px / 2);
    vec![
        vec![(x0, y0), (x1, y0)],
        vec![(x1, y0), (x1, y1)],
        vec![(x1, y1), (x0, y1)],
        vec![(x0, y1), (x0, y0)],
        vec![(x0, my), (x1, my)],
        vec![(mx, y0), (mx, y1)],
    ]
}

/// The interaction spec defines a hanzi trace card's content as a single
/// character (单字); take only the first and drop the rest rather than
/// rasterizing a multi-character line that would overrun the box width.
/// Belt-and-braces: even a single CJK glyph could in principle rasterize
/// wider than tall (an unusual glyph, or a non-square box_px), so also
/// re-rasterize at a rescaled height if the first attempt is too wide.
fn hanzi_template_strokes(font: &script::FontStack<'_>, content: &str, x0: i32, y0: i32, box_px: i32) -> Vec<Vec<(i32, i32)>> {
    let mut chars = content.chars();
    let Some(first) = chars.next() else {
        return Vec::new();
    };
    if chars.next().is_some() {
        eprintln!("riddle: trace hanzi truncated to first char");
    }
    let single = first.to_string();
    let mut px = box_px as f32 * 0.8;
    let mut raster = script::rasterize_line(font, &single, px);
    if box_px > 0 && raster.width as i32 > box_px {
        px *= box_px as f32 / raster.width as f32;
        raster = script::rasterize_line(font, &single, px);
    }
    script::thin(&mut raster);
    let local = script::trace(&raster);
    let gx0 = x0 + (box_px - raster.width as i32) / 2;
    let gy0 = y0 + (box_px - raster.height as i32) / 2;
    local.into_iter().map(|s| s.iter().map(|&(sx, sy)| (gx0 + sx, gy0 + sy)).collect()).collect()
}

fn shape_template_strokes(content: &str, x0: i32, y0: i32, box_px: i32) -> Vec<Vec<(i32, i32)>> {
    match shape_strokes(content) {
        Some(norm) => norm
            .into_iter()
            .map(|s| {
                s.iter()
                    .map(|&(nx, ny)| {
                        (x0 + (nx * box_px as f32).round() as i32, y0 + (ny * box_px as f32).round() as i32)
                    })
                    .collect()
            })
            .collect(),
        None => {
            eprintln!("riddle: trace shape {content:?} unknown");
            Vec::new()
        }
    }
}

fn trace_template_strokes(
    font: &script::FontStack<'_>,
    kind: cards::TraceKind,
    content: &str,
    x0: i32,
    y0: i32,
    box_px: i32,
) -> Vec<Vec<(i32, i32)>> {
    match kind {
        cards::TraceKind::Hanzi => hanzi_template_strokes(font, content, x0, y0, box_px),
        cards::TraceKind::Shape => shape_template_strokes(content, x0, y0, box_px),
    }
}

/// Grid plan (if any) first, then the template plan — both FADED. Pure
/// geometry: `box_px` and the origin are already decided by the caller.
fn trace_plans_at(
    common: &cards::CardCommon,
    kind: cards::TraceKind,
    content: &str,
    guide: cards::TraceGuide,
    font: &script::FontStack<'_>,
    x0: i32,
    y0: i32,
    box_px: i32,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    if matches!(guide, cards::TraceGuide::TianGrid) {
        if let Some(p) = make_plan(grid_strokes(x0, y0, box_px), surface::FADED, ppf(common.pace)) {
            out.push(p);
        }
    }
    let template = trace_template_strokes(font, kind, content, x0, y0, box_px);
    if let Some(p) = make_plan(template, surface::FADED, ppf(common.pace)) {
        out.push(p);
    }
    out
}

fn plan_trace(
    common: &cards::CardCommon,
    kind: cards::TraceKind,
    content: &str,
    guide: cards::TraceGuide,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    anchor: layout::Anchor,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    let Some((size, x0, y0)) = resolve_boxed(map, common, anchor) else {
        eprintln!("riddle: card dropped (no room)");
        return out;
    };
    let box_px = size_box(size);
    for plan in trace_plans_at(common, kind, content, guide, font, x0, y0, box_px) {
        commit(Some(plan), map, &mut out);
    }
    if out.is_empty() {
        eprintln!("riddle: card dropped (trace produced nothing)");
    }
    out
}

fn trace_plans_in_rect(
    common: &cards::CardCommon,
    kind: cards::TraceKind,
    content: &str,
    guide: cards::TraceGuide,
    font: &script::FontStack<'_>,
    rect: (i32, i32, i32, i32),
) -> Vec<RenderPlan> {
    let (x0, y0, w, h) = rect;
    let box_px = w.min(h);
    trace_plans_at(common, kind, content, guide, font, x0, y0, box_px)
}

// ---------------------------------------------------------------------
// Page: the one place the server picks explicit coordinates. Each child
// renders straight into its `rect_norm` box, bypassing `layout::resolve`
// entirely; the ink map is still updated so later top-level cards (or a
// future page) don't stack on top of what a page just drew.
// ---------------------------------------------------------------------

fn pixel_rect(rect_norm: (f32, f32, f32, f32)) -> (i32, i32, i32, i32) {
    let (nx, ny, nw, nh) = rect_norm;
    (
        (nx * fb::SCREEN_W as f32).round() as i32,
        (ny * fb::SCREEN_H as f32).round() as i32,
        (nw * fb::SCREEN_W as f32).round() as i32,
        (nh * fb::SCREEN_H as f32).round() as i32,
    )
}

fn plan_in_rect(
    card: &cards::Card,
    rect: (i32, i32, i32, i32),
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    match card {
        cards::Card::Text { common, content } => {
            commit(text_plan_in_rect(common, content, font, rect), map, &mut out);
        }
        cards::Card::Sketch { common, strokes } => {
            commit(sketch_plan_in_rect(common, strokes, rect), map, &mut out);
        }
        cards::Card::Stamp { common, name, count } => {
            for plan in stamp_plans_in_rect(common, name, *count, rect) {
                commit(Some(plan), map, &mut out);
            }
        }
        cards::Card::Count { common, n, style } => {
            for plan in count_plans_in_rect(common, *n, *style, font, rect) {
                commit(Some(plan), map, &mut out);
            }
        }
        cards::Card::Trace { common, kind, content, guide } => {
            for plan in trace_plans_in_rect(common, *kind, content, *guide, font, rect) {
                commit(Some(plan), map, &mut out);
            }
        }
        // The parser refuses a page card nested inside another page's
        // layout, but a hand-built `Card::Page` could still reach here —
        // drop it rather than recurse, exactly like `cards::convert_card`.
        cards::Card::Page { .. } => {
            eprintln!("riddle: cards: nested page card dropped in render");
        }
    }
    out
}

fn plan_page(items: &[(cards::Card, (f32, f32, f32, f32))], map: &mut layout::InkMap, font: &script::FontStack<'_>) -> Vec<RenderPlan> {
    let mut out = Vec::new();
    for (card, rect_norm) in items {
        let rect = pixel_rect(*rect_norm);
        out.extend(plan_in_rect(card, rect, map, font));
    }
    out
}

/// Turn one card into 0..=N render plans (a stamp x count, or count's dots,
/// yields several; everything else yields at most one). Placement search
/// walks the card's shrink ladder (L -> M -> S, or the text-size ladder for
/// `Text`/`Count::Numbers`) until something fits; if nothing does anywhere
/// on the page, the card is dropped quietly (an `eprintln`, never a panic)
/// and this returns an empty vec. Every plan this function returns has
/// already been `mark_rect`'d onto `map` — the caller may mark again
/// (idempotent), but does not have to.
// Temporary: consumed by the cards-test harness task.
#[allow(dead_code)]
pub fn plan_card(
    card: &cards::Card,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    new_ink_anchor: (i32, i32),
) -> Vec<RenderPlan> {
    match card {
        cards::Card::Text { common, content } => {
            let anchor = seed_anchor(common, new_ink_anchor);
            plan_text(common, content, map, font, anchor)
        }
        cards::Card::Sketch { common, strokes } => {
            let anchor = seed_anchor(common, new_ink_anchor);
            plan_sketch(common, strokes, map, anchor)
        }
        cards::Card::Stamp { common, name, count } => {
            let anchor = seed_anchor(common, new_ink_anchor);
            plan_stamp(common, name, *count, map, anchor)
        }
        cards::Card::Count { common, n, style } => {
            let anchor = seed_anchor(common, new_ink_anchor);
            plan_count(common, *n, *style, map, font, anchor)
        }
        cards::Card::Trace { common, kind, content, guide } => {
            let anchor = seed_anchor(common, new_ink_anchor);
            plan_trace(common, *kind, content, *guide, map, font, anchor)
        }
        cards::Card::Page { layout: items } => plan_page(items, map, font),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::*;
    use crate::layout::InkMap;
    use crate::script::FontStack;
    use ab_glyph::FontRef;

    fn font() -> FontStack<'static> {
        FontStack::new(
            FontRef::try_from_slice(include_bytes!("../fonts/PingFangShiGuang.ttf")).unwrap(),
            None,
        )
    }

    fn common(place: Place, size: Size) -> CardCommon {
        CardCommon { place, anchor_norm: None, size, pace: Pace::Normal }
    }

    #[test]
    fn stamp_card_yields_count_plans_inside_page() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Stamp { common: common(Place::BlankArea, Size::S), name: "star".into(), count: 3 };
        let plans = plan_card(&card, &mut map, &font(), (200, 200));
        assert_eq!(plans.len(), 3);
        for p in &plans {
            assert!(!p.strokes.is_empty());
            let (x, y, w, h) = p.region.rect();
            assert!(x >= 0 && y >= 0 && x + w <= 1620 && y + h <= 2160);
        }
    }

    #[test]
    fn sketch_scales_normalized_strokes_into_its_box() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Sketch {
            common: common(Place::BlankArea, Size::M),
            strokes: vec![vec![(0.0, 0.0), (1.0, 1.0)]],
        };
        let plans = plan_card(&card, &mut map, &font(), (100, 100));
        let p = &plans[0];
        let (_, _, w, h) = p.region.rect();
        assert!((w - 486).abs() <= 30 && (h - 486).abs() <= 30, "M box ≈486px, got {w}x{h}");
        assert!(matches!(p.color, crate::surface::BLACK));
    }

    #[test]
    fn trace_template_renders_faded_with_tian_grid() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Trace {
            common: common(Place::BlankArea, Size::L),
            kind: TraceKind::Hanzi, content: "山".into(), guide: TraceGuide::TianGrid,
        };
        let plans = plan_card(&card, &mut map, &font(), (100, 100));
        assert!(!plans.is_empty());
        assert!(plans.iter().all(|p| p.color == crate::surface::FADED));
        // 田字格 = 外框 4 边 + 十字 2 线 ≥ 6 笔（先画格再描字，格在第一个 plan）
        assert!(plans[0].strokes.len() >= 6, "expected grid strokes first");
    }

    #[test]
    fn slow_pace_reduces_points_per_frame() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Sketch {
            common: CardCommon { place: Place::BlankArea, anchor_norm: None, size: Size::S, pace: Pace::Slow },
            strokes: vec![vec![(0.0, 0.5), (1.0, 0.5)]],
        };
        let p = &plan_card(&card, &mut map, &font(), (0, 0))[0];
        assert_eq!(p.points_per_frame, 8);
    }

    #[test]
    fn crowded_page_shrinks_then_gives_up_quietly() {
        let mut map = InkMap::new(1620, 2160);
        map.mark_rect(0, 0, 1620, 2160); // 全页占满
        let card = Card::Stamp { common: common(Place::NearNewInk, Size::L), name: "sun".into(), count: 1 };
        let plans = plan_card(&card, &mut map, &font(), (800, 1000));
        assert!(plans.is_empty(), "nowhere to draw → no plans, no panic");
    }

    #[test]
    fn count_dots_render_n_circles_with_number_labels() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Count { common: common(Place::BlankArea, Size::S), n: 3, style: CountStyle::Dots };
        let plans = plan_card(&card, &mut map, &font(), (0, 0));
        assert!(plans.len() >= 3, "at least one plan per dot, got {}", plans.len());
    }

    #[test]
    fn trace_hanzi_multichar_content_stays_in_its_box() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Trace {
            common: common(Place::BlankArea, Size::S),
            kind: TraceKind::Hanzi, content: "山水火".into(), guide: TraceGuide::None,
        };
        let plans = plan_card(&card, &mut map, &font(), (0, 0));
        assert!(!plans.is_empty());
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for p in &plans {
            for s in &p.strokes {
                for &(x, y) in s {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        assert!(max_x - min_x <= 243, "width {} exceeds the S box", max_x - min_x);
        assert!(max_y - min_y <= 243, "height {} exceeds the S box", max_y - min_y);
    }

    #[test]
    fn page_text_respects_rect_height() {
        let mut map = InkMap::new(1620, 2160);
        let card = Card::Page {
            layout: vec![(
                Card::Text { common: common(Place::BlankArea, Size::L), content: "一二三四五六".into() },
                (0.1, 0.1, 0.15, 0.06), // w=243, h=130 — narrow enough that 84px wraps past it
            )],
        };
        let plans = plan_card(&card, &mut map, &font(), (0, 0));
        assert!(!plans.is_empty());
        let rect_y = (0.1_f32 * 2160.0).round() as i32;
        let rect_h = (0.06_f32 * 2160.0).round() as i32;
        for p in &plans {
            for s in &p.strokes {
                for &(_, y) in s {
                    assert!(
                        y >= rect_y && y <= rect_y + rect_h,
                        "y={y} escapes the rect's [{rect_y}, {}]",
                        rect_y + rect_h
                    );
                }
            }
        }
    }
}
