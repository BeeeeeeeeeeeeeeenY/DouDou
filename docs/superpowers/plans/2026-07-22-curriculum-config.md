# DouDou 课程配置功能实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 DouDou Server 上落地「形状小画家」微课程：课程/课时/运行记录三张表与管理接口、课时脚本注入语音轮、⟦lesson_report⟧ 打标解析与进度推进、示范课程种子数据、后台「课程」页与手机页上课模式。

**Architecture:** 沿服务器一期结构做增量：`engine/lesson.py` 承载课程纯函数与运行记录闭环（解析打标、注入复习、挂靠作品、推进指针）；phone 路由扩展出课程生命周期接口；管理接口与前端各加一页。软脚本设计——服务端不做环节状态机，只在 system prompt 里追加课时脚本。

**Tech Stack:** 与一期一致：Python 3.12 + FastAPI + SQLAlchemy 2 + httpx，pytest + respx；前端 Vite + React 18 + TS + Ant Design 5。

**Spec:** `docs/superpowers/specs/2026-07-22-mini-curriculum-design.md`（需求来源，冲突时以 spec 为准）；基线代码接口来自 `docs/superpowers/plans/2026-07-22-doudou-server-phase1.md`。

## 前置条件（执行本计划前必须满足）

1. 服务器一期计划已全部实施并合入（另一窗口并行进行中）。开工前验证：`cd server && uv run pytest` 全部通过，且以下接口存在：
   - `app/models.py`：`Base`、`Turn`、`Profile`、`Provider`、`utcnow()`
   - `app/db.py`：`make_sessionmaker(data_dir)`、`get_db`
   - `app/engine/prompt.py`：`assemble_system_prompt(persona, *, voice_hint="", protocol_suffix="")`
   - `app/engine/turn.py`：`TurnInput`（字段 source/text/audio/history/use_voice_hint 等）、`TurnRunner(sessionmaker, data_dir, tin)`（`.stream()`/`.turn_id`/`.reply_text`）
   - `app/routers/phone.py`：`POST /api/phone/voice-turn`（multipart `audio` + `history` 表单字段）
   - `web/src/App.tsx` 的 `MENU`/路由结构、`web/src/api.ts` 的 `get/post/put/del`
2. 若并行实施与一期计划有出入，**以实际代码为准适配本计划**（改动点同义平移，不改本计划的行为语义）。
3. 本计划所有后端改动不得破坏一期既有测试（最后每个任务的 `uv run pytest` 是全量跑）。

## Global Constraints

- 打标标记：`⟦lesson_report⟧`（U+27E6/U+27E7，与 riddle 召回指令同字符风格）；标记及其后内容**绝不进入 TTS**，也不出现在接口返回的 `reply_text` 里
- 复习占位符：`{prev_lesson_recap}`，用 `str.replace` 替换，**禁止 `str.format`**（脚本含 JSON 花括号示例会炸）
- `lesson_runs.status` 枚举：`running|completed|partial|skipped|abandoned`。`running` 是对 spec 终态枚举的实施补充（运行中的初始态）；终态语义按 spec：`abandoned`=未收到打标
- `curricula.status`：`draft|active|archived`，`active` 全局唯一（沿生效 profile 互斥模式）；激活时把其他 `active` 降为 `draft`（`archived` 不动）
- 指针推进：仅打标 `status=completed` 且当前指针仍指向该课时才推进；末课完成后指针置空（指针空+有末课完成 run = 本轮完成，无 `completed` 课程状态）
- 迁移：无 Alembic。新表靠 `create_all`；`turns` 加列 `lesson_run_id` 靠 `make_sessionmaker` 启动时探测 `ALTER TABLE`
- 中文输出 `ensure_ascii=False`；UI 全中文；面向家长的错误信息用中文短句
- 每任务以全量测试通过 + git commit 结束；后端测试命令统一 `cd server && uv run pytest`

---

### Task 1: 课程数据模型 + turns 加列迁移

**Files:**
- Modify: `server/app/models.py`（追加三个模型类 + Turn 加一列）
- Modify: `server/app/db.py`（加 `_migrate`，在 `make_sessionmaker` 中调用）
- Test: `server/tests/test_curriculum_models.py`

**Interfaces:**
- Consumes: 一期 `models.Base/utcnow`、`db.make_sessionmaker`
- Produces: 模型类 `Curriculum`（curricula 表：id/slug 唯一/title/age_band/description/status/current_lesson_id/created_at/updated_at）、`Lesson`（lessons 表：id/curriculum_id/seq/slug/title/goal_text/script_text/segments JSON/duration_min/materials/enhancements JSON/updated_at）、`LessonRun`（lesson_runs 表：id/lesson_id/started_at/ended_at/status/highlights/parent_tip/raw_report JSON/memory_tags JSON/artifact_turn_ids JSON/parent_note）；`Turn.lesson_run_id: int | None`；旧库自动补 `turns.lesson_run_id` 列

- [ ] **Step 1: 写失败测试**

`server/tests/test_curriculum_models.py`：

```python
import sqlite3

from sqlalchemy import text

from app import models
from app.db import make_sessionmaker


def test_curriculum_tables_exist(db):
    assert db.query(models.Curriculum).count() == 0
    assert db.query(models.Lesson).count() == 0
    assert db.query(models.LessonRun).count() == 0


def test_defaults(db):
    cur = models.Curriculum(slug="shapes-01", title="形状小画家")
    db.add(cur)
    db.flush()
    lesson = models.Lesson(curriculum_id=cur.id, seq=1)
    db.add(lesson)
    db.flush()
    run = models.LessonRun(lesson_id=lesson.id)
    db.add(run)
    db.commit()
    assert cur.status == "draft" and cur.current_lesson_id is None
    assert lesson.duration_min == 10
    assert run.status == "running" and run.ended_at is None
    assert run.memory_tags is None  # 一期恒空，二期记忆挂载点


def test_turn_has_lesson_run_id(db):
    db.add(models.Turn(source="phone"))
    db.commit()
    assert db.query(models.Turn).one().lesson_run_id is None


def test_legacy_turns_table_gains_column(tmp_path):
    # 模拟一期旧库：turns 表没有 lesson_run_id 列，启动后应被 ALTER 补上
    con = sqlite3.connect(tmp_path / "doudou.db")
    con.execute("CREATE TABLE turns (id INTEGER PRIMARY KEY, source VARCHAR(10))")
    con.commit()
    con.close()
    maker = make_sessionmaker(str(tmp_path))
    with maker() as s:
        s.execute(text("SELECT lesson_run_id FROM turns"))  # 列不存在会抛 OperationalError
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_curriculum_models.py`
Expected: FAIL（`AttributeError: models.Curriculum` 或类似）

- [ ] **Step 3: 实现**

`server/app/models.py` 末尾追加（`Turn` 类内加一行列定义）：

```python
# --- Turn 类内、status 列附近追加 ---
    lesson_run_id: Mapped[int | None] = mapped_column(Integer, nullable=True)


# --- 文件末尾追加三个课程模型 ---
class Curriculum(Base):
    __tablename__ = "curricula"
    id: Mapped[int] = mapped_column(primary_key=True)
    slug: Mapped[str] = mapped_column(String(100), unique=True)
    title: Mapped[str] = mapped_column(String(200), default="")
    age_band: Mapped[str] = mapped_column(String(10), default="")  # "3-4"|"5-6"|"6-7"
    description: Mapped[str] = mapped_column(Text, default="")
    status: Mapped[str] = mapped_column(String(10), default="draft")  # draft|active|archived，active 全局唯一
    current_lesson_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class Lesson(Base):
    __tablename__ = "lessons"
    id: Mapped[int] = mapped_column(primary_key=True)
    curriculum_id: Mapped[int] = mapped_column(ForeignKey("curricula.id"))
    seq: Mapped[int] = mapped_column(Integer)
    slug: Mapped[str] = mapped_column(String(100), default="")
    title: Mapped[str] = mapped_column(String(200), default="")
    goal_text: Mapped[str] = mapped_column(Text, default="")
    script_text: Mapped[str] = mapped_column(Text, default="")
    segments: Mapped[list | None] = mapped_column(JSON, nullable=True)
    duration_min: Mapped[int] = mapped_column(Integer, default=10)
    materials: Mapped[str] = mapped_column(Text, default="")
    enhancements: Mapped[list | None] = mapped_column(JSON, nullable=True)
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class LessonRun(Base):
    __tablename__ = "lesson_runs"
    id: Mapped[int] = mapped_column(primary_key=True)
    lesson_id: Mapped[int] = mapped_column(ForeignKey("lessons.id"))
    started_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    ended_at: Mapped[datetime | None] = mapped_column(DateTime, nullable=True)
    status: Mapped[str] = mapped_column(String(10), default="running")  # running|completed|partial|skipped|abandoned
    highlights: Mapped[str] = mapped_column(Text, default="")
    parent_tip: Mapped[str] = mapped_column(Text, default="")
    raw_report: Mapped[dict | None] = mapped_column(JSON, nullable=True)
    memory_tags: Mapped[list | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期记忆挂载点
    artifact_turn_ids: Mapped[list | None] = mapped_column(JSON, nullable=True)
    parent_note: Mapped[str] = mapped_column(Text, default="")
```

`server/app/db.py`：`make_sessionmaker` 里 `create_all` 之前插入迁移调用，并新增函数：

```python
from sqlalchemy import create_engine, inspect, text


def _migrate(engine) -> None:
    """SQLite 轻量迁移：给一期旧库的 turns 表补 lesson_run_id 列。"""
    insp = inspect(engine)
    if "turns" in insp.get_table_names():
        cols = {c["name"] for c in insp.get_columns("turns")}
        if "lesson_run_id" not in cols:
            with engine.begin() as conn:
                conn.execute(text("ALTER TABLE turns ADD COLUMN lesson_run_id INTEGER"))
```

`make_sessionmaker` 中改为：

