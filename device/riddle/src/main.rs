//! riddle — the diary of Tom Riddle, for the reMarkable Paper Pro.
//!
//! Write on the page with the pen. After a pause the diary drinks your ink,
//! and an answer writes itself onto the page in a flowing hand, then fades.
//!
//! Two display backends (picked at runtime): windowed via qtfb/AppLoad when
//! QTFB_KEY is set, or full takeover via the vendor engine (quill) when
//! built with --features takeover and launched with xochitl stopped.

mod cardrender;
mod cards;
mod display;
mod fb;
mod help;
mod ink;
mod layout;
mod memory;
mod oracle;
mod pen;
mod power;
mod qtfb;
mod script;
mod stamps;
mod surface;
mod touch;
mod turn;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ab_glyph::FontRef;

use fb::{BBox, SCREEN_H, SCREEN_W};
use oracle::Event;
use surface::{Surface, BLACK, FADED, WHITE};

const FONT_TTF: &[u8] = include_bytes!("../fonts/PingFangShiGuang.ttf");
const PNG_PATH: &str = "/tmp/riddle-page.png";

/// Pen-idle threshold before a page commits. 3-4 岁孩子涂鸦停顿频繁，默认
/// 从 riddle 的 2.8s 放宽到 6s；oracle.env 里 RIDDLE_IDLE_COMMIT_SECS 可调。
fn idle_commit_from(raw: Option<&str>) -> Duration {
    let secs = raw.and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.0).unwrap_or(6.0);
    Duration::from_millis((secs * 1000.0) as u64)
}

fn idle_commit() -> Duration {
    static CACHE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        let v = std::env::var("RIDDLE_IDLE_COMMIT_SECS");
        idle_commit_from(v.as_deref().ok())
    })
}

/// Coverage ratio (parsed) that triggers a fresh sheet once the current
/// reply finishes. 0.0 is kept as a valid, meaningful value (it means "auto
/// page turns disabled") rather than folded into the parse-failure fallback.
fn page_full_threshold_from(raw: Option<&str>) -> f32 {
    raw.and_then(|v| v.parse::<f32>().ok()).filter(|t| (0.0..=1.0).contains(t)).unwrap_or(0.55)
}

/// Coverage ratio that triggers a fresh sheet once the current reply
/// finishes. RIDDLE_PAGE_FULL overrides (0 disables auto page turns).
fn page_full_threshold() -> f32 {
    static CACHE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        let v = std::env::var("RIDDLE_PAGE_FULL");
        page_full_threshold_from(v.as_deref().ok())
    })
}
/// How long the diary waits on a silent oracle before giving up on the turn.
/// Generous: thinking models can lead with a long silence.
const ORACLE_PATIENCE: Duration = Duration::from_secs(120);
const REPLY_PX: f32 = 52.0;
const MARGIN_X: i32 = 120;
/// The thinking dot lives in the top-right corner, not page-center: with
/// co-drawing the center of the page is the child's canvas, not Tom's.
const THINK_X: i32 = SCREEN_W as i32 - 90;
const THINK_Y: i32 = 90;
/// How long the diary waits on a silent /turn server before giving up —
/// shorter than `ORACLE_PATIENCE`: a card turn is a single structured call,
/// not a model that may lead with a long thinking silence.
const TURN_PATIENCE: Duration = Duration::from_secs(30);

/// Unix seconds now (0 on a clock error). Names a turn or a page: `turn_id`
/// at every commit, `page_id` once per sheet — both sides of a fresh
/// `SystemTime` read, factored out so the two never drift apart.
fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

const USAGE: &str = "\
riddle — the diary of Tom Riddle

usage:
  riddle                      open the diary (windowed when AppLoad sets
                              QTFB_KEY, otherwise takeover via libquill)
  riddle --oracle-test [PNG]  run one oracle turn against PNG (default
                              /tmp/riddle-page.png) and print the streamed
                              reply; verifies key + endpoint + model
  riddle --cards-test F [OUT] render a /turn fixture to a PNG (no device
                              needed)
  riddle --version            print the version

configuration lives in oracle.env next to the binary — see
oracle.env.example for every RIDDLE_* variable.
";

type OracleRx = mpsc::Receiver<Result<Event, String>>;

enum State {
    Listening { last_pen: Option<Instant> },
    /// `origin`: where the reply will be planted once the oracle answers —
    /// resolved once at commit time (Task 10) via `layout::resolve` against
    /// the page as it stood then. `None` means nothing that size fit
    /// anywhere (a nearly-full page); the reply then falls back to the
    /// legacy centered placement.
    /// `page_full`: the page's ink coverage (from that same commit-time map)
    /// was already past `page_full_threshold()` before this reply even
    /// started — carried through so a completed reply can trigger the page
    /// turn afterward (Task 11).
    Thinking {
        rx: OracleRx,
        pulse: Instant,
        blot_on: bool,
        since: Instant,
        origin: Option<(i32, i32)>,
        page_full: bool,
    },
    Replying { plan: WritePlan, next: Instant, rx: Option<OracleRx>, page_full: bool },
    /// Waiting on a structured `/turn` response (Task 12's card path — the
    /// alternative to Thinking/Replying taken when `turn::turn_mode_enabled()`).
    /// `anchor`: same "hug the new ink" point `Thinking.origin` resolves
    /// from, but resolution itself waits until the response's cards are
    /// known (each card picks its own box size), so only the seed point
    /// travels here, not a pre-resolved origin. `page_full`: decided at
    /// commit time, same meaning as everywhere else it appears.
    CardTurn { rx: mpsc::Receiver<Result<cards::TurnResponse, String>>, page_full: bool, anchor: (i32, i32), since: Instant },
    /// The response's paper cards, planned into screen-space strokes
    /// (Task 6-8's `cardrender::plan_card`) and queued up to animate one
    /// after another, each at its own card's pace/color.
    DrawingCards { plans: Vec<cardrender::RenderPlan>, plan_i: usize, point_i: usize, next: Instant, page_full: bool },
    /// The guide panel. `panel: None` = dismissed, waiting for pen-up so the
    /// dismissing touch doesn't leave a mark on the page.
    Help { panel: Option<help::Help>, until: Instant },
    /// A remembered page rising through the paper: date, the writer's own
    /// past ink, Tom's old reply — all in faded ink. `saved` is today's page.
    Conjuring { plan: ConjurePlan, next: Instant, saved: Vec<u8> },
    /// The conjured memory rests on the page. Pen contact (or time) dissolves
    /// it and today's page returns. `saved: None` = dismissed, waiting pen-up.
    MemoryShown { saved: Option<Vec<u8>>, until: Instant, region: BBox },
}

/// A memory being rewritten onto the page: pre-positioned strokes with their
/// original radii, drawn in faded ink.
struct ConjurePlan {
    strokes: Vec<Vec<(i32, i32, i32)>>,
    stroke_i: usize,
    point_i: usize,
    region: BBox,
}

