//! The structured `/turn` client: an env-gated alternative to the legacy
//! chat-completions oracle (oracle.rs). Instead of a free-text vision prompt,
//! it ships the committed page as a full-page PNG plus this turn's new
//! strokes to a structured endpoint, and gets back a [`cards::TurnResponse`]
//! — paper cards the diary draws onto the page (cardrender.rs), rather than
//! prose to write in Tom's hand.
//!
//! Two ways to reach it, both gated by [`turn_mode_enabled`]:
//!  - `RIDDLE_TURN_URL` — POST the request to a real `/turn` server.
//!  - `RIDDLE_TURN_MOCK=path` — skip the network; read a canned response
//!    straight off disk. Local iteration / on-device demos before a server
//!    exists.
//!
//! Neither set: `turn_mode_enabled()` is false and main.rs never even builds
//! a request — behavior is bit-for-bit the legacy oracle path.

use crate::cards;
use crate::fb::{SCREEN_H, SCREEN_W};

use std::sync::mpsc::Sender;
use std::time::Duration;

/// Everything one `/turn` request needs. Borrowed from the caller's live
/// state at commit time — nothing here outlives the `build_request_json`
/// call it's built for.
pub struct TurnRequestMeta<'a> {
    pub turn_id: &'a str,
    /// Always "pen_idle" in this v1 client: a voice trigger or an explicit
    /// commit_now would ride a separate control channel, which is out of
    /// scope for the device client (spec §3; transport TBD on the backend).
    pub trigger: &'a str,
    pub page_png_b64: &'a str,
    /// This turn's new strokes only (same slice main.rs already carves out
    /// of `Ink::stroke_list()` for memory) — `(x, y, r, t)` per point, same
    /// shape as everywhere else in the app (ink.rs, memory.rs).
    pub new_strokes: &'a [Vec<(i32, i32, i32, u32)>],
    pub ink_coverage: f32,
    /// Names the page this turn lands on (a fresh unix-seconds id each time
    /// the page turns), so the server can tell turns on the same sheet
    /// apart from turns on a new one.
    pub page_id: &'a str,
    /// env `RIDDLE_PROFILE`, default "child_3_4".
    pub profile: &'a str,
}

/// Round to 4 decimal places — the wire precision spec §14.1 asks for on
/// normalized coordinates and pressure.
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Build the `/turn` request body (spec §14.1 shape). Stroke x/y/pressure
/// are normalized to 0..1 and rounded to 4dp; `t` (ms since page epoch)
/// passes through unchanged.
///
/// A stroke point is `(x, y, r, t)`, where `r` is the brush RADIUS the pen
/// loop already derived from raw pressure (`2 + p*3/4096`, see main.rs's
/// `pen_point` calls) — `Ink` only ever stores that radius, not the raw
/// pressure sample. `pressure_norm` here undoes that mapping
/// (`(r-2)/3`, clamped to 0..1) to recover an approximation of the
/// original 0..1 pressure. This is a lossy stand-in: the `2 + p*3/4096`
/// step already threw away precision (it quantizes pressure into a radius
/// spanning only 2..=5 px), so `pressure_norm` can only ever be as coarse
/// as that radius allows. Good enough for the server's teaching heuristics
/// today; the real fix is `Ink` keeping raw pressure alongside radius.
pub fn build_request_json(m: &TurnRequestMeta) -> String {
    let strokes: Vec<Vec<serde_json::Value>> = m
        .new_strokes
        .iter()
        .map(|stroke| {
            stroke
                .iter()
                .map(|&(x, y, r, t)| {
                    // Clamped exactly like pressure below (spec §14.1: every
                    // normalized coordinate on the wire is 0..1) — a pen
                    // sample past the screen edge (qtfb path can report
                    // slightly out-of-bounds x/y) must never emit a
                    // coordinate outside that range in the /turn request.
                    let px = round4((x as f64 / SCREEN_W as f64).clamp(0.0, 1.0));
                    let py = round4((y as f64 / SCREEN_H as f64).clamp(0.0, 1.0));
                    let pressure = round4((((r - 2) as f32 / 3.0).clamp(0.0, 1.0)) as f64);
                    serde_json::json!([px, py, pressure, t])
                })
                .collect()
        })
        .collect();

    serde_json::json!({
        "turn_id": m.turn_id,
        "trigger": m.trigger,
        "page_png": m.page_png_b64,
        "new_strokes": strokes,
        "page_state": { "ink_coverage": m.ink_coverage },
        "device_profile": { "profile": m.profile, "screen": [SCREEN_W, SCREEN_H] },
        "page_id": m.page_id,
    })
    .to_string()
}