```python
    engine = create_engine(
        f"sqlite:///{data_dir}/doudou.db", connect_args={"check_same_thread": False}
    )
    _migrate(engine)
    models.Base.metadata.create_all(engine)
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过（含一期既有测试）

- [ ] **Step 5: Commit**

```bash
git add server/app/models.py server/app/db.py server/tests/test_curriculum_models.py
git commit -m "feat(server): curriculum/lesson/lesson_run models and turns migration"
```

---

### Task 2: 课程引擎（打标解析、脚本渲染、运行记录闭环）

**Files:**
- Create: `server/app/engine/lesson.py`
- Test: `server/tests/test_lesson_engine.py`

**Interfaces:**
- Consumes: Task 1 的 `Curriculum/Lesson/LessonRun/Turn`、`models.utcnow`
- Produces:
  - `LESSON_REPORT_MARK = "⟦lesson_report⟧"`、`RECAP_TOKEN = "{prev_lesson_recap}"`
  - `parse_lesson_report(text: str) -> tuple[str, dict | None, str]`（干净文本, 报告 dict 或 None, 标记后原始串）
  - `render_lesson_script(script_text: str, prev_recap: str) -> str`
  - `format_recap(title: str, highlights: str, parent_tip: str) -> str`
  - `latest_recap(db, curriculum_id: int) -> str`（无记录返回 ""）
  - `close_run_with_report(db, run, report: dict, raw: str) -> None`（写终态+挂作品+推指针，内部 commit）
  - `attach_artifacts(db, run) -> None`、`advance_pointer(db, run) -> None`（close 内部使用，也可单测）

- [ ] **Step 1: 写失败测试**

`server/tests/test_lesson_engine.py`：

```python
from app import models
from app.engine.lesson import (
    RECAP_TOKEN,
    close_run_with_report,
    format_recap,
    latest_recap,
    parse_lesson_report,
    render_lesson_script,
)

REPORT_LINE = (
    '⟦lesson_report⟧{"lesson_id":"shapes-01-03","status":"completed",'
    '"highlights":"画了5个泡泡","parent_tip":"在家吹泡泡"}'
)


def test_parse_report_strips_and_parses():
    clean, report, raw = parse_lesson_report("今天真棒！\n" + REPORT_LINE)
    assert clean == "今天真棒！"
    assert report["status"] == "completed" and report["highlights"] == "画了5个泡泡"
    assert raw.startswith('{"lesson_id"')


def test_parse_report_absent():
    clean, report, raw = parse_lesson_report("普通回复")
    assert (clean, report, raw) == ("普通回复", None, "")


def test_parse_report_malformed_json():
    clean, report, raw = parse_lesson_report("收尾啦 ⟦lesson_report⟧{oops")
    assert clean == "收尾啦"  # 坏 JSON 也要剥离，不能进 TTS
    assert report is None and raw == "{oops"


def test_render_script_replaces_token():
    out = render_lesson_script(f"回顾：{RECAP_TOKEN}。开始", "上次画了线")
    assert out == "回顾：上次画了线。开始"
    out2 = render_lesson_script(f"回顾：{RECAP_TOKEN}。开始", "")
    assert "没有上次课的记录" in out2
    assert render_lesson_script("没有占位符 {x}", "r") == "没有占位符 {x}"


def test_format_recap():
    assert format_recap("圆圆的朋友", "画了泡泡", "") == "上次上的是《圆圆的朋友》。孩子的表现：画了泡泡"
    assert "延伸建议" in format_recap("圆圆的朋友", "画了泡泡", "吹泡泡")


def _seed_minimal(db):
    cur = models.Curriculum(slug="c", title="课")
    db.add(cur)
    db.flush()
    l1 = models.Lesson(curriculum_id=cur.id, seq=1, title="一")
    l2 = models.Lesson(curriculum_id=cur.id, seq=2, title="二")
    db.add_all([l1, l2])
    db.flush()
    cur.current_lesson_id = l1.id
    db.commit()
    return cur, l1, l2


def test_latest_recap_picks_newest_non_abandoned(db):
    cur, l1, _ = _seed_minimal(db)
    assert latest_recap(db, cur.id) == ""
    db.add(models.LessonRun(lesson_id=l1.id, status="abandoned", highlights="废"))
    db.add(models.LessonRun(lesson_id=l1.id, status="completed", highlights="真棒"))
    db.commit()
    assert "真棒" in latest_recap(db, cur.id) and "废" not in latest_recap(db, cur.id)


def test_close_run_completed_advances_and_attaches(db):
    cur, l1, l2 = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    tablet = models.Turn(source="tablet")  # ts 默认 now，落在 run 起止窗内
    other = models.Turn(source="phone")
    db.add_all([tablet, other])
    db.commit()
    close_run_with_report(db, run, {"status": "completed", "highlights": "亮", "parent_tip": "提"}, "{...}")
    assert run.status == "completed" and run.ended_at is not None
    assert run.highlights == "亮" and run.parent_tip == "提"
    assert run.artifact_turn_ids == [tablet.id]
    assert db.get(models.Turn, tablet.id).lesson_run_id == run.id
    assert db.get(models.Turn, other.id).lesson_run_id is None
    assert db.get(models.Curriculum, cur.id).current_lesson_id == l2.id


def test_close_run_partial_keeps_pointer(db):
    cur, l1, _ = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "partial", "highlights": "", "parent_tip": ""}, "")
    assert run.status == "partial"
    assert db.get(models.Curriculum, cur.id).current_lesson_id == l1.id


def test_close_run_bad_status_coerced_to_partial(db):
    _, l1, _ = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "great!!"}, "")
    assert run.status == "partial"


def test_last_lesson_completion_clears_pointer(db):
    cur, l1, l2 = _seed_minimal(db)
    cur.current_lesson_id = l2.id
    db.commit()
    run = models.LessonRun(lesson_id=l2.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "completed"}, "")
    assert db.get(models.Curriculum, cur.id).current_lesson_id is None
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_lesson_engine.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/lesson.py`：

```python
import json

from app.models import Curriculum, Lesson, LessonRun, Turn, utcnow

LESSON_REPORT_MARK = "⟦lesson_report⟧"
RECAP_TOKEN = "{prev_lesson_recap}"
NO_RECAP_TEXT = "（没有上次课的记录，简短问好后直接开始）"
VALID_REPORT_STATUS = ("completed", "partial", "skipped")


def parse_lesson_report(text: str) -> tuple[str, dict | None, str]:
    """从回复中剥离 ⟦lesson_report⟧ 标记。坏 JSON 也剥离（不能念给孩子），报告记 None。"""
    idx = text.find(LESSON_REPORT_MARK)
    if idx == -1:
        return text.strip(), None, ""
    clean = text[:idx].strip()
    raw = text[idx + len(LESSON_REPORT_MARK):].strip()
    try:
        report = json.loads(raw)
        if not isinstance(report, dict):
            report = None
    except json.JSONDecodeError:
        report = None
    return clean, report, raw


def render_lesson_script(script_text: str, prev_recap: str) -> str:
    """把 {prev_lesson_recap} 替换为上次课回顾。用 replace 不用 format（脚本含花括号示例）。"""
    if RECAP_TOKEN not in script_text:
        return script_text
    return script_text.replace(RECAP_TOKEN, prev_recap or NO_RECAP_TEXT)


def format_recap(title: str, highlights: str, parent_tip: str) -> str:
    out = f"上次上的是《{title}》。孩子的表现：{highlights}"
    if parent_tip:
        out += f"（当时给家长的延伸建议：{parent_tip}）"
    return out


def latest_recap(db, curriculum_id: int) -> str:
    row = (
        db.query(LessonRun, Lesson)
        .join(Lesson, LessonRun.lesson_id == Lesson.id)
        .filter(
            Lesson.curriculum_id == curriculum_id,
            LessonRun.status.in_(("completed", "partial")),
        )
        .order_by(LessonRun.started_at.desc(), LessonRun.id.desc())
        .first()
    )
    if row is None:
        return ""
    run, lesson = row
    return format_recap(lesson.title, run.highlights, run.parent_tip)


def attach_artifacts(db, run: LessonRun) -> None:
    """把 run 起止窗内、尚未归属的平板轮挂为本课作品（spec §8 时间窗自动挂靠）。"""
    turns = (
        db.query(Turn)
        .filter(
            Turn.source == "tablet",
            Turn.ts >= run.started_at,
            Turn.lesson_run_id.is_(None),
        )
        .order_by(Turn.id)
        .all()
    )
    ids = list(run.artifact_turn_ids or [])
    for t in turns:
        t.lesson_run_id = run.id
        ids.append(t.id)
    run.artifact_turn_ids = ids


def advance_pointer(db, run: LessonRun) -> None:
    """仅当课程指针仍指向本课时推进；末课完成后指针置空（= 本轮完成）。"""
    lesson = db.get(Lesson, run.lesson_id)
    cur = db.get(Curriculum, lesson.curriculum_id)
    if cur is None or cur.current_lesson_id != lesson.id:
        return  # 家长手动改过指针时不抢
    nxt = (
        db.query(Lesson)
        .filter(Lesson.curriculum_id == cur.id, Lesson.seq > lesson.seq)
        .order_by(Lesson.seq)
        .first()
    )
    cur.current_lesson_id = nxt.id if nxt else None


def close_run_with_report(db, run: LessonRun, report: dict, raw: str) -> None:
    status = report.get("status")
    run.status = status if status in VALID_REPORT_STATUS else "partial"
    run.highlights = str(report.get("highlights", ""))
    run.parent_tip = str(report.get("parent_tip", ""))
    run.raw_report = report
    run.ended_at = utcnow()
    attach_artifacts(db, run)
    if run.status == "completed":
        advance_pointer(db, run)
    db.commit()
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/engine/lesson.py server/tests/test_lesson_engine.py
git commit -m "feat(server): lesson engine with report parsing and run lifecycle"
```

---

### Task 3: 课程管理接口（curricula/lessons/runs）

**Files:**
- Create: `server/app/routers/admin_curricula.py`
- Modify: `server/app/main.py`（注册路由，仿一期方式追加 `admin_curricula.router`）
- Test: `server/tests/test_admin_curricula.py`

**Interfaces:**
- Consumes: Task 1 模型；一期 `get_db`
- Produces:
  - `GET/POST /api/admin/curricula`，`PUT/DELETE /api/admin/curricula/{id}`（DELETE 级联删课时与 runs）
  - `POST /api/admin/curricula/{id}/activate`（互斥）；`PUT /api/admin/curricula/{id}/pointer`（body `{"lesson_id": int | null}`）
  - `GET /api/admin/curricula/{id}/lessons`、`POST /api/admin/curricula/{id}/lessons`、`PUT /api/admin/lessons/{id}`、`DELETE /api/admin/lessons/{id}`
  - `GET /api/admin/lesson-runs?limit=50` → `{"items": [...]}` 按开始时间倒序，含 `lesson_title/lesson_seq/curriculum_title/artifact_images`（作品缩略图路径列表，来自关联 Turn 的 `input_image_path`）
  - `PUT /api/admin/lesson-runs/{id}`（body 可含 `status/parent_note/artifact_turn_ids`，家长修正用）
  - Curriculum JSON 形状 `{id, slug, title, age_band, description, status, current_lesson_id}`；Lesson JSON `{id, curriculum_id, seq, slug, title, goal_text, script_text, segments, duration_min, materials, enhancements}`（Task 7 前端、Task 6 手机接口都用）

- [ ] **Step 1: 写失败测试**

`server/tests/test_admin_curricula.py`：

```python
from app import models


