//! /turn structured response: paper_cards data model, parsing, and the
//! tablet-enforced clamps from the interaction spec §14.3. The server is
//! trusted for content, never for shape — anything malformed degrades to
//! "fewer cards", not to a crash.

use serde::Deserialize;

pub const STAMP_NAMES: [&str; 8] =
    ["star", "flower", "heart", "smiley", "check", "sun", "moon", "balloon"];
pub const MAX_CARDS: usize = 3;
pub const MAX_SKETCH_POINTS: usize = 2000;
pub const DEFAULT_MAX_TEXT_CHARS: usize = 6;
/// Stamps render as one plan per instance (Task 7); the interaction spec
/// draws reward stamps as a short row that wraps after 3, so 5 is a
/// generous ceiling that keeps an unclamped server value from exploding the
/// render queue.
pub const MAX_STAMP_COUNT: u32 = 5;

#[derive(Debug, Clone, Copy)]
pub enum Place {
    NearNewInk,
    NearAnchor,
    BlankArea,
    Margin,
    FullPage,
}

#[derive(Debug, Clone, Copy)]
pub enum Size {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Copy)]
pub enum Pace {
    Normal,
    Slow,
}

#[derive(Debug, Clone, Copy)]
pub enum PageAction {
    None,
    SuggestNewPage,
    NewPage,
}

#[derive(Debug, Clone, Copy)]
pub enum CountStyle {
    Dots,
    Tally,
    Numbers,
}

#[derive(Debug, Clone, Copy)]
pub enum TraceKind {
    Shape,
    Hanzi,
}

#[derive(Debug, Clone, Copy)]
pub enum TraceGuide {
    None,
    TianGrid,
}

#[derive(Debug, Clone, Copy)]
pub struct CardCommon {
    pub place: Place,
    pub anchor_norm: Option<(f32, f32)>,
    pub size: Size,
    pub pace: Pace,
}

#[derive(Debug, Clone)]
pub enum Card {
    Text { common: CardCommon, content: String },
    Sketch { common: CardCommon, strokes: Vec<Vec<(f32, f32)>> },
    Stamp { common: CardCommon, name: String, count: u32 },
    Count { common: CardCommon, n: u32, style: CountStyle },
    Trace { common: CardCommon, kind: TraceKind, content: String, guide: TraceGuide },
    /// `layout`: (card, rect_norm x,y,w,h) — the one place the server picks
    /// explicit coordinates (spec §14.2: "唯一允许服务器排坐标的地方").
    Page { layout: Vec<(Card, (f32, f32, f32, f32))> },
}

#[derive(Debug, Clone)]
pub struct TurnResponse {
    /// Echoed back by the server for its own correlation/logging. The
    /// tablet never reads this back out: main.rs already has its own
    /// canonical turn id (the commit-time unix-seconds `turn_id`, generated
    /// tablet-side and sent in the *request*) and has no need to compare it
    /// against what the response repeats. Read by a parser test (cards.rs's
    /// own `parses_full_response_and_truncates_to_three_cards`), so only a
    /// non-test build would otherwise warn — scoped allow, not a module-wide
    /// one, now that the rest of this module is live on the run path.
    #[allow(dead_code)]
    pub turn_id: String,
    pub spoken_text: String,
    pub paper_cards: Vec<Card>,
    pub page_action: PageAction,
    /// Parsed and kept for a future phone/server-side catalog (spec §14.2);
    /// nothing on the tablet reads a turn's own memory tags back out today
    /// (the diary's on-device catalog, memory.rs, is keyed by transcript/
    /// reply gist instead) — narrow, scoped allow rather than a module-wide
    /// one now that the rest of this module is live on the run path.
    #[allow(dead_code)]
    pub memory_tags: Vec<String>,
}

// ---- loose parsing layer: every field optional / Value-typed, so a server
// that sends extra or missing fields never fails the whole turn. ----

