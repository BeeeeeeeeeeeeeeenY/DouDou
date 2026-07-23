# 语音触发演示 · 服务器通道 + 手机清空按钮 实施计划（Plan 1/2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建成"语音教学掐点 → 平板轮询取用"的服务器通道：`⟦demo:circle⟧` 标记挂 `lesson_run.pending_demo`，`GET /turn/next` 取用并清除；顺带一条 `clear` 命令 + 手机「清空画板」按钮。

**Architecture:** 复用已验证的 `⟦…⟧` 标记解析套路（同 `parse_lesson_report`）。待办状态挂在当前 `running` 的 `lesson_run` 上（房间作用域），平板轮询 `GET /turn/next` 取用即清（clear-on-fetch，只生效一次）。本计划**只做服务器 + 手机前端**，设备端轮询/渲染是 Plan 2，对着本计划定的 `/turn/next` 契约实现。

**Tech Stack:** FastAPI + SQLAlchemy(SQLite) + pytest；前端 React/TS（`server/web`）。

## Global Constraints

- 标记行绝不外显、绝不念给孩子：与 `⟦lesson_report⟧` 同规格，解析后从 `reply_text` 剥离。
- 待办状态房间作用域：只挂"最近一个 `status=='running'` 的 `lesson_run'"，与 `/turn`、voice-turn 现有房间逻辑一致。
- 演示不计孩子作品：不落 `Turn`、不进 `artifact_turn_ids`、不满足 `run_has_drawing`（与"未开画不关课"门槛正交）。
- 形状固定小词表 `DEMO_SHAPES = ("circle",)`；本计划只 circle。
- SQLite 老库补列走 `app/db.py` 既有 `_migrate()`，不引入 alembic。
- 所有服务器测试：`cd server && uv run pytest`。

---

### Task 1: `parse_demo` 解析并剥离 `⟦demo:circle⟧`

**Files:**
- Modify: `server/app/engine/lesson.py`（顶部常量区 + 新函数，紧邻 `parse_lesson_report`）
- Test: `server/tests/test_lesson_engine.py`

**Interfaces:**
- Produces: `parse_demo(text: str) -> tuple[str, str | None]` — 返回 `(clean_text, shape|None)`；`DEMO_SHAPES: tuple[str, ...]`。

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_lesson_engine.py` 末尾追加：

```python
from app.engine.lesson import parse_demo


def test_parse_demo_extracts_and_strips_known_shape():
    clean, shape = parse_demo("先画一个圆圆的小脑袋\n⟦demo:circle⟧")
    assert shape == "circle"
    assert "demo" not in clean and clean == "先画一个圆圆的小脑袋"


def test_parse_demo_unknown_shape_stripped_but_not_recognized():
    clean, shape = parse_demo("好呀 ⟦demo:banana⟧")
    assert shape is None
    assert "demo" not in clean and clean == "好呀"


def test_parse_demo_absent_returns_text_unchanged():
    clean, shape = parse_demo("普通一句话")
    assert (clean, shape) == ("普通一句话", None)