def make_curriculum(client, **over):
    body = {"slug": "shapes-01", "title": "形状小画家", "age_band": "3-4", **over}
    r = client.post("/api/admin/curricula", json=body)
    assert r.status_code == 200
    return r.json()


def make_lesson(client, cid, seq=1, **over):
    body = {"seq": seq, "title": f"第{seq}课", "script_text": "脚本", **over}
    r = client.post(f"/api/admin/curricula/{cid}/lessons", json=body)
    assert r.status_code == 200
    return r.json()


def test_curriculum_crud(client):
    c = make_curriculum(client)
    assert c["status"] == "draft" and c["current_lesson_id"] is None
    r = client.put(f"/api/admin/curricula/{c['id']}", json={"title": "改名"})
    assert r.json()["title"] == "改名"
    assert client.get("/api/admin/curricula").json()[0]["title"] == "改名"
    assert client.delete(f"/api/admin/curricula/{c['id']}").status_code == 200
    assert client.get("/api/admin/curricula").json() == []


def test_duplicate_slug_400(client):
    make_curriculum(client)
    r = client.post("/api/admin/curricula", json={"slug": "shapes-01", "title": "重复"})
    assert r.status_code == 400


def test_activate_exclusive_and_archived_untouched(client):
    a = make_curriculum(client, slug="a")
    b = make_curriculum(client, slug="b")
    c = make_curriculum(client, slug="c")
    client.put(f"/api/admin/curricula/{c['id']}", json={"status": "archived"})
    client.post(f"/api/admin/curricula/{a['id']}/activate")
    client.post(f"/api/admin/curricula/{b['id']}/activate")
    by_slug = {x["slug"]: x["status"] for x in client.get("/api/admin/curricula").json()}
    assert by_slug == {"a": "draft", "b": "active", "c": "archived"}


def test_lessons_crud_sorted_by_seq(client):
    c = make_curriculum(client)
    make_lesson(client, c["id"], seq=2)
    l1 = make_lesson(client, c["id"], seq=1)
    lessons = client.get(f"/api/admin/curricula/{c['id']}/lessons").json()
    assert [x["seq"] for x in lessons] == [1, 2]
    r = client.put(f"/api/admin/lessons/{l1['id']}", json={"goal_text": "目标"})
    assert r.json()["goal_text"] == "目标"
    assert client.delete(f"/api/admin/lessons/{l1['id']}").status_code == 200


def test_pointer_set_and_validation(client):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    r = client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": l1["id"]})
    assert r.json()["current_lesson_id"] == l1["id"]
    r = client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": None})
    assert r.json()["current_lesson_id"] is None
    assert client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": 999}).status_code == 400


def test_delete_curriculum_cascades(client, db):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    db.add(models.LessonRun(lesson_id=l1["id"]))
    db.commit()
    client.delete(f"/api/admin/curricula/{c['id']}")
    assert db.query(models.Lesson).count() == 0
    assert db.query(models.LessonRun).count() == 0


def test_runs_list_and_correction(client, db):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1, title="圆圆的朋友")
    t = models.Turn(source="tablet", input_image_path="images/x.png")
    db.add(t)
    db.flush()
    run = models.LessonRun(lesson_id=l1["id"], status="completed",
                           highlights="亮点", artifact_turn_ids=[t.id])
    db.add(run)
    db.commit()
    items = client.get("/api/admin/lesson-runs").json()["items"]
    assert items[0]["lesson_title"] == "圆圆的朋友"
    assert items[0]["curriculum_title"] == "形状小画家"
    assert items[0]["artifact_images"] == ["images/x.png"]
    r = client.put(f"/api/admin/lesson-runs/{run.id}",
                   json={"status": "skipped", "parent_note": "当天生病"})
    assert r.json()["status"] == "skipped" and r.json()["parent_note"] == "当天生病"
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_admin_curricula.py`
Expected: FAIL（404）

- [ ] **Step 3: 实现**

`server/app/routers/admin_curricula.py`：

```python
from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Curriculum, Lesson, LessonRun, Turn

router = APIRouter(prefix="/api/admin")

CUR_FIELDS = ("slug", "title", "age_band", "description", "status")
LESSON_FIELDS = ("seq", "slug", "title", "goal_text", "script_text", "segments",
                 "duration_min", "materials", "enhancements")
RUN_STATUS = ("running", "completed", "partial", "skipped", "abandoned")


class CurriculumIn(BaseModel):
    slug: str | None = None
    title: str | None = None
    age_band: str | None = None
    description: str | None = None
    status: str | None = None


class LessonIn(BaseModel):
    seq: int | None = None
    slug: str | None = None
    title: str | None = None
    goal_text: str | None = None
    script_text: str | None = None
    segments: list | None = None
    duration_min: int | None = None
    materials: str | None = None
    enhancements: list | None = None


class PointerIn(BaseModel):
    lesson_id: int | None = None


class RunIn(BaseModel):
    status: str | None = None
    parent_note: str | None = None
    artifact_turn_ids: list | None = None


def cur_json(c: Curriculum) -> dict:
    return {f: getattr(c, f) for f in CUR_FIELDS} | {
        "id": c.id, "current_lesson_id": c.current_lesson_id,
    }


def lesson_json(l: Lesson) -> dict:
    return {f: getattr(l, f) for f in LESSON_FIELDS} | {
        "id": l.id, "curriculum_id": l.curriculum_id,
    }


def _cur_or_404(db: Session, cid: int) -> Curriculum:
    c = db.get(Curriculum, cid)
    if c is None:
        raise HTTPException(404, "课程不存在")
    return c


@router.get("/curricula")
def list_curricula(db: Session = Depends(get_db)):
    return [cur_json(c) for c in db.query(Curriculum).order_by(Curriculum.id).all()]


@router.post("/curricula")
def create_curriculum(body: CurriculumIn, db: Session = Depends(get_db)):
    if not body.slug:
        raise HTTPException(400, "slug 不能为空")
    if db.query(Curriculum).filter(Curriculum.slug == body.slug).first():
        raise HTTPException(400, "slug 已存在")
    c = Curriculum(slug=body.slug)
    for f in CUR_FIELDS[1:]:
        v = getattr(body, f)
        if v is not None:
            setattr(c, f, v)
    db.add(c)
    db.commit()
    return cur_json(c)