#[derive(Deserialize)]
struct RawResponse {
    turn_id: Option<String>,
    spoken_text: Option<String>,
    paper_cards: Option<Vec<serde_json::Value>>,
    page_action: Option<String>,
    memory_tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawCard {
    #[serde(rename = "type")]
    card_type: Option<String>,
    // common
    place: Option<String>,
    anchor_norm: Option<(f32, f32)>,
    size: Option<String>,
    pace: Option<String>,
    // text / trace
    content: Option<String>,
    // sketch
    strokes: Option<Vec<Vec<(f32, f32)>>>,
    // stamp
    name: Option<String>,
    count: Option<u32>,
    // count
    n: Option<u32>,
    style: Option<String>,
    // trace
    kind: Option<String>,
    guide: Option<String>,
    // page
    layout: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
struct RawLayoutItem {
    card: serde_json::Value,
    rect_norm: Option<(f32, f32, f32, f32)>,
}

/// Parse a `/turn` response body. Top-level JSON that doesn't parse, or that
/// is missing `turn_id`/`paper_cards`, is an error. Everything below that
/// degrades: bad cards are dropped, not fatal.
pub fn parse_turn_response(json: &str) -> Result<TurnResponse, String> {
    parse_turn_response_with(json, DEFAULT_MAX_TEXT_CHARS)
}

/// Same as [`parse_turn_response`] but with an explicit per-card text-length
/// cap. `parse_turn_response` always passes `DEFAULT_MAX_TEXT_CHARS`; a wider
/// cap is for a future literacy profile (spec §14.3.3: "识字档放宽（服务器经
/// set_profile 告知）") that isn't wired up yet.
fn parse_turn_response_with(json: &str, max_text_chars: usize) -> Result<TurnResponse, String> {
    let raw: RawResponse =
        serde_json::from_str(json).map_err(|e| format!("cards: invalid /turn response: {e}"))?;
    let turn_id = raw.turn_id.ok_or("cards: /turn response missing turn_id")?;
    let paper_cards_raw = raw.paper_cards.ok_or("cards: /turn response missing paper_cards")?;

    let mut cards: Vec<Card> =
        paper_cards_raw.iter().filter_map(|v| convert_card(v, max_text_chars, false)).collect();

    // §14.3 #1: a `page` card must stand alone. If one is present, keep only
    // the first and drop everything else this turn (including other pages).
    if cards.iter().any(|c| matches!(c, Card::Page { .. })) {
        let dropped = cards.len() - 1;
        if dropped > 0 {
            eprintln!("riddle: cards: page card present; dropping {dropped} other card(s) this turn");
        }
        cards = cards.into_iter().find(|c| matches!(c, Card::Page { .. })).into_iter().collect();
    }

    if cards.len() > MAX_CARDS {
        eprintln!("riddle: cards: {} cards exceeds max {MAX_CARDS}, truncating", cards.len());
        cards.truncate(MAX_CARDS);
    }

    Ok(TurnResponse {
        turn_id,
        spoken_text: raw.spoken_text.unwrap_or_default(),
        paper_cards: cards,
        page_action: parse_page_action(raw.page_action.as_deref()),
        memory_tags: raw.memory_tags.unwrap_or_default(),
    })
}

/// Convert one loosely-typed JSON value into a strict [`Card`], applying the
/// per-card §14.3 clamps. `None` means the card was dropped (malformed shape,
/// unknown stamp name, or an oversize sketch) — always with an `eprintln`
/// explaining why, never a panic.
///
/// `nested`: true when this card came from a `page.layout[]` entry rather
/// than the top-level `paper_cards[]`. A nested card whose own type is
/// `page` is rejected rather than recursed into — the spec's "a page card
/// must stand alone" rule means nested pages are meaningless anyway, and
/// refusing them structurally caps recursion depth at 1 regardless of how
/// deeply a malformed payload claims to nest (never a stack overflow).
fn convert_card(value: &serde_json::Value, max_text_chars: usize, nested: bool) -> Option<Card> {
    let raw: RawCard = match serde_json::from_value(value.clone()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("riddle: cards: dropping malformed card: {e}");
            return None;
        }
    };
    let card_type = raw.card_type.as_deref().unwrap_or_default().to_lowercase();
    match card_type.as_str() {
        "text" => {
            let common = raw_common(&raw);
            let Some(content) = raw.content else {
                eprintln!("riddle: cards: dropping text card with no content");
                return None;
            };
            Some(Card::Text { common, content: truncate_chars(&content, max_text_chars) })
        }
        "sketch" => {
            let common = raw_common(&raw);
            let Some(strokes) = raw.strokes else {
                eprintln!("riddle: cards: dropping sketch card with no strokes");
                return None;
            };
            let total_points: usize = strokes.iter().map(Vec::len).sum();
            if total_points > MAX_SKETCH_POINTS {
                eprintln!(
                    "riddle: cards: dropping sketch card with {total_points} points (max {MAX_SKETCH_POINTS})"
                );
                return None;
            }
            Some(Card::Sketch { common, strokes })
        }
        "stamp" => {
            let common = raw_common(&raw);
            let count = raw.count.unwrap_or(1).clamp(1, MAX_STAMP_COUNT);
            let Some(name) = raw.name else {
                eprintln!("riddle: cards: dropping stamp card with no name");
                return None;
            };
            if !STAMP_NAMES.contains(&name.as_str()) {
                eprintln!("riddle: cards: dropping stamp card with unknown name {name:?}");
                return None;
            }
            Some(Card::Stamp { common, name, count })
        }
        "count" => {
            let Some(n) = raw.n else {
                eprintln!("riddle: cards: dropping count card with no n");
                return None;
            };
            Some(Card::Count {
                common: raw_common(&raw),
                n: n.clamp(1, 20),
                style: parse_count_style(raw.style.as_deref()),
            })
        }
        "trace" => {
            let common = raw_common(&raw);
            let kind = parse_trace_kind(raw.kind.as_deref());
            let guide = parse_trace_guide(raw.guide.as_deref());
            let Some(content) = raw.content else {
                eprintln!("riddle: cards: dropping trace card with no content");
                return None;
            };
            Some(Card::Trace { common, kind, content, guide })
        }
        "page" => {
            if nested {
                eprintln!("riddle: cards: nested page card dropped");
                return None;
            }
            let Some(items) = raw.layout else {
                eprintln!("riddle: cards: dropping page card with no layout");
                return None;
            };
            let layout =
                items.iter().filter_map(|item| convert_layout_item(item, max_text_chars)).collect();
            Some(Card::Page { layout })
        }
        "" => {
            eprintln!("riddle: cards: dropping card with no type");
            None
        }
        other => {
            eprintln!("riddle: cards: dropping card of unknown type {other:?}");
            None
        }
    }
}

/// One `page.layout[]` entry: a nested card plus the server-chosen
/// `rect_norm` — the one place the server picks explicit coordinates.
/// `None` drops just this entry (malformed shape, missing rect, or the
/// nested card itself failed to convert), never the whole page.
fn convert_layout_item(
    value: &serde_json::Value,
    max_text_chars: usize,
) -> Option<(Card, (f32, f32, f32, f32))> {
    let item: RawLayoutItem = match serde_json::from_value(value.clone()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("riddle: cards: dropping page layout item: {e}");
            return None;
        }
    };
    let Some((x, y, w, h)) = item.rect_norm else {
        eprintln!("riddle: cards: dropping page layout item with no rect_norm");
        return None;
    };
    // §14.2: rect_norm is the one place the server picks explicit
    // coordinates — clamp every component to a sane normalized fraction
    // before it ever reaches cardrender's pixel math (see `clamp01`).
    let rect = (clamp01(x), clamp01(y), clamp01(w), clamp01(h));
    let card = convert_card(&item.card, max_text_chars, true)?;
    Some((card, rect))
}

