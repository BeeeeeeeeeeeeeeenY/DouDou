# 服务器：课时注入 + 精选彩图（image 卡内联） 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `/turn` (a) 注入当前课的脚本（DouDou 知道在上哪节课），(b) 让模型能按固定 subject 词表请求一张彩图，由**服务器**取自带的精选彩图、base64 内联成 image 卡的 `data`（模型永不自造图源）。这样平板纸面不再"秃"（有课时语境 + 特别时刻的彩图），且彩图由服务器按课题发出。

**Architecture:** Demo 计划 3（共 4），纯服务器（Python/FastAPI）。三块：① 精选彩图库——一次性用 Pillow 生成一组扁平大色块插画（配"形状课"：圆/方/三角 + 星/心/太阳/花/树）提交进仓库，运行时只读文件+base64、**无运行期图像库依赖**（`app/engine/art.py`）。② `cards.py` 把 image 卡从计划 1 的 `url` 占位切到 **`subject → 内联 data`**：模型只出 `{"type":"image","subject":"circle|..."}`，服务器校验 subject ∈ 词表后替换为 `{"type":"image","data":"<base64 PNG>"}`；`CARD_PROTOCOL` 增加"特别时刻可出 1 张彩图"的说明。③ `/turn` 注入课时——复用手机语音路已有的 `_active_current_lesson` + `render_lesson_script`，把当前课脚本作为 `lesson_context` 传入 `TurnInput`。

**Tech Stack:** Python 3.12 + FastAPI + SQLAlchemy 2；测试 pytest + respx。彩图生成用 Pillow，但**仅作一次性生成脚本**（`uv run --with pillow`），不进服务器运行期依赖。

**Spec:** `docs/superpowers/specs/2026-07-23-demo-lesson-color-codraw-design.md`（§4 image 卡、S2 课时注入、S3 精选彩图）。设备端（计划 2，已合入 main）认 `{"type":"image","data":"<base64 PNG>"}`——本计划服务器产出必须匹配。

## 现状锚点（实现者先读真实代码）

- 课时基础设施（`app/models.py`）：`Curriculum`（`status`，"active 全局唯一"；`current_lesson_id`，:86）、`Lesson`（`script_text`，:91-104）、`LessonRun`。解析当前课：`app/routers/phone.py:25` `_active_current_lesson(db) -> tuple[Curriculum, Lesson] | None`；脚本渲染：`app/engine/lesson.py:27` `render_lesson_script(script_text, prev_recap) -> str`、:43 `latest_recap(db, curriculum_id) -> str`。手机语音路的注入范例：`app/routers/phone.py:91-101`（`lesson_context = render_lesson_script(lesson.script_text, recap)`）。
- `/turn` 现状（`app/routers/turn.py`）：`TurnInput(source="tablet", text=..., image_png=..., device_protocol_suffix=CARD_PROTOCOL)`，**未传 `lesson_context`**（默认 ""）；runner 自己开 session（:69）。
- 卡片引擎（`app/engine/cards.py`）：`STAMP_NAMES`（:6，词表范式）、`MAX_CARDS=3`、`CARD_PROTOCOL`（:12-22，现写"本期只用 text 和 stamp"、**只字未提 image**）、`_clean_card` 的 `"image"` 臂（:83-87，读 `url`）、`build_cards` 的 ≤1 image 去重（`seen_image`，:104-112）。
- 无图像库（`pyproject.toml` 无 Pillow）；文件服务 `app/routers/files.py` 白名单 `images/audio`（本计划走内联 data，不经它）。
- 测试范式：`tests/test_phone_lesson.py:18-32` 的 `setup_course(client)`（POST provider+profile+activate → `POST /api/admin/curricula/seed-shapes01` → activate），之后 `GET /api/phone/current-lesson` 有当前课；上游用 respx + `sse_reply()` mock。conftest：`client`/`db`/`app`。

## Global Constraints