@router.put("/curricula/{cid}")
def update_curriculum(cid: int, body: CurriculumIn, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    for f in CUR_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(c, f, v)
    db.commit()
    return cur_json(c)


@router.delete("/curricula/{cid}")
def delete_curriculum(cid: int, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    lesson_ids = [l.id for l in db.query(Lesson).filter(Lesson.curriculum_id == cid).all()]
    if lesson_ids:
        db.query(LessonRun).filter(LessonRun.lesson_id.in_(lesson_ids)).delete()
        db.query(Lesson).filter(Lesson.id.in_(lesson_ids)).delete()
    db.delete(c)
    db.commit()
    return {"ok": True}


@router.post("/curricula/{cid}/activate")
def activate_curriculum(cid: int, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    db.query(Curriculum).filter(Curriculum.status == "active").update(
        {Curriculum.status: "draft"}
    )
    c.status = "active"
    db.commit()
    return cur_json(c)


@router.put("/curricula/{cid}/pointer")
def set_pointer(cid: int, body: PointerIn, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    if body.lesson_id is not None:
        lesson = db.get(Lesson, body.lesson_id)
        if lesson is None or lesson.curriculum_id != cid:
            raise HTTPException(400, "课时不属于该课程")
    c.current_lesson_id = body.lesson_id
    db.commit()
    return cur_json(c)


@router.get("/curricula/{cid}/lessons")
def list_lessons(cid: int, db: Session = Depends(get_db)):
    _cur_or_404(db, cid)
    rows = db.query(Lesson).filter(Lesson.curriculum_id == cid).order_by(Lesson.seq).all()
    return [lesson_json(l) for l in rows]


@router.post("/curricula/{cid}/lessons")
def create_lesson(cid: int, body: LessonIn, db: Session = Depends(get_db)):
    _cur_or_404(db, cid)
    l = Lesson(curriculum_id=cid, seq=body.seq or 1)
    for f in LESSON_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(l, f, v)
    db.add(l)
    db.commit()
    return lesson_json(l)


@router.put("/lessons/{lid}")
def update_lesson(lid: int, body: LessonIn, db: Session = Depends(get_db)):
    l = db.get(Lesson, lid)
    if l is None:
        raise HTTPException(404, "课时不存在")
    for f in LESSON_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(l, f, v)
    db.commit()
    return lesson_json(l)


@router.delete("/lessons/{lid}")
def delete_lesson(lid: int, db: Session = Depends(get_db)):
    l = db.get(Lesson, lid)
    if l is None:
        raise HTTPException(404, "课时不存在")
    db.query(LessonRun).filter(LessonRun.lesson_id == lid).delete()
    db.delete(l)
    db.commit()
    return {"ok": True}


@router.get("/lesson-runs")
def list_runs(limit: int = 50, db: Session = Depends(get_db)):
    rows = (
        db.query(LessonRun, Lesson, Curriculum)
        .join(Lesson, LessonRun.lesson_id == Lesson.id)
        .join(Curriculum, Lesson.curriculum_id == Curriculum.id)
        .order_by(LessonRun.started_at.desc(), LessonRun.id.desc())
        .limit(limit)
        .all()
    )
    items = []
    for run, lesson, cur in rows:
        images = []
        if run.artifact_turn_ids:
            turns = db.query(Turn).filter(Turn.id.in_(run.artifact_turn_ids)).all()
            images = [t.input_image_path for t in turns if t.input_image_path]
        items.append({
            "id": run.id, "lesson_id": lesson.id, "lesson_seq": lesson.seq,
            "lesson_title": lesson.title, "curriculum_title": cur.title,
            "started_at": run.started_at.isoformat() if run.started_at else "",
            "ended_at": run.ended_at.isoformat() if run.ended_at else "",
            "status": run.status, "highlights": run.highlights,
            "parent_tip": run.parent_tip, "parent_note": run.parent_note,
            "artifact_turn_ids": run.artifact_turn_ids or [],
            "artifact_images": images,
        })
    return {"items": items}


@router.put("/lesson-runs/{rid}")
def update_run(rid: int, body: RunIn, db: Session = Depends(get_db)):
    run = db.get(LessonRun, rid)
    if run is None:
        raise HTTPException(404, "上课记录不存在")
    if body.status is not None:
        if body.status not in RUN_STATUS:
            raise HTTPException(400, "非法状态")
        run.status = body.status
    if body.parent_note is not None:
        run.parent_note = body.parent_note
    if body.artifact_turn_ids is not None:
        run.artifact_turn_ids = body.artifact_turn_ids
    db.commit()
    return {"id": run.id, "status": run.status, "parent_note": run.parent_note,
            "artifact_turn_ids": run.artifact_turn_ids or []}
```

`server/app/main.py` 路由注册处把 `admin_curricula` 加进 import 并注册（仿一期写法）：

```python
    from app.routers import (admin_curricula, admin_profiles, admin_providers,
                             admin_voice, files, openai_compat, phone)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
    app.include_router(admin_curricula.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(files.router)
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/admin_curricula.py server/app/main.py server/tests/test_admin_curricula.py
git commit -m "feat(server): curriculum admin API (curricula/lessons/runs)"
```

---

### Task 4: 示范课程种子（形状小画家 8 课全部内容）

**Files:**
- Create: `server/app/seed_shapes.py`
- Modify: `server/app/routers/admin_curricula.py`（加 seed 接口）
- Test: `server/tests/test_seed_shapes.py`

**Interfaces:**
- Consumes: Task 1 模型、Task 2 的 `RECAP_TOKEN`
- Produces: `seed_shapes01(db) -> Curriculum`（幂等：slug `shapes-01` 已存在则原样返回，不重复建）；`POST /api/admin/curricula/seed-shapes01` → Curriculum JSON；`TABLET_PRAISE_RULE`（§6.5 平板夸奖规则文本常量，Task 7 前端展示用）

- [ ] **Step 1: 写失败测试**

`server/tests/test_seed_shapes.py`：

```python
from app import models
from app.engine.lesson import LESSON_REPORT_MARK, RECAP_TOKEN


def test_seed_creates_8_lessons(client, db):
    r = client.post("/api/admin/curricula/seed-shapes01")
    assert r.status_code == 200
    c = r.json()
    assert c["slug"] == "shapes-01" and c["age_band"] == "3-4"
    lessons = client.get(f"/api/admin/curricula/{c['id']}/lessons").json()
    assert len(lessons) == 8
    assert [l["seq"] for l in lessons] == list(range(1, 9))
    assert [l["slug"] for l in lessons] == [f"shapes-01-{i:02d}" for i in range(1, 9)]
    assert lessons[2]["title"] == "圆圆的朋友"


def test_seed_scripts_are_complete(client, db):
    client.post("/api/admin/curricula/seed-shapes01")
    lessons = db.query(models.Lesson).order_by(models.Lesson.seq).all()
    assert RECAP_TOKEN not in lessons[0].script_text        # 第 1 课无复习占位符
    for l in lessons[1:]:
        assert RECAP_TOKEN in l.script_text                 # 第 2-8 课都有
    for l in lessons:
        assert LESSON_REPORT_MARK in l.script_text          # 打标协议写进每课脚本
        assert "五环节" in l.script_text and l.goal_text and l.materials
        assert l.segments and len(l.segments) == 5
        assert l.segments[3]["channel"] == "tablet"         # 第④环节走平板


def test_seed_idempotent(client, db):
    a = client.post("/api/admin/curricula/seed-shapes01").json()
    b = client.post("/api/admin/curricula/seed-shapes01").json()
    assert a["id"] == b["id"]
    assert db.query(models.Lesson).count() == 8
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_seed_shapes.py`
Expected: FAIL（404）

- [ ] **Step 3: 实现种子模块**

`server/app/seed_shapes.py`（内容来自 spec §5/§6，`script_text` = 课时头 + 五环节 + 共用规则）：

```python
"""「形状小画家」示范课程种子数据。内容与 spec §5 八课详案一一对应。"""

from app.models import Curriculum, Lesson

TABLET_PRAISE_RULE = (
    "收到孩子的涂鸦画作时：回复第一句是具体的夸奖，必须点出画面里至少一个真实元素；"
    "然后手写画一个小符号奖励（⭐、❤ 或 ☺）；最后可以加一个关于画的小问题。"
    "全文不超过 3 行，不评价画得像不像，不提「图片/照片」。"
)

# 每课共用的固定规则（话术/弹性/判定与打标），拼接在每课脚本末尾，保证脚本自包含。
FIXED_RULES = """【话术规则】
- 每轮只说 1-2 句、一个指令，不连环提问。夸具体行为不夸聪明（说「这个泡泡好大好圆」，不说「你真棒真聪明」）。
- 不纠错：画得不像不说「不对」，把重试藏进新任务里。孩子说「不会画」时拆到最小动作，允许家长手把手。
- 你看不见平板上的画。不要假装看见，改问「给我讲讲你画了什么呀」。
- 孩子口齿不清、转写可能出错：听不懂时不猜硬答，温柔请孩子再说一遍或请家长帮忙。
【弹性规则】
- 孩子跑题：跟着聊半分钟再温柔拉回。不想画：改成空中画，或让孩子指挥家长画。
- 提前画完：追加「再画一个更大的」。烦躁：直接跳到收尾环节，参与即完成。
【判定与打标】
- 宽松参与制：孩子动手画了、有互动就算 completed；中途跳收尾算 partial；几乎没参与算 skipped。
- 课程收尾时（只在收尾那一轮），在回复最后另起一行输出（此行家长孩子都看不到，务必单行）：
⟦lesson_report⟧{"lesson_id":"<本课slug>","status":"completed|partial|skipped","highlights":"<今天孩子的具体表现，1-2句>","parent_tip":"<给家长的在家延伸小活动，1句>"}"""


def _segments(warmup: str, teach: str, draw: str) -> list:
    return [
        {"seq": 1, "kind": "warmup", "channel": "voice", "goal": warmup},
        {"seq": 2, "kind": "teach", "channel": "voice", "goal": teach},
        {"seq": 3, "kind": "draw", "channel": "voice", "goal": draw},
        {"seq": 4, "kind": "submit", "channel": "tablet", "goal": "把画发给 DouDou，手写夸奖+小符号"},
        {"seq": 5, "kind": "closing", "channel": "voice", "goal": "夸奖收尾，预告下次，输出打标"},
    ]


def _script(header: str, five_steps: str) -> str:
    return f"{header}\n【五环节脚本】\n{five_steps}\n{FIXED_RULES}"

MATERIALS = "手机打开 DouDou 语音页；平板进入画板；找个安静角落，大约 10 分钟。"

LESSONS: list[dict] = [
    dict(
        seq=1, slug="shapes-01-01", title="认识 DouDou·想画就画",
        goal_text="破冰：敢拿笔、在纸上留下任何痕迹、认识 DouDou。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 1 课「认识 DouDou·想画就画」\n"
            "【本课目标】孩子敢拿笔随便画，体验「画完给 DouDou 看」的流程。",
            "① 自我介绍：「我是 DouDou，我最爱看小朋友画画啦」，问孩子的名字并记住使用\n"
            "② 引入：笔宝宝想在纸上跳舞，它跳什么舞都好看\n"
            "③ 布置：随便画，画什么都可以；孩子画完问「给我讲讲这是什么呀」，不替孩子命名画作\n"
            "④ 请家长帮孩子把画发给我看（平板上发送）\n"
            "⑤ 夸画画这件事本身（下笔大胆、线条多），给自己拍拍手；预告下次玩长长的线",
        ),
        segments=_segments("认识彼此，问名字", "笔宝宝跳舞的联想", "自由涂鸦，零要求"),
        enhancements=["DouDou 简笔画自画像卡（二期）"],
    ),
    dict(
        seq=2, slug="shapes-01-02", title="长长的线",
        goal_text="有控制地画线：横线/竖线/波浪线任一即可。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 2 课「长长的线」\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子有控制地画出线条（横/竖/波浪任一）。",
            "① 问好；复习：上次笔宝宝跳了舞，我们在空中再挥一挥\n"
            "② 认识线条：下雨是竖线、煮面条是波浪线、小路是横线；让孩子说还有什么长长的\n"
            "③ 布置：「下雨啦，画好多好多雨丝！」（孩子更爱面条/小路就随孩子）；用拟声词带节奏「从上往下，唰——」；画完可以数数几根\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾夸线条（方向/长短/数量）；预告下次认识圆圆的朋友",
        ),
        segments=_segments("空中挥笔复习", "线条的生活联想", "画雨丝/面条/小路"),
        enhancements=["描线练习格：虚线雨丝描一描（二期）"],
    ),
    dict(
        seq=3, slug="shapes-01-03", title="圆圆的朋友",
        goal_text="画封闭的圆（歪歪扭扭都算）；说出 1 个圆的生活联想。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 3 课「圆圆的朋友」\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子敢画封闭的圆；说出一个圆圆的东西。",
            "① 问好；复习：空中画一条长长的线\n"
            "② 认识圆：问「圆圆的像什么呀」，接住任何答案再补充泡泡/太阳；空中描个圆\n"
            "③ 布置：「我们来吹泡泡吧，画大大小小的圆泡泡！」；封闭说成「让线的头和尾巴牵上手」；先大圆再小圆\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾夸泡泡（数量/大小对比，头尾没牵手不批评）；预告下次圆和线做朋友",
        ),
        segments=_segments("空中画线复习", "圆的生活联想", "画大小圆泡泡"),
        enhancements=["描圆练习格（二期）"],
    ),
    dict(
        seq=4, slug="shapes-01-04", title="圆和线做朋友",
        goal_text="组合旧元素：圆+线（太阳光芒/气球绳/棒棒糖）。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 4 课「圆和线做朋友」（综合复习课）\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子把圆和线组合在一起。",
            "① 问好；复习：空中画个圆、再画条线（两个都来一遍）\n"
            "② 引入组合：圆和线做朋友会变成什么？太阳发光芒、气球拉绳子、棒棒糖\n"
            "③ 布置：「给太阳画光芒」或「给气球拴绳子」选一个；先画圆，再「让线从圆身上长出来」\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾点出组合关系（「气球拉着长长的线要飞起来啦」）；预告下次方方的朋友",
        ),
        segments=_segments("空中画圆+画线", "圆+线的组合联想", "画太阳光芒或气球绳"),
        enhancements=["DouDou 简笔画示范卡：发光的太阳（二期）"],
    ),
    dict(
        seq=5, slug="shapes-01-05", title="方方的朋友",
        goal_text="画方形：建立「转弯、回家（闭合）」意识，歪扭都算。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 5 课「方方的朋友」\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子画出带转弯的方形。",
            "① 问好；复习：空中画个圆\n"
            "② 认识方：小饼干、小窗户、积木都是方方的；让孩子摸摸身边方方的东西\n"
            "③ 布置：「画一块小饼干」，口诀「画一条线，转个弯，再转个弯，再转个弯，回家啦！」；可以给饼干点芝麻\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾夸转弯和角角；预告下次尖尖的三角",
        ),
        segments=_segments("空中画圆复习", "方形的生活联想", "画小饼干"),
        enhancements=["描方练习格（二期）"],
    ),
    dict(
        seq=6, slug="shapes-01-06", title="尖尖的三角",
        goal_text="画三角形（上坡—下坡—回家）。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 6 课「尖尖的三角」\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子画出有尖角的三角形。",
            "① 问好；复习：空中玩「转弯回家」画方游戏\n"
            "② 认识三角：小山、小旗子、冰淇淋筒；强调「尖尖的头」\n"
            "③ 布置：「画一座小山」，三步口诀「上坡——下坡——回家！」；画得开心可以画一排山\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾夸尖尖的角（「都戳到云朵啦」）；预告下次形状变变变",
        ),
        segments=_segments("空中画方复习", "三角的生活联想", "画小山"),
        enhancements=["描三角练习格（二期）"],
    ),
    dict(
        seq=7, slug="shapes-01-07", title="形状变变变",
        goal_text="用 2-3 种形状组合成一个东西；能说出用了哪些形状。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 7 课「形状变变变」（综合复习课）\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子用旧形状拼出一个东西，并说出用了什么形状。",
            "① 问好；复习：空中快闪游戏，依次画圆、方、三角\n"
            "② 引入：形状朋友拼在一起会变身！方+三角=小房子，圆+三角=冰淇淋\n"
            "③ 布置：「给小兔子盖一座房子」或「做一个冰淇淋」选一；先问「要用哪些形状朋友呀」让孩子说，再一步一形状（先画方方的墙，再戴三角形屋顶帽子）\n"
            "④ 请家长帮孩子把画发给我看\n"
            "⑤ 收尾点出用到的形状名；预告下次开小画展",
        ),
        segments=_segments("空中快闪圆方三角", "形状组合变身联想", "拼房子或冰淇淋"),
        enhancements=["形状卡片展示：圆/方/三角（二期）"],
    ),
    dict(
        seq=8, slug="shapes-01-08", title="我的小画展",
        goal_text="自由创作一幅画并讲述出来（语言表达是主目标）；完成奖励仪式。参与即成功。",
        script=_script(
            "【今天的课】形状小画家 · 第 8 课「我的小画展」（汇报课）\n"
            "【上节课回顾】{prev_lesson_recap}\n"
            "【本课目标】孩子自由创作并讲述自己的画；隆重的奖励仪式。",
            "① 问好；复习：「我们认识了哪些形状朋友呀？」孩子说几个都对，DouDou 补全\n"
            "② 引入：今天开小画展！画一幅最喜欢的画，画完讲给我听\n"
            "③ 孩子创作；讲述引导三问：画的是什么？它在做什么？你最喜欢哪里？每答一问都热情回应\n"
            "④ 请家长帮孩子把画发给我看；这一轮要隆重：手写「小画家奖」+星星，家长郑重念出来，建议保留这一页\n"
            "⑤ 收尾总结 8 课成长（从随手涂鸦到会用形状拼东西）；highlights 里写整轮课程的成长总结",
        ),
        segments=_segments("回忆全部形状朋友", "小画展仪式引入", "自由创作+讲述三问"),
        enhancements=["结构化奖状卡片渲染（二期）"],
    ),
]


