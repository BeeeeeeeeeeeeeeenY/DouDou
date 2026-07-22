# DouDou 平板端研发实施计划（一期半 + 卡片渲染基建 + 共画状态机 + /turn 客户端）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按 `docs/superpowers/specs/2026-07-22-tablet-interaction-design.md` 把 device/riddle 从「喝墨水的日记」演进为「共画的 DouDou」：先落地三项儿童安全微改（可立即部署），再建卡片渲染基建（六种 paper_cards、布局器、离屏测试工具），最后改造状态机为就地共画并接入 /turn 结构化客户端（mock 可测，等后端二期上线即连通）。

**Architecture:** 全部改动在 `device/riddle`（单 crate Rust 应用）。新增 4 个模块：`cards.rs`（/turn 数据模型+校验）、`stamps.rs`（8 符号矢量库）、`layout.rs`(墨迹覆盖图+摆放解析)、`cardrender.rs`（卡片→笔画渲染计划）；改造 `main.rs` 状态机（去掉 Drinking/FadingReply 的常规路径）、`ink.rs`（时间戳）、`touch.rs`（五指长按）、`memory.rs`（笔画格式 v2）。服务器不可用时行为不变（纸永远能画）。

**Tech Stack:** Rust 2021（现有依赖 libc/signal-hook/png/ab_glyph/ureq；Task 4 新增 serde + serde_json，纯 Rust，交叉编译无碍）。测试与构建走 Docker（Apple Silicon 原生 aarch64 Linux 容器 = 设备目标架构）。

## Global Constraints

- 工作目录：`/Users/ben/Documents/GitHub/DouDou/.claude/worktrees/laughing-cerf-c7888a`（分支 `claude/laughing-cerf-c7888a`），crate 根：`device/riddle`。
- **测试命令（每个任务提交前必须全绿）**：
  `docker run --rm -v "$PWD/device/riddle:/work" -w /work -v riddle-cargo-registry:/usr/local/cargo/registry rust:1-bookworm cargo test`
  （从仓库根执行；基线 28 个测试已验证 0.87s 全绿。）
- 字体前置：`device/riddle/fonts/PingFangShiGuang.ttf` 必须存在（gitignored；已从主检出复制。若缺失：`cp /Users/ben/Documents/GitHub/DouDou/device/riddle/fonts/PingFangShiGuang.ttf device/riddle/fonts/`）。**永不提交字体文件。**
- 屏幕常量：竖屏 `SCREEN_W=1620`、`SCREEN_H=2160`（`fb.rs`）。颜色：`WHITE=0xFFFF / BLACK=0x0000 / FADED=0x7BCF`（`surface.rs`）。
- 提交信息风格：英文祈使句短标题（仿现有 `Fix handwritten font rendering`），正文可省，结尾加 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- 代码风格：无 rustfmt 配置，目测对齐现有风格（4 空格、`//!` 模块头注释、注释講「为什么」）。新配置一律走环境变量（`oracle.env` 是设备上的配置面），并同步到 `oracle.env.example`。
- 单元测试放各模块 `#[cfg(test)] mod tests`（现有惯例）；跨模块 fixture 放 `device/riddle/test-data/`（新目录，可提交 JSON，不提交 PNG 输出）。
- spec 引用：`docs/superpowers/specs/2026-07-22-tablet-interaction-design.md`（§14 为 /turn 契约）。
- 里程碑边界：Task 1-3 = 一期半（随时可部署）；Task 4-8 = 渲染基建（不动行为）；Task 9-11 = 共画状态机（改变默认行为）；Task 12 = /turn 客户端（`RIDDLE_TURN_URL` 未设时完全不生效）。
- 非目标（本计划不做）：控制通道推送（commit_now/recall/set_profile——等后端二期定传输）、记忆上传服务器、语音联动、彩色渲染、`page` 卡的故事底图素材。

---

### Task 1: 停笔提交阈值 2.8s → 6s（环境变量可调）

**Files:**
- Modify: `device/riddle/src/main.rs:37`（`IDLE_COMMIT` 常量）与 `main.rs:417`（使用处）
- Modify: `device/riddle/oracle.env.example`（文档化新变量）

**Interfaces:**
- Produces: `fn idle_commit() -> Duration`（main.rs 内私有，读 `RIDDLE_IDLE_COMMIT_SECS`，默认 6.0，接受小数，非法值回退默认）。

- [ ] **Step 1: 写失败测试**（加在 `main.rs` 文件尾部新 `#[cfg(test)] mod tests`）

```rust
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: 全局约束里的 docker cargo test 命令
Expected: FAIL，`idle_commit_from` 未定义（编译错误）。

- [ ] **Step 3: 实现**

删除 `main.rs:37` 的 `const IDLE_COMMIT: Duration = Duration::from_millis(2800);`，替换为：

```rust
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
```

`main.rs:417` 的 `t.elapsed() >= IDLE_COMMIT` 改为 `t.elapsed() >= idle_commit()`。

`oracle.env.example` 追加：

```sh
# 停笔多少秒后提交页面（默认 6，支持小数）。
#export RIDDLE_IDLE_COMMIT_SECS="6"
```

- [ ] **Step 4: 跑测试确认通过**（29 passed）

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/main.rs device/riddle/oracle.env.example
git commit -m "Relax pen-idle commit to a tunable 6s default"
```

---

### Task 2: 五指退出改为长按 ≥3 秒

**Files:**
- Modify: `device/riddle/src/touch.rs`（`finish_frame` 五指判定 + 新 `QuitArm` 结构）
- Modify: `device/riddle/oracle.env.example`

**Interfaces:**
- Produces: `touch.rs` 内部 `struct QuitArm { hold: Duration, since: Option<Instant>, fired: bool }`，`fn update(&mut self, finger_count: usize, now: Instant) -> bool`（返回 true = 触发退出）。`TouchDevice::drain()` 对外行为不变（仍产出 `Gesture::Quit`），仅触发条件变为持续满 5 指 ≥3s。

