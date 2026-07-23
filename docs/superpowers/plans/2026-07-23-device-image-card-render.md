# 设备端 `image` 彩图卡渲染（mode 5） 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 reMarkable 平板 riddle（takeover 模式）能把 `/turn` 返回的 `image` 彩图卡真彩渲染到纸面——解码内联的 base64 PNG，按 layout 引擎排到不重叠的空位，用 quill 波形 **mode 5** 刷出彩色。

**Architecture:** Demo 计划 2（共 4），纯设备（Rust）。承接 spike 结论（`quill_swap` mode 5 彩色最佳）与 Ben 的交付决定（**服务器内联 base64 送图，设备不联网抓图**）。三处改动：`cards.rs` 新增 `Image` 卡的解析（base64→PNG 字节）；`cardrender.rs` 给 `RenderPlan` 加一个位图 blit 载荷，新增 `plan_image`（PNG 解码→BGRA→`layout::resolve` 排位）；`main.rs` 的 DrawingCards 循环加一条 image 分支（`paste_rect` 贴 BGRA + `swap_raw(...,5,0)` 一次刷，不做逐帧动画）。**设备不缩放**——服务器（计划 3）按 size 预缩放 PNG，设备按 PNG 原生像素尺寸原样贴、并据此向 layout 要位置。

**Tech Stack:** Rust（binary crate `riddle`）；已有依赖 `png 0.17`（解码器现成，当前仅用其编码器）；**新增 `base64`**。构建/测试全在 Docker `rust:1-trixie`（Apple Silicon 上是原生 aarch64 Linux 容器）。

**Spec:** `docs/superpowers/specs/2026-07-23-demo-lesson-color-codraw-design.md`（§4 `image` 卡、§5 S0a spike 定的 mode 5）。设备渲染集成图见本计划各任务的 file:line 锚点。

## 前置条件 / 现状锚点（实现者务必先读这些真实代码，本计划的行号来自一次探路，可能有小偏移，以实际为准）

- `device/riddle/src/surface.rs`：`PixFmt`（`Rgb565` qtfb / `Rgb32` takeover）；写入原语 `put_px(x,y,c:u16)`（仅收 RGB565，Rgb32 下经 `expand565` 写成 **BGRA**：`b[i]=B,b[i+1]=G,b[i+2]=R,b[i+3]=0xFF`，见 surface.rs:11 的格式注释与 :78-86）；**唯一的原生字节块写入是 `paste_rect(x,y,w,h,data:&[u8])`（surface.rs:170）**，逐行 `copy_from_slice`、按 `stride`/`bpp()`（`bpp()`=4 for Rgb32, surface.rs:148）寻址（`stride` 可能大于 `w*bpp`，不能假设紧凑帧缓冲）。
- `device/riddle/src/cardrender.rs`：入口 `plan_card(card,&mut layout::InkMap,&script::FontStack,new_ink_anchor:(i32,i32))->Vec<RenderPlan>`（cardrender.rs:933），内部按 `Card` 变体分派到 `plan_*`（:939-961）；`RenderPlan { strokes:Vec<Vec<(i32,i32)>>, color:u16, points_per_frame:i32, region:fb::BBox }`（cardrender.rs:20）；`plan_in_rect`（:877，供 `page.layout` 用）；`plan_page`(:916)/`pixel_rect`(:867)。各 `plan_*` 内部调 `layout::resolve(map, place, anchor, want_w, want_h)->Option<(i32,i32)>`（返回像素左上角）再 `map.mark_rect`。
- `device/riddle/src/layout.rs`：`resolve(...)->Option<(i32,i32)>`（layout.rs:233），底层 `find_spot`（无重叠搜索）。
- `device/riddle/src/main.rs`：动画态 `State::DrawingCards { plans, plan_i, point_i, ... }`（main.rs:147）；plans 在 :1173-1174 由 `cardrender::plan_card` 生成；DrawingCards 处理在 **:1226-1293**，逐点 `surf.brush_line(...,2,color)`（:1251）累积 `dirty` BBox，每帧 `disp.update(x,y,w,h,true)`（:1266）→ takeover 走 `quill_swap(...,mode 0,0)`（display.rs:80）。
- `device/riddle/src/display.rs`：`swap_raw(x,y,w,h,mode,full)->u64`（display.rs:126）是**唯一收显式 vendor mode 的方法**（现仅 `--color-test` 用），takeover 外/qtfb 下返回 0。
- `device/riddle/src/cards.rs`：`enum Card`（:76）现有 `Text/Sketch/Stamp/Count/Trace/Page`，**无 `Image`**；`RawCard`（:124-147）**无 `url`/`data`**；分派 `match card_type`（:219）无 `"image"` 臂；容错风格＝畸形卡 `eprintln` + 丢弃、绝不 panic；`MAX_CARDS=3`。
- `Cargo.toml` 依赖：`libc, signal-hook, png 0.17, ab_glyph 0.2, ureq 2.10, serde, serde_json`；feature `takeover` 门控 libquill 链接。

