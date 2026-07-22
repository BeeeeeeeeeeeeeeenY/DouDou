//! Built-in reward stamps: the eight celebration symbols the server may name
//! in a `stamp` card (see `cards::STAMP_NAMES`). Kept on-device so the most
//! frequent feedback renders instantly and identically regardless of the
//! model behind the server. All geometry is normalized to a unit box, y
//! down, and generated parametrically so tweaking proportions stays a
//! one-line change rather than editing a table of magic points.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Points around a full circle, closed (first and last point coincide).
fn circle(cx: f32, cy: f32, r: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = i as f32 / n as f32 * TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Points along a circular arc sweeping from angle `a0` to `a1` (radians).
fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Points along a quadratic Bezier curve from `p0` through control point `p1`
/// to `p2`. Always starts exactly at `p0` and ends exactly at `p2`, so two
/// curves that share endpoints join with no gap or jump.
fn quad_bezier(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            (
                mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0,
                mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1,
            )
        })
        .collect()
}

/// Subdivide a coarse vertex path into a denser polyline by linearly
/// interpolating along each edge, so a handful of anchor points still yields
/// enough samples for a legible stroke.
fn polyline(vertices: &[(f32, f32)], segments_per_edge: usize) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(vertices.len() * segments_per_edge + 1);
    for w in vertices.windows(2) {
        for j in 0..segments_per_edge {
            let t = j as f32 / segments_per_edge as f32;
            out.push((w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t));
        }
    }
    if let Some(&last) = vertices.last() {
        out.push(last);
    }
    out
}

pub fn strokes_for(name: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    let mut strokes: Vec<Vec<(f32, f32)>> = match name {
        // One-stroke five-pointed star: connect the 5 outer vertices in
        // skip order (vertex i at angle -90 + i*144 degrees, i = 0..=5, the
        // last vertex re-visiting the first to close the loop). Edges are
        // subdivided so a 6-vertex pentagram still samples plenty of points.
        "star" => {
            let verts: Vec<(f32, f32)> = (0..=5)
                .map(|i| {
                    let a = (-90.0 + i as f32 * 144.0).to_radians();
                    (0.5 + 0.48 * a.cos(), 0.5 + 0.48 * a.sin())
                })
                .collect();
            vec![polyline(&verts, 3)]
        }

        // Five round petals ringed around a center disk.
        "flower" => {
            let mut petals: Vec<Vec<(f32, f32)>> = (0..5)
                .map(|p| {
                    let a = p as f32 / 5.0 * TAU - FRAC_PI_2;
                    let (px, py) = (0.5 + 0.24 * a.cos(), 0.5 + 0.24 * a.sin());
                    circle(px, py, 0.17, 10)
                })
                .collect();
            petals.push(circle(0.5, 0.5, 0.13, 12));
            petals
        }

        // Classic parametric heart curve, mirrored left/right into one loop.
        "heart" => {
            let half: Vec<(f32, f32)> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0 * PI;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos()
                        - (4.0 * t).cos();
                    (0.5 + x / 34.0, 0.42 - y / 34.0)
                })
                .collect();
            let mut whole = half.clone();
            whole.extend(half.iter().rev().skip(1).map(|&(x, y)| (1.0 - x, y)));
            vec![whole]
        }

        // Face circle, two eyes, and a smiling arc.
        "smiley" => vec![
            circle(0.5, 0.5, 0.45, 24),
            vec![(0.37, 0.38), (0.37, 0.45)],
            vec![(0.63, 0.38), (0.63, 0.45)],
            arc(0.5, 0.58, 0.22, 0.4, PI - 0.4, 10),
        ],

        // Checkmark: short down-stroke then long up-stroke, densified so the
        // two segments still clear the point-count floor as a single stroke.
        "check" => {
            let verts = [(0.14, 0.52), (0.38, 0.78), (0.90, 0.16)];
            vec![polyline(&verts, 7)]
        }

        // Circle with short rays radiating outward.
        "sun" => {
            let mut v = vec![circle(0.5, 0.5, 0.27, 20)];
            for i in 0..6 {
                let a = i as f32 / 6.0 * TAU;
                v.push(vec![
                    (0.5 + 0.34 * a.cos(), 0.5 + 0.34 * a.sin()),
                    (0.5 + 0.47 * a.cos(), 0.5 + 0.47 * a.sin()),
                ]);
            }
            v
        }

        // Crescent: two Bezier curves sharing the same top/bottom tip
        // points but bulging by different amounts, so the loop tapers to a
        // point at each tip instead of leaving a jump where they meet.
        "moon" => {
            let top = (0.60, 0.14);
            let bottom = (0.60, 0.86);
            let mut pts = quad_bezier(top, (0.16, 0.5), bottom, 14);
            pts.extend(quad_bezier(bottom, (0.40, 0.5), top, 12));
            vec![pts]
        }

        // Balloon: elliptical body, a small tied knot, and a wavy string.
        "balloon" => {
            let body: Vec<(f32, f32)> = (0..=22)
                .map(|i| {
                    let a = i as f32 / 22.0 * TAU;
                    (0.5 + 0.28 * a.cos(), 0.4 + 0.34 * a.sin())
                })
                .collect();
            let knot = vec![(0.46, 0.74), (0.5, 0.79), (0.54, 0.74)];
            let string: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let t = i as f32 / 10.0;
                    (0.5 + 0.05 * (t * PI * 3.0).sin(), 0.80 + 0.18 * t)
                })
                .collect();
            vec![body, knot, string]
        }

        _ => return None,
    };

    for stroke in &mut strokes {
        for p in stroke.iter_mut() {
            p.0 = p.0.clamp(0.0, 1.0);
            p.1 = p.1.clamp(0.0, 1.0);
        }
    }
    Some(strokes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 8] = ["star", "flower", "heart", "smiley", "check", "sun", "moon", "balloon"];

    #[test]
    fn every_stamp_exists_and_stays_in_unit_box() {
        for name in NAMES {
            let strokes = strokes_for(name).unwrap_or_else(|| panic!("missing stamp {name}"));
            assert!(!strokes.is_empty(), "{name} has no strokes");
            let total: usize = strokes.iter().map(|s| s.len()).sum();
            assert!(total >= 12, "{name} too crude: {total} points");
            assert!(strokes.len() <= 8, "{name} too many strokes for a quick reward");
            for s in &strokes {
                assert!(s.len() >= 2, "{name} has a 1-point stroke");
                for &(x, y) in s {
                    assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                        "{name} point ({x},{y}) escapes the unit box");
                }
            }
        }
    }

    #[test]
    fn unknown_name_is_none() {
        assert!(strokes_for("dragon").is_none());
    }
}