def test_parse_demo_unclosed_marker_truncates():
    clean, shape = parse_demo("画个圆 ⟦demo:circle")
    assert shape is None and clean == "画个圆"
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_lesson_engine.py -k parse_demo -q`
Expected: FAIL（`ImportError: cannot import name 'parse_demo'`）

- [ ] **Step 3: 实现**

在 `server/app/engine/lesson.py` 顶部常量区（`VALID_REPORT_STATUS` 附近）加：

```python
DEMO_MARK = "⟦demo:"
DEMO_END = "⟧"
DEMO_SHAPES = ("circle",)
```

在 `parse_lesson_report` 下方新增：

```python
def parse_demo(text: str) -> tuple[str, str | None]:
    """抽出并剥离 ⟦demo:<shape>⟧ 标记（家长孩子都看不到、绝不念）。
    shape 须在 DEMO_SHAPES 内才认；无论认不认都从文本剥离。
    返回 (clean_text, shape|None)。"""
    idx = text.find(DEMO_MARK)
    if idx == -1:
        return text, None
    end = text.find(DEMO_END, idx + len(DEMO_MARK))
    if end == -1:  # 未闭合：从标记处截断，不认形状
        return text[:idx].strip(), None
    shape = text[idx + len(DEMO_MARK):end].strip()
    clean = (text[:idx] + text[end + len(DEMO_END):]).strip()
    return clean, (shape if shape in DEMO_SHAPES else None)
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_lesson_engine.py -k parse_demo -q`
Expected: PASS（4 passed）

- [ ] **Step 5: 提交**

```bash
git add server/app/engine/lesson.py server/tests/test_lesson_engine.py
git commit -m "feat(server): parse_demo strips ⟦demo:shape⟧ marker"
```

---

### Task 2: `LessonRun` 加 `pending_demo` / `pending_command` 列 + 迁移

**Files:**
- Modify: `server/app/models.py:119`（`LessonRun` 末尾字段后）
- Modify: `server/app/db.py`（`_migrate` 补列清单）
- Test: `server/tests/test_curriculum_models.py`

**Interfaces:**
- Produces: `LessonRun.pending_demo: str | None`、`LessonRun.pending_command: str | None`（默认 `None`）。

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_curriculum_models.py` 末尾追加：

```python
def test_lesson_run_pending_fields_default_none_and_settable(db):
    from app import models
    lesson = db.query(models.Lesson).first()
    if lesson is None:
        cur = models.Curriculum(slug="t", title="t")
        db.add(cur); db.flush()
        lesson = models.Lesson(curriculum_id=cur.id, seq=1, slug="t-1", title="t")
        db.add(lesson); db.flush()
    run = models.LessonRun(lesson_id=lesson.id)
    db.add(run); db.commit()
    assert run.pending_demo is None and run.pending_command is None
    run.pending_demo = "circle"
    run.pending_command = "clear"
    db.commit()
    db.expire_all()
    got = db.get(models.LessonRun, run.id)
    assert got.pending_demo == "circle" and got.pending_command == "clear"
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_curriculum_models.py -k pending -q`
Expected: FAIL（`AttributeError: 'LessonRun' object has no attribute 'pending_demo'`）

- [ ] **Step 3: 实现**

在 `server/app/models.py` 的 `LessonRun` 类，`parent_note` 行（:119）之后加：

```python
    pending_demo: Mapped[str | None] = mapped_column(String(20), nullable=True)      # 房间待演示形状，取用即清
    pending_command: Mapped[str | None] = mapped_column(String(20), nullable=True)   # 房间待执行命令（如 clear），取用即清
```

在 `server/app/db.py` 的 `_migrate()` 函数体内，已有 `_ensure_column(...)` 之后追加两行：

```python
    _ensure_column(engine, "lesson_runs", "pending_demo", "pending_demo VARCHAR(20)")
    _ensure_column(engine, "lesson_runs", "pending_command", "pending_command VARCHAR(20)")
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_curriculum_models.py -k pending -q`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add server/app/models.py server/app/db.py server/tests/test_curriculum_models.py
git commit -m "feat(server): LessonRun.pending_demo/pending_command columns + migration"
```

---

### Task 3: `TurnRunner` 剥离 demo 标记并暴露 `demo_shape`

**Files:**
- Modify: `server/app/engine/turn.py`（import、`__init__`、`stream()` 剥离处）
- Test: `server/tests/test_turn_engine.py`

**Interfaces:**
- Consumes: `parse_demo`（Task 1）。
- Produces: `TurnRunner.demo_shape: str | None`（默认 `None`）。

- [ ] **Step 1: 写失败测试**

先看 `server/tests/test_turn_engine.py` 里现有一条跑通 `TurnRunner.stream()` 的测试，抄它的 fixture/mock 方式，追加：

```python
@respx.mock
def test_turn_runner_exposes_demo_shape(db):
    import respx, httpx, asyncio
    from app.engine.turn import TurnInput, TurnRunner
    # 复用本文件既有 setup：provider/profile 已激活（若无则照现有测试建）
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse("先画个圆\n⟦demo:circle⟧")))
    tin = TurnInput(source="tablet", text="hi")
    runner = TurnRunner(_sessionmaker, _data_dir, tin)
    asyncio.get_event_loop().run_until_complete(_drain(runner))
    assert runner.demo_shape == "circle"
    assert "demo" not in runner.reply_text
