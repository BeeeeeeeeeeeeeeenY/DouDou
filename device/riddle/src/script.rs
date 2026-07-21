//! Tom Riddle's hand: rasterize reply text in the selected handwriting font,
//! thin it to single-pixel pen paths (Zhang-Suen), trace them into ordered
//! strokes, and yield them for stroke-by-stroke animation.

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont};

/// The diary's handwriting font stack. The primary font is expected to cover
/// Latin, numbers, Chinese and punctuation; fallback remains for experiments.
pub struct FontStack<'a> {
    primary: FontRef<'a>,
    fallback: Option<FontRef<'a>>,
}

impl<'a> FontStack<'a> {
    pub fn new(primary: FontRef<'a>, fallback: Option<FontRef<'a>>) -> Self {
        Self { primary, fallback }
    }

    /// Return whether the fallback face was selected, plus the selected face.
    fn face_for(&self, c: char) -> (bool, &FontRef<'a>) {
        if self.primary.glyph_id(c).0 != 0 {
            return (false, &self.primary);
        }
        if let Some(ref fallback) = self.fallback {
            if fallback.glyph_id(c).0 != 0 {
                return (true, fallback);
            }
        }
        // Keep the primary .notdef glyph for genuinely unsupported symbols.
        (false, &self.primary)
    }

    fn baseline(&self, px: f32) -> f32 {
        let primary = self.primary.as_scaled(PxScale::from(px)).ascent();
        self.fallback
            .as_ref()
            .map(|font| font.as_scaled(PxScale::from(px)).ascent())
            .unwrap_or(primary)
            .max(primary)
    }
}

pub struct Line {
    pub width: usize,
    pub height: usize,
    /// Bit mask of inked pixels, row-major.
    pub mask: Vec<bool>,
}

/// Rasterize one line of text at `px` height into a boolean mask.
pub fn rasterize_line(font: &FontStack<'_>, text: &str, px: f32) -> Line {
    const PAD: f32 = 6.0;

    let mut glyphs: Vec<(bool, Glyph)> = Vec::new();
    let mut caret = 0.0f32;
    let mut prev: Option<(bool, ab_glyph::GlyphId)> = None;
    let baseline = font.baseline(px);
    for c in text.chars() {
        let (uses_fallback, face) = font.face_for(c);
        let scaled = face.as_scaled(PxScale::from(px));
        let id = scaled.glyph_id(c);
        if let Some((prev_fallback, p)) = prev.filter(|(which, _)| *which == uses_fallback) {
            debug_assert_eq!(prev_fallback, uses_fallback);
            caret += scaled.kern(p, id);
        }
        let mut g = id.with_scale(PxScale::from(px));
        g.position = ab_glyph::point(caret + 2.0, baseline + 2.0);
        caret += scaled.h_advance(id);
        glyphs.push((uses_fallback, g));
        prev = Some((uses_fallback, id));
    }

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (uses_fallback, g) in &glyphs {
        let face = if *uses_fallback {
            font.fallback.as_ref().expect("selected fallback must exist")
        } else {
            &font.primary
        };
        if let Some(outline) = face.outline_glyph(g.clone()) {
            let bounds = outline.px_bounds();
            min_x = min_x.min(bounds.min.x.floor());
            min_y = min_y.min(bounds.min.y.floor());
            max_x = max_x.max(bounds.max.x.ceil());
            max_y = max_y.max(bounds.max.y.ceil());
        }
    }

    if !min_x.is_finite() || !min_y.is_finite() {
        let width = (caret.ceil() as usize + (PAD as usize) * 2).max(1);
        return Line { width, height: 1, mask: vec![false; width] };
    }

    let shift_x = PAD - min_x;
    let shift_y = PAD - min_y;
    let width = ((max_x - min_x + PAD * 2.0).ceil() as usize).max(1);
    let height = ((max_y - min_y + PAD * 2.0).ceil() as usize).max(1);
    let mut mask = vec![false; width * height];
    for (uses_fallback, g) in glyphs {
        let face = if uses_fallback {
            font.fallback.as_ref().expect("selected fallback must exist")
        } else {
            &font.primary
        };
        let mut g = g;
        g.position.x += shift_x;
        g.position.y += shift_y;
        if let Some(outline) = face.outline_glyph(g) {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, cov| {
                if cov > 0.5 {
                    let px_x = bounds.min.x as i32 + x as i32;
                    let px_y = bounds.min.y as i32 + y as i32;
                    if px_x >= 0 && px_y >= 0 && (px_x as usize) < width && (px_y as usize) < height {
                        mask[px_y as usize * width + px_x as usize] = true;
                    }
                }
            });
        }
    }
    Line { width, height, mask }
}