fn raw_common(raw: &RawCard) -> CardCommon {
    CardCommon {
        place: parse_place(raw.place.as_deref()),
        anchor_norm: raw.anchor_norm.map(|(x, y)| (clamp01(x), clamp01(y))),
        size: parse_size(raw.size.as_deref()),
        pace: parse_pace(raw.pace.as_deref()),
    }
}

/// Clamp a server-supplied normalized coordinate into 0.0..=1.0. Every
/// fraction handed to cardrender's pixel math (`rect_norm`, `anchor_norm`)
/// must be a sane fraction of the page — an unclamped value like
/// `2000000.0` overflows i32 in cardrender (debug panic) or explodes
/// `brush_line`'s step count (release CPU freeze).
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Truncate by Unicode scalar count, not bytes — a multi-byte-per-char
/// Chinese string must not be sliced mid-character.
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn parse_place(s: Option<&str>) -> Place {
    match s {
        None => Place::BlankArea,
        Some(v) => match v.to_lowercase().as_str() {
            "near_new_ink" => Place::NearNewInk,
            "near_anchor" => Place::NearAnchor,
            "blank_area" => Place::BlankArea,
            "margin" => Place::Margin,
            "full_page" => Place::FullPage,
            other => {
                eprintln!("riddle: cards: unknown place {other:?}, defaulting to blank_area");
                Place::BlankArea
            }
        },
    }
}

fn parse_size(s: Option<&str>) -> Size {
    match s {
        None => Size::M,
        Some(v) => match v.to_lowercase().as_str() {
            "s" => Size::S,
            "m" => Size::M,
            "l" => Size::L,
            other => {
                eprintln!("riddle: cards: unknown size {other:?}, defaulting to M");
                Size::M
            }
        },
    }
}