- [ ] **Step 1: 写失败测试**（`touch.rs` 尾部新增 `#[cfg(test)] mod tests`）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn quit_requires_sustained_five_fingers() {
        let mut arm = QuitArm::new(Duration::from_secs(3));
        let t0 = Instant::now();
        assert!(!arm.update(5, t0)); // 刚满 5 指：不触发
        assert!(!arm.update(5, t0 + Duration::from_millis(2900))); // 未满 3s
        assert!(arm.update(5, t0 + Duration::from_millis(3001))); // 满 3s：触发
        assert!(!arm.update(5, t0 + Duration::from_millis(3200))); // 已触发过，不重复
    }

    #[test]
    fn quit_rearms_only_after_release() {
        let mut arm = QuitArm::new(Duration::from_secs(3));
        let t0 = Instant::now();
        arm.update(5, t0);
        assert!(!arm.update(3, t0 + Duration::from_millis(1000))); // 掉到 3 指：计时清零
        assert!(!arm.update(5, t0 + Duration::from_millis(1500))); // 重新满 5 指从头计
        assert!(!arm.update(5, t0 + Duration::from_millis(4400))); // 距重满仅 2.9s
        assert!(arm.update(5, t0 + Duration::from_millis(4600))); // 3.1s：触发
    }

    #[test]
    fn zero_hold_fires_immediately_for_legacy_mode() {
        let mut arm = QuitArm::new(Duration::ZERO);
        assert!(arm.update(5, Instant::now()));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**（`QuitArm` 未定义）

- [ ] **Step 3: 实现**

`touch.rs` 顶部 `use std::io;` 下加 `use std::time::{Duration, Instant};`。新增：

```rust
/// Child-safety: five-finger exit must be HELD, not tapped — a toddler's palm
/// slap reaches five contacts for an instant all the time. Hold duration from
/// RIDDLE_QUIT_HOLD_SECS (default 3; 0 restores the legacy instant tap).
struct QuitArm {
    hold: Duration,
    since: Option<Instant>,
    fired: bool,
}

impl QuitArm {
    fn new(hold: Duration) -> Self {
        Self { hold, since: None, fired: false }
    }

    fn from_env() -> Self {
        let secs = std::env::var("RIDDLE_QUIT_HOLD_SECS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|s| *s >= 0.0)
            .unwrap_or(3.0);
        Self::new(Duration::from_millis((secs * 1000.0) as u64))
    }

    fn update(&mut self, finger_count: usize, now: Instant) -> bool {
        if finger_count < 5 {
            self.since = None;
            self.fired = false;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if !self.fired && now.duration_since(since) >= self.hold {
            self.fired = true;
            return true;
        }
        false
    }
}
```

`TouchDevice` 字段 `quit_sent: bool` 换成 `quit_arm: QuitArm`；`open()` 里 `quit_sent: false` 换成 `quit_arm: QuitArm::from_env()`；`suppress()` 里 `self.quit_sent = false;` 换成 `self.quit_arm.since = None; self.quit_arm.fired = false;`。

`finish_frame` 中五指段：

```rust
        if self.quit_arm.update(count, Instant::now()) {
            out.push(Gesture::Quit);
        }
```

（替换原 `if count >= 5 && !self.quit_sent { … }` 三行；`count == 0` 复位段里的 `self.quit_sent = false;` 删除——QuitArm 在指数 <5 时自复位。）

**注意**：长按期间没有新触摸事件时 `drain()` 不会被喂帧——但主循环每 ~2ms 调 `drain_check_quit()`，且真实手指按住时触摸屏持续报告位置帧，SYN 帧不断，`finish_frame` 会被持续调用，长按可靠。

`oracle.env.example` 追加：

```sh
# 五指长按多少秒退出（默认 3；设 0 恢复旧的五指点按即退）。
#export RIDDLE_QUIT_HOLD_SECS="3"
```

- [ ] **Step 4: 跑测试确认通过**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/touch.rs device/riddle/oracle.env.example
git commit -m "Require a sustained 3s five-finger hold to exit"
```

---

### Task 3: 笔画点级时间戳（采集 + 存盘 v2 + 向后兼容加载）

**Files:**
- Modify: `device/riddle/src/ink.rs`（点元组 3→4 元；页起点计时）
- Modify: `device/riddle/src/memory.rs`（`Strokes` 类型、写盘 `x,y,r,t`、加载兼容 3/4 字段）
- Modify: `device/riddle/src/main.rs`（`ConjurePlan` 映射处忽略 t；`conjure()` 内两处）

**Interfaces:**
- Produces: `ink::Ink::stroke_list() -> &[Vec<(i32, i32, i32, u32)>]`（第 4 元 = 相对本页首个落点的毫秒偏移）；`memory::Strokes = Vec<Vec<(i32, i32, i32, u32)>>`；`.strokes` 文件 v2 行格式 `x,y,r,t;…`，加载器同时接受旧版 `x,y,r`（t 补 0）。
- Consumes: 无（独立于 Task 1/2）。

- [ ] **Step 1: 写失败测试**

`ink.rs` tests 模块追加：

```rust
    #[test]
    fn points_carry_monotonic_timestamps() {
        let (_buf, mut s) = surf();
        let mut ink = Ink::new();
        ink.pen_point(&mut s, 10, 10, 3);
        std::thread::sleep(std::time::Duration::from_millis(15));
        ink.pen_point(&mut s, 20, 10, 3);
        ink.pen_up();
        let strokes = ink.stroke_list();
        assert_eq!(strokes[0][0].3, 0, "first point of the page is t=0");
        assert!(strokes[0][1].3 >= 10, "second point ~15ms later, got {}", strokes[0][1].3);
        // 清页后重新计时。
        ink.clear();
        ink.pen_point(&mut s, 30, 30, 3);
        ink.pen_up();
        assert_eq!(ink.stroke_list()[0][0].3, 0);
    }
```

`memory.rs` tests 追加：

```rust
    #[test]
    fn strokes_v2_round_trip_keeps_time() {
        let mut s = tmp_store("v2");
        let strokes: Strokes = vec![vec![(10, 20, 3, 0), (14, 24, 3, 120)]];
        s.append(42, "t", "r", &strokes);
        let back = s.strokes(42).unwrap();
        assert_eq!(back[0][0], (10, 20, 3, 0));
        assert_eq!(back[0][1], (14, 24, 3, 120));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn legacy_three_field_strokes_still_load() {
        let s = tmp_store("legacy");
        std::fs::write(s.strokes_path(7), "10,20,3;14,24,3\n").unwrap();
        let back = s.strokes(7).unwrap();
        assert_eq!(back[0], vec![(10, 20, 3, 0), (14, 24, 3, 0)]);
        let _ = std::fs::remove_dir_all(&s.dir);
    }
```

（同时把本文件既有测试里的三元组字面量补上第 4 元 `0`：`round_trip_and_reload` 的 `(10,20,3)→(10,20,3,0)` 等、`decimation_keeps_endpoints_drops_dense` 的 `(i,0,3)→(i,0,3,0)`、`prune_forgets_oldest`/`catalog_is_numbered_newest_first` 的 `vec![vec![(1,1,1)]]→vec![vec![(1,1,1,0)]]`。）

- [ ] **Step 2: 跑测试确认失败**（元组宽度编译错误即视为失败信号）

- [ ] **Step 3: 实现**

`ink.rs`：

```rust
pub struct Ink {
    /// Finished strokes as point lists (x, y, radius, ms-since-page-start).
    strokes: Vec<Vec<(i32, i32, i32, u32)>>,
    current: Vec<(i32, i32, i32, u32)>,
    last_erase: Option<(i32, i32)>,
    /// First pen contact on this page; every point's t is measured from it.
    epoch: Option<std::time::Instant>,
    pub bbox: BBox,
}
```

- `new()`：补 `epoch: None`。`clear()`：补 `self.epoch = None;`。
- `stroke_list()` 返回类型改 `&[Vec<(i32, i32, i32, u32)>]`。
- `pen_point` 里 push 前计算 `let t = { let e = *self.epoch.get_or_insert_with(std::time::Instant::now); e.elapsed().as_millis() as u32 };`，`self.current.push((x, y, r, t));`；`if let Some(&(px, py, pr, _)) = self.current.last()` 解构补 `_`。
- `forget_near` 的模式 `for p in stroke` 内 `(p.0 - x, p.1 - y)` 不变；重建 bbox 循环解构 `&(px, py, pr, _)`。

`memory.rs`：

- `pub type Strokes = Vec<Vec<(i32, i32, i32, u32)>>;`
- `append` 写盘循环：`for &(x, y, r, t) in s { … lines.push_str(&format!("{x},{y},{r},{t}")); … }`
- `strokes()` 加载：

```rust
            for pt in line.split(';') {
                let mut n = pt.split(',');
                let (Some(x), Some(y), Some(r)) = (n.next(), n.next(), n.next()) else {
                    continue;
                };
                let t = n.next().and_then(|v| v.parse().ok()).unwrap_or(0u32);
                if let (Ok(x), Ok(y), Ok(r)) = (x.parse(), y.parse(), r.parse()) {
                    stroke.push((x, y, r, t));
                }
            }
```

- `decimate`：解构 `&(x, y, r, t)`，push `(x, y, r, t)`；`Some(&(lx, ly, _, _))`。

`main.rs`（conjure 两处消费 strokes 的地方）：

- `conjure()` 中「writer's own hand」段改为把 4 元映射成 ConjurePlan 的 3 元：

```rust
    for stroke in &strokes {
        let mapped: Vec<(i32, i32, i32)> = stroke.iter().map(|&(x, y, r, _)| (x, y, r)).collect();
        for &(x, y, r) in &mapped {
            region.add(x, y, r + 2);
            ink_bottom = ink_bottom.max(y);
        }
        all.push(mapped);
    }
```

（`ConjurePlan`/回放代码保持 3 元不动。）

- [ ] **Step 4: 跑测试确认通过**（全部既有 + 新增）

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/ink.rs device/riddle/src/memory.rs device/riddle/src/main.rs
git commit -m "Capture per-point stroke timestamps and persist v2 stroke files"
```

---

### Task 4: cards.rs——/turn 响应数据模型、解析与强制约束

**Files:**
- Create: `device/riddle/src/cards.rs`
- Modify: `device/riddle/Cargo.toml`（加 serde/serde_json）
- Modify: `device/riddle/src/main.rs`（`mod cards;` 声明，按字母序插在 `mod fb;` 前）

**Interfaces:**
- Produces（后续 Task 7/8/12 依赖，签名固定）：

```rust
pub enum Place { NearNewInk, NearAnchor, BlankArea, Margin, FullPage }
pub enum Size { S, M, L }
pub enum Pace { Normal, Slow }
pub enum PageAction { None, SuggestNewPage, NewPage }
pub struct CardCommon { pub place: Place, pub anchor_norm: Option<(f32, f32)>, pub size: Size, pub pace: Pace }
pub enum Card {
    Text { common: CardCommon, content: String },
    Sketch { common: CardCommon, strokes: Vec<Vec<(f32, f32)>> },
    Stamp { common: CardCommon, name: String, count: u32 },
    Count { common: CardCommon, n: u32, style: CountStyle },
    Trace { common: CardCommon, kind: TraceKind, content: String, guide: TraceGuide },
    Page { layout: Vec<(Card, (f32, f32, f32, f32))> },  // (card, rect_norm x,y,w,h)
}
pub enum CountStyle { Dots, Tally, Numbers }
pub enum TraceKind { Shape, Hanzi }
pub enum TraceGuide { None, TianGrid }
pub struct TurnResponse { pub turn_id: String, pub spoken_text: String, pub paper_cards: Vec<Card>, pub page_action: PageAction, pub memory_tags: Vec<String> }
pub fn parse_turn_response(json: &str) -> Result<TurnResponse, String>
```

- 解析后即执行 spec §14.3 约束（**校验并截断，不报错**）：cards >3 截前 3；`Page` 卡若与其他卡同回合则只保留它；sketch 总点数 >2000 时整卡丢弃（eprintln 警告）；text 按 `max_text_chars`（参数，默认 6）截断；stamp 名不在 8 枚举内则丢卡；count.n 夹在 1..=20。
- Consumes: 无。

- [ ] **Step 1: Cargo.toml 加依赖**（`[dependencies]` 段末尾）

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: 写失败测试**（`cards.rs` 尾部；文件先只写 `//!` 头 + tests 也行，直接进 Step 3 一并写更实际——保持一次编译）

核心用例（全部放 `#[cfg(test)] mod tests`）：

```rust
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
    fn garbage_json_is_an_error_not_a_panic() {
        assert!(parse_turn_response("not json").is_err());
        assert!(parse_turn_response(r#"{"turn_id":1}"#).is_err());
    }
```

- [ ] **Step 3: 实现**

`cards.rs` 结构：用 serde 解析成宽松的原始层（`RawResponse`/`RawCard`，全部 `Option` + `serde_json::Value` 兜底），再手工转严格层并施加约束——这样服务器多发字段/漏发字段都不炸。骨架：

```rust
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

#[derive(Debug, Clone, Copy)] pub enum Place { NearNewInk, NearAnchor, BlankArea, Margin, FullPage }
// … Size / Pace / PageAction / CountStyle / TraceKind / TraceGuide 同接口块 …

#[derive(Deserialize)]
struct RawResponse {
    turn_id: Option<String>,
    spoken_text: Option<String>,
    paper_cards: Option<Vec<serde_json::Value>>,
    page_action: Option<String>,
    memory_tags: Option<Vec<String>>,
}
```

每张卡用 `serde_json::from_value::<RawCard>` 单独解析，单卡失败只丢该卡。`parse_turn_response` 顶层 JSON 解析失败或缺 `turn_id`/`paper_cards` 字段返回 `Err(String)`。枚举字符串匹配用小写比对（`"near_new_ink"` 等，未知值取默认并 eprintln）。约束顺序：逐卡转换（丢非法）→ page 卡独占检查（有 page 则只留第一张 page）→ 截断至 3 张。`content` 截断按 `chars()` 计数（中文安全）。

`main.rs` 模块声明区（`mod display;` 之后）插入 `mod cards;`（保持字母序）。**本任务不在运行路径调用它**——`#[allow(dead_code)]` 加在模块顶（Task 12 移除）。

- [ ] **Step 4: 跑测试确认通过**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/Cargo.toml device/riddle/Cargo.lock device/riddle/src/cards.rs device/riddle/src/main.rs
git commit -m "Add paper_cards data model with spec clamps"
```

---

### Task 5: stamps.rs——8 个奖励符号的矢量笔画库

**Files:**
- Create: `device/riddle/src/stamps.rs`
- Modify: `device/riddle/src/main.rs`（`mod stamps;`）

**Interfaces:**
- Produces: `pub fn strokes_for(name: &str) -> Option<Vec<Vec<(f32, f32)>>>`——归一化 0..1 坐标（相对符号自身正方包围盒），笔画有序、每笔 ≥2 点；名字即 `cards::STAMP_NAMES` 八个。
- Consumes: 无（几何用标准库 `f32::sin/cos`）。

- [ ] **Step 1: 写失败测试**

```rust
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
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现**（几何全部参数化生成，无魔法点表）

```rust
//! Built-in reward stamps: the eight celebration symbols the server may name
//! in a `stamp` card. Kept on-device so the most frequent feedback renders
//! instantly and identically regardless of the model behind the server.
//! All geometry is normalized to a unit box, y down, and generated
//! parametrically so tweaking proportions stays a one-line change.

fn circle(cx: f32, cy: f32, r: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

pub fn strokes_for(name: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    let s = match name {
        // 一笔五角星：10 个顶点交替外圈/内圈，起于顶端，闭合。
        "star" => {
            let pts: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI * 2.0 / 10.0
                        * 2.0; // 跳点连线成五角星
                    let r = if i % 2 == 0 { 0.48 } else { 0.19 };
                    (0.5 + r * a.cos(), 0.5 + r * a.sin())
                })
                .collect();
            vec![pts]
        }
        // 五瓣小红花 + 花心圆。
        "flower" => {
            let mut v: Vec<Vec<(f32, f32)>> = (0..5)
                .map(|p| {
                    let base = p as f32 / 5.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    (0..=14)
                        .map(|i| {
                            let t = i as f32 / 14.0 * std::f32::consts::TAU;
                            let r = 0.16 * (1.0 - t.cos()) / 2.0 + 0.13;
                            let a = base + (t.sin()) * 0.5;
                            (0.5 + (0.16 + r) * a.cos() * 0.9, 0.5 + (0.16 + r) * a.sin() * 0.9)
                        })
                        .collect()
                })
                .collect();
            v.push(circle(0.5, 0.5, 0.11, 12));
            v
        }
        // 心：两段对称贝塞尔感弧线拼一笔。
        "heart" => {
            let mut pts = Vec::new();
            for i in 0..=24 {
                let t = i as f32 / 24.0 * std::f32::consts::PI;
                // 经典心形参数方程，缩放进单位盒（y 翻转成 y 向下）。
                let x = 16.0 * t.sin().powi(3);
                let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                pts.push((0.5 + x / 36.0, 0.45 - y / 36.0));
            }
            let left: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (1.0 - x, y)).rev().collect();
            let mut whole = pts;
            whole.extend(left.into_iter().skip(1));
            vec![whole]
        }
        // 笑脸：脸圆 + 两只眼点(短竖) + 微笑弧。
        "smiley" => vec![
            circle(0.5, 0.5, 0.46, 28),
            vec![(0.36, 0.36), (0.36, 0.44)],
            vec![(0.64, 0.36), (0.64, 0.44)],
            arc(0.5, 0.52, 0.24, 0.35, std::f32::consts::PI - 0.35, 12),
        ],
        // 大对勾：两段直线一笔完成。
        "check" => vec![vec![(0.12, 0.55), (0.4, 0.82), (0.88, 0.2)]],
        // 太阳：圆 + 8 根光芒。
        "sun" => {
            let mut v = vec![circle(0.5, 0.5, 0.26, 24)];
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                v.push(vec![
                    (0.5 + 0.33 * a.cos(), 0.5 + 0.33 * a.sin()),
                    (0.5 + 0.47 * a.cos(), 0.5 + 0.47 * a.sin()),
                ]);
            }
            v
        }
        // 月牙：外弧 + 内弧一笔闭合。
        "moon" => {
            let mut outer = arc(0.5, 0.5, 0.42, -1.1, 1.1 + std::f32::consts::PI, 20);
            let inner = arc(0.66, 0.5, 0.34, 1.05 + std::f32::consts::PI, -1.05, 16);
            outer.extend(inner);
            vec![outer]
        }
        // 气球：椭圆 + 尾结 + 波浪线绳。
        "balloon" => {
            let body: Vec<(f32, f32)> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    (0.5 + 0.3 * a.cos(), 0.38 + 0.36 * a.sin())
                })
                .collect();
            let string: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let t = i as f32 / 10.0;
                    (0.5 + 0.05 * (t * 9.0).sin(), 0.74 + 0.24 * t)
                })
                .collect();
            vec![body, vec![(0.46, 0.74), (0.54, 0.74)], string]
        }
        _ => return None,
    };
    Some(s)
}
```

（`star` 的跳点公式实现者需自证正确：外圈第 i 个顶点角度 `-90° + i*144°`，i=0..=5 闭合即成五角星，实现成一笔 6 点折线即可，比上面伪式更简单——测试只验证单位盒与点数，允许实现者用更简单的五角星连法。若任何点算出 <0 或 >1，clamp 到 [0,1]。）

`main.rs` 加 `mod stamps;`（字母序，`mod script;` 之后）。模块顶加 `#![allow(dead_code)]`？——不，Rust 模块内用 `#[allow(dead_code)]` 标注 `strokes_for`（Task 7 使用后移除）。

- [ ] **Step 4: 跑测试确认通过**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/stamps.rs device/riddle/src/main.rs
git commit -m "Add built-in vector stamp library"
```

---

### Task 6: layout.rs——墨迹覆盖图与摆放解析器

**Files:**
- Create: `device/riddle/src/layout.rs`
- Modify: `device/riddle/src/main.rs`（`mod layout;`）

**Interfaces:**
- Produces:

```rust
pub const CELL: usize = 25;              // 25px 格 → 1620/25=64.8 → 65 x 87 格
pub struct InkMap { cols: usize, rows: usize, cells: Vec<bool>, pub screen_w: usize, pub screen_h: usize }
impl InkMap {
    pub fn new(screen_w: usize, screen_h: usize) -> Self;              // 全空
    pub fn from_surface(surf: &Surface) -> Self;                       // luma<200 的格标脏
    pub fn mark_rect(&mut self, x: i32, y: i32, w: i32, h: i32);       // 渲染后登记占用
    pub fn coverage(&self) -> f32;                                     // 0..1 脏格比例
    pub fn find_spot(&self, want_w: i32, want_h: i32, seed: (i32, i32), margin: i32) -> Option<(i32, i32)>;
}
pub enum Anchor { Point(i32, i32), None }
pub fn resolve(map: &InkMap, place: &cards::Place, anchor: Anchor, want_w: i32, want_h: i32) -> Option<(i32, i32)>;
```

- `find_spot`：从 seed 所在格出发按环形（切比雪夫距离递增）扫描，找到第一个能容纳 `want_w×want_h` 且四周留 `margin`（默认调用方传 40）的全净矩形，返回其左上角像素坐标；越界区域视为脏。
- `resolve` 的 seed 规则：`NearNewInk`/`NearAnchor` 用传入 anchor 点（NearNewInk 的 anchor 由调用方给「新墨迹外接框右下角 + (60,60)」）；`BlankArea` seed = 屏幕中心；`Margin` 只在四边 120px 带内扫（上→下优先）；`FullPage` 直接返回 `Some((0,0))`（调用方自负责整页）。找不到返回 None（调用方缩档重试）。
- Consumes: `cards::Place`（Task 4）、`surface::Surface`。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 手工构造：把给定像素矩形标脏。
    fn map_with(dirty: &[(i32, i32, i32, i32)]) -> InkMap {
        let mut m = InkMap::new(1620, 2160);
        for &(x, y, w, h) in dirty {
            m.mark_rect(x, y, w, h);
        }
        m
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
        // 中央一大块墨迹；seed 在墨迹中心 → 必须绕到外面且留白 40px。
        let m = map_with(&[(600, 800, 400, 400)]);
        let (x, y) = m.find_spot(200, 200, (800, 1000), 40).unwrap();
        // 与脏区的间距 ≥ margin - CELL（格化容差）
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
        let m = map_with(&[(200, 200, 1200, 1700)]); // 中央大占用，只剩四边
        let got = resolve(&m, &crate::cards::Place::Margin, Anchor::None, 200, 100);
        let (x, y) = got.expect("margins are free");
        assert!(y <= 120 || y >= 2160 - 220 || x <= 120 || x >= 1620 - 320,
            "({x},{y}) is not in an edge band");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现**

要点：cells 按 `cols = screen_w.div_ceil(CELL)`、`rows = screen_h.div_ceil(CELL)`；`from_surface` 每格抽样 5×5 像素点（步长 CELL/5）而非全扫（65×87×25 采样 ≈ 14 万次 luma，够快也够准）；`find_spot` 把 want 尺寸+2*margin 换算成格数 `gw, gh`，以 seed 格为中心按半径 0..max(cols,rows) 的方形环遍历左上角候选格，逐格用二维前缀和（构造时算好 `Vec<u32>` 积分图，`mark_rect` 后失效重算——mark 次数少，简单起见每次 find_spot 现算积分图）检查区域全净；返回像素坐标 `(gx*CELL + margin, gy*CELL + margin)`。`resolve` 按接口块规则分派；`Margin` 实现：依次尝试上带 seed(cx,60)、下带 seed(cx, screen_h-180)、左右带，仍用 find_spot 但把带外全部视脏（临时 clone map + mark 带外——简单直接，map 才 5.6k 格）。

模块暂标 `#[allow(dead_code)]`（Task 7 接线后移除）。

- [ ] **Step 4: 跑测试确认通过**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/layout.rs device/riddle/src/main.rs
git commit -m "Add ink-coverage map and placement resolver"
```

---

### Task 7: cardrender.rs——卡片 → 渲染计划（含描红/田字格/点数）

**Files:**
- Create: `device/riddle/src/cardrender.rs`
- Modify: `device/riddle/src/main.rs`（`mod cardrender;`）

**Interfaces:**
- Produces:

```rust
pub struct RenderPlan {
    pub strokes: Vec<Vec<(i32, i32)>>,   // 屏幕坐标，有序
    pub color: u16,                       // BLACK 或 FADED（trace 模板/田字格用 FADED）
    pub points_per_frame: i32,            // pace: Normal=26, Slow=8（沿用现有 26 节奏）
    pub region: crate::fb::BBox,
}
/// 把一张卡解析成 0..=N 个渲染计划（stamp×count 会出多个；放不下时逐档缩小，
/// 仍放不下返回空并 eprintln）。成功后调用方须 map.mark_rect(region)。
pub fn plan_card(
    card: &cards::Card,
    map: &mut layout::InkMap,
    font: &script::FontStack<'_>,
    new_ink_anchor: (i32, i32),
) -> Vec<RenderPlan>;
```

- 尺寸表（相对 1620 页宽）：`Size::S=243px, M=486px, L=810px`（正方目标盒；text 例外——S/M/L 对应字号 44/56/84px，宽度由内容+wrap 决定，上限 1380px）。
- Consumes: `cards::{Card, Place, Size, Pace, …}`（Task 4 签名）、`stamps::strokes_for`（Task 5）、`layout::{InkMap, resolve, Anchor}`（Task 6）、`script::{wrap, rasterize_line, thin, trace, FontStack}`（现有）、`surface::{BLACK, FADED}`、`fb::BBox`。

- [ ] **Step 1: 写失败测试**（测试可用 include_bytes 字体构造 FontStack，仿 `script.rs` 测试）

```rust
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
}
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现**

各类型要点：

- 公共入口：`size_box(size) -> i32`（243/486/810）；`seed` = 按 place 与 `new_ink_anchor`/`anchor_norm×(1620,2160)` 求点；`layout::resolve` 拿左上角，None 时按 L→M→S 缩档循环（text 则字号 84→56→44），全败 `eprintln!("riddle: card dropped (no room)"); return vec![]`。
- `Text`：`script::wrap`（max = min(1380, 剩余可用宽——直接用目标盒宽 2×size_box 上限 1380)）→ 逐行 `rasterize_line + thin + trace` → 仿 `main.rs::plan_reply` 的行距/居中逻辑，但左对齐于解析点。颜色 BLACK。
- `Sketch`：strokes 逐点 `(x*bw, y*bh)` 平移到盒；等比：算原始纵横比（点云 bbox），fit 进正方盒。BLACK。
- `Stamp`：`stamps::strokes_for(name)` 缩放到 S 盒（无论卡 size，多个并排：x 方向步进 `box+24px`，超过 3 个折行）；每个实例一个 RenderPlan（region 各自），先 resolve 一个容纳整排的区域再切分。BLACK。
- `Count`：Dots = n 个直径 60px 圆圈横排 + 圈下小数字（`rasterize_line` 24px）；Tally = 每组 4 竖 1 斜；Numbers = 文本 "1 2 3 …"。BLACK。
- `Trace`：先格后字。TianGrid：外框矩形 4 边 + 横竖中线（FADED, 每边一笔折线）；字模板：`rasterize_line(content, 盒高×0.8)` → thin → trace → FADED。Shape：复用 `stamps` 里 circle/arc 几何思路，在本模块写 `shape_strokes(kind:&str)`（circle/square/triangle/star/heart/wave，直接生成归一化折线再缩放）。全部 FADED（孩子拿黑笔描在上面）。
- `Page`：忽略 map 搜索，逐条 `rect_norm×(1620,2160)` 递归调子卡渲染（子卡 place 强制视为已定位：直接以 rect 为盒）。返回全部子计划。
- 每个返回的 plan 调用方负责 `map.mark_rect`；但**本函数内**多实例（stamp×3、count dots）逐个 resolve 时要即时 `map.mark_rect` 防自叠。
- 移除 Task 4/5/6 留下的 `#[allow(dead_code)]`（本模块把它们都用上了；cardrender 自身标注，Task 9/12 接线后移除）。

- [ ] **Step 4: 跑测试确认通过**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/cardrender.rs device/riddle/src/main.rs device/riddle/src/{cards.rs,stamps.rs,layout.rs}
git commit -m "Render paper cards into stroke plans"
```

---

### Task 8: `--cards-test` 离屏渲染工具 + 演示 fixture

**Files:**
- Create: `device/riddle/test-data/cards-demo.json`
- Modify: `device/riddle/src/main.rs`（新 CLI 分支 + 离屏渲染函数 + USAGE 更新）

**Interfaces:**
- Produces: `riddle --cards-test <fixture.json> [out.png]`——无设备、无网络：解析 fixture（含可选 `"ink"` 假孩子笔迹 + `"response"` /turn 响应对象），在内存 Surface(1620×2160, Rgb32) 上先画假墨迹、再布局渲染全部卡片（跳过动画直接落墨），输出整页 PNG（默认 `/tmp/riddle-cards.png`）。stderr 打印每张卡的落点与丢弃原因。退出码 0=至少渲染一张卡。
- Consumes: Task 4/6/7 全部接口 + `png` crate（已有）。

- [ ] **Step 1: 写 fixture**（`test-data/cards-demo.json`，覆盖 6 类型中的 5 类 + 一条假墨迹；page 卡单独 `test-data/cards-page.json`）

```json
{
  "ink": [[[520, 620], [780, 600], [900, 760], [700, 900], [520, 620]]],
  "response": {
    "v": 1,
    "turn_id": "demo-1",
    "spoken_text": "哇，你画了一座小山！我们一起数三颗星星。",
    "paper_cards": [
      { "type": "stamp", "name": "star", "count": 3, "place": "near_new_ink", "size": "S" },
      { "type": "text", "content": "山", "place": "near_anchor", "anchor_norm": [0.45, 0.36], "size": "L" },
      { "type": "sketch", "place": "blank_area", "size": "M", "pace": "slow",
        "strokes": [[[0.1, 0.9], [0.3, 0.4], [0.5, 0.9]], [[0.4, 0.65], [0.6, 0.65]], [[0.5, 0.9], [0.7, 0.3], [0.9, 0.9]]] }
    ],
    "page_action": "none",
    "memory_tags": ["mountain", "stars"]
  }
}
```

`cards-page.json`：

```json
{
  "ink": [],
  "response": {
    "v": 1, "turn_id": "demo-2", "spoken_text": "来描一描「山」字！",
    "paper_cards": [
      { "type": "page", "layout": [
        { "card": { "type": "trace", "kind": "hanzi", "content": "山", "guide": "tian_grid" }, "rect_norm": [0.15, 0.08, 0.7, 0.35] },
        { "card": { "type": "trace", "kind": "shape", "content": "circle", "guide": "none" }, "rect_norm": [0.15, 0.5, 0.3, 0.22] },
        { "card": { "type": "stamp", "name": "smiley", "count": 1 }, "rect_norm": [0.6, 0.5, 0.25, 0.22] }
      ] }
    ],
    "page_action": "new_page", "memory_tags": ["tracing"]
  }
}
```

- [ ] **Step 2: 实现 CLI**（此任务测试即「跑出 PNG」，属集成验证，不写单测；核心逻辑已被 Task 4-7 单测覆盖）

`main.rs`：

- `USAGE` 增加一行 `riddle --cards-test F [OUT]  render a /turn fixture to a PNG (no device needed)`。
- `main()` match 增加分支 `Some("--cards-test") => { … std::process::exit(cards_test(&args)); }`。
- 新函数（放 `oracle_test` 旁）：

```rust
fn cards_test(args: &[String]) -> i32 {
    let Some(path) = args.get(2) else { eprintln!("usage: riddle --cards-test fixture.json [out.png]"); return 2 };
    let out = args.get(3).map(String::as_str).unwrap_or("/tmp/riddle-cards.png");
    let raw = match std::fs::read_to_string(path) { Ok(v) => v, Err(e) => { eprintln!("read {path}: {e}"); return 2 } };
    let fixture: serde_json::Value = match serde_json::from_str(&raw) { Ok(v) => v, Err(e) => { eprintln!("bad json: {e}"); return 2 } };
    let resp = match crate::cards::parse_turn_response(&fixture["response"].to_string()) {
        Ok(r) => r, Err(e) => { eprintln!("bad response: {e}"); return 2 }
    };
    let primary = FontRef::try_from_slice(FONT_TTF).expect("font");
    let font = script::FontStack::new(primary, None);

    let mut buf = vec![0u8; SCREEN_W * SCREEN_H * 4];
    let mut surf = Surface::new(buf.as_mut_ptr(), buf.len(), SCREEN_W, SCREEN_H, SCREEN_W * 4, surface::PixFmt::Rgb32);
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);

    // 假孩子墨迹（折线，笔径 3）。
    let mut anchor = (SCREEN_W as i32 / 2, SCREEN_H as i32 / 2);
    if let Some(strokes) = fixture["ink"].as_array() {
        for s in strokes {
            let pts: Vec<(i32, i32)> = s.as_array().map(|v| v.iter().filter_map(|p| {
                Some((p[0].as_i64()? as i32, p[1].as_i64()? as i32))
            }).collect()).unwrap_or_default();
            for w in pts.windows(2) { surf.brush_line(w[0].0, w[0].1, w[1].0, w[1].1, 3, BLACK); }
            if let Some(&(x, y)) = pts.last() { anchor = (x + 60, y + 60); }
        }
    }

    let mut map = layout::InkMap::from_surface(&surf);
    let mut rendered = 0;
    for card in &resp.paper_cards {
        let plans = cardrender::plan_card(card, &mut map, &font, anchor);
        if plans.is_empty() { eprintln!("card dropped: {card:?}"); continue; }
        for p in plans {
            for stroke in &p.strokes {
                for w in stroke.windows(2) { surf.brush_line(w[0].0, w[0].1, w[1].0, w[1].1, 2, p.color); }
                if let Some(&(x, y)) = stroke.first() { surf.stamp(x, y, 2, p.color); }
            }
            let (x, y, w, h) = p.region.rect();
            map.mark_rect(x, y, w, h);
            eprintln!("card at ({x},{y}) {w}x{h} color={:#06x}", p.color);
            rendered += 1;
        }
    }

    if let Err(e) = write_page_png(&surf, out) { eprintln!("png: {e}"); return 1; }
    eprintln!("wrote {out} ({rendered} plans, coverage {:.2})", map.coverage());
    (rendered == 0) as i32
}

