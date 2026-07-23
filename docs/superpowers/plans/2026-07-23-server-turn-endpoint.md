# 服务器 `/turn` 结构化端点 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 DouDou Server 加 `POST /turn` 结构化端点：接收平板的整页 PNG + 新笔画，复用现有 `TurnRunner` 做一次视觉调用，把模型的 JSON 回复解析并按平板 §14.3 约束夹紧成 `paper_cards`，返回平板 `cards.rs` 能解析的响应，并落库。

**Architecture:** 沿服务器一期结构做增量。新增 `app/routers/turn.py`（路由 + 请求/响应模型）与 `app/engine/cards.py`（模型回复→校验后的 paper_cards 纯函数）。卡片生成用**单次视觉调用**：`/turn` 复用 [`TurnRunner`](../../../server/app/engine/turn.py)，把"纸面卡片协议"经 `TurnInput.device_protocol_suffix` 注入系统提示，令模型只回一个 JSON 对象 `{spoken_text, paper_cards, page_action}`；服务器容错解析 + 夹紧后返回。解析失败降级成一张 `text` 卡（原文截断），平板永远有东西显示。本期卡片集 = `text` + `stamp` + `image`（`image` 的契约与校验在此就位，但"课题→精选彩图 url"映射属计划 3；`sketch` 本期不做）。

**Tech Stack:** Python 3.12 + FastAPI + SQLAlchemy 2 + httpx；测试 pytest + respx（异步）。与一期一致。

**Spec:** `docs/superpowers/specs/2026-07-23-demo-lesson-color-codraw-design.md`（需求来源，冲突以 spec 为准）。设备端契约见 `device/riddle/src/cards.rs`（服务器产出必须与之匹配）。

## 前置条件

1. 一期已合入并全绿：`cd server && uv run pytest` 全通过；存在 `app/engine/turn.py` 的 `TurnInput`（字段含 `source/text/image_png/history/device_protocol_suffix`）、`TurnRunner(sessionmaker, data_dir, tin)`（`.stream()` 异步生成器、`.turn_id`、`.reply_text`）；`app/engine/prompt.py` 的 `assemble_system_prompt(persona, *, voice_hint="", lesson_context="", time_line="", protocol_suffix="")` 把 `protocol_suffix` 追加到系统提示末尾；`app/db.py` 的 `make_sessionmaker`/`get_db`/`_migrate`；`app/models.py` 的 `Turn`、`utcnow`；`app/main.py` 的 `create_app` 逐个 `include_router`。
2. 若实际代码与上述有出入，**以实际代码为准适配本计划**（同义平移，不改行为语义）。
3. 每个任务以 `cd server && uv run pytest` 全量通过 + git commit 结束，不得破坏一期既有测试。

## Global Constraints

- 响应形状必须与设备 `device/riddle/src/cards.rs` 的解析器一致：顶层 `{"turn_id": str, "spoken_text": str, "paper_cards": [...], "page_action": str, "memory_tags": [str]}`；缺 `turn_id`/`paper_cards` 会被设备判为错误。
- §14.3 夹紧（服务器侧也做，纵深防御，设备还会再夹一遍）：每回合 `paper_cards` ≤ **3** 张；`text.content` 按档位截断（`child_3_4` ≤ **6** 个 Unicode 字符）；`stamp.name` 必须来自 8 枚举 `star/flower/heart/smiley/check/sun/moon/balloon`，`count` 夹到 **1..=5**；`image` 卡每回合 ≤ **1** 张且 `url` 必须非空字符串；未知类型/缺必填字段的卡**丢弃不报错**。
- 中文 JSON 输出 `ensure_ascii=False`；面向家长的错误信息用中文短句。
- 字符截断按 Unicode 标量计数（`str` 切片），**不可按字节**（中文多字节会切碎）。
- 迁移无 Alembic：新列靠 `db.py` 的 `_migrate` 启动时 `ALTER TABLE` 探测补列。
- 卡片协议里 `image` 只支持 `{"type":"image","url":...}`——url 由服务器侧（计划 3 的精选库）填，模型本期不应自造 url。

---

### Task 1: `/turn` 请求/响应模型 + 路由骨架

**Files:**
- Create: `server/app/routers/turn.py`
- Modify: `server/app/main.py`（注册 `turn.router`）
- Test: `server/tests/test_turn_endpoint.py`