- image 卡最终形状必须是设备（计划 2）认的 **`{"type":"image","data":"<base64 PNG>", place?, size?}`**——`data` 是 base64 编码的 PNG 字节。**模型只输出 `subject`**（∈ 固定词表），`data` 一律由服务器填；服务器**绝不接受模型自造的 `url`/`data`**（安全）。
- image 词表 `IMAGE_SUBJECTS`（8 个，范式同 `STAMP_NAMES`）：`circle, square, triangle, star, heart, sun, flower, tree`。未知 subject / 无对应素材 → 丢该卡（`eprintln`/log + 不报错），与 cards.py 既有容错一致。每回合 ≤1 image（`build_cards` 已有去重）。
- 运行期**不引入图像库依赖**：`pyproject.toml` 不加 Pillow；彩图是**预生成并提交的 PNG 文件**，服务器只 `read_bytes` + base64。生成脚本用 `uv run --with pillow` 一次性跑。
- 课时注入：`/turn` 若有 active 课程则注入其 `render_lesson_script(...)`；无 active 课程则 `lesson_context` 为空、正常降级（不报错）。
- 中文输出 `ensure_ascii=False`。测试 `cd server && uv run pytest` 全绿、不回归既有。

## Build/Test 命令

服务器测试：`cd server && uv run pytest <path> -q`
一次性彩图生成：`cd server && uv run --with pillow python scripts/gen_art.py`（写入 `server/app/art/*.png`）

---

### Task 1: 精选彩图库（预生成 PNG + `art.py` 读取）

**Files:**
- Create: `server/scripts/gen_art.py`（一次性生成脚本，Pillow）
- Create: `server/app/art/*.png`（8 张，生成后提交）
- Create: `server/app/engine/art.py`（词表 + 读取/内联）
- Test: `server/tests/test_art.py`

**Interfaces:**
- Produces: `art.IMAGE_SUBJECTS: tuple[str,...]`（8）；`art.load_art_png(subject) -> bytes | None`；`art.subject_data_b64(subject) -> str | None`（base64 字符串）

- [ ] **Step 1: 生成脚本**

`server/scripts/gen_art.py`：
```python
"""一次性生成扁平大色块彩图（配形状课），输出到 app/art/<subject>.png。
运行：cd server && uv run --with pillow python scripts/gen_art.py
运行期服务器不依赖 Pillow——这些 PNG 提交进仓库，art.py 只读文件。"""
import math
from pathlib import Path
from PIL import Image, ImageDraw

W, H = 500, 400
OUT = Path(__file__).resolve().parent.parent / "app" / "art"
WHITE = (255, 255, 255)


def _star_points(cx, cy, r_out, r_in, n=5):
    pts = []
    for i in range(n * 2):
        r = r_out if i % 2 == 0 else r_in
        a = -math.pi / 2 + i * math.pi / n
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def draw(subject, d):
    cx, cy = W // 2, H // 2
    if subject == "circle":
        d.ellipse([cx - 150, cy - 150, cx + 150, cy + 150], fill=(220, 40, 40))
    elif subject == "square":
        d.rectangle([cx - 140, cy - 140, cx + 140, cy + 140], fill=(40, 90, 220))
    elif subject == "triangle":
        d.polygon([(cx, cy - 160), (cx - 160, cy + 130), (cx + 160, cy + 130)], fill=(40, 170, 70))
    elif subject == "star":
        d.polygon(_star_points(cx, cy, 170, 70), fill=(240, 200, 30))
    elif subject == "heart":
        d.ellipse([cx - 150, cy - 120, cx, cy + 30], fill=(230, 60, 130))
        d.ellipse([cx, cy - 120, cx + 150, cy + 30], fill=(230, 60, 130))
        d.polygon([(cx - 150, cy - 30), (cx + 150, cy - 30), (cx, cy + 160)], fill=(230, 60, 130))
    elif subject == "sun":
        for i in range(12):
            a = i * math.pi / 6
            d.line([cx + 100 * math.cos(a), cy + 100 * math.sin(a),
                    cx + 180 * math.cos(a), cy + 180 * math.sin(a)], fill=(240, 170, 20), width=18)
        d.ellipse([cx - 100, cy - 100, cx + 100, cy + 100], fill=(250, 200, 30))
    elif subject == "flower":
        for i in range(6):
            a = i * math.pi / 3
            px, py = cx + 90 * math.cos(a), cy + 90 * math.sin(a)
            d.ellipse([px - 55, py - 55, px + 55, py + 55], fill=(230, 90, 170))
        d.ellipse([cx - 50, cy - 50, cx + 50, cy + 50], fill=(250, 210, 40))
    elif subject == "tree":
        d.rectangle([cx - 30, cy + 40, cx + 30, cy + 170], fill=(140, 90, 40))
        d.ellipse([cx - 130, cy - 170, cx + 130, cy + 70], fill=(40, 160, 70))
    else:
        raise ValueError(subject)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    subjects = ["circle", "square", "triangle", "star", "heart", "sun", "flower", "tree"]
    for s in subjects:
        img = Image.new("RGB", (W, H), WHITE)
        draw(s, ImageDraw.Draw(img))
        img.save(OUT / f"{s}.png")
        print("wrote", OUT / f"{s}.png")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 跑生成脚本，产出 8 张 PNG**

Run: `cd server && uv run --with pillow python scripts/gen_art.py`
Expected: `server/app/art/` 下出现 `circle.png … tree.png` 共 8 张。`ls server/app/art/` 确认。

- [ ] **Step 3: 写失败测试**

`server/tests/test_art.py`：
```python
from app.engine import art