```

> 注：`_sse`/`_sessionmaker`/`_data_dir`/`_drain` 用本文件既有辅助；若命名不同，按现有测试的实际写法对齐（这条测试的**唯一断言点**是 `runner.demo_shape == "circle"` 且 `reply_text` 已剥离）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_turn_engine.py -k demo_shape -q`
Expected: FAIL（`AttributeError: ... 'demo_shape'`）

- [ ] **Step 3: 实现**

`server/app/engine/turn.py`：

1) import 行（:9 `from app.engine.lesson import parse_lesson_report`）改为：

```python
from app.engine.lesson import parse_demo, parse_lesson_report
```

2) `TurnRunner.__init__` 里 `self.lesson_report_raw = ""`（:50）之后加：

```python
        self.demo_shape: str | None = None
```

3) `stream()` 剥离处（:127-131），把：

```python
            visible, post = split_transcript("".join(full))
            clean, report, raw = parse_lesson_report(visible)
            self.reply_text = clean  # ⟦lesson_report⟧ 标记绝不落库、不外显
            self.lesson_report = report
            self.lesson_report_raw = raw
```

改为：

```python
            visible, post = split_transcript("".join(full))
            clean, report, raw = parse_lesson_report(visible)
            clean, demo_shape = parse_demo(clean)  # demo 标记同样剥离、绝不外显
            self.reply_text = clean
            self.lesson_report = report
            self.lesson_report_raw = raw
            self.demo_shape = demo_shape
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_turn_engine.py -q`
Expected: PASS（含新测试，且原有测试不回归）

- [ ] **Step 5: 提交**

```bash
git add server/app/engine/turn.py server/tests/test_turn_engine.py
git commit -m "feat(server): TurnRunner strips and exposes demo_shape"
```

---

### Task 4: voice-turn 把 `demo_shape` 挂到 `run.pending_demo`

**Files:**
- Modify: `server/app/routers/phone.py`（voice-turn 收尾的 `with sessionmaker` 块，:163-172 区域）
- Test: `server/tests/test_phone_lesson.py`

**Interfaces:**
- Consumes: `runner.demo_shape`（Task 3）、`LessonRun.pending_demo`（Task 2）。

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_phone_lesson.py` 追加（`setup_course`/`sse_reply` 本文件已有）：

```python
@respx.mock
def test_voice_turn_sets_pending_demo(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "该画了"}))
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("我们先画一个圆圆的\n⟦demo:circle⟧")))
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3"))
    j = client.post("/api/phone/voice-turn",
                    files={"audio": ("a.webm", b"x", "audio/webm")},
                    data={"history": "[]", "lesson_run_id": str(run_id)}).json()
    assert "demo" not in j["reply_text"]                 # 标记不外泄
    assert db.get(models.LessonRun, run_id).pending_demo == "circle"
    assert db.get(models.LessonRun, run_id).status == "running"  # demo 不关课
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_phone_lesson.py -k pending_demo -q`
Expected: FAIL（`pending_demo` 为 None）

- [ ] **Step 3: 实现**

`server/app/routers/phone.py`，在 voice-turn 收尾块里，`if run is not None and run.status == "running":` 之下、"未开画不关课"逻辑**之前**，插入挂 pending_demo：

```python
            if run is not None and run.status == "running":
                if runner.demo_shape:
                    run.pending_demo = runner.demo_shape  # 语音教学掐点：挂待演示
                # 未开画不关课：孩子还没在平板上画东西前...（以下为既有逻辑，不动）
                if run_has_drawing(db, run):
                    ...