def seed_shapes01(db) -> Curriculum:
    """幂等导入示范课程：slug 已存在则直接返回既有课程。"""
    existing = db.query(Curriculum).filter(Curriculum.slug == "shapes-01").first()
    if existing is not None:
        return existing
    cur = Curriculum(
        slug="shapes-01", title="形状小画家", age_band="3-4",
        description="3-4 岁涂鸦启蒙微课程：8 课，每课约 10 分钟。语音主线+平板提交点，"
                    "沿线条→圆→方→三角→组合→自由创作的梯度，参与即成功。",
    )
    db.add(cur)
    db.flush()
    first_lesson_id = None
    for spec in LESSONS:
        lesson = Lesson(
            curriculum_id=cur.id, seq=spec["seq"], slug=spec["slug"], title=spec["title"],
            goal_text=spec["goal_text"], script_text=spec["script"],
            segments=spec["segments"], duration_min=10, materials=MATERIALS,
            enhancements=spec["enhancements"],
        )
        db.add(lesson)
        db.flush()
        if spec["seq"] == 1:
            first_lesson_id = lesson.id
    cur.current_lesson_id = first_lesson_id
    db.commit()
    return cur
```

`server/app/routers/admin_curricula.py` 追加（文件末尾）：

```python
from app.seed_shapes import seed_shapes01


@router.post("/curricula/seed-shapes01")
def seed_demo_curriculum(db: Session = Depends(get_db)):
    return cur_json(seed_shapes01(db))
```

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/seed_shapes.py server/app/routers/admin_curricula.py server/tests/test_seed_shapes.py
git commit -m "feat(server): seed shapes-01 demo curriculum with 8 lesson scripts"
```

---

### Task 5: 提示词与 Turn 引擎扩展（lesson_context 注入）

**Files:**
- Modify: `server/app/engine/prompt.py`（`assemble_system_prompt` 加 `lesson_context` 参数）
- Modify: `server/app/engine/turn.py`（`TurnInput` 加 2 字段；`stream()` 里接线）
- Test: `server/tests/test_lesson_turn.py`

**Interfaces:**
- Consumes: 一期 `assemble_system_prompt`/`TurnInput`/`TurnRunner`
- Produces: `assemble_system_prompt(persona, *, voice_hint="", lesson_context="", protocol_suffix="")`（顺序：人设→voice_hint→课时脚本→记忆协议后缀；空串跳过）；`TurnInput` 新增 `lesson_context: str = ""`、`lesson_run_id: int | None = None`；`TurnRunner.stream()` 组装时注入 lesson_context 并把 `turn.lesson_run_id` 落库。**默认值保证一期所有调用点行为不变**

- [ ] **Step 1: 写失败测试**

`server/tests/test_lesson_turn.py`：

```python
import httpx
import respx

from app import models
from app.engine.prompt import assemble_system_prompt
from app.engine.turn import TurnInput, TurnRunner

SSE = (
    'data: {"choices":[{"delta":{"content":"我们开始上课啦"}}]}\n\n'
    "data: [DONE]\n\n"
)


def test_assemble_with_lesson_context():
    out = assemble_system_prompt(
        "你是 DouDou。", voice_hint="口语化。", lesson_context="【今天的课】第 3 课",
        protocol_suffix="\n\n记忆协议：xxx",
    )
    assert out == "你是 DouDou。\n\n口语化。\n\n【今天的课】第 3 课\n\n记忆协议：xxx"


def test_assemble_blank_lesson_context_ignored():
    assert assemble_system_prompt("你是 DouDou。", lesson_context="  ") == "你是 DouDou。"


def _setup_profile(db):
    p = models.Provider(name="up", base_url="https://up.test/v1", api_key="sk")
    db.add(p)
    db.flush()
    db.add(models.Profile(name="小班", persona_text="你是 DouDou。", provider_id=p.id,
                          model="m", is_active=True))
    db.commit()


@respx.mock
async def test_runner_injects_context_and_stamps_run_id(app, db):
    _setup_profile(db)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    tin = TurnInput(source="phone", text="开始吧", use_voice_hint=True,
                    lesson_context="【今天的课】圆圆的朋友", lesson_run_id=42)
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir, tin)
    [_ async for _ in runner.stream()]
    import json
    body = json.loads(route.calls[0].request.content)
    assert "【今天的课】圆圆的朋友" in body["messages"][0]["content"]
    turn = db.query(models.Turn).one()
    assert turn.lesson_run_id == 42
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_lesson_turn.py`
Expected: FAIL（`assemble_system_prompt` 不接受 `lesson_context`）

- [ ] **Step 3: 实现**

`server/app/engine/prompt.py` 的 `assemble_system_prompt` 改为：

```python
def assemble_system_prompt(
    persona: str, *, voice_hint: str = "", lesson_context: str = "", protocol_suffix: str = ""
) -> str:
    out = persona.strip()
    if voice_hint.strip():
        out += "\n\n" + voice_hint.strip()
    if lesson_context.strip():
        out += "\n\n" + lesson_context.strip()
    if protocol_suffix:
        out += protocol_suffix  # 后缀自带 \n\n 前导
    return out
```

`server/app/engine/turn.py`：`TurnInput` 追加两个字段：

```python
    lesson_context: str = ""
    lesson_run_id: int | None = None
```

`TurnRunner.stream()` 中两处接线：建 `Turn(...)` 后加：

```python
        turn.lesson_run_id = tin.lesson_run_id
```

`assemble_system_prompt` 调用处改为：

```python
                self.system_prompt = assemble_system_prompt(
                    profile.persona_text,
                    voice_hint=profile.voice_hint if tin.use_voice_hint else "",
                    lesson_context=tin.lesson_context,
                    protocol_suffix=tin.device_protocol_suffix,
                )
```

- [ ] **Step 4: 运行确认通过（全量，一期 prompt/turn 测试必须仍绿）**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/engine/prompt.py server/app/engine/turn.py server/tests/test_lesson_turn.py
git commit -m "feat(server): lesson context injection in prompt and turn engine"
```

---

### Task 6: 手机课程接口（上课生命周期 + 打标闭环）

**Files:**
- Modify: `server/app/routers/phone.py`（加 3 个课程接口；voice-turn 扩展）
- Test: `server/tests/test_phone_lesson.py`

**Interfaces:**
- Consumes: Task 2 引擎函数、Task 4 seed（测试用）、Task 5 的 TurnInput 新字段、一期 phone 路由结构
- Produces:
  - `GET /api/phone/current-lesson` → `{"available": false}` 或 `{"available": true, "curriculum_title", "lesson_id", "lesson_seq", "lesson_title"}`
  - `POST /api/phone/lesson-runs` → `{"lesson_run_id", "lesson_seq", "lesson_title"}`；无生效课程/指针为空时 400 中文错误；创建前把所有遗留 `running` 的 run 置为 `abandoned`
  - `POST /api/phone/lesson-runs/{run_id}/end` → `{"ok": true, "status"}`（仅 `running` 会被置 `abandoned`，幂等）
  - `POST /api/phone/voice-turn` 新增可选表单字段 `lesson_run_id`；响应新增 `"lesson_report": null | {"status", "highlights", "parent_tip"}`；打标行绝不进 `reply_text`/TTS；打标到达时更新 run、挂作品、推指针

- [ ] **Step 1: 写失败测试**

`server/tests/test_phone_lesson.py`：

```python
import json

import httpx
import respx

from app import models


def sse_reply(text: str) -> str:
    delta = json.dumps({"choices": [{"delta": {"content": text}}]}, ensure_ascii=False)
    return f"data: {delta}\n\ndata: [DONE]\n\n"