struct WritePlan {
    strokes: Vec<Vec<(i32, i32)>>,
    stroke_i: usize,
    point_i: usize,
    region: BBox,
    /// Where the next streamed chunk's first line starts.
    next_y: i32,
    /// The left-align x from this reply's `origin`, so a streamed
    /// continuation (`append_reply`) keeps lining up with the first chunk
    /// instead of snapping back to the centered legacy layout.
    origin_x: Option<i32>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // Diagnostic: run one oracle turn and print the streamed chunks.
        // Lets you verify your endpoint + key + model before ever launching
        // the diary. No display needed.
        Some("--oracle-test") => {
            let png = args.get(2).map(String::as_str).unwrap_or(PNG_PATH);
            std::process::exit(oracle_test(png));
        }
        // Diagnostic: render a /turn fixture (fake ink + response JSON) to a
        // full-page PNG, off-screen. No device, no network — lets a human
        // eyeball every card type before ever wiring cards into run().
        Some("--cards-test") => {
            std::process::exit(cards_test(&args));
        }
        // Spike (S0a): paint colour bands and sweep vendor waveform modes on
        // the real takeover display, timing each swap. A human watches the
        // e-paper to judge whether colour is viable (which mode, how fast,
        // how much flicker). Needs the takeover backend + xochitl stopped.
        Some("--color-test") => {
            std::process::exit(color_test(&args));
        }
        // Spike (Demo Plan 2, Task 3): synthesize a multi-band colour PNG,
        // run it through the real cards::Card::Image -> cardrender::plan_image
        // decode/layout path, then paste_rect + swap_raw(mode 5) it onto the
        // real takeover display. A human watches the panel to confirm colour
        // survives the mode-5 waveform. Needs the takeover backend +
        // xochitl stopped.
        Some("--image-test") => {
            std::process::exit(image_test(&args));
        }
        // Colour calibration: paint a 6-family x 5-candidate RGB swatch grid at
        // full 8-bit (paste_rect, not the 565 fill_rect) and swap at mode 5, so
        // a human can photograph which RGB the panel actually renders as a clean
        // target colour. Rows top->bottom: red, orange, yellow, green, blue, pink.
        Some("--swatch-test") => {
            std::process::exit(swatch_test(&args));
        }
        Some("--version" | "-V") => {
            println!("riddle {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return;
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("riddle: unknown flag {flag}\n");
            eprint!("{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }
    if let Err(e) = run() {
        eprintln!("riddle: fatal: {e}");
        std::process::exit(1);
    }
}

/// S0a colour spike: paint 9 horizontal colour bands into the real takeover
/// framebuffer, then re-swap the whole screen at a sweep of vendor waveform
/// `mode` values, timing each. The buffer never changes between swaps, so the
/// ONLY variable a watcher sees is the waveform. Prints a banner before each
/// swap so the operator can correlate what's on the panel with the mode.
fn color_test(args: &[String]) -> i32 {
    // Optional hold seconds per mode (default 5): `--color-test [secs]`.
    let hold = args.get(2).and_then(|s| s.parse::<u64>().ok()).filter(|s| *s > 0).unwrap_or(5);
    let (display, mut surf) = match display::Display::open() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("color-test: display open failed: {e}");
            eprintln!("  (needs the takeover build + xochitl stopped)");
            return 1;
        }
    };
    let (w, h) = (surf.w, surf.h);
    eprintln!("color-test: surface {w}x{h} open, hold {hold}s/mode");

    // RGB565 band colours (Surface expands to RGB32 on the takeover buffer).
    // Painted once; only the top mode-indicator strip changes per swap.
    let bands: [(u16, &str); 8] = [
        (0xF800, "red"),
        (0x07E0, "green"),
        (0x001F, "blue"),
        (0xFFE0, "yellow"),
        (0x07FF, "cyan"),
        (0xF81F, "magenta"),
        (0x7BCF, "mid-gray"),
        (0x0000, "black"),
    ];
    // Leave the top STRIP px as a white mode-indicator band; split the rest
    // among the colour bands.
    const STRIP: usize = 150;
    let bh = (h - STRIP) / bands.len();
    for (i, (c, name)) in bands.iter().enumerate() {
        surf.fill_rect(0, STRIP + i * bh, w, bh, *c);
        eprintln!("  band {i}: {name} (0x{c:04X})");
    }

    // Loop the COLOUR modes (0/3/4/5; 1/2 render grey, 6/7 are rejected) so a
    // watcher can A/B them side by side over time. The on-screen indicator
    // draws (mode+1) black squares so the watcher can read which mode is
    // showing: 1 square = mode 0, 4 = mode 3, 5 = mode 4, 6 = mode 5.
    // Runs forever; stop the systemd unit to end it (xochitl auto-restores).
    let sweep: [(i32, i32); 4] = [(0, 0), (3, 0), (4, 0), (5, 0)];
    eprintln!("color-test: looping colour modes {:?} at {hold}s each; stop the unit to end",
        sweep.iter().map(|(m, _)| *m).collect::<Vec<_>>());
    let mut round = 0u64;
    loop {
        round += 1;
        for (mode, full) in sweep {
            // Repaint the indicator strip: white bg + (mode+1) black squares.
            surf.fill_rect(0, 0, w, STRIP, 0xFFFF);
            for k in 0..=(mode as usize) {
                surf.fill_rect(30 + k * 130, 40, 90, 90, 0x0000);
            }
            let t0 = std::time::Instant::now();
            let ret = display.swap_raw(0, 0, w as i32, h as i32, mode, full);
            let ms = t0.elapsed().as_millis();
            eprintln!("round {round} mode={mode} ({} squares): swap {ret}, {ms}ms; hold {hold}s",
                mode + 1);
            std::thread::sleep(std::time::Duration::from_secs(hold));
        }
    }
}

/// Demo Plan 2 Task 3 diagnostic: synthesize a multi-band colour PNG, wrap it
/// in a synthetic `/turn` response as an `image` card, then run it through
/// the REAL parse -> plan_card (-> plan_image) -> paste_rect -> swap_raw(mode
/// 5) path onto the takeover display. Unlike `--color-test` (which paints
/// raw bands directly), this exercises the actual image-card rendering
/// primitive end to end. A human watches the panel; red/green/blue/yellow
/// bands should be distinguishable after the mode-5 swap.
fn image_test(_args: &[String]) -> i32 {
    use base64::Engine;
    let (w, h) = (600u32, 400u32);
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let bands = [[255u8, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let c = bands[(row as usize * bands.len() / h as usize).min(bands.len() - 1)];
            for _ in 0..w {
                data.extend_from_slice(&c);
            }
        }
        writer.write_image_data(&data).unwrap();
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let json = format!(
        r#"{{"turn_id":"t","spoken_text":"","paper_cards":[{{"type":"image","data":"{b64}","place":"blank_area","size":"l"}}],"page_action":"none","memory_tags":[]}}"#
    );
    let resp = match cards::parse_turn_response(&json) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("image-test: {e}");
            return 1;
        }
    };
    let (disp, mut surf) = match display::Display::open() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("image-test: display open failed: {e} (needs takeover + xochitl stopped)");
            return 1;
        }
    };
    surf.fill_rect(0, 0, surf.w, surf.h, WHITE);
    let mut map = layout::InkMap::from_surface(&surf);
    let primary = FontRef::try_from_slice(FONT_TTF).expect("font");
    let font = script::FontStack::new(primary, None);
    for card in &resp.paper_cards {
        for p in cardrender::plan_card(card, &mut map, &font, (surf.w as i32 / 2, surf.h as i32 / 2)) {
            if let Some(blit) = &p.blit {
                let (x, y, bw, bh) = blit.rect;
                surf.paste_rect(x as usize, y as usize, bw as usize, bh as usize, &blit.bgra);
                eprintln!(">>> WATCH — image blit {bw}x{bh} at ({x},{y}), swapping at mode 5");
                disp.swap_raw(x, y, bw, bh, 5, 0);
            }
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(20));
    eprintln!("image-test: done (xochitl auto-restored on exit)");
    0
}

/// Colour-calibration swatch grid (see dispatch comment). 6 rows (red, orange,
/// yellow, green, blue, pink), 5 candidate RGBs per row, painted full 8-bit via
/// paste_rect + swap_raw(mode 5). Logs every cell's rgb so a photo of the panel
/// can be mapped back to source values.
fn swatch_test(_args: &[String]) -> i32 {
    let palette: [[(u8, u8, u8); 5]; 6] = [
        [(200, 30, 30), (230, 45, 40), (255, 60, 50), (255, 95, 65), (240, 120, 95)], // red
        [(240, 110, 20), (255, 140, 25), (255, 160, 45), (255, 180, 70), (250, 150, 60)], // orange
        [(230, 190, 20), (245, 210, 20), (255, 225, 20), (255, 235, 80), (250, 220, 120)], // yellow
        [(30, 140, 60), (45, 175, 75), (55, 205, 90), (95, 215, 120), (135, 225, 150)], // green
        [(30, 60, 180), (30, 95, 220), (40, 125, 240), (65, 145, 255), (95, 165, 255)], // blue
        [(220, 55, 115), (240, 80, 150), (255, 100, 175), (255, 130, 195), (230, 45, 105)], // pink
    ];
    let names = ["red", "orange", "yellow", "green", "blue", "pink"];
    let (rows, cols) = (6usize, 5usize);
    let (sw, sh, gap) = (170usize, 90usize, 18usize);
    let gw = cols * sw + (cols + 1) * gap;
    let gh = rows * sh + (rows + 1) * gap;
    let mut bgra = vec![255u8; gw * gh * 4]; // white canvas
    for r in 0..rows {
        for c in 0..cols {
            let (cr, cg, cb) = palette[r][c];
            eprintln!("swatch r{r}c{c} {} = rgb({cr},{cg},{cb})", names[r]);
            let (x0, y0) = (gap + c * (sw + gap), gap + r * (sh + gap));
            for y in y0..y0 + sh {
                for x in x0..x0 + sw {
                    let i = (y * gw + x) * 4;
                    bgra[i] = cb;
                    bgra[i + 1] = cg;
                    bgra[i + 2] = cr;
                    bgra[i + 3] = 0xFF;
                }
            }
        }
    }
    let (disp, mut surf) = match display::Display::open() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("swatch-test: display open failed: {e} (needs takeover + xochitl stopped)");
            return 1;
        }
    };
    surf.fill_rect(0, 0, surf.w, surf.h, WHITE);
    let gx = surf.w.saturating_sub(gw) / 2;
    let gy = 300usize;
    surf.paste_rect(gx, gy, gw, gh, &bgra);
    eprintln!(">>> WATCH — {rows}x{cols} swatch grid at ({gx},{gy}) {gw}x{gh}, mode 5");
    disp.swap_raw(gx as i32, gy as i32, gw as i32, gh as i32, 5, 0);
    std::thread::sleep(std::time::Duration::from_secs(45));
    eprintln!("swatch-test: done");
    0
}