```

（`db.commit()` 已在该块内既有的关课/TTS 路径触发；为确保仅挂 demo、不关课的这轮也落库，在块尾补一次 `db.commit()`——见下）

在同一个 `with request.app.state.sessionmaker() as db:` 块的**末尾**（TTS try/except 之后）确保有一次提交。若既有代码在 TTS 成功分支才 commit，则在块尾补：

```python
        db.commit()  # 保证仅挂 pending_demo（无关课、TTS 失败）的这轮也落库
```

> 校验点：`test_voice_turn_report_before_any_drawing_keeps_run_running`（已有）仍须通过——demo 与关课门槛正交。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_phone_lesson.py -q`
Expected: PASS（新测试 + 全部既有 phone_lesson 测试）

- [ ] **Step 5: 提交**

```bash
git add server/app/routers/phone.py server/tests/test_phone_lesson.py
git commit -m "feat(server): voice-turn hangs demo_shape on run.pending_demo"
```

---

### Task 5: `GET /turn/next` 端点（取用并清除 demo/command）

**Files:**
- Modify: `server/app/routers/turn.py`（新增路由，文件末尾）
- Test: `server/tests/test_turn_endpoint.py`

**Interfaces:**
- Consumes: `LessonRun.pending_demo`/`pending_command`（Task 2）。
- Produces: `GET /turn/next` → `{"demo": {"shape","place","pace"} | null, "command": str | null}`。

- [ ] **Step 1: 写失败测试**

在 `server/tests/test_turn_endpoint.py` 追加（抄本文件既有 `setup`/`client`/`db` 方式）：

```python
def test_turn_next_returns_and_clears_pending_demo(client, db):
    from app import models
    _setup_active_lesson(client)  # 用本文件既有辅助建生效课程；若无则照既有测试建
    run = models.LessonRun(lesson_id=db.query(models.Lesson).first().id,
                           status="running", pending_demo="circle")
    db.add(run); db.commit()
    j = client.get("/turn/next").json()
    assert j["demo"] == {"shape": "circle", "place": "blank_area", "pace": "slow"}
    assert j["command"] is None
    # 取用即清：再请求得 null
    assert client.get("/turn/next").json()["demo"] is None
    assert db.get(models.LessonRun, run.id).pending_demo is None


def test_turn_next_returns_and_clears_command(client, db):
    from app import models
    _setup_active_lesson(client)
    run = models.LessonRun(lesson_id=db.query(models.Lesson).first().id,
                           status="running", pending_command="clear")
    db.add(run); db.commit()
    j = client.get("/turn/next").json()
    assert j["command"] == "clear"
    assert client.get("/turn/next").json()["command"] is None


def test_turn_next_no_running_run_is_empty(client, db):
    assert client.get("/turn/next").json() == {"demo": None, "command": None}
```

> 若本文件没有 `_setup_active_lesson`，直接用 `client.post("/api/admin/curricula/seed-shapes01")` + activate（见 `test_phone_lesson.setup_course`），并只需要 `db.query(models.Lesson).first()` 拿一个 lesson_id。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py -k turn_next -q`
Expected: FAIL（404，路由不存在）

- [ ] **Step 3: 实现**

`server/app/routers/turn.py` 末尾追加：

```python
@router.get("/turn/next")
def turn_next(request: Request):
    """平板空闲轮询：取当前房间（最近一个 running lesson_run）待办的演示/命令，
    取用即清（clear-on-fetch，只生效一次）。无 running run → 全 null。"""
    with request.app.state.sessionmaker() as db:
        run = (db.query(LessonRun).filter(LessonRun.status == "running")
               .order_by(LessonRun.id.desc()).first())
        if run is None:
            return {"demo": None, "command": None}
        demo = None
        if run.pending_demo:
            demo = {"shape": run.pending_demo, "place": "blank_area", "pace": "slow"}
            run.pending_demo = None
        command = run.pending_command or None
        run.pending_command = None
        db.commit()
        return {"demo": demo, "command": command}