fn parse_pace(s: Option<&str>) -> Pace {
    match s {
        None => Pace::Normal,
        Some(v) => match v.to_lowercase().as_str() {
            "normal" => Pace::Normal,
            "slow" => Pace::Slow,
            other => {
                eprintln!("riddle: cards: unknown pace {other:?}, defaulting to normal");
                Pace::Normal
            }
        },
    }
}

fn parse_page_action(s: Option<&str>) -> PageAction {
    match s {
        None => PageAction::None,
        Some(v) => match v.to_lowercase().as_str() {
            "none" => PageAction::None,
            "suggest_new_page" => PageAction::SuggestNewPage,
            "new_page" => PageAction::NewPage,
            other => {
                eprintln!("riddle: cards: unknown page_action {other:?}, defaulting to none");
                PageAction::None
            }
        },
    }
}

fn parse_count_style(s: Option<&str>) -> CountStyle {
    match s {
        None => CountStyle::Dots,
        Some(v) => match v.to_lowercase().as_str() {
            "dots" => CountStyle::Dots,
            "tally" => CountStyle::Tally,
            "numbers" => CountStyle::Numbers,
            other => {
                eprintln!("riddle: cards: unknown count style {other:?}, defaulting to dots");
                CountStyle::Dots
            }
        },
    }
}

fn parse_trace_kind(s: Option<&str>) -> TraceKind {
    match s {
        None => TraceKind::Shape,
        Some(v) => match v.to_lowercase().as_str() {
            "shape" => TraceKind::Shape,
            "hanzi" => TraceKind::Hanzi,
            other => {
                eprintln!("riddle: cards: unknown trace kind {other:?}, defaulting to shape");
                TraceKind::Shape
            }
        },
    }
}