## Global Constraints

- **交付形态＝内联 base64**：`image` 卡形状 `{"type":"image","data":"<base64 PNG>","place":"...","size":"..."}`。设备**只认 `data`，不实现 url 抓取**（无联网、无 url 白名单攻击面）。
- **不缩放**：设备按解码出的 PNG 原生像素 `(w,h)` 原样 `paste_rect`，并用该 `(w,h)` 向 `layout::resolve` 要位置。缩放是服务器（计划 3）的事。
- **彩色仅在 takeover `Rgb32` 后端生效**：blit 字节序 **BGRA**（`B,G,R,0xFF`）、按 `stride` 逐行（用 `paste_rect`）。qtfb（`Rgb565`）后端无彩色波形——本计划图卡在 qtfb 下允许降级/跳过（Demo 跑 takeover）。
- **图卡走 mode 5**：贴完 BGRA 后用 `disp.swap_raw(x,y,w,h,5,0)` 刷（spike 选定），**不做逐帧动画**（一次贴、一次刷）。
- **容错**：`data` 缺失/非法 base64/PNG 解码失败/超过 `MAX_IMAGE_BYTES` → `eprintln` + 丢该卡，绝不 panic（与 cards.rs 既有各臂一致）。图卡仍受 `MAX_CARDS=3` 约束（≤1 image 由服务器侧保证，设备不额外限制）。
- **PNG 色型**：只处理 `Rgba`/`Rgb` 8-bit（服务器控制素材格式）；其它色型（palette/gray）丢弃并 `eprintln`。
- 构建/测试：单测 `cargo test --bin riddle <filter>`（Docker `rust:1-trixie`，不带 `takeover`）；上板用 takeover 构建 + systemd-run 接管路径（见任务 3 验证节）。跟随 cards.rs/cardrender.rs 既有写法与命名。

## 构建 / 测试命令（容器封装，各任务复用）

单元测试（原生 aarch64，无需 takeover / libquill）：
```bash
cd /Users/ben/Documents/GitHub/DouDou
docker run --rm -v "$PWD:/work" -v riddle-cargo-registry:/usr/local/cargo/registry \
  -w /work/device/riddle rust:1-trixie \
  bash -c 'export PATH=/usr/local/cargo/bin:$PATH; cargo test --bin riddle <FILTER> 2>&1 | tail -20'
```
takeover 构建（任务 3 上板用）：
```bash
docker run --rm -v "$PWD:/work" -v riddle-rm-sdk-3-27:/sdk -v riddle-cargo-registry:/usr/local/cargo/registry \
  -w /work/device/riddle -e RIDDLE_RM_SDK=/sdk rust:1-trixie \
  bash -c 'export PATH=/usr/local/cargo/bin:$PATH; ./build-takeover.sh'
```

---

### Task 1: `cards.rs` 解析 `image` 卡（base64 → PNG 字节）+ base64 依赖

**Files:**
- Modify: `device/riddle/Cargo.toml`（加 `base64`）
- Modify: `device/riddle/src/cards.rs`（`Card::Image` 变体、`RawCard.data`、`"image"` 臂、`MAX_IMAGE_BYTES`、测试）

**Interfaces:**
- Produces: `Card::Image { common: CardCommon, png: Vec<u8> }`（`png` = 已 base64-解码的原始 PNG 字节，尚未 PNG-解码）；解析对畸形一律丢弃。