REPORT = ('⟦lesson_report⟧{"lesson_id":"shapes-01-01","status":"completed",'
          '"highlights":"敢下笔了","parent_tip":"在家一起涂鸦"}')


def setup_course(client):
    p = client.post("/api/admin/providers",
                    json={"name": "up", "base_url": "https://up.test/v1", "api_key": "sk"}).json()
    prof = client.post("/api/admin/profiles", json={
        "name": "生效", "persona_text": "你是 DouDou。", "voice_hint": "口语化",
        "provider_id": p["id"], "model": "m",
    }).json()
    client.post(f"/api/admin/profiles/{prof['id']}/activate")
    client.put("/api/admin/voice-settings", json={
        "stt_provider_id": p["id"], "stt_model": "whisper-1",
        "tts_provider_id": p["id"], "tts_model": "tts-1", "tts_voice": "alloy",
    })
    c = client.post("/api/admin/curricula/seed-shapes01").json()
    client.post(f"/api/admin/curricula/{c['id']}/activate")
    return c


def test_current_lesson_unavailable_by_default(client):
    assert client.get("/api/phone/current-lesson").json() == {"available": False}


def test_current_lesson_and_run_creation(client):
    setup_course(client)
    j = client.get("/api/phone/current-lesson").json()
    assert j["available"] is True and j["lesson_seq"] == 1
    assert j["curriculum_title"] == "形状小画家"

    r = client.post("/api/phone/lesson-runs").json()
    assert r["lesson_run_id"] > 0 and r["lesson_title"] == "认识 DouDou·想画就画"


def test_run_creation_without_course_400(client):
    r = client.post("/api/phone/lesson-runs")
    assert r.status_code == 400 and "课程" in r.json()["detail"]


def test_stale_running_swept_on_new_run(client, db):
    setup_course(client)
    a = client.post("/api/phone/lesson-runs").json()
    b = client.post("/api/phone/lesson-runs").json()
    assert db.get(models.LessonRun, a["lesson_run_id"]).status == "abandoned"
    assert db.get(models.LessonRun, b["lesson_run_id"]).status == "running"


def test_end_endpoint_abandons_running(client, db):
    setup_course(client)
    r = client.post("/api/phone/lesson-runs").json()
    j = client.post(f"/api/phone/lesson-runs/{r['lesson_run_id']}/end").json()
    assert j == {"ok": True, "status": "abandoned"}
    # 幂等：再次 end 不改状态
    j2 = client.post(f"/api/phone/lesson-runs/{r['lesson_run_id']}/end").json()
    assert j2["status"] == "abandoned"


@respx.mock
def test_voice_turn_with_lesson_full_loop(client, db):
    c = setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]

    # 课中一轮平板提交（作品）
    tablet = models.Turn(source="tablet", input_image_path="images/draw.png")
    db.add(tablet)
    db.commit()

    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "我画完啦"})
    )
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("太棒啦，下次见！\n" + REPORT))
    )
    speech = respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    r = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"AUDIO", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    )
    assert r.status_code == 200
    j = r.json()
    # 打标行绝不外泄
    assert j["reply_text"] == "太棒啦，下次见！"
    assert j["lesson_report"] == {"status": "completed", "highlights": "敢下笔了",
                                  "parent_tip": "在家一起涂鸦"}
    # system prompt 注入了课时脚本
    sent = json.loads(chat.calls[0].request.content)
    assert "第 1 课" in sent["messages"][0]["content"]
    # TTS 收到的是干净文本
    tts_body = json.loads(speech.calls[0].request.content)
    assert "lesson_report" not in tts_body["input"]
    # run 关闭、作品挂靠、指针推进
    run = db.get(models.LessonRun, run_id)
    assert run.status == "completed" and run.artifact_turn_ids == [tablet.id]
    cur = db.query(models.Curriculum).filter(models.Curriculum.slug == "shapes-01").one()
    lesson2 = db.query(models.Lesson).filter(
        models.Lesson.curriculum_id == cur.id, models.Lesson.seq == 2).one()
    assert cur.current_lesson_id == lesson2.id
    # 落库 turn 干净且归属本课
    turn = db.query(models.Turn).filter(models.Turn.source == "phone").one()
    assert turn.lesson_run_id == run_id and "lesson_report" not in turn.reply_text


@respx.mock
def test_voice_turn_mid_lesson_no_report(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "画好了三个泡泡"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("三个泡泡真圆呀"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["lesson_report"] is None
    assert db.get(models.LessonRun, run_id).status == "running"


@respx.mock
def test_voice_turn_with_closed_run_falls_back_to_chat(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    client.post(f"/api/phone/lesson-runs/{run_id}/end")
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "随便聊聊"})
    )
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("好呀"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["lesson_report"] is None
    sent = json.loads(chat.calls[0].request.content)
    assert "今天的课" not in sent["messages"][0]["content"]  # 不注入课时脚本
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_phone_lesson.py`
Expected: FAIL（404 / 响应缺 `lesson_report` 字段）

- [ ] **Step 3: 实现**

`server/app/routers/phone.py` 顶部 import 追加：

```python
from fastapi import Depends
from app.db import get_db
from app.engine.lesson import (close_run_with_report, latest_recap,
                               parse_lesson_report, render_lesson_script)
from app.models import Curriculum, Lesson, LessonRun, utcnow
```

追加三个课程接口：

```python
def _active_current_lesson(db) -> tuple[Curriculum, Lesson] | None:
    cur = db.query(Curriculum).filter(Curriculum.status == "active").first()
    if cur is None or cur.current_lesson_id is None:
        return None
    lesson = db.get(Lesson, cur.current_lesson_id)
    if lesson is None:
        return None
    return cur, lesson


@router.get("/current-lesson")
def current_lesson(db: Session = Depends(get_db)):
    found = _active_current_lesson(db)
    if found is None:
        return {"available": False}
    cur, lesson = found
    return {"available": True, "curriculum_title": cur.title, "lesson_id": lesson.id,
            "lesson_seq": lesson.seq, "lesson_title": lesson.title}


@router.post("/lesson-runs")
def start_lesson_run(db: Session = Depends(get_db)):
    found = _active_current_lesson(db)
    if found is None:
        raise HTTPException(400, "请先在 DouDou 后台设置生效课程与当前课时")
    _, lesson = found
    for stale in db.query(LessonRun).filter(LessonRun.status == "running").all():
        stale.status = "abandoned"
        stale.ended_at = utcnow()
    run = LessonRun(lesson_id=lesson.id)
    db.add(run)
    db.commit()
    return {"lesson_run_id": run.id, "lesson_seq": lesson.seq, "lesson_title": lesson.title}


@router.post("/lesson-runs/{run_id}/end")
def end_lesson_run(run_id: int, db: Session = Depends(get_db)):
    run = db.get(LessonRun, run_id)
    if run is None:
        raise HTTPException(404, "上课记录不存在")
    if run.status == "running":
        run.status = "abandoned"
        run.ended_at = utcnow()
        db.commit()
    return {"ok": True, "status": run.status}
```

`voice_turn` 端点整体替换为（在一期版本上加课程逻辑，其余不变）：

```python
@router.post("/voice-turn")
async def voice_turn(
    request: Request,
    audio: UploadFile,
    history: str = Form("[]"),
    lesson_run_id: int | None = Form(None),
):
    pairs = json.loads(history)  # [["user","assistant"], ...]
    msgs: list[dict] = []
    for u, a in pairs:
        msgs.append({"role": "user", "content": u})
        msgs.append({"role": "assistant", "content": a})

    # 课程模式：run 有效且 running 时注入课时脚本（短事务，取完即关）
    lesson_context = ""
    active_run_id: int | None = None
    if lesson_run_id is not None:
        with request.app.state.sessionmaker() as db:  # type: Session
            run = db.get(LessonRun, lesson_run_id)
            if run is not None and run.status == "running":
                lesson = db.get(Lesson, run.lesson_id)
                if lesson is not None:
                    recap = latest_recap(db, lesson.curriculum_id)
                    lesson_context = render_lesson_script(lesson.script_text, recap)
                    active_run_id = run.id

    data = await audio.read()
    tin = TurnInput(source="phone", audio=data,
                    audio_filename=audio.filename or "audio.webm",
                    history=msgs, use_voice_hint=True,
                    lesson_context=lesson_context, lesson_run_id=active_run_id)
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError as e:
        raise HTTPException(502, f"模型服务出错（{e.status_code}）")

    # 打标剥离：无论是否课程模式，标记行都不外泄、不进 TTS
    clean_text, report, raw = parse_lesson_report(runner.reply_text)
    lesson_report_out = None

    with request.app.state.sessionmaker() as db:  # type: Session
        turn = db.get(Turn, runner.turn_id)
        if turn is not None and clean_text != runner.reply_text:
            turn.reply_text = clean_text
        if report is not None and active_run_id is not None:
            run = db.get(LessonRun, active_run_id)
            if run is not None and run.status == "running":
                close_run_with_report(db, run, report, raw)
                lesson_report_out = {"status": run.status, "highlights": run.highlights,
                                     "parent_tip": run.parent_tip}
        try:
            _, tts_cfg = load_voice_config(db)
            audio_bytes = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"],
                                           tts_cfg["model"], tts_cfg["voice"],
                                           clean_text, tts_cfg["speed"])
            rel = f"audio/{uuid.uuid4().hex}.mp3"
            with open(f"{request.app.state.data_dir}/{rel}", "wb") as f:
                f.write(audio_bytes)
            turn = db.get(Turn, runner.turn_id)
            turn.reply_audio_path = rel
            db.commit()
            audio_url = f"/api/files/{rel}"
        except (ConfigError, UpstreamError):
            db.commit()  # 打标/剥离结果仍要落库
            audio_url = ""  # TTS 失败不阻塞文字回复

    return {"turn_id": runner.turn_id, "transcript": runner.transcript,
            "reply_text": clean_text, "audio_url": audio_url,
            "lesson_report": lesson_report_out}
```

注意：一期 `test_phone.py` 断言的响应字段仍全部存在（新增 `lesson_report` 不破坏旧断言）。

- [ ] **Step 4: 运行确认通过（全量）**

Run: `cd server && uv run pytest`
Expected: 全部通过（含一期 `test_phone.py`）

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/phone.py server/tests/test_phone_lesson.py
git commit -m "feat(server): phone lesson lifecycle and report loop"
```

---

### Task 7: 后台「课程」页

**Files:**
- Create: `server/web/src/pages/Curricula.tsx`
- Modify: `server/web/src/App.tsx`（MENU 加一项 + 路由加一条 + import）
- Test: `cd server/web && npm run build`（TS 编译即验证）