**Interfaces:**
- Consumes: 一期 `create_app`、`app.state`
- Produces: `POST /turn`；请求体（宽松）`{turn_id, trigger, page_png(base64 str), new_strokes(list), page_state{ink_coverage, page_id}, device_profile{profile, screen}, page_id}`；响应 `{"v":1, "turn_id", "spoken_text", "paper_cards", "page_action", "memory_tags"}`。本任务先返回**空卡骨架**（echo `turn_id`、`spoken_text=""`、`paper_cards=[]`），锁定端点与形状。

- [ ] **Step 1: 写失败测试**

`server/tests/test_turn_endpoint.py`：

```python
def _min_body(**over):
    body = {
        "turn_id": "t-1",
        "trigger": "pen_idle",
        "page_png": "QUJD",  # base64 "ABC"
        "new_strokes": [],
        "page_state": {"ink_coverage": 0.1, "page_id": "p-1"},
        "device_profile": {"profile": "child_3_4", "screen": [1620, 2160]},
        "page_id": "p-1",
    }
    body.update(over)
    return body


def test_turn_returns_contract_shape(client):
    r = client.post("/turn", json=_min_body())
    assert r.status_code == 200
    j = r.json()
    assert j["v"] == 1
    assert j["turn_id"] == "t-1"
    assert j["spoken_text"] == ""
    assert j["paper_cards"] == []
    assert j["page_action"] == "none"
    assert j["memory_tags"] == []


def test_turn_tolerates_missing_optional_fields(client):
    r = client.post("/turn", json={"turn_id": "t-2", "page_png": "QUJD"})
    assert r.status_code == 200
    assert r.json()["turn_id"] == "t-2"
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py`
Expected: FAIL（404，路由不存在）

- [ ] **Step 3: 实现路由骨架**

`server/app/routers/turn.py`：

```python
from fastapi import APIRouter, Request
from pydantic import BaseModel, Field

router = APIRouter()


class PageState(BaseModel):
    ink_coverage: float = 0.0
    page_id: str = ""


class DeviceProfile(BaseModel):
    profile: str = "child_3_4"
    screen: list[int] = Field(default_factory=lambda: [1620, 2160])


class TurnRequest(BaseModel):
    turn_id: str = ""
    trigger: str = "pen_idle"
    page_png: str = ""            # base64 灰度整页
    new_strokes: list = Field(default_factory=list)
    page_state: PageState = Field(default_factory=PageState)
    device_profile: DeviceProfile = Field(default_factory=DeviceProfile)
    page_id: str = ""


def _response(turn_id: str, spoken_text: str, cards: list,
              page_action: str = "none", memory_tags: list | None = None) -> dict:
    return {
        "v": 1,
        "turn_id": turn_id,
        "spoken_text": spoken_text,
        "paper_cards": cards,
        "page_action": page_action,
        "memory_tags": memory_tags or [],
    }


@router.post("/turn")
async def turn(req: TurnRequest, request: Request):
    # 骨架：形状先锁定，模型调用在 Task 3 接线。
    return _response(req.turn_id, "", [])
```

`server/app/main.py`：在 import 与注册处加入 `turn`（仿一期写法）：

```python
    from app.routers import (admin_curricula, admin_profiles, admin_providers,
                             admin_test, admin_turns, admin_voice, files,
                             openai_compat, phone, turn)
    ...
    app.include_router(phone.router)
    app.include_router(turn.router)
    app.include_router(files.router)
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/turn.py server/app/main.py server/tests/test_turn_endpoint.py
git commit -m "feat(server): /turn endpoint skeleton with request/response contract"
```

---

### Task 2: 卡片引擎（模型回复 JSON → 校验后的 paper_cards）

**Files:**
- Create: `server/app/engine/cards.py`
- Test: `server/tests/test_cards_engine.py`

**Interfaces:**
- Consumes: 无（纯函数，仅标准库 `json`）
- Produces:
  - 常量 `STAMP_NAMES`（8 枚举 tuple）、`MAX_CARDS=3`、`MAX_STAMP_COUNT=5`、`PROFILE_TEXT_CAPS={"child_3_4":6}`、`DEFAULT_TEXT_CAP=6`、`CARD_PROTOCOL`（注入系统提示的纸面卡片协议文本）
  - `text_cap(profile: str) -> int`
  - `extract_json_object(text: str) -> dict | None`（从可能含围栏/杂散文字的回复里抠出第一个平衡 JSON 对象）
  - `build_cards(reply_text: str, profile: str) -> tuple[str, list[dict], str, list[str]]` 返回 `(spoken_text, paper_cards, page_action, memory_tags)`；解析失败降级为一张 `text` 卡（原文按档位截断），`page_action="none"`