fn parse_trace_guide(s: Option<&str>) -> TraceGuide {
    match s {
        None => TraceGuide::None,
        Some(v) => match v.to_lowercase().as_str() {
            "none" => TraceGuide::None,
            "tian_grid" => TraceGuide::TianGrid,
            other => {
                eprintln!("riddle: cards: unknown trace guide {other:?}, defaulting to none");
                TraceGuide::None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
      "v": 1, "turn_id": "t-1", "spoken_text": "哇！",
      "paper_cards": [
        {"type":"stamp","name":"star","count":3,"place":"near_new_ink","size":"S"},
        {"type":"sketch","strokes":[[[0.1,0.2],[0.9,0.8]]],"place":"blank_area","size":"M","pace":"slow"},
        {"type":"text","content":"太阳","place":"near_anchor","anchor_norm":[0.42,0.31],"size":"L"},
        {"type":"count","n":3,"style":"dots","place":"margin","size":"S"}
      ],
      "page_action":"none","memory_tags":["sun"]
    }"#;

    #[test]
    fn parses_full_response_and_truncates_to_three_cards() {
        let r = parse_turn_response(FULL).unwrap();
        assert_eq!(r.turn_id, "t-1");
        assert_eq!(r.paper_cards.len(), 3, "4th card dropped by the ≤3 rule");
        match &r.paper_cards[0] {
            Card::Stamp { name, count, common } => {
                assert_eq!(name, "star");
                assert_eq!(*count, 3);
                assert!(matches!(common.place, Place::NearNewInk));
                assert!(matches!(common.pace, Pace::Normal)); // 缺省
            }
            other => panic!("expected stamp, got {other:?}"),
        }
    }

    #[test]
    fn defaults_missing_fields_and_rejects_unknown_stamp() {
        let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"stamp","name":"dragon","count":1},
            {"type":"text","content":"你好呀小朋友们真棒"}
        ],"page_action":"suggest_new_page","memory_tags":[]}"#;
        let r = parse_turn_response(json).unwrap();
        assert_eq!(r.paper_cards.len(), 1, "unknown stamp dropped");
        match &r.paper_cards[0] {
            Card::Text { content, common } => {
                assert_eq!(content, "你好呀小朋友"); // 默认档 6 字截断
                assert!(matches!(common.place, Place::BlankArea)); // place 缺省
                assert!(matches!(common.size, Size::M)); // size 缺省
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(r.page_action, PageAction::SuggestNewPage));
    }

    #[test]
    fn oversize_sketch_is_dropped() {
        let pts: String = (0..2100).map(|i| format!("[{}.0,0.5]", i % 2)).collect::<Vec<_>>().join(",");
        let json = format!(
            r#"{{"turn_id":"t","spoken_text":"","paper_cards":[{{"type":"sketch","strokes":[[{pts}]]}}],"page_action":"none","memory_tags":[]}}"#
        );
        let r = parse_turn_response(&json).unwrap();
        assert!(r.paper_cards.is_empty());
    }

    #[test]
    fn page_card_parses_nested_layout_and_stands_alone() {
        let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"page","layout":[{"card":{"type":"trace","kind":"hanzi","content":"山","guide":"tian_grid"},"rect_norm":[0.1,0.1,0.8,0.3]}]},
            {"type":"stamp","name":"star","count":1}
        ],"page_action":"new_page","memory_tags":[]}"#;
        let r = parse_turn_response(json).unwrap();
        assert_eq!(r.paper_cards.len(), 1, "page card must stand alone");
        match &r.paper_cards[0] {
            Card::Page { layout } => {
                assert_eq!(layout.len(), 1);
                let (card, rect) = &layout[0];
                assert!(matches!(card, Card::Trace { .. }));
                assert!((rect.2 - 0.8).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn page_rect_and_anchor_are_clamped() {
        // Two hostile layout items in one page: the first has a wildly
        // out-of-range rect_norm (would overflow i32 in cardrender / explode
        // brush_line's step count), the second has an out-of-range
        // anchor_norm on its nested card. Both must come out clamped to
        // 0.0..=1.0 rather than passing through raw.
        let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"page","layout":[
                {"card":{"type":"stamp","name":"star","count":1},"rect_norm":[2000000.0,-5.0,9.0,9.0]},
                {"card":{"type":"text","content":"x","anchor_norm":[50.0,-1.0]},"rect_norm":[0.1,0.1,0.2,0.2]}
            ]}
        ],"page_action":"none","memory_tags":[]}"#;
        let r = parse_turn_response(json).unwrap();
        match &r.paper_cards[0] {
            Card::Page { layout } => {
                assert_eq!(layout.len(), 2);

                let (_, rect) = &layout[0];
                assert!((0.0..=1.0).contains(&rect.0), "x not clamped: {rect:?}");
                assert!((0.0..=1.0).contains(&rect.1), "y not clamped: {rect:?}");
                assert!((0.0..=1.0).contains(&rect.2), "w not clamped: {rect:?}");
                assert!((0.0..=1.0).contains(&rect.3), "h not clamped: {rect:?}");

                let (card, _) = &layout[1];
                match card {
                    Card::Text { common, .. } => {
                        let (ax, ay) = common.anchor_norm.expect("anchor present");
                        assert!((0.0..=1.0).contains(&ax), "anchor x not clamped: {ax}");
                        assert!((0.0..=1.0).contains(&ay), "anchor y not clamped: {ay}");
                    }
                    other => panic!("expected text card, got {other:?}"),
                }
            }
            other => panic!("expected page, got {other:?}"),
        }
    }

    #[test]
    fn stamp_count_is_clamped_to_five() {
        let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"stamp","name":"star","count":99}],"page_action":"none","memory_tags":[]}"#;
        let r = parse_turn_response(json).unwrap();
        match &r.paper_cards[0] {
            Card::Stamp { count, .. } => assert_eq!(*count, 5),
            other => panic!("{other:?}"),
        }

        // 0 is meaningless for a reward stamp; clamp up to the floor of 1.
        let json_zero = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"stamp","name":"star","count":0}],"page_action":"none","memory_tags":[]}"#;
        let r0 = parse_turn_response(json_zero).unwrap();
        match &r0.paper_cards[0] {
            Card::Stamp { count, .. } => assert_eq!(*count, 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nested_page_cards_are_dropped_not_recursed() {
        let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
            {"type":"page","layout":[
                {"card":{"type":"page","layout":[]},"rect_norm":[0.1,0.1,0.5,0.2]},
                {"card":{"type":"stamp","name":"star","count":1},"rect_norm":[0.1,0.4,0.3,0.2]}
            ]}],"page_action":"none","memory_tags":[]}"#;
        let r = parse_turn_response(json).unwrap();
        match &r.paper_cards[0] {
            Card::Page { layout } => {
                assert_eq!(layout.len(), 1, "nested page dropped, stamp kept");
                assert!(matches!(layout[0].0, Card::Stamp { .. }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn garbage_json_is_an_error_not_a_panic() {
        assert!(parse_turn_response("not json").is_err());
        assert!(parse_turn_response(r#"{"turn_id":1}"#).is_err());
    }
}