```

（`LessonRun` 已在 `turn.py:11` import。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_turn_endpoint.py -q`
Expected: PASS（3 新测试 + 无回归）

- [ ] **Step 5: 提交**

```bash
git add server/app/routers/turn.py server/tests/test_turn_endpoint.py
git commit -m "feat(server): GET /turn/next serves+clears pending demo/command"
```

---

### Task 6: `POST /api/phone/clear-board` 挂 `clear` 命令

**Files:**
- Modify: `server/app/routers/phone.py`（新增路由）
- Test: `server/tests/test_phone_lesson.py`

**Interfaces:**
- Produces: `POST /api/phone/clear-board` → `{"ok": bool}`；置当前 running run 的 `pending_command="clear"`。

- [ ] **Step 1: 写失败测试**

`server/tests/test_phone_lesson.py` 追加：

```python
def test_clear_board_sets_pending_command(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    j = client.post("/api/phone/clear-board").json()
    assert j["ok"] is True
    assert db.get(models.LessonRun, run_id).pending_command == "clear"


def test_clear_board_without_running_lesson(client, db):
    j = client.post("/api/phone/clear-board").json()
    assert j["ok"] is False
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_phone_lesson.py -k clear_board -q`
Expected: FAIL（404）

- [ ] **Step 3: 实现**

`server/app/routers/phone.py` 追加（放在 `end_lesson_run` 附近）：

```python
@router.post("/clear-board")
def clear_board(db: Session = Depends(get_db)):
    """手机「清空画板」按钮：给当前房间挂 clear 命令，平板轮询到即清屏。"""
    run = (db.query(LessonRun).filter(LessonRun.status == "running")
           .order_by(LessonRun.id.desc()).first())
    if run is None:
        return {"ok": False, "reason": "no_running_lesson"}
    run.pending_command = "clear"
    db.commit()
    return {"ok": True}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_phone_lesson.py -k clear_board -q`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add server/app/routers/phone.py server/tests/test_phone_lesson.py
git commit -m "feat(server): POST /api/phone/clear-board sets clear command"
```

---

### Task 7: 第 3 课脚本加 `⟦demo:circle⟧` 教学指令

**Files:**
- Modify: `server/app/seed_shapes.py`（第 3 课 `shapes-01-03` 的 `_script` 五环节 ③）
- Test: `server/tests/test_seed_shapes.py`

**Interfaces:** 无（数据种子）。

- [ ] **Step 1: 写失败测试**

`server/tests/test_seed_shapes.py` 的 `test_seed_scripts_are_complete` 内，`for l in lessons:` 循环**之后**追加一段：

```python
    # 第 3 课「圆圆的朋友」带 demo 触发指令；其余课不带
    l3 = next(x for x in lessons if x.slug == "shapes-01-03")
    assert "⟦demo:circle⟧" in l3.script_text
    for l in lessons:
        if l.slug != "shapes-01-03":
            assert "⟦demo:circle⟧" not in l.script_text
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd server && uv run pytest tests/test_seed_shapes.py -k complete -q`
Expected: FAIL（第 3 课不含标记）

- [ ] **Step 3: 实现**

`server/app/seed_shapes.py` 第 3 课 `dict(seq=3, slug="shapes-01-03", ...)` 的 `_script(...)` 第二参数（五环节字符串）里，③ 那行末尾追加一句括注。把：

```python
            "③ 布置：「我们来吹泡泡吧，画大大小小的圆泡泡！」；封闭说成「让线的头和尾巴牵上手」；先大圆再小圆\n"