/// Measure the advance width of text at `px` without rasterizing.
pub fn measure(font: &FontStack<'_>, text: &str, px: f32) -> f32 {
    let mut caret = 0.0f32;
    let mut prev: Option<(bool, ab_glyph::GlyphId)> = None;
    for c in text.chars() {
        let (uses_fallback, face) = font.face_for(c);
        let scaled = face.as_scaled(PxScale::from(px));
        let id = scaled.glyph_id(c);
        if let Some((_, p)) = prev.filter(|(which, _)| *which == uses_fallback) {
            caret += scaled.kern(p, id);
        }
        caret += scaled.h_advance(id);
        prev = Some((uses_fallback, id));
    }
    caret
}

/// Zhang-Suen thinning: reduce the mask to 1px-wide skeleton lines.
pub fn thin(line: &mut Line) {
    let (w, h) = (line.width, line.height);
    let idx = |x: usize, y: usize| y * w + x;
    loop {
        let mut changed = false;
        for phase in 0..2 {
            let mut to_clear = Vec::new();
            for y in 1..h.saturating_sub(1) {
                for x in 1..w.saturating_sub(1) {
                    if !line.mask[idx(x, y)] {
                        continue;
                    }
                    let p = [
                        line.mask[idx(x, y - 1)],     // p2 N
                        line.mask[idx(x + 1, y - 1)], // p3 NE
                        line.mask[idx(x + 1, y)],     // p4 E
                        line.mask[idx(x + 1, y + 1)], // p5 SE
                        line.mask[idx(x, y + 1)],     // p6 S
                        line.mask[idx(x - 1, y + 1)], // p7 SW
                        line.mask[idx(x - 1, y)],     // p8 W
                        line.mask[idx(x - 1, y - 1)], // p9 NW
                    ];
                    let b: u32 = p.iter().map(|&v| v as u32).sum();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let mut a = 0;
                    for i in 0..8 {
                        if !p[i] && p[(i + 1) % 8] {
                            a += 1;
                        }
                    }
                    if a != 1 {
                        continue;
                    }
                    let (c1, c2) = if phase == 0 {
                        (!(p[0] && p[2] && p[4]), !(p[2] && p[4] && p[6]))
                    } else {
                        (!(p[0] && p[2] && p[6]), !(p[0] && p[4] && p[6]))
                    };
                    if c1 && c2 {
                        to_clear.push(idx(x, y));
                    }
                }
            }
            if !to_clear.is_empty() {
                changed = true;
                for i in to_clear {
                    line.mask[i] = false;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Trace the skeleton into polyline strokes, ordered left-to-right so the
/// animation writes like a hand.
pub fn trace(line: &Line) -> Vec<Vec<(i32, i32)>> {
    let (w, h) = (line.width, line.height);
    let at = |x: i32, y: i32| -> bool {
        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && line.mask[y as usize * w + x as usize]
    };
    let neighbors = |x: i32, y: i32| -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                if (dx != 0 || dy != 0) && at(x + dx, y + dy) {
                    out.push((x + dx, y + dy));
                }
            }
        }
        out
    };

    let mut visited = vec![false; w * h];
    let vis = |v: &mut Vec<bool>, x: i32, y: i32| {
        v[y as usize * w + x as usize] = true;
    };
    let seen = |v: &Vec<bool>, x: i32, y: i32| -> bool { v[y as usize * w + x as usize] };

    // Endpoints first (degree 1), then any remaining pixels (loops).
    let mut starts: Vec<(i32, i32)> = Vec::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if at(x, y) && neighbors(x, y).len() == 1 {
                starts.push((x, y));
            }
        }
    }
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if at(x, y) {
                starts.push((x, y));
            }
        }
    }

    let mut strokes: Vec<Vec<(i32, i32)>> = Vec::new();
    for (sx, sy) in starts {
        if seen(&visited, sx, sy) {
            continue;
        }
        let mut path = vec![(sx, sy)];
        vis(&mut visited, sx, sy);
        let (mut cx, mut cy) = (sx, sy);
        loop {
            let next = neighbors(cx, cy)
                .into_iter()
                .find(|&(nx, ny)| !seen(&visited, nx, ny));
            match next {
                Some((nx, ny)) => {
                    vis(&mut visited, nx, ny);
                    path.push((nx, ny));
                    cx = nx;
                    cy = ny;
                }
                None => break,
            }
        }
        if path.len() >= 3 {
            strokes.push(path);
        }
    }
    strokes.sort_by_key(|s| s.iter().map(|&(x, _)| x).min().unwrap_or(0));
    strokes
}

/// True for Han, Kana, Hangul and CJK punctuation. These scripts normally do
/// not put spaces between words and therefore provide a break after each
/// character instead of relying on `split_whitespace`.
pub fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x312F
            | 0x31A0..=0x31BF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFFEF
            | 0x20000..=0x323AF
    )
}

#[derive(Debug)]
struct WrapToken {
    text: String,
    space_before: bool,
}