- [ ] **Step 1: 加 base64 依赖**

`device/riddle/Cargo.toml` 的 `[dependencies]` 加一行（版本对齐生态现行）：
```toml
base64 = "0.22"
```

- [ ] **Step 2: 写失败测试**

在 `device/riddle/src/cards.rs` 的 `#[cfg(test)] mod tests` 里追加（用现有的 `png` 编码器现造一张 PNG，再 base64，喂给解析器闭环验证）：

```rust
// —— helper：造一张 w×h 纯色 RGBA PNG，返回其 base64 —— //
fn png_b64(w: u32, h: u32, rgba: [u8; 4]) -> String {
    use base64::Engine;
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let data: Vec<u8> = (0..(w * h)).flat_map(|_| rgba).collect();
        writer.write_image_data(&data).unwrap();
    }
    base64::engine::general_purpose::STANDARD.encode(&png)
}

#[test]
fn parses_image_card_from_inline_base64() {
    let b64 = png_b64(4, 4, [255, 0, 0, 255]);
    let json = format!(
        r#"{{"turn_id":"t","spoken_text":"","paper_cards":[
            {{"type":"image","data":"{b64}","place":"blank_area","size":"l"}}
        ],"page_action":"none","memory_tags":[]}}"#
    );
    let r = parse_turn_response(&json).unwrap();
    assert_eq!(r.paper_cards.len(), 1);
    match &r.paper_cards[0] {
        Card::Image { png, common } => {
            assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "keeps decoded PNG bytes");
            assert!(matches!(common.place, Place::BlankArea));
            assert!(matches!(common.size, Size::L));
        }
        other => panic!("expected image, got {other:?}"),
    }
}

#[test]
fn drops_image_card_with_missing_or_bad_data() {
    let json = r#"{"turn_id":"t","spoken_text":"","paper_cards":[
        {"type":"image","place":"blank_area"},
        {"type":"image","data":"@@ not base64 @@"}
    ],"page_action":"none","memory_tags":[]}"#;
    let r = parse_turn_response(json).unwrap();
    assert!(r.paper_cards.is_empty(), "missing data and bad base64 both dropped");
}

#[test]
fn drops_oversize_image_card() {
    // 造一张超过 MAX_IMAGE_BYTES 的 PNG（够大的纯色图即可）。
    let b64 = png_b64(1200, 1200, [0, 0, 0, 255]);
    let json = format!(
        r#"{{"turn_id":"t","spoken_text":"","paper_cards":[{{"type":"image","data":"{b64}"}}],"page_action":"none","memory_tags":[]}}"#
    );
    let r = parse_turn_response(&json).unwrap();
    assert!(r.paper_cards.is_empty(), "oversize image dropped");
}
```
> 说明：`drops_oversize_image_card` 依赖 `MAX_IMAGE_BYTES`（Step 3 设为 1 MB）。1200×1200×4 的未压缩数据虽会被 PNG 压掉，但纯色压缩后仍需确保 > 1 MB——若纯色压得太小导致该测试不成立，改用随机/渐变像素填充把体积顶上去（实现者按实际压缩结果调整填充，使解码后的 PNG 字节数确实 > `MAX_IMAGE_BYTES`），保持断言语义不变。

- [ ] **Step 3: 实现解析**

`device/riddle/src/cards.rs`：

1) `Card` 枚举（:76）加变体：
```rust
    /// `image`: 服务器内联的真彩位图（base64 PNG，已解码为原始 PNG 字节）。
    /// 设备按 PNG 原生像素尺寸原样渲染，不缩放（服务器按 size 预缩放）。
    Image { common: CardCommon, png: Vec<u8> },
```

2) `RawCard`（:124）加字段：
```rust
    // image
    data: Option<String>,
```

3) 模块常量（与 `MAX_SKETCH_POINTS` 等并列）：
```rust
/// 单张 image 卡解码后 PNG 字节上限（防内存爆掉）。一张纸面插画远小于此。
pub const MAX_IMAGE_BYTES: usize = 1_000_000;
```