/// Fetch the `/turn` response body: `RIDDLE_TURN_MOCK=path` reads it straight
/// off disk (no HTTP, no network — `json_body` is ignored entirely in this
/// mode); otherwise POST `json_body` to `RIDDLE_TURN_URL`. Either way, any
/// failure is a string error, never a panic.
fn turn_response_body(json_body: &str) -> Result<String, String> {
    if let Ok(path) = std::env::var("RIDDLE_TURN_MOCK") {
        return std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"));
    }
    let url = std::env::var("RIDDLE_TURN_URL")
        .map_err(|_| "neither RIDDLE_TURN_URL nor RIDDLE_TURN_MOCK is set".to_string())?;
    // Mirrors oracle.rs's HttpOracle error stringification: a plain
    // POST/read here (no persistent agent to warm — unlike the oracle,
    // a card turn is a one-shot request, not a resident stream).
    let resp = ureq::post(&url)
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(30))
        .send_string(json_body);
    match resp {
        Ok(r) => r.into_string().map_err(|e| format!("read response: {e}")),
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            Err(format!("http {code}: {}", detail.trim()))
        }
        Err(e) => Err(format!("request failed: {e}")),
    }
}

/// Run one `/turn` call on its own thread (so the caller's event loop never
/// blocks on it) and deliver the parsed response — or any error, fetch or
/// parse alike — exactly once on `tx`.
pub fn fetch(json_body: String, tx: Sender<Result<cards::TurnResponse, String>>) {
    std::thread::spawn(move || {
        let body = match turn_response_body(&json_body) {
            Ok(b) => b,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let _ = tx.send(cards::parse_turn_response(&body));
    });
}

/// Whether the structured `/turn` path is configured at all — either a real
/// endpoint or a local mock file. `main.rs` gates its whole new commit path
/// on this; when it's false, behavior is bit-for-bit the legacy oracle path.
pub fn turn_mode_enabled() -> bool {
    std::env::var_os("RIDDLE_TURN_URL").is_some() || std::env::var_os("RIDDLE_TURN_MOCK").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_json_matches_spec_shape() {
        // Third point is a raw pen sample past the screen edge (e.g. a qtfb
        // report slightly out of bounds) — its normalized x/y must clamp to
        // 1.0, exactly like an out-of-range pressure already clamps.
        let strokes =
            vec![vec![(810, 1080, 3, 0u32), (972, 1080, 4, 12), (99_999, 99_999, 3, 20)]];
        let m = TurnRequestMeta {
            turn_id: "t-1",
            trigger: "pen_idle",
            page_png_b64: "QUJD",
            new_strokes: &strokes,
            ink_coverage: 0.42,
            page_id: "p-1",
            profile: "child_3_4",
        };
        let v: serde_json::Value = serde_json::from_str(&build_request_json(&m)).unwrap();
        assert_eq!(v["turn_id"], "t-1");
        assert_eq!(v["trigger"], "pen_idle");
        assert_eq!(v["page_png"], "QUJD");
        assert_eq!(v["page_id"], "p-1");
        assert_eq!(v["device_profile"]["profile"], "child_3_4");
        assert_eq!(v["device_profile"]["screen"][0], 1620);
        assert_eq!(v["device_profile"]["screen"][1], 2160);
        // f32 -> JSON round-trip loses a little precision (~1e-7 for a value
        // like 0.42); 1e-6 comfortably covers that without pretending to
        // check bit-exact equality.
        assert!((v["page_state"]["ink_coverage"].as_f64().unwrap() - 0.42).abs() < 1e-6);

        let p0 = &v["new_strokes"][0][0];
        assert!((p0[0].as_f64().unwrap() - 0.5).abs() < 1e-6, "x normalized: {p0:?}");
        assert!((p0[1].as_f64().unwrap() - 0.5).abs() < 1e-6, "y normalized: {p0:?}");
        // radius 3 -> pressure_norm = (3-2)/3 ~= 0.3333 (see build_request_json's
        // doc comment on why this is a lossy approximation, not raw pressure).
        assert!((p0[2].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-3, "pressure approx: {p0:?}");
        assert_eq!(p0[3], 0);

        let p1 = &v["new_strokes"][0][1];
        assert_eq!(p1[3], 12, "t passes through unchanged");

        let p2 = &v["new_strokes"][0][2];
        assert_eq!(p2[0].as_f64().unwrap(), 1.0, "x clamps to 1.0 past the screen edge: {p2:?}");
        assert_eq!(p2[1].as_f64().unwrap(), 1.0, "y clamps to 1.0 past the screen edge: {p2:?}");
    }

    #[test]
    fn mock_file_mode_round_trips() {
        let dir = std::env::temp_dir().join(format!("riddle-turn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("resp.json");
        std::fs::write(&p, r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"stamp","name":"star","count":1}],"page_action":"none","memory_tags":[]}"#).unwrap();
        std::env::set_var("RIDDLE_TURN_MOCK", &p);
        let (tx, rx) = std::sync::mpsc::channel();
        fetch(String::from("{}"), tx);
        let r = rx.recv().unwrap().unwrap();
        assert_eq!(r.paper_cards.len(), 1);
        std::env::remove_var("RIDDLE_TURN_MOCK");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