fn oracle_test(png: &str) -> i32 {
    let store = memory::MemoryStore::open();
    let o = match oracle::Oracle::spawn(store.is_some()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oracle spawn failed: {e}");
            return 1;
        }
    };
    let ctx = build_ctx(&store);
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    o.ask(png, &ctx, tx);
    let mut got = String::new();
    loop {
        match rx.recv() {
            Ok(Ok(Event::Ink(chunk))) => {
                if got.is_empty() {
                    eprintln!("first chunk +{}ms", t0.elapsed().as_millis());
                }
                print!("{chunk} ");
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                got.push_str(&chunk);
            }
            Ok(Ok(Event::Show(id))) => {
                println!("[would conjure memory {id} — {}]", memory::spoken_date(id));
                got.push_str("(show)");
            }
            Ok(Ok(Event::Transcript(t))) => eprintln!("\n[transcript] {t}"),
            Ok(Err(e)) => {
                eprintln!("\noracle error: {e}");
                return 1;
            }
            Err(_) => break, // disconnected = reply complete
        }
    }
    println!("\n--- reply complete ({}ms, {} chars) ---", t0.elapsed().as_millis(), got.len());
    if got.trim().is_empty() { 1 } else { 0 }
}

/// Diagnostic: parse a `/turn` fixture (fake child ink + a `response` body)
/// and render every paper card it contains onto a blank in-memory page,
/// off-screen. No device, no network: lets a human eyeball card placement
/// (and a developer catch drop reasons on stderr) before cards are wired
/// into `run()`'s real turn loop. Exit codes: 2 = usage/read/parse error,
/// 1 = the PNG failed to write or nothing rendered, 0 = at least one plan.
fn cards_test(args: &[String]) -> i32 {
    let Some(path) = args.get(2) else {
        eprintln!("usage: riddle --cards-test fixture.json [out.png]");
        return 2;
    };
    let out = args.get(3).map(String::as_str).unwrap_or("/tmp/riddle-cards.png");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return 2;
        }
    };
    let fixture: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bad json: {e}");
            return 2;
        }
    };
    let resp = match cards::parse_turn_response(&fixture["response"].to_string()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bad response: {e}");
            return 2;
        }
    };
    // The font is compiled into the binary, not fixture-supplied — a
    // failure here means a broken build, not bad input, so `expect` (rather
    // than degrading) matches how `run()` treats the same failure.
    let primary = FontRef::try_from_slice(FONT_TTF).expect("font");
    let font = script::FontStack::new(primary, None);

    let mut buf = vec![0u8; SCREEN_W * SCREEN_H * 4];
    let mut surf =
        Surface::new(buf.as_mut_ptr(), buf.len(), SCREEN_W, SCREEN_H, SCREEN_W * 4, surface::PixFmt::Rgb32);
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);

    // Fake child ink (a polyline per stroke, brush radius 3) — optional; an
    // absent or empty "ink" array just leaves the page blank and the anchor
    // at page center. Every access below degrades to `None`/skip rather
    // than panicking on a malformed point, since this walks fixture content
    // straight out of an untrusted JSON file.
    let mut anchor = (SCREEN_W as i32 / 2, SCREEN_H as i32 / 2);
    if let Some(strokes) = fixture["ink"].as_array() {
        for s in strokes {
            let pts: Vec<(i32, i32)> = s
                .as_array()
                .map(|v| v.iter().filter_map(|p| Some((p[0].as_i64()? as i32, p[1].as_i64()? as i32))).collect())
                .unwrap_or_default();
            for w in pts.windows(2) {
                surf.brush_line(w[0].0, w[0].1, w[1].0, w[1].1, 3, BLACK);
            }
            if let Some(&(x, y)) = pts.last() {
                anchor = (x + 60, y + 60);
            }
        }
    }

    let mut map = layout::InkMap::from_surface(&surf);
    let mut rendered = 0;
    for card in &resp.paper_cards {
        let plans = cardrender::plan_card(card, &mut map, &font, anchor);
        if plans.is_empty() {
            eprintln!("card dropped: {card:?}");
            continue;
        }
        for p in plans {
            for stroke in &p.strokes {
                for w in stroke.windows(2) {
                    surf.brush_line(w[0].0, w[0].1, w[1].0, w[1].1, 2, p.color);
                }
                if let Some(&(x, y)) = stroke.first() {
                    surf.stamp(x, y, 2, p.color);
                }
            }
            if let Some(blit) = &p.blit {
                let (x, y, w, h) = blit.rect;
                surf.paste_rect(x as usize, y as usize, w as usize, h as usize, &blit.bgra);
                eprintln!("image blit at ({x},{y}) {w}x{h}");
            }
            // plan_card already marked this plan's region on `map` (via its
            // internal commit()); no need to mark_rect again here.
            let (x, y, w, h) = p.region.rect();
            eprintln!("card at ({x},{y}) {w}x{h} color={:#06x}", p.color);
            rendered += 1;
        }
    }

    if let Err(e) = write_page_png(&surf, out) {
        eprintln!("png: {e}");
        return 1;
    }
    eprintln!("wrote {out} ({rendered} plans, coverage {:.2})", map.coverage());
    (rendered == 0) as i32
}

/// Full-page grayscale pixels (2x downscale -> 810x1080, plenty for
/// eyeballing or for a model to read): `(bytes, width, height)`. Shared by
/// `write_page_png` (a file, for `--cards-test`/`--oracle-test`) and
/// `page_png_bytes` (in-memory, for the /turn client) — one box filter, two
/// destinations.
fn page_gray(surf: &Surface) -> (Vec<u8>, usize, usize) {
    let (w, h) = (surf.w / 2, surf.h / 2);
    let mut gray = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    acc += surf.luma((x * 2 + dx) as i32, (y * 2 + dy) as i32) as u32;
                }
            }
            gray[y * w + x] = (acc / 4) as u8;
        }
    }
    (gray, w, h)
}

/// Full-page grayscale PNG (2x downscale -> 810x1080, plenty for eyeballing).
fn write_page_png(surf: &Surface, path: &str) -> std::io::Result<()> {
    let (gray, w, h) = page_gray(surf);
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(&gray).map_err(std::io::Error::other)
}

/// Same image as `write_page_png`, encoded straight into memory: what the
/// /turn client base64-encodes into `page_png` (Task 12) — the whole page,
/// unlike the legacy oracle's ink-bbox crop (`Ink::to_png`), since a card
/// turn's server needs to see the full sheet (existing cards, margins, all
/// of it) to place new ones without overlapping.
fn page_png_bytes(surf: &Surface) -> std::io::Result<Vec<u8>> {
    let (gray, w, h) = page_gray(surf);
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(std::io::Error::other)?;
        writer.write_image_data(&gray).map_err(std::io::Error::other)?;
    }
    Ok(buf)
}

/// Wipe the page and start counting ink from zero: the page-turn action
/// taken once a finished reply/card-turn's ink coverage was already past
/// `page_full_threshold()` at commit time (Task 11) — shared by the legacy
/// Replying path and the card-turn DrawingCards path (Task 12) so the two
/// can never drift apart. `page_id` resets too: it names the new sheet for
/// the next `/turn` request.
fn turn_the_page(
    surf: &mut Surface,
    disp: &display::Display,
    user_ink: &mut ink::Ink,
    committed_strokes: &mut usize,
    page_id: &mut u64,
) {
    eprintln!("riddle: page turned (coverage past threshold)");
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.full_refresh(surf.w, surf.h);
    user_ink.clear();
    *committed_strokes = 0;
    *page_id = unix_secs();
}

/// Gate for `turn_the_page`, called from both completion sites (legacy
/// Replying and CardTurn/DrawingCards) once `page_full` says a fresh sheet is
/// due. A child may keep drawing during Thinking/CardTurn/DrawingCards
/// (co-drawing is intended, spec's premise for those states not locking the
/// page) — those new strokes land past `committed_strokes`. Turning the page
/// right now would run `user_ink.clear()` and silently discard that
/// un-answered ink (spec §24: "孩子的作品神圣"). So: only turn the page when
/// there is no new ink since the last commit; otherwise defer — page_full
/// gets recomputed at the next commit, so the turn simply happens after the
/// next response once the child pauses. Never drops ink, just postpones the
/// sheet change.
fn maybe_turn_the_page(
    surf: &mut Surface,
    disp: &display::Display,
    user_ink: &mut ink::Ink,
    committed_strokes: &mut usize,
    page_id: &mut u64,
) {
    if user_ink.stroke_list().len() == *committed_strokes {
        turn_the_page(surf, disp, user_ink, committed_strokes, page_id);
    } else {
        eprintln!("riddle: page turn deferred (child still drawing)");
    }
}