4) `convert_card` 的 `match card_type`（:219）加臂（放在 `"stamp"` 之后即可）：
```rust
        "image" => {
            use base64::Engine;
            let common = raw_common(&raw);
            let Some(data) = raw.data else {
                eprintln!("riddle: cards: dropping image card with no data");
                return None;
            };
            let png = match base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("riddle: cards: dropping image card with bad base64: {e}");
                    return None;
                }
            };
            if png.len() > MAX_IMAGE_BYTES {
                eprintln!(
                    "riddle: cards: dropping image card of {} bytes (max {MAX_IMAGE_BYTES})",
                    png.len()
                );
                return None;
            }
            Some(Card::Image { common, png })
        }
```

- [ ] **Step 4: 运行测试（GREEN）**

Run（容器）：`cargo test --bin riddle cards:: 2>&1 | tail -20`
Expected: 原有 8 + 新增 3 全 PASS。

- [ ] **Step 5: Commit**

```bash
git add device/riddle/Cargo.toml device/riddle/Cargo.lock device/riddle/src/cards.rs
git commit -m "feat(device): parse inline-base64 image card in cards.rs"
```

---

### Task 2: `cardrender.rs` — `plan_image`（PNG 解码 → BGRA → 排位）+ `RenderPlan` 位图载荷

**Files:**
- Modify: `device/riddle/src/cardrender.rs`
- （视需要）Modify: `device/riddle/src/main.rs`（仅当 `RenderPlan` 在 main.rs 有构造点需补 `blit: None`）

**Interfaces:**
- Consumes: Task 1 的 `Card::Image { common, png }`
- Produces:
  - `pub struct ImageBlit { pub rect: (i32, i32, i32, i32), pub bgra: Vec<u8> }`（rect 像素 x,y,w,h；bgra 长度 = w*h*4，字节序 B,G,R,0xFF）
  - `RenderPlan` 增字段 `pub blit: Option<ImageBlit>`（笔画卡为 `None`）
  - `plan_image(card, &mut layout::InkMap) -> Vec<RenderPlan>`：解码失败/无处安放 → 返回空 `vec![]`（丢弃，不 panic）

- [ ] **Step 1: 写失败测试**

在 `device/riddle/src/cardrender.rs` 的测试模块（若无则新建 `#[cfg(test)] mod tests`）追加：

```rust
#[cfg(test)]
mod image_tests {
    use super::*;
    use crate::cards::{Card, CardCommon, Place, Size, Pace};

    fn red_png_card(w: u32, h: u32) -> Card {
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            let data: Vec<u8> = (0..(w * h)).flat_map(|_| [255u8, 0, 0, 255]).collect();
            writer.write_image_data(&data).unwrap();
        }
        Card::Image {
            common: CardCommon { place: Place::BlankArea, anchor_norm: None, size: Size::L, pace: Pace::Normal },
            png,
        }
    }

    #[test]
    fn plan_image_decodes_to_bgra_blit_of_native_size() {
        let mut map = layout::InkMap::new();
        let card = red_png_card(6, 5);
        let plans = plan_image(&card, &mut map);
        assert_eq!(plans.len(), 1, "one blit plan");
        let blit = plans[0].blit.as_ref().expect("blit present");
        let (_, _, w, h) = blit.rect;
        assert_eq!((w, h), (6, 5), "native PNG size, no scaling");
        assert_eq!(blit.bgra.len(), (6 * 5 * 4) as usize);
        // 红像素 RGBA[255,0,0,255] → BGRA[0,0,255,255]
        assert_eq!(&blit.bgra[0..4], &[0u8, 0, 255, 255]);
        assert!(plans[0].strokes.is_empty(), "image plan carries no strokes");
    }

    #[test]
    fn plan_image_drops_undecodable_png() {
        let mut map = layout::InkMap::new();
        let card = Card::Image {
            common: CardCommon { place: Place::BlankArea, anchor_norm: None, size: Size::L, pace: Pace::Normal },
            png: vec![1, 2, 3, 4],  // 非 PNG
        };
        assert!(plan_image(&card, &mut map).is_empty());
    }
}
```
> 实现者注意：`layout::InkMap::new()`、`CardCommon`/`Place`/`Size`/`Pace` 的真实构造/可见性以实际代码为准（若字段私有，用现有的公开构造或测试辅助）；断言语义（原生尺寸、BGRA、无笔画、坏图丢弃）不变。