fn tokens(para: &str) -> Vec<WrapToken> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut word_space = false;
    let mut pending_space = false;

    for c in para.chars() {
        if c.is_whitespace() {
            if !word.is_empty() {
                out.push(WrapToken { text: std::mem::take(&mut word), space_before: word_space });
            }
            pending_space = true;
        } else if is_cjk(c) {
            if !word.is_empty() {
                out.push(WrapToken { text: std::mem::take(&mut word), space_before: word_space });
            }
            out.push(WrapToken { text: c.to_string(), space_before: pending_space });
            pending_space = false;
        } else {
            if word.is_empty() {
                word_space = pending_space;
                pending_space = false;
            }
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(WrapToken { text: word, space_before: word_space });
    }
    out
}

fn is_closing_punctuation(text: &str) -> bool {
    text.chars().count() == 1
        && text
            .chars()
            .next()
            .is_some_and(|c| "，。！？；：、）】》〉」』〕］｝…—".contains(c))
}

fn is_opening_punctuation(c: char) -> bool {
    "（【《〈「『〔［｛".contains(c)
}

/// Wrap mixed Latin/CJK text without requiring spaces between Chinese words.
pub fn wrap(font: &FontStack<'_>, text: &str, px: f32, max_px: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut cur = String::new();
        for token in tokens(para) {
            let separator = if token.space_before && !cur.is_empty() { " " } else { "" };
            let cand = format!("{cur}{separator}{}", token.text);
            if measure(font, &cand, px) <= max_px {
                cur = cand;
            } else if !cur.is_empty() && is_closing_punctuation(&token.text) {
                // A closing mark belongs to the preceding line, even if that
                // makes it a few pixels wider than the nominal margin.
                cur.push_str(&token.text);
            } else {
                // Do not strand an opening quote/bracket at the end of a line.
                let opener = cur
                    .chars()
                    .last()
                    .filter(|&c| is_opening_punctuation(c))
                    .and_then(|_| cur.char_indices().last().map(|(i, _)| cur.split_off(i)));
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                if let Some(open) = opener {
                    cur.push_str(&open);
                }

                // Exceptionally long unspaced Latin strings still need a
                // character-level fallback so they cannot leave the screen.
                for c in token.text.chars() {
                    let next = format!("{cur}{c}");
                    if !cur.is_empty() && measure(font, &next, px) > max_px {
                        lines.push(std::mem::take(&mut cur));
                    }
                    cur.push(c);
                }
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_produces_strokes() {
        let font = FontStack::new(
            FontRef::try_from_slice(include_bytes!("../fonts/PingFangShiGuang.ttf")).unwrap(),
            None,
        );
        let mut line = rasterize_line(&font, "Yes, Harry?", 96.0);
        assert!(line.width > 100 && line.height > 50);
        let inked_before: usize = line.mask.iter().filter(|&&v| v).count();
        thin(&mut line);
        let inked_after: usize = line.mask.iter().filter(|&&v| v).count();
        assert!(inked_after * 3 < inked_before, "thinning should slim the glyphs: {inked_before} -> {inked_after}");
        let strokes = trace(&line);
        assert!(!strokes.is_empty());
        let total: usize = strokes.iter().map(|s| s.len()).sum();
        println!("strokes={} total_points={} ({}x{})", strokes.len(), total, line.width, line.height);
        assert!(total > 200, "expected a decent path length, got {total}");
        // Wrap sanity.
        let lines = wrap(&font, "Do you know anything about the Chamber of Secrets?", 96.0, 1380.0);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn unified_font_renders_mixed_cjk_latin_text() {
        let font = FontStack::new(
            FontRef::try_from_slice(include_bytes!("../fonts/PingFangShiGuang.ttf")).unwrap(),
            None,
        );
        let text = "你好，我是一本会回答问题的日记。Hello，世界！";
        let lines = wrap(&font, text, 88.0, 520.0);
        assert!(lines.len() >= 3, "expected CJK wrapping, got {lines:?}");
        assert!(lines.iter().all(|line| measure(&font, line, 88.0) <= 620.0));

        let mut raster = rasterize_line(&font, "你好，Harry！", 88.0);
        assert!(raster.mask.iter().any(|&pixel| pixel));
        thin(&mut raster);
        assert!(!trace(&raster).is_empty());
    }

    #[test]
    fn rasterization_keeps_ink_away_from_edges() {
        let font = FontStack::new(
            FontRef::try_from_slice(include_bytes!("../fonts/PingFangShiGuang.ttf")).unwrap(),
            None,
        );
        let raster = rasterize_line(&font, "中g？你好。", 52.0);
        let border = 2usize;
        let mut edge_ink = 0usize;
        for y in 0..raster.height {
            for x in 0..raster.width {
                if x < border
                    || y < border
                    || x + border >= raster.width
                    || y + border >= raster.height
                {
                    edge_ink += raster.mask[y * raster.width + x] as usize;
                }
            }
        }
        assert_eq!(edge_ink, 0, "glyph ink reached the raster edge");
    }
}