def test_all_subjects_load_as_png():
    assert len(art.IMAGE_SUBJECTS) == 8
    for s in art.IMAGE_SUBJECTS:
        b = art.load_art_png(s)
        assert b is not None and b[:4] == b"\x89PNG", f"{s} not a PNG"


def test_unknown_subject_is_none():
    assert art.load_art_png("dragon") is None
    assert art.subject_data_b64("dragon") is None


def test_subject_data_b64_roundtrips_to_png():
    import base64
    b64 = art.subject_data_b64("circle")
    assert b64 and base64.b64decode(b64)[:4] == b"\x89PNG"
```

- [ ] **Step 4: 实现 `art.py`**

`server/app/engine/art.py`：
```python
"""精选彩图库：subject → 预生成的 PNG（提交进仓库，见 scripts/gen_art.py）。
运行期只读文件 + base64，无图像库依赖。"""
import base64
from functools import lru_cache
from pathlib import Path

IMAGE_SUBJECTS: tuple[str, ...] = (
    "circle", "square", "triangle", "star", "heart", "sun", "flower", "tree",
)
_ART_DIR = Path(__file__).resolve().parent.parent / "art"


@lru_cache(maxsize=len(IMAGE_SUBJECTS))
def load_art_png(subject: str) -> bytes | None:
    if subject not in IMAGE_SUBJECTS:
        return None
    p = _ART_DIR / f"{subject}.png"
    return p.read_bytes() if p.is_file() else None


def subject_data_b64(subject: str) -> str | None:
    png = load_art_png(subject)
    return base64.b64encode(png).decode() if png is not None else None