- [ ] **Step 2: 运行确认失败**

Run（容器）：`cargo test --bin riddle cardrender::image_tests 2>&1 | tail -20`
Expected: 编译失败（`plan_image`/`ImageBlit`/`blit` 不存在）。

- [ ] **Step 3: 实现**

`device/riddle/src/cardrender.rs`：

1) `RenderPlan`（:20）加字段：
```rust
    /// 位图卡载荷：设为 Some 时该 plan 不走逐帧笔画动画，而是一次性
    /// paste_rect(bgra) + swap_raw(mode 5)。笔画卡恒为 None。
    pub blit: Option<ImageBlit>,
```
并新增：
```rust
#[derive(Debug, Clone)]
pub struct ImageBlit {
    /// 像素 (x, y, w, h)。
    pub rect: (i32, i32, i32, i32),
    /// w*h*4 的 BGRA（B,G,R,0xFF），供 Surface::paste_rect 直接贴（Rgb32）。
    pub bgra: Vec<u8>,
}
```
> **所有现有 `RenderPlan { ... }` 构造点都要补 `blit: None`。** 集中构造多经 `make_plan(...)` 辅助（cardrender.rs 内），在那里补 `blit: None` 即可覆盖大多数；对任何直接字面构造 `RenderPlan{...}` 的地方（含 main.rs，若有）也逐一补上。实现者用编译器报错定位所有构造点。

2) 新增 `plan_image`：
```rust
/// 把一张 `image` 卡解码成一个 BGRA blit plan，按 layout 排到不重叠的空位。
/// 设备不缩放：用 PNG 原生像素尺寸向 layout 要位置。解码失败或无处安放 → 空 vec。
pub fn plan_image(card: &cards::Card, map: &mut layout::InkMap) -> Vec<RenderPlan> {
    let cards::Card::Image { common, png } = card else { return Vec::new() };

    // —— PNG 解码为 (w,h) + RGBA8/RGB8 —— //
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = match decoder.read_info() {
        Ok(r) => r,
        Err(e) => { eprintln!("riddle: cards: image PNG header error: {e}"); return Vec::new(); }
    };
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = match reader.next_frame(&mut buf) {
        Ok(i) => i,
        Err(e) => { eprintln!("riddle: cards: image PNG decode error: {e}"); return Vec::new(); }
    };
    let (w, h) = (info.width as i32, info.height as i32);
    if w <= 0 || h <= 0 { return Vec::new(); }

    // —— 组 BGRA（B,G,R,0xFF）—— //
    let px = &buf[..info.buffer_size()];
    let bgra: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.chunks_exact(4)
            .flat_map(|c| [c[2], c[1], c[0], 0xFF]).collect(),
        png::ColorType::Rgb => px.chunks_exact(3)
            .flat_map(|c| [c[2], c[1], c[0], 0xFF]).collect(),
        other => {
            eprintln!("riddle: cards: unsupported image color type {other:?}, dropping");
            return Vec::new();
        }
    };

    // —— 向 layout 要一个 w×h 的不重叠位置 —— //
    let anchor = common.anchor_norm
        .map(|(nx, ny)| ((nx * fb::SCREEN_W as f32) as i32, (ny * fb::SCREEN_H as f32) as i32))
        .unwrap_or((0, 0));
    let Some((x, y)) = layout::resolve(map, common.place, anchor, w, h) else {
        eprintln!("riddle: cards: no room for {w}x{h} image card, dropping");
        return Vec::new();
    };
    map.mark_rect(x, y, w, h);

    let mut region = fb::BBox::empty();
    region.grow(x, y);
    region.grow(x + w, y + h);
    vec![RenderPlan {
        strokes: Vec::new(),
        color: surface::BLACK,
        points_per_frame: 0,
        region,
        blit: Some(ImageBlit { rect: (x, y, w, h), bgra }),
    }]
}
```
> 实现者：`fb::SCREEN_W/H`、`fb::BBox` 的实际 API（`empty`/`grow`/或其它构造）、`layout::resolve` 的确切签名（`place`/`anchor` 参数类型）、`map.mark_rect` 的名字与签名，以实际代码为准平移——上面用的是探路所见名字，若有出入按真实签名改，语义不变（解码→BGRA→resolve→plan）。