**Interfaces:**
- Consumes: Task 3/4 的 `/api/admin/curricula*`、`/api/admin/lesson-runs*`、`/api/admin/curricula/seed-shapes01`；一期 `api.ts` 的 `get/post/put`
- Produces: `/admin/curricula` 页面：课程列表（生效/指针）、课时编辑抽屉、上课记录（小结卡+作品图+修正）、一键导入示范课程、平板夸奖规则提示块

- [ ] **Step 1: App 路由**

`server/web/src/App.tsx`：import 区加 `import Curricula from './pages/Curricula'`；`MENU` 在「人设 Profile」之后插入：

```tsx
  { key: '/admin/curricula', label: '课程' },
```

`Routes` 内加：

```tsx
          <Route path="curricula" element={<Curricula />} />
```

- [ ] **Step 2: 课程页组件**

`server/web/src/pages/Curricula.tsx`：

```tsx
import { Alert, Button, Card, Drawer, Form, Input, InputNumber, Select, Space, Table, Tag, Typography, message } from 'antd'
import { useEffect, useState } from 'react'
import { get, post, put } from '../api'

type Curriculum = {
  id: number; slug: string; title: string; age_band: string
  description: string; status: string; current_lesson_id: number | null
}
type Lesson = {
  id: number; curriculum_id: number; seq: number; slug: string; title: string
  goal_text: string; script_text: string; duration_min: number; materials: string
}
type Run = {
  id: number; lesson_seq: number; lesson_title: string; curriculum_title: string
  started_at: string; status: string; highlights: string; parent_tip: string
  parent_note: string; artifact_images: string[]
}

const STATUS = { draft: '草稿', active: '生效中', archived: '已归档' } as Record<string, string>
const RUN_STATUS = {
  running: '进行中', completed: '完成', partial: '部分完成',
  skipped: '未参与', abandoned: '未收尾',
} as Record<string, string>
const RUN_COLOR = {
  running: 'blue', completed: 'green', partial: 'gold', skipped: 'default', abandoned: 'default',
} as Record<string, string>

const TABLET_PRAISE_RULE =
  '收到孩子的涂鸦画作时：回复第一句是具体的夸奖，必须点出画面里至少一个真实元素；' +
  '然后手写画一个小符号奖励（⭐、❤ 或 ☺）；最后可以加一个关于画的小问题。' +
  '全文不超过 3 行，不评价画得像不像，不提「图片/照片」。'

export default function Curricula() {
  const [curricula, setCurricula] = useState<Curriculum[]>([])
  const [lessons, setLessons] = useState<Lesson[]>([])
  const [selected, setSelected] = useState<Curriculum | null>(null)
  const [runs, setRuns] = useState<Run[]>([])
  const [editing, setEditing] = useState<Lesson | null>(null)
  const [form] = Form.useForm()

  const loadCurricula = async () => {
    const list: Curriculum[] = await get('/api/admin/curricula')
    setCurricula(list)
    const cur = list.find(c => selected && c.id === selected.id) ?? list.find(c => c.status === 'active') ?? list[0] ?? null
    setSelected(cur)
    if (cur) setLessons(await get(`/api/admin/curricula/${cur.id}/lessons`))
    else setLessons([])
  }
  const loadRuns = async () => setRuns((await get('/api/admin/lesson-runs?limit=50')).items)

  useEffect(() => { loadCurricula(); loadRuns() }, [])

  const seed = async () => {
    await post('/api/admin/curricula/seed-shapes01')
    message.success('已导入「形状小画家」示范课程')
    loadCurricula()
  }
  const activate = async (c: Curriculum) => { await post(`/api/admin/curricula/${c.id}/activate`); loadCurricula() }
  const setPointer = async (lessonId: number | null) => {
    if (!selected) return
    await put(`/api/admin/curricula/${selected.id}/pointer`, { lesson_id: lessonId })
    message.success('当前课时已更新')
    loadCurricula()
  }
  const pickCurriculum = async (c: Curriculum) => {
    setSelected(c)
    setLessons(await get(`/api/admin/curricula/${c.id}/lessons`))
  }
  const saveLesson = async () => {
    if (!editing) return
    await put(`/api/admin/lessons/${editing.id}`, await form.validateFields())
    setEditing(null)
    message.success('课时已保存')
    if (selected) setLessons(await get(`/api/admin/curricula/${selected.id}/lessons`))
  }
  const fixRun = async (r: Run, patch: object) => {
    await put(`/api/admin/lesson-runs/${r.id}`, patch)
    loadRuns()
  }

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Alert type="info" showIcon message="平板夸奖规则（复制到 3-4 岁 profile 的人设文本末尾，让平板提交轮的手写回复符合课程设计）"
        description={<Typography.Paragraph copyable style={{ marginBottom: 0 }}>{TABLET_PRAISE_RULE}</Typography.Paragraph>} />

      <Card title="课程" extra={<Button onClick={seed}>一键导入示范课程「形状小画家」</Button>}>
        <Table rowKey="id" dataSource={curricula} pagination={false}
          onRow={c => ({ onClick: () => pickCurriculum(c) })}
          columns={[
            { title: '名称', dataIndex: 'title' },
            { title: '年龄段', dataIndex: 'age_band' },
            { title: '状态', dataIndex: 'status', render: (v: string) => <Tag color={v === 'active' ? 'green' : 'default'}>{STATUS[v] ?? v}</Tag> },
            {
              title: '当前课时', render: (_, c) => {
                const l = lessons.find(x => x.id === c.current_lesson_id)
                if (c.id !== selected?.id) return c.current_lesson_id ?? '—'
                return l ? `第 ${l.seq} 课 ${l.title}` : '（本轮已完成）'
              },
            },
            {
              title: '操作', render: (_, c) => (
                <Button size="small" disabled={c.status === 'active'} onClick={e => { e.stopPropagation(); activate(c) }}>设为生效</Button>
              ),
            },
          ]} />
      </Card>

      {selected && (
        <Card title={`课时（${selected.title}）`}
          extra={
            <Space>
              <span>当前课时：</span>
              <Select style={{ width: 220 }} value={selected.current_lesson_id}
                onChange={v => setPointer(v)} allowClear placeholder="（本轮已完成）"
                options={lessons.map(l => ({ value: l.id, label: `第 ${l.seq} 课 ${l.title}` }))} />
            </Space>
          }>
          <Table rowKey="id" dataSource={lessons} pagination={false} columns={[
            { title: '#', dataIndex: 'seq', width: 50 },
            { title: '课名', dataIndex: 'title' },
            { title: '目标', dataIndex: 'goal_text', ellipsis: true },
            { title: '时长', dataIndex: 'duration_min', width: 70, render: (v: number) => `${v}'` },
            {
              title: '', width: 80, render: (_, l) => (
                <Button size="small" onClick={() => { setEditing(l); form.setFieldsValue(l) }}>编辑</Button>
              ),
            },
          ]} />
        </Card>
      )}

      <Card title="上课记录">
        <Table rowKey="id" dataSource={runs} columns={[
          { title: '时间', dataIndex: 'started_at', render: (v: string) => (v ? new Date(v + 'Z').toLocaleString('zh-CN') : '') },
          { title: '课时', render: (_, r) => `第 ${r.lesson_seq} 课 ${r.lesson_title}` },
          {
            title: '状态', dataIndex: 'status', render: (v: string, r) => (
              <Select size="small" value={v} style={{ width: 110 }}
                onChange={s => fixRun(r, { status: s })}
                options={Object.entries(RUN_STATUS).map(([value, label]) => ({ value, label }))}
                labelRender={() => <Tag color={RUN_COLOR[v]}>{RUN_STATUS[v] ?? v}</Tag>} />
            ),
          },
          { title: '亮点', dataIndex: 'highlights', ellipsis: true },
          { title: '在家延伸', dataIndex: 'parent_tip', ellipsis: true },
          {
            title: '作品', render: (_, r) => (
              <Space>
                {r.artifact_images.map(p => (
                  <img key={p} src={`/api/files/${p}`} alt="" style={{ height: 40 }} />
                ))}
              </Space>
            ),
          },
          {
            title: '家长补记', dataIndex: 'parent_note', render: (v: string, r) => (
              <Typography.Text editable={{ onChange: t => fixRun(r, { parent_note: t }) }}>{v}</Typography.Text>
            ),
          },
        ]} />
      </Card>

      <Drawer open={!!editing} width={720} title={editing ? `第 ${editing.seq} 课 ${editing.title}` : ''}
        onClose={() => setEditing(null)}
        extra={<Button type="primary" onClick={saveLesson}>保存</Button>}>
        <Form form={form} layout="vertical">
          <Form.Item name="title" label="课名"><Input /></Form.Item>
          <Form.Item name="goal_text" label="教学目标"><Input.TextArea rows={2} /></Form.Item>
          <Form.Item name="script_text" label="课时脚本（注入语音轮 system prompt；{prev_lesson_recap} 会替换为上次课小结）">
            <Input.TextArea rows={16} />
          </Form.Item>
          <Form.Item name="materials" label="课前准备（家长可见）"><Input.TextArea rows={2} /></Form.Item>
          <Form.Item name="duration_min" label="目标时长（分钟）"><InputNumber min={1} max={30} /></Form.Item>
        </Form>
      </Drawer>
    </Space>
  )
}
```

- [ ] **Step 3: 构建验证**

Run: `cd server/web && npm run build`
Expected: `tsc` 无错误

- [ ] **Step 4: Commit**

```bash
git add server/web/src/pages/Curricula.tsx server/web/src/App.tsx
git commit -m "feat(web): curriculum admin page"
```

---

### Task 8: 手机页上课模式

**Files:**
- Modify: `server/web/src/pages/Phone.tsx`
- Test: `cd server/web && npm run build`

**Interfaces:**
- Consumes: Task 6 的 `/api/phone/current-lesson`、`/api/phone/lesson-runs*`、voice-turn 的 `lesson_run_id` 表单字段与 `lesson_report` 响应字段
- Produces: 手机页顶部课程条：有当前课时则显示「开始上课：第 N 课 ×××」；上课中显示课名横幅 + 「结束」按钮；voice-turn 自动携带 `lesson_run_id`；收到 `lesson_report` 后显示完成气泡并自动退出上课状态

- [ ] **Step 1: 扩展 Phone 组件**

`server/web/src/pages/Phone.tsx` 覆盖为（在一期版本上加课程条与 lesson_run 状态；录音逻辑不变）：

```tsx
import { useEffect, useRef, useState } from 'react'

type Bubble = { role: 'user' | 'assistant' | 'system'; text: string }
type CurrentLesson = { available: boolean; lesson_seq?: number; lesson_title?: string; curriculum_title?: string }

const REPORT_LABEL = { completed: '完成', partial: '部分完成', skipped: '未参与' } as Record<string, string>