```

- [ ] **Step 5: 测试通过 + Commit**

Run: `cd server && uv run pytest tests/test_art.py -q` → PASS。
```bash
git add server/scripts/gen_art.py server/app/art/*.png server/app/engine/art.py server/tests/test_art.py
git commit -m "feat(server): curated colour-art library (pre-generated PNGs + art.py)"
```

---

### Task 2: `cards.py` — image 卡 `subject → 内联 data` + 协议说明

**Files:**
- Modify: `server/app/engine/cards.py`
- Test: `server/tests/test_cards_engine.py`（追加）

**Interfaces:**
- Consumes: Task 1 的 `art.IMAGE_SUBJECTS` / `art.subject_data_b64`
- Produces: `build_cards` 输出的 image 卡形如 `{"type":"image","data":"<base64 PNG>", **common}`；模型侧只认 `subject`

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_cards_engine.py` 追加：
```python
def test_image_card_subject_becomes_inline_data():
    import base64, json
    reply = json.dumps({"spoken_text": "画好啦", "paper_cards": [
        {"type": "image", "subject": "circle", "size": "l"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1
    c = cards[0]
    assert c["type"] == "image" and "url" not in c
    assert base64.b64decode(c["data"])[:4] == b"\x89PNG"


def test_image_card_unknown_subject_dropped():
    import json
    reply = json.dumps({"spoken_text": "", "paper_cards": [
        {"type": "image", "subject": "dragon"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert cards == []


def test_at_most_one_image_still_holds_with_subjects():
    import json
    reply = json.dumps({"spoken_text": "", "paper_cards": [
        {"type": "image", "subject": "circle"},
        {"type": "image", "subject": "star"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1


def test_card_protocol_mentions_image_subjects():
    assert "image" in CARD_PROTOCOL
    assert "circle" in CARD_PROTOCOL
```
> 说明：计划 1 遗留的 `test_build_cards_at_most_one_image` / `test_build_cards_drops_image_without_url` 用的是旧 `url` 形状——本任务把 image 契约从 `url` 切到 `subject`，这两个旧用例**改写**为 subject 形状（`at_most_one` 用两个合法 subject；`drops_without_url` 改成 `drops_without_subject`：`{"type":"image"}` 无 subject → 丢弃）。实现者相应更新，保持"≤1 image"“无有效标识则丢弃”的语义。

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_cards_engine.py -q`
Expected: 新用例 FAIL（image 仍走 url）。

- [ ] **Step 3: 实现**

`server/app/engine/cards.py`：
1) 顶部 import：`from app.engine.art import IMAGE_SUBJECTS, subject_data_b64`
2) `_clean_card` 的 `image` 分支（现 :83-87 读 url）改为：
```python
    if ctype == "image":
        subject = raw.get("subject")
        if subject not in IMAGE_SUBJECTS:
            return None
        data = subject_data_b64(subject)
        if data is None:
            return None
        return {"type": "image", "data": data, **common}
```
3) `CARD_PROTOCOL`（:12-22）更新：把"本期只用 text 和 stamp"改为允许在**特别时刻**出 1 张彩图，并列出 image 卡形状与 subject 词表：
```
（在协议文本里加入，措辞对齐既有中文风格）
- 特别时刻（孩子画完、值得庆祝）可以放最多 1 张彩图卡：
  {"type":"image","subject":"circle|square|triangle|star|heart|sun|flower|tree","place":"blank_area","size":"l"}
  只在合适时用，别每回合都出；subject 必须来自上面 8 个词。
```

- [ ] **Step 4: 测试通过 + Commit**

Run: `cd server && uv run pytest tests/test_cards_engine.py -q` → 全绿（含改写后的旧用例）。整体 `cd server && uv run pytest -q` 不回归。
```bash
git add server/app/engine/cards.py server/tests/test_cards_engine.py
git commit -m "feat(server): image card resolves subject to inline base64 art data"
```

---

### Task 3: `/turn` 注入当前课时脚本

**Files:**
- Modify: `server/app/routers/turn.py`
- （可选）Modify: `server/app/engine/lesson.py`（若把 `_active_current_lesson` 提为公共 helper）
- Test: `server/tests/test_turn_endpoint.py`（追加）

**Interfaces:**
- Consumes: `_active_current_lesson`（phone.py:25，或提到 lesson.py）、`render_lesson_script`/`latest_recap`（lesson.py）
- Produces: `/turn` 在有 active 课程时，系统提示包含当前课脚本；无则照旧

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_turn_endpoint.py` 追加（复用 test_phone_lesson 的 `setup_course` 范式；若 helper 不在本文件，仿其步骤内联）：
```python
import respx, httpx, json


def _setup_course(client, db):
    # provider+profile 已有 _setup_active_profile；再激活形状课
    _setup_active_profile(db)
    r = client.post("/api/admin/curricula/seed-shapes01")
    cid = r.json()["id"]
    client.post(f"/api/admin/curricula/{cid}/activate")


@respx.mock
def test_turn_injects_active_lesson_script(client, db):
    _setup_course(client, db)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(json.dumps(
            {"spoken_text": "好", "paper_cards": []}, ensure_ascii=False))))
    client.post("/turn", json=_min_body(page_png="QUJD"))
    sys_prompt = json.loads(route.calls[0].request.content)["messages"][0]["content"]
    assert "形状" in sys_prompt or "圆" in sys_prompt, "lesson script injected"


@respx.mock
def test_turn_without_active_curriculum_still_works(client, db):
    _setup_active_profile(db)  # 无 active 课程
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(json.dumps(
            {"spoken_text": "好", "paper_cards": []}, ensure_ascii=False))))
    r = client.post("/turn", json=_min_body(page_png="QUJD"))
    assert r.status_code == 200
```
> `seed-shapes01` 的真实 admin 路径/返回体以 `app/routers/admin_curricula.py` 为准（Explore 指 `POST /api/admin/curricula/seed-shapes01` + `/activate`）；实现者对齐真实路由与返回字段（拿 curriculum id 的方式）。

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py::test_turn_injects_active_lesson_script -q`
Expected: FAIL（当前 /turn 不注入课时，system prompt 无课程文本）。

- [ ] **Step 3: 实现**

`server/app/routers/turn.py` 的 `turn` handler，在构造 `TurnInput` 前解析 active 课时：
```python
    from app.engine.lesson import render_lesson_script, latest_recap
    from app.routers.phone import _active_current_lesson  # 或提到 lesson.py 后从那里导入

    lesson_context = ""
    with request.app.state.sessionmaker() as db:
        found = _active_current_lesson(db)
        if found is not None:
            _curriculum, lesson = found
            lesson_context = render_lesson_script(
                lesson.script_text, latest_recap(db, lesson.curriculum_id))

    tin = TurnInput(
        source="tablet",
        text=TURN_USER_TEXT,
        image_png=image_png,
        device_protocol_suffix=cards_engine.CARD_PROTOCOL,
        lesson_context=lesson_context,
    )
```
> 若 router→router 导入 `_active_current_lesson` 觉得别扭，**推荐**把该函数原样移到 `app/engine/lesson.py` 作公共 helper，再从 turn.py 与 phone.py 同时导入（phone.py 改为 re-import，保持其行为）。实现者择一并报告；`TurnInput` 是否已有 `lesson_context` 字段见 `app/engine/turn.py`（计划 1 前置条件确认过其存在）。

- [ ] **Step 4: 测试通过 + Commit**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py -q` → 全绿；`cd server && uv run pytest -q` 整体不回归。
```bash
git add server/app/routers/turn.py server/app/engine/lesson.py server/app/routers/phone.py server/tests/test_turn_endpoint.py
git commit -m "feat(server): /turn injects the active lesson script as lesson_context"
```

---

## 自查（写完对照 spec）

- **spec 覆盖**：S2 课时注入 = Task 3；S3 精选彩图 = Task 1 + Task 2；§4 image 卡真彩内联 = Task 2 产出 `{"type":"image","data":...}`（与计划 2 设备端契约对齐）。
- **本计划不含**：计划 4（真机串一节完整课、端到端调）。
- **契约一致性**：服务器产出 image 卡 = `{"type":"image","data":"<base64 PNG>"}` ↔ 设备（计划 2）`cards.rs` 认 `data`(base64 PNG)。模型侧只出 `subject`（∈ `IMAGE_SUBJECTS`）；`data` 全由服务器填，堵死"模型自造图源"。≤1 image 由 `build_cards` 既有去重保证。
- **计划 1 遗留对齐**：计划 1 的 image-with-`url` 占位（`_clean_card` + 两个 url 用例）在 Task 2 被切到 `subject→data` 并改写用例——这正是计划 1/计划 2 备忘里预告的"计划 3 把 url 切 data"。
- **占位符扫描**：Task 2/3 有几处"以实际路由/签名为准平移"（seed-shapes01 返回体、`_active_current_lesson` 导入方式、`TurnInput.lesson_context` 字段）——均给了完整目标代码 + 语义，属定向指令非空白。
- **运行期无新依赖**：Pillow 只在 `scripts/gen_art.py`（`uv run --with pillow`），不进 `pyproject.toml`；服务器只读已提交 PNG。

## 执行交接

见文首 header 的 REQUIRED SUB-SKILL。逐任务：`cd server && uv run pytest` 全绿 + commit。三任务后做最终全分支评审。