3) `plan_card`（:939 的分派）加臂：
```rust
        cards::Card::Image { .. } => plan_image(card, map),
```
4) `plan_in_rect`（:877/:884，供 `page.layout` 用）加对 `Card::Image` 的处理：在给定 `rect` 内解码并贴（复用上面解码+BGRA 逻辑，位置直接用 `rect` 的像素左上角、尺寸仍用 PNG 原生尺寸，不缩放；若 PNG 比 rect 大，超出部分由 `paste_rect` 的裁剪/或此处裁剪处理——最简：仍按原生尺寸，交给 Task 3 的 `paste_rect` 边界裁剪）。**若 `page.layout` 内嵌 image 在本 Demo 用不到，可在此臂 `eprintln` + 返回空、留到需要时再实现**——实现者按 `plan_in_rect` 现有结构择一，报告选择。

- [ ] **Step 4: 运行测试（GREEN）**

Run（容器）：`cargo test --bin riddle cardrender 2>&1 | tail -25`
Expected: 新增 image_tests 全 PASS；`cargo test --bin riddle 2>&1 | tail` 整体不回归。

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/cardrender.rs device/riddle/src/main.rs
git commit -m "feat(device): plan_image decodes PNG to a BGRA blit plan placed by layout"
```

---

### Task 3: DrawingCards 的 image 分支 + 离线 `--cards-test` 合成 + 上板 `--image-test` 诊断

> 现状订正（实现者注意）：`--cards-test fixture.json [out.png]`（main.rs:339-429）是**读 fixture JSON 文件离线渲染成灰度 PNG**（`write_page_png`→`page_gray` 2× 降采样为灰度），其渲染循环（main.rs:400-421）**只画 `p.strokes`、不处理 `p.blit`**。所以：离线 `--cards-test` 补 blit 合成后只能验**排位**（灰度），**颜色**要靠上板的 `--image-test`（走 takeover 真彩 + mode 5）。

**Files:**
- Modify: `device/riddle/src/main.rs`（① DrawingCards 加 blit 分支＝真实特性；② `cards_test` 渲染循环补 `p.blit` 合成＝离线排位验证；③ 新增 `--image-test` 上板彩色诊断）
- Create: `device/riddle/fixtures/image-card.json`（离线验证用 fixture；若已有 fixtures 目录则放入，否则新建）

**Interfaces:**
- Consumes: Task 2 的 `RenderPlan.blit` / `ImageBlit`
- Produces: 图卡在 takeover 下 `paste_rect`+`swap_raw(...,5,0)` 渲染；离线 `--cards-test` 能合成图卡到灰度 PNG；`--image-test` 上板直验彩色

- [ ] **Step 1: DrawingCards 加 blit 分支（真实特性）**

在 `main.rs` DrawingCards 处理（:1226-1293）里，取到当前 `plan = &plans[plan_i]` 后、**进入逐点笔画动画之前**插入：
```rust
        // 位图卡：一次性贴 BGRA + mode 5 刷，不做逐帧动画。
        if let Some(blit) = plan.blit.clone() {
            let (x, y, w, h) = blit.rect;
            surf.paste_rect(x, y, w, h, &blit.bgra);
            disp.swap_raw(x, y, w, h, 5, 0);
            // —— 复用该 handler 现有的"一张 plan 画完→推进"路径（plan_i/point_i
            //    更新、最后一张后切回的 State），勿另造状态机。——
            <推进到下一 plan 的现有逻辑>;
            return; // 或 continue，与该 handler 现有控制流一致
        }