/// Full-page grayscale PNG (2x downscale → 810x1080, plenty for eyeballing).
fn write_page_png(surf: &Surface, path: &str) -> std::io::Result<()> {
    let (w, h) = (surf.w / 2, surf.h / 2);
    let mut gray = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0u32;
            for dy in 0..2 { for dx in 0..2 { acc += surf.luma((x * 2 + dx) as i32, (y * 2 + dy) as i32) as u32; } }
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
```

- [ ] **Step 3: 集成验证**（Docker 内跑二进制）

```bash
docker run --rm -v "$PWD/device/riddle:/work" -w /work -v riddle-cargo-registry:/usr/local/cargo/registry rust:1-bookworm \
  sh -c "cargo run --quiet -- --cards-test test-data/cards-demo.json /work/target/cards-demo.png && cargo run --quiet -- --cards-test test-data/cards-page.json /work/target/cards-page.png"
```

Expected: 两个 PNG 生成、stderr 列出每张卡落点、exit 0。把 `device/riddle/target/cards-demo.png` 用 Read 工具（或让用户）目检：山形墨迹保留、三颗星在其旁、大「山」字在锚点旁、简笔画在空白处、page fixture 出田字格+淡墨「山」。

- [ ] **Step 4: 全量测试仍绿**

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/main.rs device/riddle/test-data/
git commit -m "Add offscreen cards-test harness with demo fixtures"
```

---

### Task 9: 共画状态机 I——不吃墨、角落思考点、思考期可继续画

**Files:**
- Modify: `device/riddle/src/main.rs`（状态机、提交路径）

**Interfaces:**
- Produces: 行为变化——提交后孩子墨迹保留（删除 Drinking 常规路径）；思考指示从页面中央墨点改为右上角 (SCREEN_W-90, 90) r=9 呼吸点；Listening **和 Thinking** 期间都能落墨。`State::Drinking` 变体删除；`ink::dissolve_pass` 仅 conjure/FadingReply 路径继续使用。
- Consumes: Task 1（idle_commit()）。

- [ ] **Step 1: 行为红线先写下**（无法单测状态机——评审以代码走查 + Task 8 工具与后续真机验证为准；本任务以编译通过 + 全量测试绿 + 走查清单为门）

走查清单（实现后自查并写进 commit message）：
1. 提交路径不再调用 `user_ink.clear()`（除换页外）；
2. `State::Drinking` 全删，提交直接进 `Thinking`；
3. Thinking 分支的 pen 处理与 Listening 相同（落墨 + 更新 bbox + `stylus_tapped` 逻辑不破坏 Help/Conjure 的解散语义）；
4. 思考点绘制/擦除只动右上角 28×28 区域，不再动页面中央；
5. `region_all_white` 检查改用「本回合新增笔画的 bbox」（见 Step 2 的 new-ink 追踪）。

- [ ] **Step 2: 实现**

关键改动（逐处）：

a. **新墨迹追踪**：`run()` 局部变量加 `let mut committed_strokes: usize = 0;`（上次提交时 `user_ink.stroke_list().len()` 的快照）。提交条件由 `!user_ink.is_empty()` 改为：

```rust
let new_strokes = user_ink.stroke_list().len().saturating_sub(committed_strokes);
// ≥2 笔新墨迹才值得回应（spec §3 防误触发）
Some(t) if !pen_down && t.elapsed() >= idle_commit() && new_strokes >= 2 => {
```

b. **新墨迹 bbox**：提交时计算 `let new_bbox = bbox_of(&user_ink.stroke_list()[committed_strokes..]);`（新增辅助函数遍历 4 元组求 BBox）。`region_all_white(&surf, new_bbox)` 替代原全量 bbox 判断；`?` 手势检测传 `&user_ink.stroke_list()[committed_strokes..]`（只看新笔画——但 `help::looks_like_question_mark` 接受 `&[Vec<(i32,i32,i32)>]`，需同步适配 4 元组切片，函数体内解构补 `_`）。

c. **提交后**：`turn_strokes = user_ink.stroke_list()[committed_strokes..].to_vec(); committed_strokes = user_ink.stroke_list().len();` PNG 仍整页 `user_ink.to_png(&surf, PNG_PATH)`（服务器要看全页语境）。**删除 Drinking 状态构造**，直接：

```rust
State::Thinking { rx, pulse: Instant::now(), blot_on: false, since: Instant::now() }
```

（`State` 枚举删除 `Drinking` 变体及其 match 分支。）

d. **思考点**：Thinking 分支内三处 `SCREEN_W/2±14 / SCREEN_H/2±14` 座标全部替换为右上角常量：

```rust
const THINK_X: i32 = SCREEN_W as i32 - 90;
const THINK_Y: i32 = 90;
```

擦除/绘制矩形 `(THINK_X-14, THINK_Y-14, 28, 28)`，stamp 中心 `(THINK_X, THINK_Y)` r=9。

e. **Thinking 期落墨**：pen drain 的 `match state` 里给 `State::Thinking { .. }` 加与 Listening 相同的落墨臂（抽个小闭包/内联复制均可，注意 Thinking 无 `last_pen` 可更新——落墨即可，不重置提交计时）。

f. Lingering 的「触笔即开始淡出」臂**保留不动**（本任务不改回应留存——Task 10 处理）。

- [ ] **Step 3: 全量测试绿 + `--cards-test` 仍出图**（回归）

- [ ] **Step 4: Commit**

```bash
git add device/riddle/src/main.rs device/riddle/src/help.rs
git commit -m "Co-drawing I: keep the child's ink, corner thinking dot, draw while thinking"
```

---

### Task 10: 共画状态机 II——回应就地摆放、不再淡出、静默规则

**Files:**
- Modify: `device/riddle/src/main.rs`

**Interfaces:**
- Produces: 文本回复经 `layout::resolve` 放进空白区（不再固定居中/往下推）；回应写完后**留在纸上**（常规回合不再进 `FadingReply`；`Lingering` 删除）；回应完成 → `committed_strokes` 同步 → 回到 Listening 等新墨迹（静默规则天然成立：无新笔画不会再触发）。旧的 conjure 浮现路径完整保留（含淡出）。
- Consumes: Task 6/9。

- [ ] **Step 1: 实现**

a. **回复摆放**：`plan_reply` 增参 `origin: Option<(i32, i32)>`：Some 时首行左上角对齐 origin（x 不再按屏宽居中，行宽上限 = `min(1380, SCREEN_W - origin.x - 60)`）。提交成功进入 Thinking 前先算好摆放：

```rust
let mut map = layout::InkMap::from_surface(&surf);
let anchor = (new_bbox.x1 + 60, new_bbox.y0);
let reply_origin = layout::resolve(&map, &cards::Place::NearNewInk, layout::Anchor::Point(anchor.0, anchor.1), 700, 320)
    .or_else(|| layout::resolve(&map, &cards::Place::BlankArea, layout::Anchor::None, 700, 320));
```

`reply_origin` 存进 Thinking→Replying 的路径（`State::Thinking` 加字段 `origin: Option<(i32,i32)>` 透传；第一个 `Event::Ink` 时 `plan_reply(&font, &text, None, origin)`）。origin 为 None（整页几乎满）时回退现状逻辑（居中偏上），并在 stderr 记 `page full`。

b. **不淡出**：Replying 完成分支：

```rust
if plan.stroke_i >= plan.strokes.len() && rx.is_none() {
    if !turn_failed && !turn_reply.is_empty() { /* store.append 原样 */ }
    turn_strokes = Vec::new();
    // 回应留在纸上；DouDou 的墨也算页面占用——同步 committed 快照即可，
    // 児童再画新笔画才会开启下一回合（静默规则）。
    State::Listening { last_pen: None }
}
```

删除 `State::Lingering` 与 `State::FadingReply` 变体 + 分支（conjure 的 MemoryShown 淡出走的是 `paste_rect` 恢复，不受影响；确认 `FadingReply` 仅被 Lingering/pen-in-Lingering 引用后整链删除）。pen drain 里 `State::Lingering` 臂删除。

c. **oracle 无法回答时**（`oracle_excuse` 路径）：错误文案**不再写上纸**（spec §12：安静失败）——改为 `eprintln!` + 直接回 Listening。保留 `oracle.is_none()` 分支同样处理。`oracle_excuse` 函数与调用一并删除。

- [ ] **Step 2: 全量测试绿**（`plan_reply` 新签名会牵动 conjure 内调用——那两处传 `None` origin 维持居中旧观感）

- [ ] **Step 3: Commit**

```bash
git add device/riddle/src/main.rs
git commit -m "Co-drawing II: place replies in blank space and let them stay"
```

---

### Task 11: 共画状态机 III——页满自动换新纸

**Files:**
- Modify: `device/riddle/src/main.rs`

**Interfaces:**
- Produces: 提交时若 `InkMap::coverage() > 0.55`，本回合照常送出；回应写完后执行**换页**：整页白 + `full_refresh` + `user_ink.clear()` + `committed_strokes = 0`（下一页从头计时间戳）。stderr 记 `page turned (coverage X)`。阈值 env `RIDDLE_PAGE_FULL`（默认 0.55）。
- Consumes: Task 6/10。

- [ ] **Step 1: 实现**

提交路径算 coverage（已建 map，顺手 `map.coverage()`）存 `page_full: bool` 随状态透传；Replying 完成分支在回 Listening 前：

```rust
if page_full {
    eprintln!("riddle: page turned (coverage past threshold)");
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.full_refresh(surf.w, surf.h);
    user_ink.clear();
    committed_strokes = 0;
}
```

（memory 已按回合存档，换页无需额外持久化。）

- [ ] **Step 2: 全量测试绿；Step 3: Commit**

```bash
git commit -am "Co-drawing III: auto page turn when the sheet fills up"
```

---

### Task 12: /turn 结构化客户端（env 开关 + mock 文件模式）

**Files:**
- Create: `device/riddle/src/turn.rs`
- Modify: `device/riddle/src/main.rs`（提交路径分流 + 卡片回合渲染状态）
- Modify: `device/riddle/oracle.env.example`

**Interfaces:**
- Produces:

```rust
pub struct TurnRequestMeta<'a> {
    pub turn_id: &'a str,
    pub trigger: &'a str,                     // "pen_idle"
    pub page_png_b64: &'a str,
    pub new_strokes: &'a [Vec<(i32, i32, i32, u32)>],
    pub ink_coverage: f32,
    pub page_id: &'a str,
    pub profile: &'a str,                     // env RIDDLE_PROFILE，默认 "child_3_4"
}
pub fn build_request_json(m: &TurnRequestMeta) -> String;   // spec §14.1 形状；坐标/压感归一化，t 原样
/// RIDDLE_TURN_MOCK=path → 读文件；否则 POST RIDDLE_TURN_URL。在调用方线程外用。
pub fn fetch(json_body: String, tx: std::sync::mpsc::Sender<Result<cards::TurnResponse, String>>);
pub fn turn_mode_enabled() -> bool;           // RIDDLE_TURN_URL 或 RIDDLE_TURN_MOCK 已设
```

- main.rs：提交时若 `turn_mode_enabled()`，走新路径——整页 PNG（新增 `Ink::page_png_b64(&surf)`? 不必：`cards_test` 的 `write_page_png` 提炼成可复用 `page_png_bytes(&surf) -> Vec<u8>`，base64 用 oracle.rs 已有的 base64 实现——`oracle::tests::base64_matches_known_vector` 表明有现成 `b64` 函数，`pub(crate)` 化复用）；新状态 `State::CardTurn { rx }` 收到 `TurnResponse` 后逐卡 `plan_card` → 复用 Replying 的逐笔动画机制（把多个 RenderPlan 串成队列 `Vec<RenderPlan>` 逐个画，`points_per_frame` 按 plan）；`page_action==NewPage` → 渲染完执行 Task 11 的换页块。错误/超时（30s）→ 安静回 Listening（仅 stderr）。
- Consumes: Task 3（时间戳笔画）、4（parse_turn_response）、6/7（布局渲染）、11（换页块）。

- [ ] **Step 1: 写失败测试**（`turn.rs` tests）

```rust
    #[test]
    fn request_json_matches_spec_shape() {
        let strokes = vec![vec![(810, 1080, 3, 0u32), (972, 1080, 4, 12)]];
        let m = TurnRequestMeta {
            turn_id: "t-1", trigger: "pen_idle", page_png_b64: "QUJD",
            new_strokes: &strokes, ink_coverage: 0.42, page_id: "p-1", profile: "child_3_4",
        };
        let v: serde_json::Value = serde_json::from_str(&build_request_json(&m)).unwrap();
        assert_eq!(v["turn_id"], "t-1");
        assert_eq!(v["trigger"], "pen_idle");
        assert_eq!(v["page_png"], "QUJD");
        assert_eq!(v["page_state"]["ink_coverage"].as_f64().unwrap(), 0.42f64 as f64);
        assert_eq!(v["device_profile"]["profile"], "child_3_4");
        assert_eq!(v["device_profile"]["screen"][0], 1620);
        let p0 = &v["new_strokes"][0][0];
        assert!((p0[0].as_f64().unwrap() - 0.5).abs() < 0.01, "x normalized");
        assert!((p0[2].as_f64().unwrap() - 3.0 / 4096.0 * 4096.0 / 4096.0).abs() < 1.0); // 压感 0..1
        assert_eq!(p0[3], 0);
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
```

（压感断言实现者按最终归一化公式校正：`p/4096` 保留 3 位小数即可。）

- [ ] **Step 2: 跑测试确认失败 → Step 3: 实现**

`build_request_json` 用 serde_json::json! 宏构造；new_strokes 点序列化为 `[x/1620, y/2160, p/4096, t]` 各留 4 位小数（json! 里先 round）。`fetch`：spawn thread；MOCK 路径读文件字符串→parse_turn_response→send；URL 路径 `ureq::post(&url).set("content-type","application/json").timeout(Duration::from_secs(30)).send_string(&body)` → 读响应体 → parse → send。任何错误 `let _ = tx.send(Err(e.to_string()));`。

`main.rs` 接线：提交分支开头 `if turn::turn_mode_enabled() { … State::CardTurn { rx, page_full } }`；`State::CardTurn` 分支：try_recv → Ok(resp) → 逐卡 plan_card（map 现建）→ `State::DrawingCards { plans, plan_i, point_i, next, page_full: page_full || matches!(resp.page_action, PageAction::NewPage) }`；DrawingCards 帧循环仿 Replying 的 budget 写法但 budget=当前 plan 的 `points_per_frame`，全部画完 → 执行换页块（若 page_full）→ Listening。turn_id 生成沿用现有 unix 秒；page_id 用「页首次落墨的 unix 秒」（新变量，换页时刷新）。memory：卡片回合的 `reply` 存 `spoken_text`（供 catalog 检索），strokes 照旧。

`oracle.env.example` 追加：

```sh
# 二期 /turn 结构化接口（未设则完全走现有 OpenAI 兼容通道）。
#export RIDDLE_TURN_URL="http://<Mac IP>:8787/turn"
# 本地联调：跳过 HTTP，直接用文件当响应。
#export RIDDLE_TURN_MOCK="/home/root/riddle-data/mock-turn.json"
#export RIDDLE_PROFILE="child_3_4"
```

- [ ] **Step 4: 全量测试绿 + 集成验证**：`--cards-test` 两个 fixture 出图不变；（可选真机验证留给用户：设 `RIDDLE_TURN_MOCK` 指向 demo response，在设备上画两笔停 6 秒应看到星星+大字+简笔画就地画出。）

- [ ] **Step 5: Commit**

```bash
git add device/riddle/src/turn.rs device/riddle/src/main.rs device/riddle/oracle.env.example
git commit -m "Add /turn structured client with mock mode and card-drawing state"
```

---

## 里程碑验收

- **A（Task 1-3）**：可立即交付真机——用现有 build 流程重编部署即可；行为差异仅停笔时长、退出手势、strokes 文件多一列。
- **B（Task 4-8）**：`--cards-test` 出的两张 PNG 是给产品负责人的视觉验收物（星星/大字/简笔画/田字格描红各就各位、互不叠墨、不压假墨迹）。
- **C（Task 9-12）**：`RIDDLE_TURN_MOCK` 真机演示 = 二期交互的可触摸原型；服务器 /turn 上线后仅需设 `RIDDLE_TURN_URL`。

## Self-Review 记录

1. **Spec 覆盖**：§3 双通道触发——语音触发/commit_now 依赖控制通道，明确列为非目标（传输选型待后端）；§3 其余（6s、≥2 笔、静默、思考不锁页、不淡出）→ Task 1/9/10。§4/§14.2 六卡片 → Task 4/5/7；§5 教学映射不涉及设备代码；§6 布局器 → Task 6；§7 手势 → Task 2（「?」停用属 profile 逻辑，依赖 set_profile 推送，暂保留现状并入非目标——3-4 岁画不出规整大「?」，风险可接受）；§8 换页 → Task 11（语音建议部分属手机端）；§14.1 → Task 12（trigger 仅 pen_idle，其余枚举值预留）；§14.4/14.5 → 非目标。
2. **占位符**：无 TBD/TODO；stamps 的 star 公式已注明允许实现者改用更简单连法（测试锁行为不锁实现）。
3. **类型一致性**：`stroke_list()` 4 元组贯穿 Task 3→9→12；`plan_card` 签名 Task 7 定义、Task 8/12 消费一致；`cards::Place` Task 4 定义、Task 6 resolve 消费一致。