/// What the diary sends alongside the page: its memory of recent turns and
/// the catalog the oracle picks conjured pages from. Empty when memory is off.
fn build_ctx(store: &Option<memory::MemoryStore>) -> oracle::TurnContext {
    let Some(s) = store else { return oracle::TurnContext::default() };
    let turns: usize = std::env::var("RIDDLE_MEMORY_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let (catalog_lines, catalog_ids) = s.catalog(40);
    oracle::TurnContext { history: s.recent_dialogue(turns), catalog_lines, catalog_ids }
}

fn run() -> std::io::Result<()> {
    let primary = FontRef::try_from_slice(FONT_TTF).map_err(std::io::Error::other)?;
    eprintln!("riddle: handwriting font = PingFangShiGuang");
    let font = script::FontStack::new(primary, None);

    let (disp, mut surf) = display::Display::open()?;
    let takeover = matches!(disp, display::Display::Quill);
    eprintln!(
        "riddle: display {} ({}x{} stride {})",
        if takeover { "quill/takeover" } else { "qtfb" },
        surf.w,
        surf.h,
        surf.stride
    );

    let mut pen_dev = match pen::PenDevice::open() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("riddle: raw pen unavailable ({e}), falling back to qtfb pen events");
            None
        }
    };
    // Takeover mode: touch is ours too; 5-finger tap = quit.
    let mut touch_dev = if takeover { touch::TouchDevice::open().ok() } else { None };
    // Takeover mode: the power button is ours too (sleep page + suspend).
    let mut power_dev = if takeover {
        power::PowerButton::open().map_err(|e| eprintln!("riddle: no power button ({e})")).ok()
    } else {
        None
    };
    // Ignore power presses briefly after a wake: the waking press itself (and
    // key bounce) arrives on our grabbed fd and must not re-suspend.
    let mut power_grace = Instant::now();

    let sigterm = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&sigterm))?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&sigterm))?;

    // Blank page.
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.update_all(surf.w, surf.h);

    // The diary's memory (None = RIDDLE_MEMORY=off or the dir is unusable).
    let mut store = memory::MemoryStore::open();
    if let Some(ref s) = store {
        eprintln!("riddle: memory holds {} pages", s.entries.len());
    }

    // Warm the oracle now: pi loads Node + extensions + codex auth ONCE here,
    // while you're still picking up the pen, so replies pay only model latency.
    let oracle = match oracle::Oracle::spawn(store.is_some()) {
        Ok(o) => {
            eprintln!("riddle: oracle ready");
            Some(o)
        }
        Err(e) => {
            eprintln!("riddle: oracle spawn failed: {e}");
            None
        }
    };

    let mut user_ink = ink::Ink::new();
    let mut state = State::Listening { last_pen: None };
    let mut pen_down = false;
    // The turn being remembered: strokes captured at commit, transcript and
    // reply accumulated as they stream, stored when the turn completes.
    let mut turn_id: u64 = 0;
    let mut turn_strokes: memory::Strokes = Vec::new();
    let mut turn_reply = String::new();
    let mut turn_transcript: Option<String> = None;
    let mut turn_failed = false;
    // Names the current sheet of paper for the /turn client: a fresh
    // unix-seconds id at app start, and again every time the page turns
    // (`turn_the_page`, shared by Task 11's legacy page-turn and
    // DrawingCards' own). Kept up to date regardless of which oracle path is
    // active, but only ever read when turn mode is on — the legacy path
    // never builds a request that needs it.
    let mut page_id: u64 = unix_secs();
    // Which teaching profile the server should adapt cards to (spec §5) —
    // read once; nothing on the tablet changes it mid-session yet.
    let profile = std::env::var("RIDDLE_PROFILE").unwrap_or_else(|_| "child_3_4".into());
    // Snapshot of `user_ink.stroke_list().len()` as of the last commit.
    // Strokes at or past this index are "new": drawn since the diary last
    // asked the oracle. Co-drawing keeps every committed stroke on the page
    // (no more clearing after each turn), so "is there enough ink to answer"
    // and "what should the oracle see as this turn's mark" must both count
    // only what's new, not the whole page's history.
    let mut committed_strokes: usize = 0;
    // Raw stylus contact, tracked in every state (the guide dismisses on it).
    // `stylus_on` is the level; `stylus_tapped` latches any contact seen this
    // loop iteration, so a tap that starts AND ends within one drain still
    // registers.
    let mut stylus_on = false;
    let mut stylus_tapped = false;
    let mut ink_dirty = BBox::empty();
    let mut last_flush = Instant::now();
    // Takeover swaps are cheap and synchronous; qtfb needs coalescing.
    let flush_every = if takeover { Duration::from_millis(8) } else { Duration::from_millis(35) };

    eprintln!("riddle: the diary is open");

    loop {
        if sigterm.load(Ordering::Relaxed) {
            break;
        }
        if let Some(ref mut t) = touch_dev {
            if t.drain_check_quit() {
                eprintln!("riddle: 5-finger quit");
                break;
            }
        }

        // ---- power button: sleep page, suspend, restore on wake ----
        if let Some(ref mut p) = power_dev {
            let pressed = p.drain_pressed();
            if pressed && Instant::now() >= power_grace {
                eprintln!("riddle: sleeping (power button)");
                let saved = help::show_sleep(&mut surf, &font);
                disp.full_refresh(surf.w, surf.h);
                // Let the flashing refresh finish before the panel loses power.
                std::thread::sleep(Duration::from_millis(800));
                // Suspend, and confirm via the kernel's success counter. The
                // EPD regulator refuses to sleep while its post-update vpdd
                // timer (≤30s) runs — the whole suspend aborts with "Some
                // devices failed to suspend" — so retry until it sticks.
                let count0 = power::suspend_count();
                let mut attempts = 0;
                'sleeping: loop {
                    if p.grabbed {
                        let _ = std::process::Command::new("systemctl").arg("suspend").status();
                    }
                    attempts += 1;
                    let t0 = Instant::now();
                    while t0.elapsed() < Duration::from_secs(6) {
                        std::thread::sleep(Duration::from_millis(400));
                        if power::suspend_count() > count0 {
                            break 'sleeping;
                        }
                    }
                    if attempts >= 8 {
                        eprintln!("riddle: suspend never happened ({attempts} tries); waking the page");
                        break;
                    }
                    eprintln!("riddle: suspend aborted (EPD discharge timer), retrying");
                }
                eprintln!("riddle: waking");
                help::restore_sleep(&mut surf, &saved);
                disp.full_refresh(surf.w, surf.h);
                power::wifi_heal();
                // Discard input that queued while asleep — stale pen events
                // would otherwise replay as phantom ink on the restored page.
                if let Some(ref mut pd) = pen_dev {
                    let _ = pd.drain();
                }
                if let Some(ref mut td) = touch_dev {
                    let _ = td.drain_check_quit();
                }
                p.drain_pressed();
                power_grace = Instant::now() + Duration::from_secs(3);
            }
        }

        // ---- raw pen (preferred path) ----
        if let Some(ref mut pdev) = pen_dev {
            for s in pdev.drain() {
                let writing = s.touching && s.pressure > 40;
                stylus_on = writing;
                stylus_tapped |= writing;
                if !writing {
                    if pen_down {
                        pen_down = false;
                        user_ink.pen_up();
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                    continue;
                }
                match state {
                    State::Listening { ref mut last_pen } => {
                        pen_down = true;
                        let d = match s.tool {
                            pen::Tool::Pen => {
                                let r = 2 + s.pressure * 3 / pen::MAX_PRESSURE;
                                user_ink.pen_point(&mut surf, s.x, s.y, r)
                            }
                            pen::Tool::Eraser => user_ink.erase_point(&mut surf, s.x, s.y, 22),
                        };
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    }
                    // Thinking no longer freezes the page: the child can keep
                    // drawing right beside their own ink while Tom composes.
                    // Same inking as Listening, but there is no idle timer
                    // here to update — none of these three states has a
                    // `last_pen`. CardTurn (waiting on /turn) and
                    // DrawingCards (the cards animating in) get the same
                    // treatment: composing/animating never locks the page.
                    State::Thinking { .. } | State::CardTurn { .. } | State::DrawingCards { .. } => {
                        pen_down = true;
                        let d = match s.tool {
                            pen::Tool::Pen => {
                                let r = 2 + s.pressure * 3 / pen::MAX_PRESSURE;
                                user_ink.pen_point(&mut surf, s.x, s.y, r)
                            }
                            pen::Tool::Eraser => user_ink.erase_point(&mut surf, s.x, s.y, 22),
                        };
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                    }
                    _ => {}
                }
            }
        }

        // ---- window-system events (qtfb close detection + pen fallback) ----
        let events = match disp.pump() {
            Ok(v) => v,
            Err(_) => break, // qtfb window closed
        };
        for ev in events {
            if pen_dev.is_some() {
                continue;
            }
            match ev.input_type {
                qtfb::INPUT_PEN_PRESS | qtfb::INPUT_PEN_UPDATE => {
                    stylus_on = true;
                    stylus_tapped = true;
                    if let State::Listening { ref mut last_pen } = state {
                        pen_down = true;
                        let r = 2 + ev.d.clamp(0, 100) / 45;
                        let d = user_ink.pen_point(&mut surf, ev.x, ev.y, r);
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    } else if matches!(state, State::Thinking { .. } | State::CardTurn { .. } | State::DrawingCards { .. }) {
                        // Same as Listening's inking: no idle timer to touch.
                        pen_down = true;
                        let r = 2 + ev.d.clamp(0, 100) / 45;
                        let d = user_ink.pen_point(&mut surf, ev.x, ev.y, r);
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                    }
                }
                qtfb::INPUT_PEN_RELEASE => {
                    stylus_on = false;
                    if pen_down {
                        pen_down = false;
                        user_ink.pen_up();
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                }
                _ => {}
            }
        }

        // ---- coalesced ink flush ----
        if !ink_dirty.is_empty() && last_flush.elapsed() >= flush_every {
            let (x, y, w, h) = ink_dirty.rect();
            disp.update(x, y, w, h, true);
            ink_dirty = BBox::empty();
            last_flush = Instant::now();
        }

        // ---- state machine ----
        state = match state {
            State::Listening { last_pen } => match last_pen {
                Some(t)
                    if !pen_down
                        && t.elapsed() >= idle_commit()
                        && user_ink.stroke_list().len().saturating_sub(committed_strokes) >= 1 =>
                {
                    // A single new stroke commits — a child who draws one shape
                    // (one continuous stroke) should be answered. The longer
                    // idle_commit() window (6s default) is what guards against a
                    // stray dot triggering; the region_all_white check below
                    // still drops ink that was erased during the pause. This
                    // arm only runs with >=1 new stroke, so the page holds ink.
                    debug_assert!(!user_ink.is_empty());
                    // Scope every commit-time check to the ink drawn since the
                    // last commit, not the whole page's history: older,
                    // already-answered strokes stay on the page untouched.
                    let new_bbox = bbox_of(&user_ink.stroke_list()[committed_strokes..]);
                    if region_all_white(&surf, new_bbox) {
                        // The new ink was erased before the pause: nothing to
                        // commit (and no phantom "?" from erased strokes).
                        // Mark it seen so it stops counting as "new ink"
                        // without touching any older, already-committed ink.
                        committed_strokes = user_ink.stroke_list().len();
                        State::Listening { last_pen: None }
                    } else if help::looks_like_question_mark(&user_ink.stroke_list()[committed_strokes..]) {
                        // Absorb the "?" and open the guide instead of asking.
                        // Legacy page-clearing behavior: this gesture still
                        // wipes the whole page until set_profile (phase 2)
                        // disables it for the child profile.
                        let (qx, qy, qw, qh) = user_ink.bbox.rect();
                        surf.fill_rect(qx as usize, qy as usize, qw as usize, qh as usize, WHITE);
                        disp.update(qx, qy, qw, qh, false);
                        user_ink.clear();
                        committed_strokes = 0;
                        let panel = help::show(&mut surf, &font, takeover);
                        let (px, py, pw, ph) = panel.region.rect();
                        disp.update(px, py, pw, ph, false);
                        eprintln!("riddle: guide shown");
                        State::Help { panel: Some(panel), until: Instant::now() + Duration::from_secs(45) }
                    } else if !turn::turn_mode_enabled() && oracle.is_none() {
                        // No spirit at all: don't eat the ink that nothing
                        // will answer, but don't write an excuse on the page
                        // either (spec §12: a child must never see error
                        // text) — stderr only. Mark this ink committed so a
                        // later stray pen touch can't re-trigger on this SAME
                        // ink alone; nothing is actually lost, since the next
                        // real commit still sends the whole page (`to_png`
                        // rasterizes the full ink bbox, not just what's
                        // "new") — the child drawing more just carries this
                        // ink along into that turn.
                        // (Gated on turn mode too: /turn is a self-contained
                        // path with its own server/mock, independent of the
                        // legacy chat-completions oracle — a card-turn demo
                        // shouldn't require a working legacy oracle as well.)
                        eprintln!("riddle: no oracle configured, staying quiet");
                        committed_strokes = user_ink.stroke_list().len();
                        State::Listening { last_pen: None }
                    } else {
                        // Shared by both paths below: what the page looks
                        // like right now, whether it's already past the
                        // page-full threshold, and where a reply/card should
                        // aim to land — all decided while `surf` still shows
                        // exactly what the child just drew (Task 10).
                        let map = layout::InkMap::from_surface(&surf);
                        // Same map, no second scan: past the threshold means
                        // this reply is the page's last before a fresh sheet.
                        let threshold = page_full_threshold();
                        let page_full = threshold > 0.0 && map.coverage() > threshold;
                        let anchor = (new_bbox.x1 + 60, new_bbox.y0);

                        // Remember this turn's new strokes for memory — the
                        // page itself is no longer cleared: co-drawing keeps
                        // every committed mark in view beside Tom's reply.
                        turn_id = unix_secs();
                        turn_strokes = user_ink.stroke_list()[committed_strokes..].to_vec();
                        committed_strokes = user_ink.stroke_list().len();
                        turn_reply.clear();
                        turn_transcript = None;
                        turn_failed = false;

                        if turn::turn_mode_enabled() {
                            // ---- structured /turn path (Task 12) ----
                            let page_bytes = match page_png_bytes(&surf) {
                                Ok(b) => b,
                                Err(e) => {
                                    eprintln!("riddle: turn page encode failed: {e}");
                                    Vec::new()
                                }
                            };
                            let page_png_b64 = oracle::base64(&page_bytes);
                            let turn_id_str = turn_id.to_string();
                            let page_id_str = page_id.to_string();
                            let meta = turn::TurnRequestMeta {
                                turn_id: &turn_id_str,
                                trigger: "pen_idle",
                                page_png_b64: &page_png_b64,
                                new_strokes: &turn_strokes,
                                ink_coverage: map.coverage(),
                                page_id: &page_id_str,
                                profile: &profile,
                            };
                            let body = turn::build_request_json(&meta);
                            let (tx, rx) = mpsc::channel();
                            turn::fetch(body, tx);
                            // The corner thinking dot, once — simpler v1: no
                            // pulsing (unlike Thinking's blot_on toggle),
                            // since a single structured call is normally
                            // much quicker than a model's free-text reply.
                            surf.stamp(THINK_X, THINK_Y, 9, BLACK);
                            disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                            State::CardTurn { rx, page_full, anchor, since: Instant::now() }
                        } else {
                            // ---- legacy chat-completions oracle path ----
                            if let Err(e) = user_ink.to_png(&surf, PNG_PATH) {
                                eprintln!("riddle: rasterize failed: {e}");
                            }
                            // `None` only when nothing this size fits anywhere
                            // — the reply then falls back to the legacy
                            // centered placement.
                            let reply_origin = layout::resolve(
                                &map,
                                &cards::Place::NearNewInk,
                                layout::Anchor::Point(anchor.0, anchor.1),
                                700,
                                320,
                            )
                            .or_else(|| {
                                layout::resolve(&map, &cards::Place::BlankArea, layout::Anchor::None, 700, 320)
                            });
                            if reply_origin.is_none() {
                                eprintln!("riddle: page full, reply placed center");
                            }
                            // Ask NOW: the model streams while the diary
                            // thinks, hiding most of the reply latency in the
                            // animation.
                            let (tx, rx) = mpsc::channel();
                            if let Some(ref o) = oracle {
                                o.ask(PNG_PATH, &build_ctx(&store), tx);
                            }
                            // Both backends read the page before ask() returns;
                            // the writer's words don't need to sit on disk
                            // afterwards.
                            if std::env::var_os("RIDDLE_KEEP_PAGE").is_none() {
                                let _ = std::fs::remove_file(PNG_PATH);
                            }
                            // The child's ink stays on the page: no dissolve,
                            // no Drinking state — straight to Thinking.
                            State::Thinking {
                                rx,
                                pulse: Instant::now(),
                                blot_on: false,
                                since: Instant::now(),
                                origin: reply_origin,
                                page_full,
                            }
                        }
                    }
                }
                _ => State::Listening { last_pen },
            },

            State::Thinking { rx, pulse, blot_on, since, origin, page_full } => match rx.try_recv() {
                Ok(result) => {
                    surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                    disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                    // First streamed event: start writing now; keep the
                    // receiver so the rest of the reply can append itself.
                    match result {
                        Ok(Event::Show(id)) => {
                            // An incantation: the rest of this turn is the
                            // conjured memory, not a reply. (rx drops here.)
                            match conjure(&font, &store, id, &mut surf, &disp) {
                                Some(st) => st,
                                None => {
                                    // Quiet failure (spec §12): no excuse on
                                    // the page, stderr only. committed_strokes
                                    // was already advanced at commit time, so
                                    // this can't re-trigger on its own — the
                                    // child drawing more starts the next turn.
                                    eprintln!("riddle: memory {id} is missing");
                                    State::Listening { last_pen: None }
                                }
                            }
                        }
                        Ok(Event::Ink(text)) => {
                            turn_reply.push_str(&text);
                            let plan = plan_reply(&font, &text, None, origin);
                            State::Replying { plan, next: Instant::now(), rx: Some(rx), page_full }
                        }
                        Ok(Event::Transcript(t)) => {
                            // Transcript with no prose (model skipped the
                            // reply): remember the words, keep waiting.
                            turn_transcript = Some(t);
                            State::Thinking { rx, pulse, blot_on, since, origin, page_full }
                        }
                        Err(e) => {
                            // Quiet failure (spec §12): stderr only, nothing
                            // written to the page. committed_strokes is
                            // already advanced from this turn's commit, so
                            // silence holds until the child draws again.
                            eprintln!("riddle: oracle failed: {e}");
                            State::Listening { last_pen: None }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if since.elapsed() >= ORACLE_PATIENCE {
                        // The oracle never answered (stalled stream, dead pi):
                        // stop pulsing and go quiet rather than thinking
                        // forever or writing an excuse on the page.
                        eprintln!("riddle: oracle timed out after {}s", ORACLE_PATIENCE.as_secs());
                        surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                        disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                        State::Listening { last_pen: None }
                    } else if pulse.elapsed() >= Duration::from_millis(600) {
                        if blot_on {
                            surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                        } else {
                            surf.stamp(THINK_X, THINK_Y, 9, BLACK);
                        }
                        disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                        State::Thinking { rx, pulse: Instant::now(), blot_on: !blot_on, since, origin, page_full }
                    } else {
                        State::Thinking { rx, pulse, blot_on, since, origin, page_full }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Fast-follow from the Task 10 review: this arm used to
                    // return to Listening without erasing the corner dot,
                    // leaving a stray blot when the channel drops without a
                    // final Ok/Err (mirrors the timeout arm's cleanup above).
                    surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                    disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                    State::Listening { last_pen: None }
                }
            },

            State::Replying { mut plan, next, mut rx, page_full } => {
                // More of the reply may still be streaming in: append each
                // new chunk below what is already planned, mid-animation.
                if let Some(ref r) = rx {
                    let drop_rx = match r.try_recv() {
                        Ok(Ok(Event::Ink(more))) => {
                            if plan.next_y > SCREEN_H as i32 - 200 {
                                // The page is full: let the rest go unwritten
                                // rather than inking below the visible page.
                                eprintln!("riddle: reply reached the page bottom; trailing text dropped");
                                true
                            } else {
                                turn_reply.push_str(" ");
                                turn_reply.push_str(&more);
                                append_reply(&font, &mut plan, &more);
                                false
                            }
                        }
                        Ok(Ok(Event::Transcript(t))) => {
                            turn_transcript = Some(t);
                            false // the disconnect is still coming
                        }
                        Ok(Ok(Event::Show(_))) => {
                            eprintln!("riddle: conjuring directive mid-reply ignored");
                            false
                        }
                        Ok(Err(e)) => {
                            eprintln!("riddle: oracle failed mid-reply: {e}");
                            turn_failed = true;
                            true
                        }
                        Err(mpsc::TryRecvError::Disconnected) => true,
                        Err(mpsc::TryRecvError::Empty) => false,
                    };
                    if drop_rx {
                        rx = None;
                    }
                }
                if Instant::now() >= next {
                    let mut dirty = BBox::empty();
                    let mut budget = 26;
                    while budget > 0 && plan.stroke_i < plan.strokes.len() {
                        let stroke = &plan.strokes[plan.stroke_i];
                        if plan.point_i >= stroke.len() {
                            plan.stroke_i += 1;
                            plan.point_i = 0;
                            continue;
                        }
                        let (x, y) = stroke[plan.point_i];
                        if plan.point_i > 0 {
                            let (px, py) = stroke[plan.point_i - 1];
                            surf.brush_line(px, py, x, y, 2, BLACK);
                        } else {
                            surf.stamp(x, y, 2, BLACK);
                        }
                        dirty.add(x, y, 4);
                        plan.point_i += 1;
                        budget -= 1;
                    }
                    if !dirty.is_empty() {
                        let (x, y, w, h) = dirty.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if plan.stroke_i >= plan.strokes.len() && rx.is_none() {
                        // The turn is complete: the diary remembers it.
                        if !turn_failed && !turn_reply.is_empty() {
                            if let Some(ref mut s) = store {
                                s.append(
                                    turn_id,
                                    turn_transcript.as_deref().unwrap_or(""),
                                    turn_reply.trim(),
                                    &turn_strokes,
                                );
                            }
                            // `page_full` was decided at commit time (Task
                            // 11), before the oracle was even asked — so only
                            // a reply that actually finished turns the page.
                            // Nested inside the same `!turn_failed` gate as
                            // the memory write above: a Thinking-side failure
                            // (oracle error, ORACLE_PATIENCE timeout, or a
                            // channel disconnect) never even reaches this
                            // branch, and a mid-reply failure reaches it but
                            // sets `turn_failed` — either way an aborted
                            // turn's ink is left untouched, same as memory.
                            if page_full {
                                maybe_turn_the_page(
                                    &mut surf,
                                    &disp,
                                    &mut user_ink,
                                    &mut committed_strokes,
                                    &mut page_id,
                                );
                            }
                        }
                        turn_strokes = Vec::new();
                        // The reply stays on the page for good now — no more
                        // Lingering/FadingReply. DouDou's strokes are drawn
                        // straight into `surf`, never into `user_ink`, so they
                        // never move `committed_strokes`; the idle-commit gate
                        // already requires >=2 *new* child strokes past that
                        // snapshot, so silence holds on its own until the
                        // child draws again.
                        State::Listening { last_pen: None }
                    } else {
                        State::Replying { plan, next: Instant::now() + Duration::from_millis(14), rx, page_full }
                    }
                } else {
                    State::Replying { plan, next, rx, page_full }
                }
            }

            State::CardTurn { rx, page_full, anchor, since } => match rx.try_recv() {
                Ok(Ok(resp)) => {
                    // Erase the corner dot on every exit path out of
                    // CardTurn, same discipline as Thinking's cleanup.
                    surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                    disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);

                    let mut map = layout::InkMap::from_surface(&surf);
                    let mut plans = Vec::new();
                    for card in &resp.paper_cards {
                        plans.extend(cardrender::plan_card(card, &mut map, &font, anchor));
                    }
                    // Memory: a card turn has no vision-transcript postscript
                    // (the ⁂ protocol belongs to the legacy chat-completions
                    // path only, spoken by a model that reads the persona's
                    // MEMORY_PROTOCOL — /turn's server-side prompt is out of
                    // scope here), so transcript is "". `spoken_text` is kept
                    // as the reply gist for catalog/recall — the cards
                    // themselves aren't re-derivable from a Card later, but
                    // the words said about them are enough to find the turn
                    // again. Gated on a non-empty reply, matching the legacy
                    // Replying path (`!turn_reply.is_empty()`): a blank
                    // spoken_text would otherwise leave an empty "(reply: )"
                    // catalog gist. Strokes-only turns simply aren't
                    // cataloged, same as legacy.
                    if !resp.spoken_text.is_empty() {
                        if let Some(ref mut s) = store {
                            s.append(turn_id, "", &resp.spoken_text, &turn_strokes);
                        }
                    }
                    turn_strokes = Vec::new();
                    if plans.is_empty() {
                        eprintln!("riddle: card turn produced nothing to draw");
                        State::Listening { last_pen: None }
                    } else {
                        let page_full = page_full || matches!(resp.page_action, cards::PageAction::NewPage);
                        State::DrawingCards { plans, plan_i: 0, point_i: 0, next: Instant::now(), page_full }
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("riddle: turn failed: {e}");
                    surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                    disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                    State::Listening { last_pen: None }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if since.elapsed() >= TURN_PATIENCE {
                        eprintln!("riddle: turn timed out after {}s", TURN_PATIENCE.as_secs());
                        surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                        disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                        State::Listening { last_pen: None }
                    } else {
                        State::CardTurn { rx, page_full, anchor, since }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                    disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                    State::Listening { last_pen: None }
                }
            },

            State::DrawingCards { mut plans, mut plan_i, mut point_i, next, page_full } => {
                if Instant::now() >= next {
                    let mut dirty = BBox::empty();
                    if plan_i < plans.len() {
                        // 位图卡：一次性贴 BGRA + mode 5 刷，不做逐帧动画 —
                        // 有 blit 的 plan 从不带 strokes（见 plan_image），所以
                        // 必须在进入下面的逐点动画之前单独分支处理，否则会被
                        // strokes.is_empty() 直接跳过、图从不上屏。
                        if let Some(blit) = plans[plan_i].blit.clone() {
                            let (x, y, w, h) = blit.rect;
                            if surf.fmt == surface::PixFmt::Rgb32 {
                                surf.paste_rect(x as usize, y as usize, w as usize, h as usize, &blit.bgra);
                                disp.swap_raw(x, y, w, h, 5, 0);
                            } else {
                                // qtfb 是 Rgb565：直接贴 BGRA 会错色，跳过（Demo 走 takeover）。
                                eprintln!(
                                    "riddle: DrawingCards: qtfb (Rgb565) surface can't take a BGRA image blit, skipping"
                                );
                            }
                            // 复用该 handler 现有的"一张 plan 画完→推进"路径：
                            // 与下面 strokes.is_empty() 分支完全同构，勿另造状态机。
                            plan_i += 1;
                            point_i = 0;
                        } else {
                            // Budget and color both come from the CURRENT plan:
                            // a slow sketch and a normal-paced stamp don't share
                            // a pace, and grid/template sub-plans of the same
                            // trace card don't share a color (FADED vs BLACK).
                            let mut budget = plans[plan_i].points_per_frame;
                            let color = plans[plan_i].color;
                            // Mirrors Replying's stroke-walking loop, but each
                            // plan's own inner stroke list is drained from the
                            // front as it finishes (`strokes.remove(0)`) instead
                            // of tracking a separate stroke index — plan_i/point_i
                            // are the only position fields DrawingCards carries.
                            while budget > 0 && !plans[plan_i].strokes.is_empty() {
                                let stroke_len = plans[plan_i].strokes[0].len();
                                if point_i >= stroke_len {
                                    plans[plan_i].strokes.remove(0);
                                    point_i = 0;
                                    continue;
                                }
                                let (x, y) = plans[plan_i].strokes[0][point_i];
                                if point_i > 0 {
                                    let (px, py) = plans[plan_i].strokes[0][point_i - 1];
                                    surf.brush_line(px, py, x, y, 2, color);
                                } else {
                                    surf.stamp(x, y, 2, color);
                                }
                                dirty.add(x, y, 4);
                                point_i += 1;
                                budget -= 1;
                            }
                            if plans[plan_i].strokes.is_empty() {
                                plan_i += 1;
                                point_i = 0;
                            }
                        }
                    }
                    if !dirty.is_empty() {
                        let (x, y, w, h) = dirty.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if plan_i >= plans.len() {
                        // Every card is drawn: same page-turn action as a
                        // finished reply (Task 11), gated the same way.
                        if page_full {
                            maybe_turn_the_page(
                                &mut surf,
                                &disp,
                                &mut user_ink,
                                &mut committed_strokes,
                                &mut page_id,
                            );
                        }
                        State::Listening { last_pen: None }
                    } else {
                        State::DrawingCards {
                            plans,
                            plan_i,
                            point_i,
                            next: Instant::now() + Duration::from_millis(14),
                            page_full,
                        }
                    }
                } else {
                    State::DrawingCards { plans, plan_i, point_i, next, page_full }
                }
            }

            State::Help { panel, until } => match panel {
                Some(p) => {
                    if stylus_tapped || Instant::now() >= until {
                        let region = p.dismiss(&mut surf);
                        let (x, y, w, h) = region.rect();
                        disp.update(x, y, w, h, false);
                        eprintln!("riddle: guide dismissed");
                        State::Help { panel: None, until }
                    } else {
                        State::Help { panel: Some(p), until }
                    }
                }
                // Dismissed: swallow the closing touch, listen again on pen-up.
                None if stylus_on => State::Help { panel: None, until },
                None => State::Listening { last_pen: None },
            },

            State::Conjuring { mut plan, next, saved } => {
                if stylus_tapped {
                    // The writer interrupts: today's page returns at once.
                    surf.paste_rect(0, 0, SCREEN_W, SCREEN_H, &saved);
                    disp.full_refresh(surf.w, surf.h);
                    State::MemoryShown { saved: None, until: Instant::now(), region: plan.region }
                } else if Instant::now() >= next {
                    // The memory pours back faster than Tom writes: it is
                    // remembered, not composed.
                    let mut dirty = BBox::empty();
                    let mut budget = 48;
                    while budget > 0 && plan.stroke_i < plan.strokes.len() {
                        let stroke = &plan.strokes[plan.stroke_i];
                        if plan.point_i >= stroke.len() {
                            plan.stroke_i += 1;
                            plan.point_i = 0;
                            continue;
                        }
                        let (x, y, r) = stroke[plan.point_i];
                        if plan.point_i > 0 {
                            let (px, py, pr) = stroke[plan.point_i - 1];
                            surf.brush_line(px, py, x, y, r.min(pr + 1), FADED);
                        } else {
                            surf.stamp(x, y, r, FADED);
                        }
                        dirty.add(x, y, r + 2);
                        plan.point_i += 1;
                        budget -= 1;
                    }
                    if !dirty.is_empty() {
                        let (x, y, w, h) = dirty.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if plan.stroke_i >= plan.strokes.len() {
                        let region = plan.region;
                        State::MemoryShown {
                            saved: Some(saved),
                            until: Instant::now() + Duration::from_secs(120),
                            region,
                        }
                    } else {
                        State::Conjuring { plan, next: Instant::now() + Duration::from_millis(10), saved }
                    }
                } else {
                    State::Conjuring { plan, next, saved }
                }
            }

            State::MemoryShown { saved, until, region } => match saved {
                Some(s) => {
                    if stylus_tapped || Instant::now() >= until {
                        // The paper swallows its memory; today's page returns.
                        surf.paste_rect(0, 0, SCREEN_W, SCREEN_H, &s);
                        disp.full_refresh(surf.w, surf.h);
                        eprintln!("riddle: memory dismissed");
                        State::MemoryShown { saved: None, until, region }
                    } else {
                        State::MemoryShown { saved: Some(s), until, region }
                    }
                }
                // Dismissed: swallow the closing touch, listen again on pen-up.
                None if stylus_on => State::MemoryShown { saved: None, until, region },
                None => State::Listening { last_pen: None },
            },
        };

        stylus_tapped = false;
        std::thread::sleep(Duration::from_millis(2));
    }

    eprintln!("riddle: the diary closes");
    disp.terminate();
    Ok(())
}

/// Bounding box of a set of finished strokes (4-tuples: x, y, radius,
/// ms-since-page-start). Used to scope commit-time checks — "was this fully
/// erased", "does this look like a ?" — to only the ink drawn since the last
/// commit, not the whole page's co-drawn history.
fn bbox_of(strokes: &[Vec<(i32, i32, i32, u32)>]) -> BBox {
    let mut bbox = BBox::empty();
    for stroke in strokes {
        for &(x, y, r, _) in stroke {
            bbox.add(x, y, r + 2);
        }
    }
    bbox
}

/// True if the region no longer holds any dark pixels (fully erased).
fn region_all_white(surf: &Surface, region: BBox) -> bool {
    if region.is_empty() {
        return true;
    }
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            if surf.luma(x, y) < 200 {
                return false;
            }
        }
    }
    true
}

/// Summon a remembered page: snapshot today's page, clear the paper, and plan
/// the memory's rewriting — the date in a small hand, the writer's own strokes
/// exactly as they were penned, Tom's old reply beneath — all in faded ink.
fn conjure(
    font: &script::FontStack<'_>,
    store: &Option<memory::MemoryStore>,
    id: u64,
    surf: &mut Surface,
    disp: &display::Display,
) -> Option<State> {
    let s = store.as_ref()?;
    let entry = s.get(id)?.clone();
    let strokes = s.strokes(id).unwrap_or_default();
    eprintln!("riddle: conjuring memory {id} ({})", memory::spoken_date(id));

    let saved = surf.copy_rect(0, 0, SCREEN_W, SCREEN_H);
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.update_all(surf.w, surf.h);

    let mut all: Vec<Vec<(i32, i32, i32)>> = Vec::new();
    let mut region = BBox::empty();

    // The date, small and centered near the top, like a diary heading.
    let date = memory::spoken_date(entry.id);
    let mut raster = script::rasterize_line(font, &date, 54.0);
    script::thin(&mut raster);
    let x0 = (SCREEN_W as i32 - raster.width as i32) / 2;
    let mut ink_bottom = 64;
    for stroke in script::trace(&raster) {
        let mapped: Vec<(i32, i32, i32)> =
            stroke.iter().map(|&(sx, sy)| (x0 + sx, 64 + sy, 1)).collect();
        for &(x, y, r) in &mapped {
            region.add(x, y, r + 2);
            ink_bottom = ink_bottom.max(y);
        }
        all.push(mapped);
    }

    // The writer's own hand, exactly as it was penned (replay ignores t).
    for stroke in &strokes {
        let mapped: Vec<(i32, i32, i32)> = stroke.iter().map(|&(x, y, r, _)| (x, y, r)).collect();
        for &(x, y, r) in &mapped {
            region.add(x, y, r + 2);
            ink_bottom = ink_bottom.max(y);
        }
        all.push(mapped);
    }

    // Tom's old reply, below.
    if !entry.reply.is_empty() {
        let y = (ink_bottom + 130).min(SCREEN_H as i32 - 400);
        let reply = plan_reply(font, &entry.reply, Some(y), None);
        for stroke in reply.strokes {
            let mapped: Vec<(i32, i32, i32)> = stroke.iter().map(|&(x, y)| (x, y, 2)).collect();
            for &(x, y, r) in &mapped {
                region.add(x, y, r + 2);
            }
            all.push(mapped);
        }
    }

    Some(State::Conjuring {
        plan: ConjurePlan { strokes: all, stroke_i: 0, point_i: 0, region },
        next: Instant::now(),
        saved,
    })
}

/// Lay out reply text and produce screen-space strokes.
///
/// `y_start` continues a streamed reply below its previous chunk; `None`
/// places the first chunk. `origin`: when `Some((ox, oy))`, the first line's
/// top-left anchors at `(ox, oy)` and every line left-aligns at `ox` instead
/// of centering on the page, with the wrap width capped so the text still
/// fits before the page's right margin. `None` keeps the legacy look —
/// lines centered on the page, y from `y_start` or the default upper-third
/// rule. When both `y_start` and `origin` are given (a streamed continuation
/// of an origin-placed reply), `y_start` wins for the y coordinate;
/// `origin.0` still supplies the left-align x.
fn plan_reply(
    font: &script::FontStack<'_>,
    text: &str,
    y_start: Option<i32>,
    origin: Option<(i32, i32)>,
) -> WritePlan {
    let reply_px = REPLY_PX;
    let max_w = match origin {
        Some((ox, _)) => (SCREEN_W as i32 - ox - 60).min(1380).max(1) as f32,
        None => (SCREEN_W as i32 - 2 * MARGIN_X) as f32,
    };
    let lines = script::wrap(font, text, reply_px, max_w);

    let mut prepared = Vec::new();
    let mut line_h = (reply_px * 1.45) as i32;
    for line_text in &lines {
        let mut raster = script::rasterize_line(font, line_text, reply_px);
        script::thin(&mut raster);
        let line_strokes = script::trace(&raster);
        line_h = line_h.max(raster.height as i32 + 10);
        prepared.push((raster.width, line_strokes));
    }

    let total_h = line_h * prepared.len() as i32;
    let default_y = match origin {
        Some((_, oy)) => oy,
        None => ((SCREEN_H as i32 - total_h) / 3).max(60),
    };
    let mut y = y_start.unwrap_or(default_y);
    let mut strokes = Vec::new();
    let mut region = BBox::empty();
    let mut seed = 0x1234u32;
    let mut jitter = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 16) % 7) as i32 - 3
    };

    for (line_width, line_strokes) in prepared {
        let x0 = match origin {
            Some((ox, _)) => ox,
            None => (SCREEN_W as i32 - line_width as i32) / 2,
        };
        let wobble = jitter();
        for s in line_strokes {
            let mapped: Vec<(i32, i32)> = s
                .iter()
                .map(|&(sx, sy)| (x0 + sx, y + sy + wobble))
                .collect();
            for &(x, yy) in &mapped {
                region.add(x, yy, 5);
            }
            strokes.push(mapped);
        }
        y += line_h;
    }

    WritePlan { strokes, stroke_i: 0, point_i: 0, region, next_y: y, origin_x: origin.map(|(ox, _)| ox) }
}

/// Splice a streamed continuation chunk into a running write animation.
fn append_reply(font: &script::FontStack<'_>, plan: &mut WritePlan, more: &str) {
    // Same left-align x as the first chunk (if any), so a streamed
    // continuation keeps lining up instead of snapping back to center.
    let origin = plan.origin_x.map(|ox| (ox, plan.next_y));
    let cont = plan_reply(font, more, Some(plan.next_y), origin);
    if cont.strokes.is_empty() {
        return;
    }
    plan.region.add(cont.region.x0, cont.region.y0, 0);
    plan.region.add(cont.region.x1, cont.region.y1, 0);
    plan.strokes.extend(cont.strokes);
    plan.next_y = cont.next_y;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_commit_defaults_to_6s_and_parses_overrides() {
        assert_eq!(idle_commit_from(None), Duration::from_millis(6000));
        assert_eq!(idle_commit_from(Some("4.5")), Duration::from_millis(4500));
        assert_eq!(idle_commit_from(Some("abc")), Duration::from_millis(6000));
        assert_eq!(idle_commit_from(Some("0")), Duration::from_millis(6000)); // 0 无意义，回退
    }

    #[test]
    fn page_full_threshold_defaults_to_0_55_and_parses_overrides() {
        assert_eq!(page_full_threshold_from(None), 0.55);
        assert_eq!(page_full_threshold_from(Some("0.7")), 0.7);
        assert_eq!(page_full_threshold_from(Some("abc")), 0.55);
        assert_eq!(page_full_threshold_from(Some("1.5")), 0.55); // 超出 0..=1，回退
        assert_eq!(page_full_threshold_from(Some("0")), 0.0); // 0 是合法值：关闭自动换页
    }

    // Demo Plan 2 Task 3: prove an `image` card fixture is legal end to
    // end — not just cards::parse_turn_response (already covered in
    // cards.rs), but through cardrender::plan_card too, since Task 3's job
    // is wiring the resulting blit into the real render paths.
    #[test]
    fn image_card_fixture_parses_and_plans_a_blit() {
        use base64::Engine;
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, 4, 4);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            let data: Vec<u8> = (0..(4 * 4)).flat_map(|_| [255u8, 0, 0, 255]).collect();
            writer.write_image_data(&data).unwrap();
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        let json = format!(
            r#"{{"turn_id":"t","spoken_text":"","paper_cards":[{{"type":"image","data":"{b64}","place":"blank_area","size":"l"}}],"page_action":"none","memory_tags":[]}}"#
        );
        let resp = cards::parse_turn_response(&json).expect("parses");
        assert_eq!(resp.paper_cards.len(), 1);
        assert!(matches!(&resp.paper_cards[0], cards::Card::Image { .. }));

        let mut buf = vec![0u8; SCREEN_W * SCREEN_H * 4];
        let mut surf =
            Surface::new(buf.as_mut_ptr(), buf.len(), SCREEN_W, SCREEN_H, SCREEN_W * 4, surface::PixFmt::Rgb32);
        surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
        let mut map = layout::InkMap::from_surface(&surf);
        let font = script::FontStack::new(FontRef::try_from_slice(FONT_TTF).expect("font"), None);
        let plans = cardrender::plan_card(
            &resp.paper_cards[0],
            &mut map,
            &font,
            (SCREEN_W as i32 / 2, SCREEN_H as i32 / 2),
        );
        assert_eq!(plans.len(), 1, "one blit plan");
        let blit = plans[0].blit.as_ref().expect("blit present");
        let (_, _, w, h) = blit.rect;
        assert_eq!(blit.bgra.len(), (w * h * 4) as usize);
    }
}