export default function Phone() {
  const [bubbles, setBubbles] = useState<Bubble[]>([])
  const [state, setState] = useState<'idle' | 'recording' | 'thinking'>('idle')
  const [error, setError] = useState('')
  const [lesson, setLesson] = useState<CurrentLesson>({ available: false })
  const [runId, setRunId] = useState<number | null>(null)
  const [runTitle, setRunTitle] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const historyRef = useRef<[string, string][]>([])
  const runRef = useRef<number | null>(null)
  runRef.current = runId

  useEffect(() => {
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }, [])

  const startLesson = async () => {
    setError('')
    try {
      const resp = await fetch('/api/phone/lesson-runs', { method: 'POST' })
      if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
      const j = await resp.json()
      setRunId(j.lesson_run_id)
      setRunTitle(`第 ${j.lesson_seq} 课 · ${j.lesson_title}`)
      historyRef.current = []
      setBubbles([{ role: 'system', text: `开始上课：第 ${j.lesson_seq} 课《${j.lesson_title}》。按住下面的按钮，让孩子跟豆豆打个招呼吧！` }])
    } catch (e) { setError(String(e)) }
  }

  const endLesson = async () => {
    if (runRef.current != null) {
      await fetch(`/api/phone/lesson-runs/${runRef.current}/end`, { method: 'POST' }).catch(() => {})
    }
    setRunId(null)
    setRunTitle('')
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }

  const start = async () => {
    if (state !== 'idle') return
    setError('')
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      const rec = new MediaRecorder(stream)
      const chunks: Blob[] = []
      rec.ondataavailable = e => chunks.push(e.data)
      rec.onstop = async () => {
        stream.getTracks().forEach(t => t.stop())
        setState('thinking')
        const fd = new FormData()
        fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'say.webm')
        fd.append('history', JSON.stringify(historyRef.current.slice(-5)))
        if (runRef.current != null) fd.append('lesson_run_id', String(runRef.current))
        try {
          const resp = await fetch('/api/phone/voice-turn', { method: 'POST', body: fd })
          if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
          const j = await resp.json()
          historyRef.current.push([j.transcript, j.reply_text])
          setBubbles(b => [...b, { role: 'user', text: j.transcript }, { role: 'assistant', text: j.reply_text }])
          if (j.audio_url) new Audio(j.audio_url).play()
          if (j.lesson_report) {
            const label = REPORT_LABEL[j.lesson_report.status] ?? j.lesson_report.status
            setBubbles(b => [...b, {
              role: 'system',
              text: `⭐ 今天的课${label}啦！\n亮点：${j.lesson_report.highlights}\n在家可以试试：${j.lesson_report.parent_tip}`,
            }])
            setRunId(null)
            setRunTitle('')
            fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
          }
        } catch (e) {
          setError(String(e))
        } finally { setState('idle') }
      }
      recRef.current = rec
      rec.start()
      setState('recording')
    } catch {
      setError('无法使用麦克风：请确认已用 https 打开本页并允许麦克风权限')
    }
  }

  const stop = () => { if (state === 'recording') recRef.current?.stop() }

  return (
    <div style={{
      minHeight: '100vh', display: 'flex', flexDirection: 'column',
      background: '#fffbe6', fontFamily: 'sans-serif',
    }}>
      <div style={{ padding: 16, fontSize: 20, fontWeight: 700, textAlign: 'center' }}>豆豆 🎈</div>
      {runId == null && lesson.available && (
        <div style={{ padding: '0 16px 8px', textAlign: 'center' }}>
          <button onClick={startLesson} style={{
            padding: '10px 18px', borderRadius: 20, border: 'none', fontSize: 16,
            background: '#52c41a', color: '#fff',
          }}>
            开始上课：第 {lesson.lesson_seq} 课《{lesson.lesson_title}》
          </button>
        </div>
      )}
      {runId != null && (
        <div style={{
          margin: '0 16px 8px', padding: '8px 12px', borderRadius: 12, background: '#f6ffed',
          border: '1px solid #b7eb8f', display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        }}>
          <span>📚 上课中：{runTitle}</span>
          <button onClick={endLesson} style={{ border: 'none', background: 'transparent', color: '#999' }}>结束</button>
        </div>
      )}
      <div style={{ flex: 1, overflowY: 'auto', padding: '0 16px' }}>
        {bubbles.map((b, i) => (
          <p key={i} style={{ textAlign: b.role === 'user' ? 'right' : b.role === 'system' ? 'center' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '10px 14px', borderRadius: 16, fontSize: 17,
              maxWidth: '80%', whiteSpace: 'pre-wrap', textAlign: 'left',
              background: b.role === 'user' ? '#bae0ff' : b.role === 'system' ? '#fff7e6' : '#fff',
              boxShadow: '0 1px 2px rgba(0,0,0,.1)',
            }}>{b.text}</span>
          </p>
        ))}
        {error && <p style={{ color: 'red', textAlign: 'center' }}>{error}</p>}
      </div>
      <div style={{ padding: 24, textAlign: 'center' }}>
        <button
          onPointerDown={start} onPointerUp={stop} onPointerLeave={stop}
          style={{
            width: 120, height: 120, borderRadius: '50%', border: 'none', fontSize: 18,
            color: '#fff', touchAction: 'none', userSelect: 'none', WebkitUserSelect: 'none',
            background: state === 'recording' ? '#ff4d4f' : state === 'thinking' ? '#d9d9d9' : '#1677ff',
          }}>
          {state === 'recording' ? '松开提问' : state === 'thinking' ? '豆豆想…' : '按住说话'}
        </button>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: 构建验证**

Run: `cd server/web && npm run build`
Expected: `tsc` 无错误

- [ ] **Step 3: Commit**

```bash
git add server/web/src/pages/Phone.tsx
git commit -m "feat(web): phone lesson mode with report bubble"
```

---

### Task 9: README 更新 + 手工验收清单

**Files:**
- Modify: `server/README.md`（一期建的服务器说明文件；若一期实际放在别处，同义平移）

**Interfaces:**
- Consumes: 全部前序任务

- [ ] **Step 1: README 追加「课程」章节**

在 `server/README.md` 末尾追加：

```markdown
## 课程（形状小画家）

1. 后台「课程」页点「一键导入示范课程」，再点「设为生效」。
2. 把页面顶部的「平板夸奖规则」复制进 3-4 岁 profile 的人设文本末尾（平板提交轮的手写夸奖靠它）。
3. 手机页出现「开始上课」按钮：点开始 → 按住说话跟着豆豆走五环节 →
   孩子画完后在平板上把画发给豆豆（作品会按时间自动挂到本课记录）→
   课程收尾后手机页出现小结气泡，后台「课程」页可看每课的亮点、在家延伸与作品。
4. 中途退出：点课程条上的「结束」（记录状态为「未收尾」，家长可在后台修正）。

设计文档：`docs/superpowers/specs/2026-07-22-mini-curriculum-design.md`
```

- [ ] **Step 2: 手工验收（需真实 API key 与已配置的语音 provider）**

1. `cd server && uv run uvicorn --factory app.main:create_app --port 8787`，后台导入并生效课程。
2. 手机/浏览器开 `/phone`：开始上课 → 说「你好」→ 应听到第 1 课开场（自我介绍+问名字）。
3. 正常聊 2-3 轮后说「今天上完啦，再见」→ 回复应出现小结气泡，且语音里**听不到** lesson_report 内容。
4. 后台「课程」页：该 run 状态为完成、有亮点/在家延伸；当前课时推进到第 2 课。
5. 测试台贴一张手写图发平板通路一轮，再上一节课收尾 → 该图片出现在本课「作品」列。

- [ ] **Step 3: 全量回归 + Commit**

Run: `cd server && uv run pytest && cd web && npm run build`
Expected: 全部通过、无 TS 错误

```bash
git add server/README.md
git commit -m "docs(server): curriculum usage and acceptance checklist"
```

---

## 实施决策记录（后端批次 Task 1-6 执行后追加，供二期与前端批次参考）

1. **recap 取材范围**：`latest_recap` 只取 `completed/partial` 的 run，**不含 `skipped`**。spec §8 原文「取上一条非 abandoned 的 run」字面上含 skipped；实施取更优教学语义（几乎未参与的课不值得作为「上次课回顾」喂给模型）。如产品负责人倾向 spec 字面义，改一行过滤条件即可。
2. **`turns.lesson_run_id` 为普通 Integer 列而非外键**：SQLite `ALTER TABLE ADD COLUMN` 不能带 FK 约束，且本列为可空的弱关联（关联 run 删除后允许悬空，由应用层清理）。属有意为之。
3. **打标失败兜底**（终审补强）：打标 JSON 损坏时，run 以 `close_run_malformed` 关闭——`status=abandoned`、`raw_report={"_raw": 原文}`、作品照常挂靠；标记文本仍绝不进入回复/TTS。
4. **作品挂靠触发点**：`close_run_with_report`（正常收尾）与 `end_lesson_run`（家长手动结束）都挂靠；`start_lesson_run` 的遗留 run 清扫**不**挂靠（跨天窗口会误挂无关涂鸦）。
5. **标记剥离下沉引擎**：`⟦lesson_report⟧` 的剥离在 `TurnRunner` 持久化之前完成（非计划原文的路由器层剥离），使「标记绝不落库」成为绝对不变量；`TurnRunner` 暴露 `lesson_report`/`lesson_report_raw` 给路由器。
6. **搁置项（前端批次时顺手处理）**：admin_turns 接口暴露 `lesson_run_id`；`artifact_images` 按 `artifact_turn_ids` 保序；课程/课时级联删除时清理 turns 的悬空 `lesson_run_id`；`update_curriculum` 禁止把 slug 置空。

## Self-Review 结论（已按此修订）

1. **Spec 覆盖**：§8 三表+turns 加列 → Task 1；§6.3 打标协议/兜底 → Task 2/6；§6.2 触发形态（生效互斥、指针、自动推进、手动修正）→ Task 3/6；§5 八课详案 → Task 4 种子；§6.1 软脚本注入与 §4 复习闭环 → Task 2/5/6；§7 小结卡与家长修正 → Task 3/6/7；§6.5 平板夸奖规则 → Task 4 常量 + Task 7 提示块 + Task 9 README；§8 时间窗挂靠 → Task 2 `attach_artifacts`。spec §8 的 `memory_tags` 一期恒空——Task 1 只建列，无任务写入，符合挂载点定位。
2. **占位符扫描**：无 TBD/TODO；所有代码步骤给出完整代码。
3. **类型一致性**：`parse_lesson_report` 三元组、`lesson_report` 响应字段、`RECAP_TOKEN`、`lesson_run_id` 表单字段名在 Task 2/5/6/8 间一致；`cur_json`/`lesson_json` 形状与 Task 7 前端 type 对齐。