- [ ] **Step 1: 写失败测试**

`server/tests/test_cards_engine.py`：

```python
import json

from app.engine.cards import (
    CARD_PROTOCOL,
    build_cards,
    extract_json_object,
    text_cap,
)


def test_text_cap_by_profile():
    assert text_cap("child_3_4") == 6
    assert text_cap("unknown") == 6


def test_extract_json_from_fenced_reply():
    reply = '好的\n```json\n{"spoken_text":"哇","paper_cards":[]}\n```\n'
    obj = extract_json_object(reply)
    assert obj["spoken_text"] == "哇"


def test_build_cards_full_and_truncates_to_three():
    reply = json.dumps({
        "spoken_text": "哇，三颗星！",
        "paper_cards": [
            {"type": "stamp", "name": "star", "count": 3, "place": "near_new_ink", "size": "S"},
            {"type": "text", "content": "你画得真好看极了", "place": "blank_area", "size": "L"},
            {"type": "image", "url": "/api/files/lesson-art/sun.png", "size": "L"},
            {"type": "text", "content": "多余", "size": "S"},
        ],
        "page_action": "none",
        "memory_tags": ["sun"],
    }, ensure_ascii=False)
    spoken, cards, page_action, tags = build_cards(reply, "child_3_4")
    assert spoken == "哇，三颗星！"
    assert len(cards) == 3            # 第 4 张被 ≤3 丢弃
    assert cards[0]["type"] == "stamp" and cards[0]["count"] == 3
    assert cards[1]["type"] == "text" and cards[1]["content"] == "你画得真好看"  # 6 字截断
    assert cards[2]["type"] == "image" and cards[2]["url"].endswith("sun.png")
    assert tags == ["sun"]


def test_build_cards_drops_unknown_stamp_and_clamps_count():
    reply = json.dumps({
        "spoken_text": "",
        "paper_cards": [
            {"type": "stamp", "name": "dragon", "count": 2},
            {"type": "stamp", "name": "heart", "count": 99},
        ],
    }, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1
    assert cards[0]["name"] == "heart" and cards[0]["count"] == 5  # 未知 stamp 丢弃；count 夹到 5


def test_build_cards_at_most_one_image():
    reply = json.dumps({
        "spoken_text": "",
        "paper_cards": [
            {"type": "image", "url": "a.png"},
            {"type": "image", "url": "b.png"},
        ],
    }, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1 and cards[0]["url"] == "a.png"


def test_build_cards_drops_image_without_url():
    reply = json.dumps({"spoken_text": "", "paper_cards": [{"type": "image"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert cards == []


def test_build_cards_degrades_non_json_to_single_text_card():
    spoken, cards, page_action, tags = build_cards("太阳会发光是因为核聚变呀真棒", "child_3_4")
    assert len(cards) == 1 and cards[0]["type"] == "text"
    assert cards[0]["content"] == "太阳会发光"   # 原文 6 字截断
    assert spoken == "太阳会发光是因为核聚变呀真棒"  # 语音仍念完整原文
    assert page_action == "none"


def test_card_protocol_mentions_json_and_stamp_names():
    assert "JSON" in CARD_PROTOCOL or "json" in CARD_PROTOCOL
    assert "star" in CARD_PROTOCOL
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_cards_engine.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/cards.py`：

