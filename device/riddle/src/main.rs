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
/// How long the diary waits on a silent oracle before giving up on the turn.
/// Generous: thinking models can lead with a long silence.
const ORACLE_PATIENCE: Duration = Duration::from_secs(120);
const REPLY_PX: f32 = 52.0;
const MARGIN_X: i32 = 120;
/// The thinking dot lives in the top-right corner, not page-center: with
/// co-drawing the center of the page is the child's canvas, not Tom's.
const THINK_X: i32 = SCREEN_W as i32 - 90;
const THINK_Y: i32 = 90;

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
    Thinking { rx: OracleRx, pulse: Instant, blot_on: bool, since: Instant },
    Replying { plan: WritePlan, next: Instant, rx: Option<OracleRx> },
    Lingering { until: Instant, region: BBox },
    FadingReply { stage: u32, next: Instant, region: BBox },
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

/// Full-page grayscale PNG (2x downscale -> 810x1080, plenty for eyeballing).
fn write_page_png(surf: &Surface, path: &str) -> std::io::Result<()> {
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
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(&gray).map_err(std::io::Error::other)
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
                    // here to update — Thinking has no `last_pen`.
                    State::Thinking { .. } => {
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
                    State::Lingering { region, .. } => {
                        state = State::FadingReply { stage: 0, next: Instant::now(), region };
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
                    } else if let State::Thinking { .. } = state {
                        // Same as Listening's inking: no idle timer to touch.
                        pen_down = true;
                        let r = 2 + ev.d.clamp(0, 100) / 45;
                        let d = user_ink.pen_point(&mut surf, ev.x, ev.y, r);
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                    } else if let State::Lingering { region, .. } = state {
                        state = State::FadingReply { stage: 0, next: Instant::now(), region };
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
                        && user_ink.stroke_list().len().saturating_sub(committed_strokes) >= 2 =>
                {
                    // `new_strokes >= 2` above already implies the page holds
                    // ink; assert that invariant explicitly, since
                    // `committed_strokes` is a raw index that only tracks
                    // growth and could drift if ink were ever erased away
                    // from under it.
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
                    } else if oracle.is_none() {
                        // No spirit at all: don't eat ink that nothing will
                        // answer — leave the writing and put the reason below.
                        // Placement must clear ALL preserved ink, not just
                        // this turn's: with co-drawing, older ink can extend
                        // past new_bbox, so this uses the full accumulated
                        // bbox (revisit once Task 10 redesigns placement).
                        let y = (user_ink.bbox.y1 + 90).min(SCREEN_H as i32 - 400);
                        let plan = plan_reply(&font, &oracle_excuse("no oracle"), Some(y));
                        State::Replying { plan, next: Instant::now(), rx: None }
                    } else {
                        if let Err(e) = user_ink.to_png(&surf, PNG_PATH) {
                            eprintln!("riddle: rasterize failed: {e}");
                        }
                        // Remember this turn's new strokes for memory — the
                        // page itself is no longer cleared: co-drawing keeps
                        // every committed mark in view beside Tom's reply.
                        turn_id = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        turn_strokes = user_ink.stroke_list()[committed_strokes..].to_vec();
                        committed_strokes = user_ink.stroke_list().len();
                        turn_reply.clear();
                        turn_transcript = None;
                        turn_failed = false;
                        // Ask NOW: the model streams while the diary thinks,
                        // hiding most of the reply latency in the animation.
                        let (tx, rx) = mpsc::channel();
                        if let Some(ref o) = oracle {
                            o.ask(PNG_PATH, &build_ctx(&store), tx);
                        }
                        // Both backends read the page before ask() returns; the
                        // writer's words don't need to sit on disk afterwards.
                        if std::env::var_os("RIDDLE_KEEP_PAGE").is_none() {
                            let _ = std::fs::remove_file(PNG_PATH);
                        }
                        // The child's ink stays on the page: no dissolve, no
                        // Drinking state — straight to Thinking.
                        State::Thinking { rx, pulse: Instant::now(), blot_on: false, since: Instant::now() }
                    }
                }
                _ => State::Listening { last_pen },
            },

            State::Thinking { rx, pulse, blot_on, since } => match rx.try_recv() {
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
                                    eprintln!("riddle: memory {id} is missing");
                                    let plan = plan_reply(&font, &oracle_excuse("lost page"), None);
                                    turn_failed = true;
                                    State::Replying { plan, next: Instant::now(), rx: None }
                                }
                            }
                        }
                        Ok(Event::Ink(text)) => {
                            turn_reply.push_str(&text);
                            let plan = plan_reply(&font, &text, None);
                            State::Replying { plan, next: Instant::now(), rx: Some(rx) }
                        }
                        Ok(Event::Transcript(t)) => {
                            // Transcript with no prose (model skipped the
                            // reply): remember the words, keep waiting.
                            turn_transcript = Some(t);
                            State::Thinking { rx, pulse, blot_on, since }
                        }
                        Err(e) => {
                            eprintln!("riddle: oracle failed: {e}");
                            turn_failed = true;
                            let plan = plan_reply(&font, &oracle_excuse(&e), None);
                            State::Replying { plan, next: Instant::now(), rx: None }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if since.elapsed() >= ORACLE_PATIENCE {
                        // The oracle never answered (stalled stream, dead pi):
                        // stop pulsing and say so instead of thinking forever.
                        eprintln!("riddle: oracle timed out after {}s", ORACLE_PATIENCE.as_secs());
                        surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                        disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                        let plan = plan_reply(&font, &oracle_excuse("timed out"), None);
                        State::Replying { plan, next: Instant::now(), rx: None }
                    } else if pulse.elapsed() >= Duration::from_millis(600) {
                        if blot_on {
                            surf.fill_rect((THINK_X - 14) as usize, (THINK_Y - 14) as usize, 28, 28, WHITE);
                        } else {
                            surf.stamp(THINK_X, THINK_Y, 9, BLACK);
                        }
                        disp.update(THINK_X - 14, THINK_Y - 14, 28, 28, true);
                        State::Thinking { rx, pulse: Instant::now(), blot_on: !blot_on, since }
                    } else {
                        State::Thinking { rx, pulse, blot_on, since }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => State::Listening { last_pen: None },
            },

            State::Replying { mut plan, next, mut rx } => {
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
                        }
                        turn_strokes = Vec::new();
                        let chars: usize = plan.strokes.iter().map(|s| s.len()).sum();
                        let linger = Duration::from_millis(4000 + (chars as u64) * 2);
                        let region = plan.region;
                        State::Lingering { until: Instant::now() + linger.min(Duration::from_secs(20)), region }
                    } else {
                        State::Replying { plan, next: Instant::now() + Duration::from_millis(14), rx }
                    }
                } else {
                    State::Replying { plan, next, rx }
                }
            }

            State::Lingering { until, region } => {
                if Instant::now() >= until {
                    State::FadingReply { stage: 0, next: Instant::now(), region }
                } else {
                    State::Lingering { until, region }
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

            State::FadingReply { stage, next, region } => {
                const STAGES: u32 = 10;
                if Instant::now() >= next {
                    ink::dissolve_pass(&mut surf, region, stage, STAGES);
                    let (x, y, w, h) = region.rect();
                    disp.update(x, y, w, h, true);
                    if stage + 1 >= STAGES {
                        disp.full_refresh(surf.w, surf.h);
                        State::Listening { last_pen: None }
                    } else {
                        State::FadingReply { stage: stage + 1, next: Instant::now() + Duration::from_millis(80), region }
                    }
                } else {
                    State::FadingReply { stage, next, region }
                }
            }
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

/// What Tom writes when the spirit cannot answer: short, in a diary's voice,
/// but specific enough to act on. The raw error still goes to stderr.
fn oracle_excuse(e: &str) -> String {
    if e.contains("no oracle") {
        "The diary lies dormant: it found no oracle. \
         Put an API key in oracle.env, then open me again."
            .into()
    } else if e.starts_with("http 401") || e.starts_with("http 403") {
        "The oracle refused the diary's key. Check RIDDLE_OPENAI_KEY in oracle.env.".into()
    } else if e.starts_with("http ") {
        let code = e.split(':').next().unwrap_or("an error");
        format!("The oracle rejected the diary's plea ({code}). Check the model and endpoint in oracle.env.")
    } else if e.contains("request failed") || e.contains("timed out") {
        "The diary cannot reach its oracle. Is the tablet connected to Wi-Fi?".into()
    } else if e.contains("empty reply") {
        "The spirit read your words but said nothing. Write again.".into()
    } else {
        "The ink blurred before it could answer. Write again.".into()
    }
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
        let reply = plan_reply(font, &entry.reply, Some(y));
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

/// Lay out reply text and produce screen-space strokes. `y_start` continues a
/// streamed reply below its previous chunk; None places the first chunk.
fn plan_reply(font: &script::FontStack<'_>, text: &str, y_start: Option<i32>) -> WritePlan {
    let reply_px = REPLY_PX;
    let max_w = (SCREEN_W as i32 - 2 * MARGIN_X) as f32;
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
    let mut y = y_start.unwrap_or(((SCREEN_H as i32 - total_h) / 3).max(60));
    let mut strokes = Vec::new();
    let mut region = BBox::empty();
    let mut seed = 0x1234u32;
    let mut jitter = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 16) % 7) as i32 - 3
    };

    for (line_width, line_strokes) in prepared {
        let x0 = (SCREEN_W as i32 - line_width as i32) / 2;
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

    WritePlan { strokes, stroke_i: 0, point_i: 0, region, next_y: y }
}

/// Splice a streamed continuation chunk into a running write animation.
fn append_reply(font: &script::FontStack<'_>, plan: &mut WritePlan, more: &str) {
    let cont = plan_reply(font, more, Some(plan.next_y));
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
}