```
> `surf.paste_rect`/`disp.swap_raw` 均已存在（surface.rs:170 / display.rs:126）。**qtfb 降级**：qtfb 是 `Rgb565`，直接贴 BGRA 会错色——用 `surf.fmt`（或等价查询）分支，非 `Rgb32` 时跳过 blit + `eprintln` 一句（Demo 跑 takeover）；实现者报告所选处理。

- [ ] **Step 2: `cards_test` 渲染循环补 `p.blit` 合成（离线排位验证）**

在 `cards_test` 的 plan 渲染循环（main.rs:406-420，`for p in plans { ... }` 里、画完 `p.strokes` 之后）追加：
```rust
            if let Some(blit) = &p.blit {
                let (x, y, w, h) = blit.rect;
                surf.paste_rect(x, y, w, h, &blit.bgra);
                eprintln!("image blit at ({x},{y}) {w}x{h}");
            }
```
这样带 `image` 卡的 fixture 也能离线渲染进灰度 PNG（验证解码成功 + layout 排到不重叠空位）。

- [ ] **Step 3: 上板彩色诊断 `--image-test`**

仿 `--color-test`（main.rs:201-202 的分派、:230 的 `color_test`）新增 `Some("--image-test") => std::process::exit(image_test(&args))` 与：
```rust
/// 上板真彩自检：合成一张多色带 PNG，走 plan_image 得到 blit，再 paste_rect
/// + swap_raw(mode 5) 刷到 takeover 面板。验证 mode-5 彩色渲染这条真实原语。
fn image_test(_args: &[String]) -> i32 {
    use base64::Engine;
    let (w, h) = (600u32, 400u32);
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let bands = [[255u8,0,0,255],[0,255,0,255],[0,0,255,255],[255,255,0,255]];
        let mut data = Vec::with_capacity((w*h*4) as usize);
        for row in 0..h {
            let c = bands[(row as usize * bands.len() / h as usize).min(bands.len()-1)];
            for _ in 0..w { data.extend_from_slice(&c); }
        }
        writer.write_image_data(&data).unwrap();
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let json = format!(
        r#"{{"turn_id":"t","spoken_text":"","paper_cards":[{{"type":"image","data":"{b64}","place":"blank_area","size":"l"}}],"page_action":"none","memory_tags":[]}}"#
    );
    let resp = match cards::parse_turn_response(&json) { Ok(r) => r, Err(e) => { eprintln!("image-test: {e}"); return 1; } };
    let (disp, mut surf) = match display::Display::open() {
        Ok(v) => v,
        Err(e) => { eprintln!("image-test: display open failed: {e} (needs takeover + xochitl stopped)"); return 1; }
    };
    surf.fill_rect(0, 0, surf.w, surf.h, surface::WHITE);
    let mut map = layout::InkMap::from_surface(&surf);
    let font = script::FontStack::new(FontRef::try_from_slice(FONT_TTF).expect("font"), None);
    for card in &resp.paper_cards {
        for p in cardrender::plan_card(card, &mut map, &font, (surf.w as i32/2, surf.h as i32/2)) {
            if let Some(blit) = &p.blit {
                let (x, y, bw, bh) = blit.rect;
                surf.paste_rect(x, y, bw, bh, &blit.bgra);
                eprintln!(">>> WATCH — image blit {bw}x{bh} at ({x},{y}), swapping at mode 5");
                disp.swap_raw(x, y, bw, bh, 5, 0);
            }
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(20));
    eprintln!("image-test: done (xochitl auto-restored on exit)");
    0
}
```
> 实现者：`display`/`surface`/`layout`/`cardrender`/`FontRef`/`FONT_TTF` 的路径与 `color_test` 里一致（照抄其 use/引用）；`Display::open`/`InkMap::from_surface`/`plan_card` 签名以实际为准。

- [ ] **Step 4: 单测 + 构建 + 离线渲染 fixture**

- 加一个纯函数单测证明含 image 卡的 fixture 合法解析（放 cards 或 main 的测试模块）：
```rust
#[test]
fn image_card_fixture_parses() {
    // 与 Task1 的 png_b64 同法造一张小 PNG 拼进 response，验证解析出 Card::Image
    // （实现者复用 Task1/Task2 的 PNG helper；断言 paper_cards 里有一张 Image）。
}
```
Run（容器）：`cargo test --bin riddle 2>&1 | tail -20` → 全绿、无回归。
- takeover 构建：见文首"takeover 构建"命令；产物 `target/aarch64-unknown-linux-gnu/release/riddle-takeover`。
- 造 `device/riddle/fixtures/image-card.json`：`{"response":{"turn_id":"t","spoken_text":"","paper_cards":[{"type":"image","data":"<小PNG的base64>","place":"blank_area","size":"l"},{"type":"text","content":"太阳","place":"near_anchor","anchor_norm":[0.5,0.4]}],"page_action":"none","memory_tags":[]},"ink":[[[300,400],[500,600]]]}`（base64 用一张现造的小彩图；也可在报告里给出生成命令）。**离线渲染自检（控制器会做）**：`--cards-test fixtures/image-card.json /tmp/out.png` 后灰度 PNG 里图卡出现在空白处、不压 ink/text 卡。

- [ ] **Step 5: 真机彩色验证（人工，控制器执行 + Ben 看屏）**

走**安全的 systemd-run 接管路径**（勿 SSH 前台、勿手动 juggle xochitl）：`scp` 二进制到 `/home/root/riddle-imagetest` → 仿 spike 的 `colortest-launch.sh` 写启动脚本（`systemctl stop xochitl; rm -f /tmp/epframebuffer.lock; sleep 1; LD_LIBRARY_PATH=<bundle>:/usr/lib/plugins/scenegraph HOME=/home/root /home/root/riddle-imagetest --image-test`）→ `systemd-run --unit=riddle-imagetest --collect --property=ExecStopPost=-/bin/systemctl start xochitl /bin/bash <脚本>`。**判通过**：彩色带图卡真彩上屏（红/绿/蓝/黄可辨）、mode 5 观感可接受、不崩、xochitl 自动恢复。子代理只交付代码 + 容器单测/构建全绿；本步由控制器执行、Ben 判读。

- [ ] **Step 6: Commit**

```bash
git add device/riddle/src/main.rs device/riddle/fixtures/image-card.json
git commit -m "feat(device): render image cards at waveform mode 5 (+cards-test blit, --image-test)"
```

---

## 自查（写完对照 spec / 现状）

- **spec 覆盖**：§4 `image` 彩图卡真彩渲染 = Task 1（解析）+ Task 2（解码/排位）+ Task 3（mode 5 刷）；§5 S0a 定的 mode 5 = Task 3 的 `swap_raw(...,5,0)`；"不重叠"复用既有 `layout::resolve`（Task 2）。
- **本计划不含**：服务器发 image 卡（计划 3：精选彩图库 + `cards.py` 把 `image` 从占位 `url` 改为内联 `data` + 按 size 预缩放 PNG）、课时注入（计划 3）、真机串课（计划 4）。因此 Task 3 靠 `--cards-test` 合成图自证，不依赖服务器。
- **契约一致性**：设备 `image` 卡认 `data`(base64 PNG)；这与计划 1 里 `cards.py` 现有的 `image`+`url` 占位**故意不一致**——计划 3 会把服务器侧从 `url` 切到 `data`。在计划 3 落地前，服务器不会真的发 image 卡（模型被告知本期只出 text/stamp），故无运行期冲突。
- **占位符扫描**：Task 2/3 有几处"以实际签名为准平移"的显式指引（`RenderPlan` 构造点、`layout::resolve`/`fb::BBox`/`mark_rect` 真实签名、DrawingCards 的 plan 推进逻辑、`--cards-test` 接入点）——这些是"照现有代码模式改"的定向指令，不是待填空白；每处都给了完整目标代码与语义。
- **类型一致性**：`Card::Image{common,png:Vec<u8>}`（Task1）↔ `plan_image` 解构（Task2）↔ `RenderPlan.blit:Option<ImageBlit>` / `ImageBlit{rect,bgra}`（Task2）↔ DrawingCards `paste_rect(x,y,w,h,&bgra)`+`swap_raw(x,y,w,h,5,0)`（Task3）贯通一致。

## 执行交接

见文首 header 的 REQUIRED SUB-SKILL。Task 1–2 子代理交付（容器内 `cargo test --bin riddle` 全绿）；Task 3 子代理交付代码 + 单测/构建全绿，**真机 mode-5 目视由控制器执行、请 Ben 判读**。