```python
"""模型回复 → 校验后的 paper_cards。服务器信内容不信形状：任何畸形都降级成
「更少的卡」而非报错，与设备 cards.rs 的容错策略一致。"""

import json

STAMP_NAMES = ("star", "flower", "heart", "smiley", "check", "sun", "moon", "balloon")
MAX_CARDS = 3
MAX_STAMP_COUNT = 5
PROFILE_TEXT_CAPS = {"child_3_4": 6}
DEFAULT_TEXT_CAP = 6

CARD_PROTOCOL = (
    "\n\n【纸面卡片协议】你正在通过 DouDou 平板回应孩子。只输出一个 JSON 对象，"
    "不要任何多余文字、不要 Markdown 围栏：\n"
    '{"spoken_text":"给孩子/家长听的一句话","paper_cards":[卡片...],"page_action":"none"}\n'
    "卡片类型（本期只用 text 和 stamp）：\n"
    '- {"type":"text","content":"手写短句，最多6个字","place":"blank_area","size":"L"}\n'
    '- {"type":"stamp","name":"star|flower|heart|smiley|check|sun|moon|balloon",'
    '"count":1,"place":"near_new_ink","size":"S"}\n'
    "规则：最多 3 张卡；text 的 content 不超过 6 个字；stamp 的 name 必须来自上面 8 个；"
    "spoken_text 与卡片讲同一件事；不要输出表情符号。"
)


def text_cap(profile: str) -> int:
    return PROFILE_TEXT_CAPS.get(profile, DEFAULT_TEXT_CAP)


def _truncate(s: str, cap: int) -> str:
    return s[:cap]  # str 切片按 Unicode 标量，中文安全


def extract_json_object(text: str) -> dict | None:
    """抠出回复里第一个平衡的 JSON 对象（容忍 ```json 围栏与前后杂散文字）。"""
    start = text.find("{")
    if start == -1:
        return None
    depth = 0
    in_str = False
    esc = False
    for i in range(start, len(text)):
        c = text[i]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            continue
        if c == '"':
            in_str = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                try:
                    obj = json.loads(text[start:i + 1])
                    return obj if isinstance(obj, dict) else None
                except json.JSONDecodeError:
                    return None
    return None


def _clean_card(raw: dict, cap: int) -> dict | None:
    if not isinstance(raw, dict):
        return None
    ctype = str(raw.get("type", "")).lower()
    common = {k: raw[k] for k in ("place", "anchor_norm", "size", "pace") if k in raw}
    if ctype == "text":
        content = raw.get("content")
        if not isinstance(content, str) or not content:
            return None
        return {"type": "text", "content": _truncate(content, cap), **common}
    if ctype == "stamp":
        name = raw.get("name")
        if name not in STAMP_NAMES:
            return None
        count = raw.get("count", 1)
        count = count if isinstance(count, int) else 1
        return {"type": "stamp", "name": name, "count": max(1, min(count, MAX_STAMP_COUNT)), **common}
    if ctype == "image":
        url = raw.get("url")
        if not isinstance(url, str) or not url:
            return None
        return {"type": "image", "url": url, **common}
    return None  # 未知/本期不支持的类型（sketch/count/trace/page）：丢弃


def build_cards(reply_text: str, profile: str) -> tuple[str, list[dict], str, list[str]]:
    """返回 (spoken_text, paper_cards, page_action, memory_tags)。"""
    cap = text_cap(profile)
    obj = extract_json_object(reply_text)
    if obj is None:
        # 降级：模型没按协议出 JSON。语音念完整原文，纸面给一张截断 text 卡。
        return reply_text, [{"type": "text", "content": _truncate(reply_text, cap),
                             "place": "blank_area", "size": "L"}], "none", []

    spoken = str(obj.get("spoken_text", "") or "")
    raw_cards = obj.get("paper_cards")
    raw_cards = raw_cards if isinstance(raw_cards, list) else []
    cards: list[dict] = []
    seen_image = False
    for rc in raw_cards:
        card = _clean_card(rc, cap)
        if card is None:
            continue
        if card["type"] == "image":
            if seen_image:
                continue          # 每回合 ≤1 张 image
            seen_image = True
        cards.append(card)
        if len(cards) >= MAX_CARDS:
            break

    page_action = str(obj.get("page_action", "none") or "none")
    tags = obj.get("memory_tags")
    tags = [str(t) for t in tags] if isinstance(tags, list) else []
    return spoken, cards, page_action, tags
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/engine/cards.py server/tests/test_cards_engine.py
git commit -m "feat(server): cards engine — model JSON reply to clamped paper_cards"
```

---

### Task 3: `/turn` 接入模型调用

**Files:**
- Modify: `server/app/routers/turn.py`（在骨架里接 TurnRunner + cards 引擎）
- Test: `server/tests/test_turn_endpoint.py`（新增带 mock 上游的用例）

**Interfaces:**
- Consumes: Task 2 的 `cards.build_cards` / `cards.CARD_PROTOCOL`；一期 `TurnInput`/`TurnRunner`；`app.state.sessionmaker`/`data_dir`
- Produces: `POST /turn` 真正跑一次视觉调用，把模型 JSON 回复转成 `paper_cards` 返回。无生效人设/模型时返回 400 中文短句；上游错误返回 502。

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_turn_endpoint.py` 追加：

```python
import base64
import json

import httpx
import respx

from app import models


def _setup_active_profile(db):
    p = models.Provider(name="up", base_url="https://up.test/v1", api_key="sk")
    db.add(p)
    db.flush()
    prof = models.Profile(name="小班", age_band="3-4", persona_text="你是 DouDou。",
                          provider_id=p.id, model="gpt-4o-mini", max_tokens=1500,
                          is_active=True)
    db.add(prof)
    db.commit()


def _sse(text: str) -> str:
    # 单块 SSE，content 即整段回复
    payload = json.dumps({"choices": [{"delta": {"content": text}}]}, ensure_ascii=False)
    return f"data: {payload}\n\ndata: [DONE]\n\n"


@respx.mock
def test_turn_runs_model_and_returns_cards(client, db):
    _setup_active_profile(db)
    reply = json.dumps({
        "spoken_text": "哇，三颗星星！",
        "paper_cards": [{"type": "stamp", "name": "star", "count": 3, "place": "near_new_ink"}],
        "page_action": "none", "memory_tags": ["star"],
    }, ensure_ascii=False)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(reply)))

    body = _min_body(page_png=base64.b64encode(b"\x89PNG-fake").decode())
    r = client.post("/turn", json=body)
    assert r.status_code == 200
    j = r.json()
    assert j["spoken_text"] == "哇，三颗星星！"
    assert len(j["paper_cards"]) == 1 and j["paper_cards"][0]["name"] == "star"
    assert j["memory_tags"] == ["star"]

    # 系统提示带上了卡片协议；用户消息带上了整页图
    sent = json.loads(route.calls[0].request.content)
    sys_prompt = sent["messages"][0]["content"]
    assert "纸面卡片协议" in sys_prompt
    user = sent["messages"][-1]["content"]
    assert user[1]["image_url"]["url"].startswith("data:image/png;base64,")


def test_turn_without_active_profile_returns_400(client):
    r = client.post("/turn", json=_min_body())
    assert r.status_code == 400
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py`
Expected: FAIL（骨架仍返回空卡；且无 400 分支）

- [ ] **Step 3: 实现**

改写 `server/app/routers/turn.py` 的 `turn` 处理函数（保留 Task 1 的模型类与 `_response`）：

```python
import base64

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from app.engine import cards as cards_engine
from app.engine.errors import ConfigError
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

# ... （Task 1 的 PageState/DeviceProfile/TurnRequest/_response 保持不变）...

TURN_USER_TEXT = "（这是孩子刚画的整页）请按纸面卡片协议回应。"


@router.post("/turn")
async def turn(req: TurnRequest, request: Request):
    image_png = None
    if req.page_png:
        try:
            image_png = base64.b64decode(req.page_png)
        except (ValueError, TypeError):
            image_png = None

    tin = TurnInput(
        source="tablet",
        text=TURN_USER_TEXT,
        image_png=image_png,
        device_protocol_suffix=cards_engine.CARD_PROTOCOL,
    )
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError:
        raise HTTPException(502, "模型服务出错，请在后台检查配置")

    spoken, cards, page_action, tags = cards_engine.build_cards(
        runner.reply_text, req.device_profile.profile)
    return _response(req.turn_id, spoken, cards, page_action, tags)
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过（含 Task 1 的形状用例仍绿——注意 `test_turn_returns_contract_shape` 现在没有 mock 上游也没有生效人设，会走 400 分支！需把它改成期望 400，或给它加 `_setup_active_profile` + mock。**执行时：把 Task 1 的两个无人设用例调整为断言 400**，因为端点现在真的会调用模型。）

> 修正 Task 1 遗留用例：`test_turn_returns_contract_shape` 与 `test_turn_tolerates_missing_optional_fields` 改为断言 `status_code == 400`（无生效人设），形状断言移交给本任务的 `test_turn_runs_model_and_returns_cards`。

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/turn.py server/tests/test_turn_endpoint.py
git commit -m "feat(server): /turn runs the vision model and returns paper_cards"
```

---

### Task 4: 落库（Turn 加 `cards_json` 列 + 迁移）

**Files:**
- Modify: `server/app/models.py`（`Turn` 加一列）
- Modify: `server/app/db.py`（`_migrate` 补列）
- Modify: `server/app/routers/turn.py`（返回前把卡片写回该 Turn 行）
- Test: `server/tests/test_turn_endpoint.py`（新增落库断言）

**Interfaces:**
- Consumes: Task 3 的 `/turn`；`runner.turn_id`
- Produces: `Turn.cards_json: dict | None`（存本回合返回的 `{spoken_text, paper_cards, page_action, memory_tags}`）；旧库自动补 `turns.cards_json` 列

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_turn_endpoint.py` 追加：

```python
@respx.mock
def test_turn_persists_cards_json(client, db):
    _setup_active_profile(db)
    reply = json.dumps({"spoken_text": "好", "paper_cards": [
        {"type": "text", "content": "太阳", "size": "L"}]}, ensure_ascii=False)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(reply)))

    client.post("/turn", json=_min_body(page_png="QUJD"))

    from app import models as m
    row = db.query(m.Turn).filter(m.Turn.source == "tablet").order_by(m.Turn.id.desc()).first()
    assert row is not None
    assert row.cards_json is not None
    assert row.cards_json["paper_cards"][0]["content"] == "太阳"


def test_legacy_turns_table_gains_cards_json_column(tmp_path):
    import sqlite3
    from sqlalchemy import text
    con = sqlite3.connect(tmp_path / "doudou.db")
    con.execute("CREATE TABLE turns (id INTEGER PRIMARY KEY, source VARCHAR(10))")
    con.commit()
    con.close()
    from app.db import make_sessionmaker
    maker = make_sessionmaker(str(tmp_path))
    with maker() as s:
        s.execute(text("SELECT cards_json FROM turns"))  # 列不存在会抛 OperationalError
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py::test_turn_persists_cards_json tests/test_turn_endpoint.py::test_legacy_turns_table_gains_cards_json_column`
Expected: FAIL（`Turn` 无 `cards_json`；旧库无该列）

- [ ] **Step 3: 实现**

`server/app/models.py` 的 `Turn` 类内追加一列（与既有列风格一致；确认文件顶部已 import `JSON`，否则从 `sqlalchemy` 补 import）：

```python
    cards_json: Mapped[dict | None] = mapped_column(JSON, nullable=True)
```

`server/app/db.py` 的 `_migrate` 里，`turns` 表补列逻辑追加（与既有 `lesson_run_id` 补列同款）：

```python
        if "cards_json" not in cols:
            with engine.begin() as conn:
                conn.execute(text("ALTER TABLE turns ADD COLUMN cards_json JSON"))
```

> 注意：`_migrate` 里 `cols` 只取一次；两处补列都用同一份 `cols` 判断即可。若一期 `_migrate` 结构不同，按其实际写法平移「探测列缺失 → ALTER ADD COLUMN」。

`server/app/routers/turn.py` 的 `turn` 处理末尾，返回前落库：

```python
    resp = _response(req.turn_id, spoken, cards, page_action, tags)
    if runner.turn_id is not None:
        with request.app.state.sessionmaker() as db:
            t = db.get(Turn, runner.turn_id)
            if t is not None:
                t.cards_json = {"spoken_text": spoken, "paper_cards": cards,
                                "page_action": page_action, "memory_tags": tags}
                db.commit()
    return resp
```

（文件顶部补 `from app.models import Turn`。）

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/models.py server/app/db.py server/app/routers/turn.py server/tests/test_turn_endpoint.py
git commit -m "feat(server): persist /turn paper_cards on the Turn row"
```

---

## 自查（写完对照 spec）

- **spec 覆盖**：§3 数据流服务器侧（收 page_png/strokes → 跑视觉 → 出 cards）= Task 1/3；§4 卡片词汇 text/stamp/image + §14.3 夹紧 = Task 2；`image` 契约与 ≤1 校验 = Task 2；落库供后台/作品挂靠 = Task 4。**本计划不含**：`image` 的「课题→精选彩图 url」映射（计划 3/S3）、课时注入（计划 3/S2）、`sketch`、设备端 `image` 渲染（计划 2）、`page_action` 的自动换页联动（暂恒 none 透传）。
- **契约一致性**：响应字段 `turn_id/spoken_text/paper_cards/page_action/memory_tags` 与 `device/riddle/src/cards.rs` 的 `RawResponse` 对齐；stamp 8 枚举、count 1..5、text 档位截断、image≤1 与设备夹紧一致（纵深防御）。
- **待执行注意**：Task 3 会让 Task 1 写的两个「无人设也返回 200 空卡」用例失效——执行 Task 3 时按其 Step 4 的说明把它们改成断言 400。

## 执行交接

见文首 header 的 REQUIRED SUB-SKILL。逐任务实施，每个任务 `cd server && uv run pytest` 全绿 + commit。