```

改为：

```python
            "③ 布置：「我们来吹泡泡吧，画大大小小的圆泡泡！」；封闭说成「让线的头和尾巴牵上手」；先大圆再小圆"
            "（当你说到"我们先画一个圆圆的泡泡"这类引导下笔的话时，在该轮回复最后另起一行输出 ⟦demo:circle⟧——"
            "家长孩子都看不到，平板据此在旁边一笔笔演示画个圆给孩子看。仅这一轮、只输出一次。）\n"
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd server && uv run pytest tests/test_seed_shapes.py -q`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add server/app/seed_shapes.py server/tests/test_seed_shapes.py
git commit -m "feat(server): lesson 3 script emits ⟦demo:circle⟧ at draw cue"
```

---

### Task 8: 手机「清空画板」按钮（前端）

**Files:**
- Modify: `server/web/src/pages/Phone.tsx`
- 手动验收（前端无单测框架）

**Interfaces:** Consumes: `POST /api/phone/clear-board`（Task 6）。

- [ ] **Step 1: 加按钮**

在 `server/web/src/pages/Phone.tsx` 上课界面（有 `lesson` 且已开课的区域）加一个按钮，沿用文件既有 `fetch('/api/phone/...')` 惯例：

```tsx
<button
  onClick={() => { fetch('/api/phone/clear-board', { method: 'POST' }).catch(() => {}) }}
  style={{ marginTop: 12, padding: '8px 16px' }}
>
  清空画板
</button>
```

放在开课后可见、结课前的容器内（参考文件里 `lesson-runs/{id}/end` 按钮的位置）。

- [ ] **Step 2: 构建前端**

Run: `cd server/web && npm run build`
Expected: 构建成功，产物落到服务器静态目录（沿用既有构建配置）。

- [ ] **Step 3: 手动验收（服务器侧即可，不依赖设备）**

先重启服务器加载新代码：`cd server && pkill -f "uvicorn --factory app.main:create_app"; sleep 2; nohup uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8789 > /tmp/doudou-server.log 2>&1 &`
然后：手机开课 → 点「清空画板」→ `curl -s http://127.0.0.1:8789/turn/next` 应返回 `{"demo":null,"command":"clear"}`，再次 curl 得 `command:null`（取用即清）。

- [ ] **Step 4: 提交**

```bash
git add server/web/src/pages/Phone.tsx server/web/<built-assets>
git commit -m "feat(phone): 清空画板 button posts /api/phone/clear-board"
```

---

## 全量回归 + 部署

- [ ] **全测**：`cd server && uv run pytest -q`（预期全绿，含既有 144 + 本计划新增）。
- [ ] **重启服务器**：`cd server && pkill -f "uvicorn --factory app.main:create_app"; sleep 2; nohup uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8789 > /tmp/doudou-server.log 2>&1 &`
- [ ] **重 seed 第 3 课**：`⟦demo:circle⟧` 进 DB（seed 幂等，需 delete+re-seed shapes-01 或直接改 lesson script_text）；并把生效课时指到第 3 课「圆圆的朋友」以便演示。
- [ ] **契约自测**：`curl -s http://127.0.0.1:8789/turn/next`（无 running run → `{"demo":null,"command":null}`）。

## 交付给 Plan 2（设备端）的契约

设备 Plan 2 对着以下稳定契约实现轮询：
- `GET /turn/next` → `{"demo": {"shape":"circle","place":"blank_area","pace":"slow"} | null, "command": "clear" | null}`。
- 语义：clear-on-fetch，只生效一次；无 running lesson_run 时全 null。
- 设备行为：`demo` → 本地 `shape_strokes(shape)` 建实心 sketch `RenderPlan` 慢速动画、layout 摆位空白角；`command=="clear"` → `user_ink.clear()` + 重绘空白。
