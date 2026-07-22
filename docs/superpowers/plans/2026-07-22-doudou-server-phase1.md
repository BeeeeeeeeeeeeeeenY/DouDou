# DouDou Server 一期实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `server/` 下建成 DouDou Server：OpenAI 兼容门面（平板零改动接入）+ 中文管理后台（模型/人设/语音/测试台/对话记录）+ 手机按住说话页。

**Architecture:** FastAPI 单应用，核心是 Turn 引擎（`TurnRunner`）：所有入口（平板门面 `/v1/chat/completions`、测试台、手机语音）走同一 `run` 管线 —— 取生效 profile → 组装 system prompt → httpx 流式调上游 → 落库 turns。SQLite 存配置与记录，React+AntD 提供中文界面，FastAPI 托管前端构建产物。

**Tech Stack:** Python 3.12 + uv + FastAPI + SQLAlchemy 2 + httpx；测试 pytest + pytest-asyncio + respx；前端 Node 20+ + Vite + React 18 + TypeScript + Ant Design 5 + react-router-dom。

**Spec:** `docs/superpowers/specs/2026-07-22-doudou-server-phase1-design.md`（本计划的需求来源，冲突时以 spec 为准）

## Global Constraints

- 目录：后端 `server/app`，测试 `server/tests`，前端 `server/web`，运行数据 `server/data`（git 忽略，含 `images/`、`audio/`、`doudou.db`）
- 端口：http `8787`（平板+管理），https `8788`（手机页，mkcert 证书存在时才启用）
- 平板侧唯一改动是 `oracle.env`（`RIDDLE_OPENAI_BASE=http://<Mac IP>:8787/v1`），**不改任何 Rust 代码**
- 记忆协议标记：`\n\n记忆协议：`（设备 system prompt 中该标记起的后缀必须原样保留追加）
- 转写标记：`⁂`（U+2042）；门面转发给平板的流**不得剥离** `⁂` 后缀，仅服务端解析落库
- 面向 riddle 的错误响应用 `PlainTextResponse` 中文短句，且**正文不得包含字符串 `max_completion_tokens`**（riddle 见此串会换字段名重试）
- 所有 `json.dumps` 输出中文时 `ensure_ascii=False`
- profiles 表 `knowledge_base`、`memory` 两个 JSON 列一期恒为 NULL，只建列不实现
- 无登录鉴权；API key 明文存本地 SQLite（仅局域网个人部署，README 注明）
- UI 全中文（AntD `zhCN` locale）
- 每个任务以通过测试 + git commit 结束；后端测试命令统一 `cd server && uv run pytest`

---

### Task 1: 项目脚手架 + 数据库模型

**Files:**
- Create: `server/pyproject.toml`
- Create: `server/.gitignore`
- Create: `server/app/__init__.py`（空文件）
- Create: `server/app/config.py`
- Create: `server/app/models.py`
- Create: `server/app/db.py`
- Create: `server/app/main.py`
- Create: `server/tests/__init__.py`（空文件）
- Create: `server/tests/conftest.py`
- Test: `server/tests/test_db.py`

**Interfaces:**
- Produces: `create_app(data_dir: str | None = None) -> FastAPI`（`app.state.sessionmaker`、`app.state.data_dir` 可用）；模型类 `Provider/Profile/VoiceSettings/Turn`；`get_db(request)` FastAPI 依赖，yield SQLAlchemy Session；`GET /api/health` 返回 `{"ok": true}`
- Consumes: 无（首个任务）

- [ ] **Step 1: 写 pyproject 与 .gitignore**

`server/pyproject.toml`：

```toml
[project]
name = "doudou-server"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
    "fastapi>=0.115",
    "uvicorn[standard]>=0.30",
    "sqlalchemy>=2.0",
    "httpx>=0.27",
    "python-multipart>=0.0.9",
]

[dependency-groups]
dev = [
    "pytest>=8.0",
    "pytest-asyncio>=0.24",
    "respx>=0.21",
]

[tool.pytest.ini_options]
asyncio_mode = "auto"
testpaths = ["tests"]
```

`server/.gitignore`：

```
data/
certs/
__pycache__/
.venv/
web/node_modules/
web/dist/
```

- [ ] **Step 2: 写失败测试**

`server/tests/conftest.py`：

```python
import pytest
from fastapi.testclient import TestClient

from app.main import create_app


@pytest.fixture()
def app(tmp_path):
    return create_app(data_dir=str(tmp_path))


@pytest.fixture()
def client(app):
    with TestClient(app) as c:
        yield c


@pytest.fixture()
def db(app):
    with app.state.sessionmaker() as session:
        yield session
```

`server/tests/test_db.py`：

```python
from app import models


def test_health(client):
    assert client.get("/api/health").json() == {"ok": True}


def test_tables_created_and_voice_singleton(db):
    # create_app 应建好所有表，并保证 voice_settings 有且仅有 id=1 一行
    assert db.query(models.Provider).count() == 0
    assert db.query(models.Profile).count() == 0
    assert db.query(models.Turn).count() == 0
    vs = db.query(models.VoiceSettings).all()
    assert len(vs) == 1 and vs[0].id == 1


def test_data_subdirs_created(app):
    import os
    for sub in ("images", "audio"):
        assert os.path.isdir(os.path.join(app.state.data_dir, sub))
```

- [ ] **Step 3: 运行确认失败**

Run: `cd server && uv run pytest`
Expected: FAIL（`ModuleNotFoundError: app.main` 或类似）

- [ ] **Step 4: 实现**

`server/app/config.py`：

```python
import os

SERVER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_data_dir(data_dir: str | None) -> str:
    d = data_dir or os.environ.get("DOUDOU_DATA") or os.path.join(SERVER_DIR, "data")
    for sub in ("", "images", "audio"):
        os.makedirs(os.path.join(d, sub), exist_ok=True)
    return d
```

`server/app/models.py`：

```python
from datetime import datetime, timezone

from sqlalchemy import JSON, DateTime, Float, ForeignKey, Integer, String, Text
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class Base(DeclarativeBase):
    pass


class Provider(Base):
    __tablename__ = "providers"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(100))
    base_url: Mapped[str] = mapped_column(String(500))  # 不带尾斜杠
    api_key: Mapped[str] = mapped_column(String(500), default="")
    enabled: Mapped[bool] = mapped_column(default=True)
    notes: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)


class Profile(Base):
    __tablename__ = "profiles"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(100))
    age_band: Mapped[str] = mapped_column(String(10), default="")  # "3-4"|"5-6"|"6-7"
    persona_text: Mapped[str] = mapped_column(Text, default="")
    voice_hint: Mapped[str] = mapped_column(Text, default="")
    provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    model: Mapped[str] = mapped_column(String(200), default="")
    temperature: Mapped[float | None] = mapped_column(Float, nullable=True)
    max_tokens: Mapped[int] = mapped_column(Integer, default=2000)
    reasoning_effort: Mapped[str] = mapped_column(String(10), default="")  # ""|low|medium|high
    is_active: Mapped[bool] = mapped_column(default=False)
    knowledge_base: Mapped[dict | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期挂载点
    memory: Mapped[dict | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期挂载点
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class VoiceSettings(Base):
    __tablename__ = "voice_settings"
    id: Mapped[int] = mapped_column(primary_key=True)  # 恒为 1 的单行表
    stt_provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    stt_model: Mapped[str] = mapped_column(String(200), default="")
    tts_provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    tts_model: Mapped[str] = mapped_column(String(200), default="")
    tts_voice: Mapped[str] = mapped_column(String(100), default="")
    tts_speed: Mapped[float] = mapped_column(Float, default=1.0)


class Turn(Base):
    __tablename__ = "turns"
    id: Mapped[int] = mapped_column(primary_key=True)
    ts: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    source: Mapped[str] = mapped_column(String(10))  # tablet|test|phone
    profile_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    profile_name: Mapped[str] = mapped_column(String(100), default="")
    model: Mapped[str] = mapped_column(String(200), default="")
    system_prompt: Mapped[str] = mapped_column(Text, default="")
    input_text: Mapped[str] = mapped_column(Text, default="")
    input_image_path: Mapped[str] = mapped_column(String(500), default="")
    input_audio_path: Mapped[str] = mapped_column(String(500), default="")
    transcript: Mapped[str] = mapped_column(Text, default="")
    reply_text: Mapped[str] = mapped_column(Text, default="")
    reply_audio_path: Mapped[str] = mapped_column(String(500), default="")
    latency_ms: Mapped[int] = mapped_column(Integer, default=0)
    status: Mapped[str] = mapped_column(String(10), default="ok")  # ok|error
    error: Mapped[str] = mapped_column(Text, default="")
```

`server/app/db.py`：

```python
from fastapi import Request
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker

from app import models


def make_sessionmaker(data_dir: str):
    engine = create_engine(
        f"sqlite:///{data_dir}/doudou.db", connect_args={"check_same_thread": False}
    )
    models.Base.metadata.create_all(engine)
    maker = sessionmaker(bind=engine, expire_on_commit=False)
    with maker() as s:  # voice_settings 单行保底
        if s.get(models.VoiceSettings, 1) is None:
            s.add(models.VoiceSettings(id=1))
            s.commit()
    return maker


def get_db(request: Request):
    with request.app.state.sessionmaker() as session:  # type: Session
        yield session
```

`server/app/main.py`：

```python
from fastapi import FastAPI

from app.config import resolve_data_dir
from app.db import make_sessionmaker


def create_app(data_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="DouDou Server")
    app.state.data_dir = resolve_data_dir(data_dir)
    app.state.sessionmaker = make_sessionmaker(app.state.data_dir)

    @app.get("/api/health")
    def health():
        return {"ok": True}

    return app
```

- [ ] **Step 5: 运行确认通过**

Run: `cd server && uv run pytest`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add server/pyproject.toml server/.gitignore server/app server/tests server/uv.lock
git commit -m "feat(server): scaffold FastAPI app with SQLite models"
```

---

### Task 2: 提示词组装与转写解析（纯函数）

**Files:**
- Create: `server/app/engine/__init__.py`（空文件）
- Create: `server/app/engine/prompt.py`
- Create: `server/app/engine/transcript.py`
- Test: `server/tests/test_prompt.py`
- Test: `server/tests/test_transcript.py`

**Interfaces:**
- Produces: `split_protocol_suffix(device_system: str) -> tuple[str, str]`；`assemble_system_prompt(persona: str, *, voice_hint: str = "", protocol_suffix: str = "") -> str`；`split_transcript(full_text: str) -> tuple[str, str]`（可见回复, 转写）
- Consumes: 无

- [ ] **Step 1: 写失败测试**

`server/tests/test_prompt.py`：

```python
from app.engine.prompt import assemble_system_prompt, split_protocol_suffix

DEVICE_SYSTEM = "你是设备内置人设。\n\n记忆协议：系统会给你一个记忆目录，⟦show:N⟧ 规则等等。"


def test_split_finds_protocol_suffix():
    persona, suffix = split_protocol_suffix(DEVICE_SYSTEM)
    assert persona == "你是设备内置人设。"
    assert suffix.startswith("\n\n记忆协议：")
    assert "⟦show:N⟧" in suffix


def test_split_without_marker():
    persona, suffix = split_protocol_suffix("只有人设没有协议")
    assert persona == "只有人设没有协议"
    assert suffix == ""


def test_assemble_persona_only():
    assert assemble_system_prompt("  你是 DouDou。  ") == "你是 DouDou。"


def test_assemble_with_voice_hint_and_suffix():
    out = assemble_system_prompt(
        "你是 DouDou。", voice_hint="这是语音对话，口语化。", protocol_suffix="\n\n记忆协议：xxx"
    )
    assert out == "你是 DouDou。\n\n这是语音对话，口语化。\n\n记忆协议：xxx"


def test_assemble_blank_voice_hint_ignored():
    assert assemble_system_prompt("你是 DouDou。", voice_hint="   ") == "你是 DouDou。"
```

`server/tests/test_transcript.py`：

```python
from app.engine.transcript import split_transcript


def test_split_transcript():
    visible, transcript = split_transcript("答案是三。\n⁂静夜思 三")
    assert visible == "答案是三。"
    assert transcript == "静夜思 三"


def test_no_transcript_marker():
    visible, transcript = split_transcript("普通回复")
    assert visible == "普通回复"
    assert transcript == ""
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_prompt.py tests/test_transcript.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/prompt.py`：

```python
PROTOCOL_MARKER = "\n\n记忆协议："


def split_protocol_suffix(device_system: str) -> tuple[str, str]:
    """把设备 system prompt 切成 (设备人设, 记忆协议后缀)。无标记时后缀为空。"""
    idx = device_system.find(PROTOCOL_MARKER)
    if idx == -1:
        return device_system, ""
    return device_system[:idx], device_system[idx:]


def assemble_system_prompt(
    persona: str, *, voice_hint: str = "", protocol_suffix: str = ""
) -> str:
    out = persona.strip()
    if voice_hint.strip():
        out += "\n\n" + voice_hint.strip()
    if protocol_suffix:
        out += protocol_suffix  # 后缀自带 \n\n 前导
    return out
```

`server/app/engine/transcript.py`：

```python
TRANSCRIPT_MARK = "⁂"  # U+2042，riddle 记忆协议的转写后缀标记


def split_transcript(full_text: str) -> tuple[str, str]:
    if TRANSCRIPT_MARK not in full_text:
        return full_text.strip(), ""
    visible, _, post = full_text.partition(TRANSCRIPT_MARK)
    return visible.strip(), post.strip()
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_prompt.py tests/test_transcript.py`
Expected: 7 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/engine server/tests/test_prompt.py server/tests/test_transcript.py
git commit -m "feat(server): prompt assembly and transcript parsing"
```

---

### Task 3: 上游模型流式客户端

**Files:**
- Create: `server/app/engine/upstream.py`
- Test: `server/tests/test_upstream.py`

**Interfaces:**
- Produces: `class UpstreamError(Exception)`（属性 `status_code: int`、`detail: str`）；`build_chat_body(model, messages, *, temperature=None, max_tokens=2000, reasoning_effort="") -> dict`；`async def stream_chat(base_url: str, api_key: str, body: dict) -> AsyncIterator[str]`（逐段 yield delta 文本）
- Consumes: 无

- [ ] **Step 1: 写失败测试**

`server/tests/test_upstream.py`：

```python
import httpx
import pytest
import respx

from app.engine.upstream import UpstreamError, build_chat_body, stream_chat

SSE = (
    'data: {"choices":[{"delta":{"content":"你好"}}]}\n\n'
    'data: {"choices":[{"delta":{"content":"，小朋友"}}]}\n\n'
    'data: {"choices":[{"delta":{}}]}\n\n'
    "data: [DONE]\n\n"
)


def test_build_chat_body_full():
    body = build_chat_body(
        "gpt-4o-mini",
        [{"role": "user", "content": "hi"}],
        temperature=0.7,
        max_tokens=500,
        reasoning_effort="low",
    )
    assert body == {
        "model": "gpt-4o-mini",
        "stream": True,
        "max_tokens": 500,
        "temperature": 0.7,
        "reasoning_effort": "low",
        "messages": [{"role": "user", "content": "hi"}],
    }


def test_build_chat_body_omits_unset():
    body = build_chat_body("m", [])
    assert "temperature" not in body and "reasoning_effort" not in body
    assert body["max_tokens"] == 2000


@respx.mock
async def test_stream_chat_yields_deltas():
    respx.post("https://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    chunks = [c async for c in stream_chat("https://api.test/v1", "sk-x", build_chat_body("m", []))]
    assert chunks == ["你好", "，小朋友"]


@respx.mock
async def test_stream_chat_error_raises():
    respx.post("https://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(401, text='{"error":"bad key"}')
    )
    with pytest.raises(UpstreamError) as ei:
        async for _ in stream_chat("https://api.test/v1", "sk-x", build_chat_body("m", [])):
            pass
    assert ei.value.status_code == 401
    assert "bad key" in ei.value.detail
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_upstream.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/upstream.py`：

```python
import json
from typing import AsyncIterator

import httpx


class UpstreamError(Exception):
    def __init__(self, status_code: int, detail: str):
        self.status_code = status_code
        self.detail = detail
        super().__init__(f"upstream {status_code}: {detail}")


def build_chat_body(
    model: str,
    messages: list[dict],
    *,
    temperature: float | None = None,
    max_tokens: int = 2000,
    reasoning_effort: str = "",
) -> dict:
    body: dict = {"model": model, "stream": True, "max_tokens": max_tokens, "messages": messages}
    if temperature is not None:
        body["temperature"] = temperature
    if reasoning_effort:
        body["reasoning_effort"] = reasoning_effort
    return body


async def stream_chat(base_url: str, api_key: str, body: dict) -> AsyncIterator[str]:
    """流式调用 OpenAI 兼容 /chat/completions，逐段 yield delta 文本。"""
    timeout = httpx.Timeout(10, read=90)
    async with httpx.AsyncClient(timeout=timeout) as client:
        async with client.stream(
            "POST",
            f"{base_url.rstrip('/')}/chat/completions",
            headers={"Authorization": f"Bearer {api_key}"},
            json=body,
        ) as resp:
            if resp.status_code != 200:
                raw = (await resp.aread()).decode("utf-8", "replace")
                raise UpstreamError(resp.status_code, raw)
            async for line in resp.aiter_lines():
                if not line.startswith("data: "):
                    continue
                data = line[6:].strip()
                if data == "[DONE]":
                    return
                try:
                    delta = json.loads(data)["choices"][0].get("delta", {}).get("content")
                except (json.JSONDecodeError, KeyError, IndexError):
                    continue
                if delta:
                    yield delta
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_upstream.py`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/engine/upstream.py server/tests/test_upstream.py
git commit -m "feat(server): streaming upstream chat client"
```

---

### Task 4: Provider 管理接口（CRUD + 连通性测试）

**Files:**
- Create: `server/app/routers/__init__.py`（空文件）
- Create: `server/app/routers/admin_providers.py`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_admin_providers.py`

**Interfaces:**
- Consumes: Task 1 的 `get_db`、`Provider`
- Produces: REST：`GET/POST /api/admin/providers`，`PUT/DELETE /api/admin/providers/{id}`，`POST /api/admin/providers/{id}/test` 返回 `{"ok": bool, "latency_ms": int, "models": [str], "error": str}`。Provider JSON 形状 `{id, name, base_url, api_key, enabled, notes}`（后续任务的 profile 下拉、语音配置都用它）

- [ ] **Step 1: 写失败测试**

`server/tests/test_admin_providers.py`：

```python
import httpx
import respx


def make_provider(client, **over):
    body = {"name": "测试商", "base_url": "https://api.test/v1", "api_key": "sk-1", **over}
    r = client.post("/api/admin/providers", json=body)
    assert r.status_code == 200
    return r.json()


def test_crud_roundtrip(client):
    p = make_provider(client)
    assert p["id"] > 0 and p["enabled"] is True

    r = client.get("/api/admin/providers")
    assert [x["name"] for x in r.json()] == ["测试商"]

    r = client.put(f"/api/admin/providers/{p['id']}", json={"name": "改名", "enabled": False})
    assert r.json()["name"] == "改名" and r.json()["enabled"] is False

    assert client.delete(f"/api/admin/providers/{p['id']}").status_code == 200
    assert client.get("/api/admin/providers").json() == []


def test_update_missing_404(client):
    assert client.put("/api/admin/providers/999", json={"name": "x"}).status_code == 404


@respx.mock
def test_connectivity_ok(client):
    p = make_provider(client)
    respx.get("https://api.test/v1/models").mock(
        return_value=httpx.Response(200, json={"data": [{"id": "gpt-4o-mini"}, {"id": "o4"}]})
    )
    r = client.post(f"/api/admin/providers/{p['id']}/test").json()
    assert r["ok"] is True and r["models"] == ["gpt-4o-mini", "o4"] and r["latency_ms"] >= 0


@respx.mock
def test_connectivity_failure(client):
    p = make_provider(client)
    respx.get("https://api.test/v1/models").mock(return_value=httpx.Response(401, text="denied"))
    r = client.post(f"/api/admin/providers/{p['id']}/test").json()
    assert r["ok"] is False and "401" in r["error"]
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_admin_providers.py`
Expected: FAIL（404，路由不存在）

- [ ] **Step 3: 实现**

`server/app/routers/admin_providers.py`：

```python
import time

import httpx
from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Provider

router = APIRouter(prefix="/api/admin/providers")


class ProviderIn(BaseModel):
    name: str | None = None
    base_url: str | None = None
    api_key: str | None = None
    enabled: bool | None = None
    notes: str | None = None


def to_json(p: Provider) -> dict:
    return {
        "id": p.id, "name": p.name, "base_url": p.base_url,
        "api_key": p.api_key, "enabled": p.enabled, "notes": p.notes,
    }


@router.get("")
def list_providers(db: Session = Depends(get_db)):
    return [to_json(p) for p in db.query(Provider).order_by(Provider.id).all()]


@router.post("")
def create_provider(body: ProviderIn, db: Session = Depends(get_db)):
    p = Provider(
        name=body.name or "", base_url=(body.base_url or "").rstrip("/"),
        api_key=body.api_key or "", enabled=body.enabled if body.enabled is not None else True,
        notes=body.notes or "",
    )
    db.add(p)
    db.commit()
    return to_json(p)


def _get_or_404(db: Session, pid: int) -> Provider:
    p = db.get(Provider, pid)
    if p is None:
        raise HTTPException(404, "provider 不存在")
    return p


@router.put("/{pid}")
def update_provider(pid: int, body: ProviderIn, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    for field in ("name", "api_key", "notes", "enabled"):
        v = getattr(body, field)
        if v is not None:
            setattr(p, field, v)
    if body.base_url is not None:
        p.base_url = body.base_url.rstrip("/")
    db.commit()
    return to_json(p)


@router.delete("/{pid}")
def delete_provider(pid: int, db: Session = Depends(get_db)):
    db.delete(_get_or_404(db, pid))
    db.commit()
    return {"ok": True}


@router.post("/{pid}/test")
def test_provider(pid: int, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    t0 = time.monotonic()
    try:
        resp = httpx.get(
            f"{p.base_url}/models",
            headers={"Authorization": f"Bearer {p.api_key}"},
            timeout=8,
        )
        latency = int((time.monotonic() - t0) * 1000)
        if resp.status_code != 200:
            return {"ok": False, "latency_ms": latency, "models": [],
                    "error": f"HTTP {resp.status_code}: {resp.text[:200]}"}
        models = [m.get("id", "") for m in resp.json().get("data", [])]
        return {"ok": True, "latency_ms": latency, "models": models, "error": ""}
    except httpx.HTTPError as e:
        latency = int((time.monotonic() - t0) * 1000)
        return {"ok": False, "latency_ms": latency, "models": [], "error": str(e)}
```

`server/app/main.py` 的 `create_app` 中、`health` 定义之后追加：

```python
    from app.routers import admin_providers

    app.include_router(admin_providers.router)
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_admin_providers.py`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/routers server/app/main.py server/tests/test_admin_providers.py
git commit -m "feat(server): provider admin CRUD and connectivity test"
```

---

### Task 5: Profile 管理接口（CRUD + 生效互斥）

**Files:**
- Create: `server/app/routers/admin_profiles.py`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_admin_profiles.py`

**Interfaces:**
- Consumes: Task 1 的 `get_db`、`Profile`
- Produces: `GET/POST /api/admin/profiles`，`PUT/DELETE /api/admin/profiles/{id}`，`POST /api/admin/profiles/{id}/activate`。Profile JSON `{id, name, age_band, persona_text, voice_hint, provider_id, model, temperature, max_tokens, reasoning_effort, is_active}`。Turn 引擎（Task 7）依赖「至多一个 `is_active=True`」这一不变量

- [ ] **Step 1: 写失败测试**

`server/tests/test_admin_profiles.py`：

```python
def make_profile(client, **over):
    body = {"name": "小班", "age_band": "3-4", "persona_text": "你是 DouDou。", **over}
    r = client.post("/api/admin/profiles", json=body)
    assert r.status_code == 200
    return r.json()


def test_crud_roundtrip(client):
    p = make_profile(client)
    assert p["is_active"] is False and p["max_tokens"] == 2000

    r = client.put(f"/api/admin/profiles/{p['id']}",
                   json={"voice_hint": "口语化", "temperature": 0.6, "model": "gpt-4o-mini"})
    j = r.json()
    assert j["voice_hint"] == "口语化" and j["temperature"] == 0.6

    assert client.delete(f"/api/admin/profiles/{p['id']}").status_code == 200
    assert client.get("/api/admin/profiles").json() == []


def test_activate_is_exclusive(client):
    a = make_profile(client, name="A")
    b = make_profile(client, name="B")
    client.post(f"/api/admin/profiles/{a['id']}/activate")
    client.post(f"/api/admin/profiles/{b['id']}/activate")
    by_name = {x["name"]: x["is_active"] for x in client.get("/api/admin/profiles").json()}
    assert by_name == {"A": False, "B": True}


def test_activate_missing_404(client):
    assert client.post("/api/admin/profiles/999/activate").status_code == 404
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_admin_profiles.py`
Expected: FAIL（404）

- [ ] **Step 3: 实现**

`server/app/routers/admin_profiles.py`：

```python
from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Profile

router = APIRouter(prefix="/api/admin/profiles")

FIELDS = ("name", "age_band", "persona_text", "voice_hint", "provider_id",
          "model", "temperature", "max_tokens", "reasoning_effort")


class ProfileIn(BaseModel):
    name: str | None = None
    age_band: str | None = None
    persona_text: str | None = None
    voice_hint: str | None = None
    provider_id: int | None = None
    model: str | None = None
    temperature: float | None = None
    max_tokens: int | None = None
    reasoning_effort: str | None = None


def to_json(p: Profile) -> dict:
    return {f: getattr(p, f) for f in FIELDS} | {"id": p.id, "is_active": p.is_active}


@router.get("")
def list_profiles(db: Session = Depends(get_db)):
    return [to_json(p) for p in db.query(Profile).order_by(Profile.id).all()]


@router.post("")
def create_profile(body: ProfileIn, db: Session = Depends(get_db)):
    p = Profile()
    for f in FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(p, f, v)
    db.add(p)
    db.commit()
    return to_json(p)


def _get_or_404(db: Session, pid: int) -> Profile:
    p = db.get(Profile, pid)
    if p is None:
        raise HTTPException(404, "profile 不存在")
    return p


@router.put("/{pid}")
def update_profile(pid: int, body: ProfileIn, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    for f in FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(p, f, v)
    db.commit()
    return to_json(p)


@router.delete("/{pid}")
def delete_profile(pid: int, db: Session = Depends(get_db)):
    db.delete(_get_or_404(db, pid))
    db.commit()
    return {"ok": True}


@router.post("/{pid}/activate")
def activate_profile(pid: int, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    db.query(Profile).update({Profile.is_active: False})
    p.is_active = True
    db.commit()
    return to_json(p)
```

`server/app/main.py` 中路由注册处改为：

```python
    from app.routers import admin_profiles, admin_providers

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_admin_profiles.py`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/admin_profiles.py server/app/main.py server/tests/test_admin_profiles.py
git commit -m "feat(server): profile admin CRUD with exclusive activation"
```

---

### Task 6: 语音基础（STT/TTS 客户端 + 语音设置接口）

**Files:**
- Create: `server/app/engine/stt.py`
- Create: `server/app/engine/tts.py`
- Create: `server/app/routers/admin_voice.py`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_voice.py`

**Interfaces:**
- Consumes: Task 1 的 `VoiceSettings`、`Provider`；Task 3 的 `UpstreamError`
- Produces: `async def transcribe(base_url, api_key, model, audio: bytes, filename: str) -> str`；`async def synthesize(base_url, api_key, model, voice, text, speed=1.0) -> bytes`；`GET/PUT /api/admin/voice-settings`（JSON `{stt_provider_id, stt_model, tts_provider_id, tts_model, tts_voice, tts_speed}`）；`POST /api/admin/voice/stt-test`（multipart `audio`）→ `{"text": str}`；`POST /api/admin/voice/tts-test`（JSON `{text}`）→ `audio/mpeg` bytes。Task 9/10 依赖 `load_voice_config(db) -> tuple[stt_cfg, tts_cfg]`（配置不全时抛 `ConfigError("请先在 DouDou 后台完成语音配置")`）
- Produces: `class ConfigError(Exception)`（放在 `server/app/engine/stt.py` 顶部之前，实际定义于新文件 `server/app/engine/errors.py`，属性 `message: str`）

- [ ] **Step 1: 写失败测试**

`server/tests/test_voice.py`：

```python
import httpx
import pytest
import respx

from app.engine.errors import ConfigError
from app.engine.stt import transcribe
from app.engine.tts import synthesize


@respx.mock
async def test_transcribe_posts_multipart():
    route = respx.post("https://v.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "你好豆豆"})
    )
    text = await transcribe("https://v.test/v1", "sk", "whisper-1", b"AUDIO", "a.webm")
    assert text == "你好豆豆"
    assert b"whisper-1" in route.calls[0].request.content


@respx.mock
async def test_synthesize_returns_bytes():
    respx.post("https://v.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3DATA")
    )
    audio = await synthesize("https://v.test/v1", "sk", "tts-1", "alloy", "你好", speed=1.2)
    assert audio == b"MP3DATA"


def test_voice_settings_get_put(client):
    r = client.get("/api/admin/voice-settings").json()
    assert r["stt_model"] == "" and r["tts_speed"] == 1.0

    p = client.post("/api/admin/providers",
                    json={"name": "v", "base_url": "https://v.test/v1", "api_key": "sk"}).json()
    r = client.put("/api/admin/voice-settings", json={
        "stt_provider_id": p["id"], "stt_model": "whisper-1",
        "tts_provider_id": p["id"], "tts_model": "tts-1", "tts_voice": "alloy", "tts_speed": 1.1,
    }).json()
    assert r["stt_model"] == "whisper-1" and r["tts_voice"] == "alloy"


@respx.mock
def test_stt_and_tts_test_endpoints(client):
    p = client.post("/api/admin/providers",
                    json={"name": "v", "base_url": "https://v.test/v1", "api_key": "sk"}).json()
    client.put("/api/admin/voice-settings", json={
        "stt_provider_id": p["id"], "stt_model": "whisper-1",
        "tts_provider_id": p["id"], "tts_model": "tts-1", "tts_voice": "alloy",
    })
    respx.post("https://v.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "测到了"})
    )
    respx.post("https://v.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    r = client.post("/api/admin/voice/stt-test", files={"audio": ("a.webm", b"xx", "audio/webm")})
    assert r.json() == {"text": "测到了"}
    r = client.post("/api/admin/voice/tts-test", json={"text": "你好"})
    assert r.content == b"MP3"


def test_stt_test_unconfigured_400(client):
    r = client.post("/api/admin/voice/stt-test", files={"audio": ("a.webm", b"xx", "audio/webm")})
    assert r.status_code == 400
    assert "语音配置" in r.json()["detail"]
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_voice.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/errors.py`：

```python
class ConfigError(Exception):
    """配置缺失导致无法执行（面向用户的中文提示）。"""

    def __init__(self, message: str):
        self.message = message
        super().__init__(message)
```

`server/app/engine/stt.py`：

```python
import httpx

from app.engine.upstream import UpstreamError


async def transcribe(base_url: str, api_key: str, model: str, audio: bytes, filename: str) -> str:
    async with httpx.AsyncClient(timeout=60) as client:
        resp = await client.post(
            f"{base_url.rstrip('/')}/audio/transcriptions",
            headers={"Authorization": f"Bearer {api_key}"},
            files={"file": (filename, audio, "application/octet-stream")},
            data={"model": model},
        )
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    return resp.json().get("text", "")
```

`server/app/engine/tts.py`：

```python
import httpx

from app.engine.upstream import UpstreamError


async def synthesize(
    base_url: str, api_key: str, model: str, voice: str, text: str, speed: float = 1.0
) -> bytes:
    async with httpx.AsyncClient(timeout=60) as client:
        resp = await client.post(
            f"{base_url.rstrip('/')}/audio/speech",
            headers={"Authorization": f"Bearer {api_key}"},
            json={"model": model, "voice": voice, "input": text, "speed": speed},
        )
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    return resp.content
```

`server/app/routers/admin_voice.py`：

```python
from fastapi import APIRouter, Depends, HTTPException, UploadFile
from fastapi.responses import Response
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.engine.errors import ConfigError
from app.engine.stt import transcribe
from app.engine.tts import synthesize
from app.models import Provider, VoiceSettings

router = APIRouter(prefix="/api/admin")

VS_FIELDS = ("stt_provider_id", "stt_model", "tts_provider_id", "tts_model", "tts_voice", "tts_speed")


class VoiceIn(BaseModel):
    stt_provider_id: int | None = None
    stt_model: str | None = None
    tts_provider_id: int | None = None
    tts_model: str | None = None
    tts_voice: str | None = None
    tts_speed: float | None = None


def to_json(v: VoiceSettings) -> dict:
    return {f: getattr(v, f) for f in VS_FIELDS}


def load_voice_config(db: Session) -> tuple[dict, dict]:
    """返回 (stt_cfg, tts_cfg)，各含 base_url/api_key/model(/voice/speed)。配置不全抛 ConfigError。"""
    vs = db.get(VoiceSettings, 1)
    stt_p = db.get(Provider, vs.stt_provider_id) if vs.stt_provider_id else None
    tts_p = db.get(Provider, vs.tts_provider_id) if vs.tts_provider_id else None
    if not (stt_p and vs.stt_model and tts_p and vs.tts_model):
        raise ConfigError("请先在 DouDou 后台完成语音配置")
    return (
        {"base_url": stt_p.base_url, "api_key": stt_p.api_key, "model": vs.stt_model},
        {"base_url": tts_p.base_url, "api_key": tts_p.api_key, "model": vs.tts_model,
         "voice": vs.tts_voice, "speed": vs.tts_speed},
    )


@router.get("/voice-settings")
def get_settings(db: Session = Depends(get_db)):
    return to_json(db.get(VoiceSettings, 1))


@router.put("/voice-settings")
def put_settings(body: VoiceIn, db: Session = Depends(get_db)):
    vs = db.get(VoiceSettings, 1)
    for f in VS_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(vs, f, v)
    db.commit()
    return to_json(vs)


@router.post("/voice/stt-test")
async def stt_test(audio: UploadFile, db: Session = Depends(get_db)):
    try:
        stt_cfg, _ = load_voice_config(db)
    except ConfigError as e:
        raise HTTPException(400, e.message)
    data = await audio.read()
    text = await transcribe(stt_cfg["base_url"], stt_cfg["api_key"], stt_cfg["model"],
                            data, audio.filename or "audio.webm")
    return {"text": text}


class TtsIn(BaseModel):
    text: str


@router.post("/voice/tts-test")
async def tts_test(body: TtsIn, db: Session = Depends(get_db)):
    try:
        _, tts_cfg = load_voice_config(db)
    except ConfigError as e:
        raise HTTPException(400, e.message)
    audio = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"], tts_cfg["model"],
                             tts_cfg["voice"], body.text, tts_cfg["speed"])
    return Response(content=audio, media_type="audio/mpeg")
```

`server/app/main.py` 路由注册处改为：

```python
    from app.routers import admin_profiles, admin_providers, admin_voice

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_voice.py`
Expected: 6 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/engine server/app/routers/admin_voice.py server/app/main.py server/tests/test_voice.py
git commit -m "feat(server): STT/TTS clients and voice settings admin"
```

---

### Task 7: Turn 引擎（TurnRunner）

**Files:**
- Create: `server/app/engine/turn.py`
- Test: `server/tests/test_turn_engine.py`

**Interfaces:**
- Consumes: Task 1 的模型与 sessionmaker；Task 2 的 `assemble_system_prompt`/`split_transcript`；Task 3 的 `build_chat_body`/`stream_chat`/`UpstreamError`；Task 6 的 `ConfigError`、`transcribe`、`load_voice_config`
- Produces:

```python
@dataclass
class TurnInput:
    source: str                      # "tablet" | "test" | "phone"
    text: str = ""
    image_png: bytes | None = None
    audio: bytes | None = None       # 有音频时引擎先做 STT，结果并入 input_text/transcript
    audio_filename: str = "audio.webm"
    history: list[dict] = field(default_factory=list)   # OpenAI messages 形状，原样透传
    device_protocol_suffix: str = ""  # 平板记忆协议后缀
    use_voice_hint: bool = False      # phone/测试台语音模式时 True

class TurnRunner:
    def __init__(self, sessionmaker, data_dir: str, tin: TurnInput): ...
    async def stream(self) -> AsyncIterator[str]   # yield 回复 delta；结束（含出错）后自动落库
    # stream 消费完后可读属性：
    #   .turn_id: int | None   .reply_text: str   .transcript: str   .system_prompt: str
```

配置缺失时 `stream()` 在产出任何 delta 前抛 `ConfigError`；上游错误抛 `UpstreamError`；两者都已先落库（status="error"）。

- [ ] **Step 1: 写失败测试**

`server/tests/test_turn_engine.py`：

```python
import httpx
import pytest
import respx

from app import models
from app.engine.errors import ConfigError
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

SSE = (
    'data: {"choices":[{"delta":{"content":"三颗星。"}}]}\n\n'
    'data: {"choices":[{"delta":{"content":"\\n⁂数星星"}}]}\n\n'
    "data: [DONE]\n\n"
)


def setup_active_profile(db):
    p = models.Provider(name="up", base_url="https://up.test/v1", api_key="sk")
    db.add(p)
    db.flush()
    prof = models.Profile(name="小班", age_band="3-4", persona_text="你是 DouDou。",
                          voice_hint="语音要更短。", provider_id=p.id,
                          model="gpt-4o-mini", max_tokens=1500, is_active=True)
    db.add(prof)
    db.commit()
    return prof


@respx.mock
async def test_stream_and_log(app, db):
    setup_active_profile(db)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    tin = TurnInput(source="tablet", text="（手写页）", image_png=b"\x89PNG-fake",
                    history=[{"role": "user", "content": "(an earlier page) 早"},
                             {"role": "assistant", "content": "早呀"}],
                    device_protocol_suffix="\n\n记忆协议：xxx")
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir, tin)
    chunks = [c async for c in runner.stream()]
    assert "".join(chunks) == "三颗星。\n⁂数星星"  # 流原样转发，不剥 ⁂

    import json
    body = json.loads(route.calls[0].request.content)
    assert body["model"] == "gpt-4o-mini" and body["max_tokens"] == 1500
    assert body["messages"][0]["role"] == "system"
    assert body["messages"][0]["content"] == "你是 DouDou。\n\n记忆协议：xxx"  # 无 voice_hint
    assert body["messages"][1]["content"] == "(an earlier page) 早"
    user = body["messages"][-1]["content"]
    assert user[0] == {"type": "text", "text": "（手写页）"}
    assert user[1]["image_url"]["url"].startswith("data:image/png;base64,")

    assert runner.reply_text == "三颗星。" and runner.transcript == "数星星"
    turn = db.query(models.Turn).one()
    assert turn.source == "tablet" and turn.status == "ok"
    assert turn.reply_text == "三颗星。" and turn.transcript == "数星星"
    assert turn.input_image_path.endswith(".png") and turn.latency_ms >= 0
    import os
    assert os.path.exists(os.path.join(app.state.data_dir, turn.input_image_path))


@respx.mock
async def test_voice_hint_applied_for_phone(app, db):
    setup_active_profile(db)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="phone", text="你好", use_voice_hint=True))
    [_ async for _ in runner.stream()]
    import json
    body = json.loads(route.calls[0].request.content)
    assert body["messages"][0]["content"] == "你是 DouDou。\n\n语音要更短。"


async def test_no_active_profile_raises_config_error(app, db):
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="test", text="hi"))
    with pytest.raises(ConfigError) as ei:
        async for _ in runner.stream():
            pass
    assert "后台配置" in ei.value.message
    turn = db.query(models.Turn).one()
    assert turn.status == "error"


@respx.mock
async def test_upstream_error_logged(app, db):
    setup_active_profile(db)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(429, text="rate limited")
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="test", text="hi"))
    with pytest.raises(UpstreamError):
        async for _ in runner.stream():
            pass
    turn = db.query(models.Turn).one()
    assert turn.status == "error" and "rate limited" in turn.error
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_turn_engine.py`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

`server/app/engine/turn.py`：

```python
import base64
import time
import uuid
from dataclasses import dataclass, field
from typing import AsyncIterator

from app.engine.errors import ConfigError
from app.engine.prompt import assemble_system_prompt
from app.engine.transcript import split_transcript
from app.engine.upstream import UpstreamError, build_chat_body, stream_chat
from app.models import Profile, Provider, Turn


@dataclass
class TurnInput:
    source: str
    text: str = ""
    image_png: bytes | None = None
    audio: bytes | None = None
    audio_filename: str = "audio.webm"
    history: list[dict] = field(default_factory=list)
    device_protocol_suffix: str = ""
    use_voice_hint: bool = False


class TurnRunner:
    def __init__(self, sessionmaker, data_dir: str, tin: TurnInput):
        self._sm = sessionmaker
        self._data_dir = data_dir
        self._tin = tin
        self.turn_id: int | None = None
        self.reply_text = ""
        self.transcript = ""
        self.system_prompt = ""
        self.input_text = tin.text

    def _save_file(self, sub: str, ext: str, data: bytes) -> str:
        rel = f"{sub}/{uuid.uuid4().hex}.{ext}"
        with open(f"{self._data_dir}/{rel}", "wb") as f:
            f.write(data)
        return rel

    async def stream(self) -> AsyncIterator[str]:
        tin = self._tin
        t0 = time.monotonic()
        full: list[str] = []
        turn = Turn(source=tin.source, input_text=tin.text)
        if tin.image_png:
            turn.input_image_path = self._save_file("images", "png", tin.image_png)
        if tin.audio:
            turn.input_audio_path = self._save_file("audio", "webm", tin.audio)
        try:
            with self._sm() as db:
                profile = db.query(Profile).filter(Profile.is_active.is_(True)).first()
                if profile is None:
                    raise ConfigError("请先在 DouDou 后台设置生效的人设")
                provider = db.get(Provider, profile.provider_id) if profile.provider_id else None
                if provider is None or not provider.enabled or not profile.model:
                    raise ConfigError("请先在 DouDou 后台配置模型")
                turn.profile_id, turn.profile_name, turn.model = profile.id, profile.name, profile.model

                if tin.audio is not None:
                    from app.engine.stt import transcribe
                    from app.routers.admin_voice import load_voice_config
                    stt_cfg, _ = load_voice_config(db)
                    heard = await transcribe(stt_cfg["base_url"], stt_cfg["api_key"],
                                             stt_cfg["model"], tin.audio, tin.audio_filename)
                    self.input_text = heard
                    turn.input_text = heard
                    turn.transcript = heard

                self.system_prompt = assemble_system_prompt(
                    profile.persona_text,
                    voice_hint=profile.voice_hint if tin.use_voice_hint else "",
                    protocol_suffix=tin.device_protocol_suffix,
                )
                turn.system_prompt = self.system_prompt

                user_content: object = self.input_text
                if tin.image_png is not None:
                    b64 = base64.b64encode(tin.image_png).decode()
                    user_content = [
                        {"type": "text", "text": self.input_text},
                        {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
                    ]
                messages = (
                    [{"role": "system", "content": self.system_prompt}]
                    + tin.history
                    + [{"role": "user", "content": user_content}]
                )
                body = build_chat_body(
                    profile.model, messages,
                    temperature=profile.temperature,
                    max_tokens=profile.max_tokens,
                    reasoning_effort=profile.reasoning_effort,
                )
                base_url, api_key = provider.base_url, provider.api_key

            async for delta in stream_chat(base_url, api_key, body):
                full.append(delta)
                yield delta

            visible, post = split_transcript("".join(full))
            self.reply_text = visible
            if post:  # 语音/测试轮无 ⁂ 时保留 STT 转写
                self.transcript = post
            turn.reply_text, turn.transcript = self.reply_text, self.transcript
        except ConfigError as e:
            turn.status, turn.error = "error", e.message
            raise
        except UpstreamError as e:
            turn.status, turn.error = "error", f"{e.status_code}: {e.detail[:500]}"
            raise
        finally:
            turn.latency_ms = int((time.monotonic() - t0) * 1000)
            with self._sm() as db:
                db.add(turn)
                db.commit()
                self.turn_id = turn.id
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_turn_engine.py`
Expected: 4 passed

- [ ] **Step 5: 全量回归**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 6: Commit**

```bash
git add server/app/engine/turn.py server/tests/test_turn_engine.py
git commit -m "feat(server): TurnRunner core engine with logging"
```

---

### Task 8: OpenAI 兼容门面（平板入口）

**Files:**
- Create: `server/app/routers/openai_compat.py`
- Create: `server/tests/fixtures/riddle_body.json`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_openai_compat.py`

**Interfaces:**
- Consumes: Task 2 的 `split_protocol_suffix`；Task 7 的 `TurnInput`/`TurnRunner`；Task 6 的 `ConfigError`；Task 3 的 `UpstreamError`
- Produces: `POST /v1/chat/completions` —— 接受 riddle 请求体（含 `max_tokens` 或 `max_completion_tokens` 任一，均忽略），SSE 流式返回 OpenAI chunk 格式（`data: {"choices":[{"delta":{"content":...},"index":0,"finish_reason":null}]}` … `data: [DONE]`）。配置错误 → `PlainTextResponse(400, 中文)`；上游首包前失败 → `PlainTextResponse(502, 中文+上游码)`

- [ ] **Step 1: 写 riddle 请求体 fixture**

`server/tests/fixtures/riddle_body.json`（`iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGNiAAAABgADNjd8qAAAAABJRU5ErkJggg==` 是合法 1x1 PNG）：

```json
{
  "model": "gpt-4o-mini",
  "stream": true,
  "max_tokens": 2000,
  "messages": [
    {
      "role": "system",
      "content": "你是设备内置的 DouDou。\n\n记忆协议：系统会给你一个当前可用的记忆目录，⟦show:N⟧，每次回复最后追加 ⁂ 转写。"
    },
    { "role": "user", "content": "(an earlier page) 昨天我画了猫" },
    { "role": "assistant", "content": "小猫真可爱！" },
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "（这是孩子当前的手写页）" },
        { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGNiAAAABgADNjd8qAAAAABJRU5ErkJggg==" } }
      ]
    }
  ]
}
```

- [ ] **Step 2: 写失败测试**

`server/tests/test_openai_compat.py`：

```python
import json
import os

import httpx
import respx

from app import models

FIXTURE = os.path.join(os.path.dirname(__file__), "fixtures", "riddle_body.json")
SSE = (
    'data: {"choices":[{"delta":{"content":"喵！"}}]}\n\n'
    'data: {"choices":[{"delta":{"content":"\\n⁂猫"}}]}\n\n'
    "data: [DONE]\n\n"
)


def riddle_body():
    with open(FIXTURE, encoding="utf-8") as f:
        return json.load(f)


def setup(client):
    p = client.post("/api/admin/providers",
                    json={"name": "up", "base_url": "https://up.test/v1", "api_key": "sk"}).json()
    prof = client.post("/api/admin/profiles", json={
        "name": "生效", "persona_text": "你是服务器版 DouDou。",
        "provider_id": p["id"], "model": "server-model", "max_tokens": 999,
    }).json()
    client.post(f"/api/admin/profiles/{prof['id']}/activate")


@respx.mock
def test_facade_replaces_persona_keeps_protocol(client, db):
    setup(client)
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    resp = client.post("/v1/chat/completions", json=riddle_body())
    assert resp.status_code == 200
    assert resp.headers["content-type"].startswith("text/event-stream")
    assert '"content":"喵！"' in resp.text and "⁂" in resp.text  # ⁂ 原样转发
    assert "data: [DONE]" in resp.text

    sent = json.loads(route.calls[0].request.content)
    sys = sent["messages"][0]["content"]
    assert sys.startswith("你是服务器版 DouDou。")           # 服务器人设替换
    assert "\n\n记忆协议：" in sys and "⟦show:N⟧" in sys      # 协议后缀保留
    assert "设备内置" not in sys                              # 设备人设被丢弃
    assert sent["model"] == "server-model" and sent["max_tokens"] == 999  # profile 参数生效
    assert sent["messages"][1]["content"] == "(an earlier page) 昨天我画了猫"  # 历史透传
    assert sent["messages"][-1]["content"][1]["type"] == "image_url"       # 图片透传

    turn = db.query(models.Turn).one()
    assert turn.source == "tablet" and turn.reply_text == "喵！" and turn.transcript == "猫"


@respx.mock
def test_facade_accepts_max_completion_tokens(client):
    setup(client)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    body = riddle_body()
    del body["max_tokens"]
    body["max_completion_tokens"] = 1234
    assert client.post("/v1/chat/completions", json=body).status_code == 200


def test_facade_no_profile_plaintext_400(client):
    resp = client.post("/v1/chat/completions", json=riddle_body())
    assert resp.status_code == 400
    assert "后台" in resp.text
    assert "max_completion_tokens" not in resp.text  # 防 riddle 换字段名重试


@respx.mock
def test_facade_upstream_error_502(client):
    setup(client)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(429, text="slow down")
    )
    resp = client.post("/v1/chat/completions", json=riddle_body())
    assert resp.status_code == 502
    assert "429" in resp.text and "max_completion_tokens" not in resp.text
```

- [ ] **Step 3: 运行确认失败**

Run: `cd server && uv run pytest tests/test_openai_compat.py`
Expected: FAIL（404）

- [ ] **Step 4: 实现**

`server/app/routers/openai_compat.py`：

```python
import base64
import json

from fastapi import APIRouter, Request
from fastapi.responses import PlainTextResponse, StreamingResponse

from app.engine.errors import ConfigError
from app.engine.prompt import split_protocol_suffix
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

router = APIRouter()


def _chunk(delta: str) -> str:
    payload = {"choices": [{"delta": {"content": delta}, "index": 0, "finish_reason": None}]}
    return f"data: {json.dumps(payload, ensure_ascii=False)}\n\n"


def parse_riddle_body(body: dict) -> TurnInput:
    messages: list[dict] = body.get("messages", [])
    protocol_suffix = ""
    history: list[dict] = []
    text, image_png = "", None

    if messages and messages[0].get("role") == "system":
        _, protocol_suffix = split_protocol_suffix(str(messages[0].get("content", "")))
        middle = messages[1:-1]
    else:
        middle = messages[:-1]
    history = middle

    if messages:
        content = messages[-1].get("content", "")
        if isinstance(content, str):
            text = content
        else:  # [{type:text},{type:image_url}]
            for part in content:
                if part.get("type") == "text":
                    text = part.get("text", "")
                elif part.get("type") == "image_url":
                    url = part.get("image_url", {}).get("url", "")
                    if url.startswith("data:image/png;base64,"):
                        image_png = base64.b64decode(url.split(",", 1)[1])
    return TurnInput(source="tablet", text=text, image_png=image_png,
                     history=history, device_protocol_suffix=protocol_suffix)


@router.post("/v1/chat/completions")
async def chat_completions(request: Request):
    tin = parse_riddle_body(await request.json())
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    agen = runner.stream()
    try:
        first = await anext(agen)
    except StopAsyncIteration:
        first = None
    except ConfigError as e:
        return PlainTextResponse(e.message, status_code=400)
    except UpstreamError as e:
        return PlainTextResponse(f"模型服务出错（{e.status_code}），请在后台检查配置", status_code=502)

    async def sse():
        if first is not None:
            yield _chunk(first)
        try:
            async for delta in agen:
                yield _chunk(delta)
        except UpstreamError:
            pass  # 中途断流：结束响应，riddle 端有读超时兜底
        payload = {"choices": [{"delta": {}, "index": 0, "finish_reason": "stop"}]}
        yield f"data: {json.dumps(payload)}\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(sse(), media_type="text/event-stream")
```

`server/app/main.py` 路由注册处改为：

```python
    from app.routers import admin_profiles, admin_providers, admin_voice, openai_compat

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
    app.include_router(openai_compat.router)
```

- [ ] **Step 5: 运行确认通过**

Run: `cd server && uv run pytest tests/test_openai_compat.py`
Expected: 4 passed

- [ ] **Step 6: Commit**

```bash
git add server/app/routers/openai_compat.py server/app/main.py server/tests/fixtures server/tests/test_openai_compat.py
git commit -m "feat(server): OpenAI-compatible facade for the tablet"
```

---

### Task 9: 手机语音接口 + 数据文件访问

**Files:**
- Create: `server/app/routers/phone.py`
- Create: `server/app/routers/files.py`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_phone.py`

**Interfaces:**
- Consumes: Task 7 的 `TurnRunner`；Task 6 的 `load_voice_config`/`synthesize`/`ConfigError`
- Produces: `POST /api/phone/voice-turn`（multipart：`audio` 文件 + `history` 表单字段，JSON 串 `[["用户话","回复"],...]`）→ `{"turn_id", "transcript", "reply_text", "audio_url"}`；`GET /api/files/{sub}/{name}`（sub 限 `images|audio`，防路径穿越）供前端取图与音频

- [ ] **Step 1: 写失败测试**

`server/tests/test_phone.py`：

```python
import json

import httpx
import respx

from app import models

SSE = (
    'data: {"choices":[{"delta":{"content":"我们来数星星呀。"}}]}\n\n'
    "data: [DONE]\n\n"
)


def setup(client):
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


@respx.mock
def test_voice_turn_full_loop(client, db):
    setup(client)
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "天上有几颗星星"})
    )
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3REPLY")
    )
    r = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("say.webm", b"AUDIO", "audio/webm")},
        data={"history": json.dumps([["昨天的问题", "昨天的回答"]])},
    )
    assert r.status_code == 200
    j = r.json()
    assert j["transcript"] == "天上有几颗星星"
    assert j["reply_text"] == "我们来数星星呀。"

    sent = json.loads(chat.calls[0].request.content)
    assert "口语化" in sent["messages"][0]["content"]      # voice_hint 生效
    assert sent["messages"][1] == {"role": "user", "content": "昨天的问题"}
    assert sent["messages"][2] == {"role": "assistant", "content": "昨天的回答"}

    audio = client.get(j["audio_url"])
    assert audio.status_code == 200 and audio.content == b"MP3REPLY"

    turn = db.query(models.Turn).one()
    assert turn.source == "phone" and turn.input_audio_path and turn.reply_audio_path


def test_voice_turn_unconfigured_400(client):
    r = client.post("/api/phone/voice-turn",
                    files={"audio": ("a.webm", b"x", "audio/webm")}, data={"history": "[]"})
    assert r.status_code == 400 and "后台" in r.json()["detail"]


def test_files_route_rejects_traversal(client):
    assert client.get("/api/files/audio/..%2Fdoudou.db").status_code in (400, 404)
    assert client.get("/api/files/other/x.png").status_code == 400
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_phone.py`
Expected: FAIL（404）

- [ ] **Step 3: 实现**

`server/app/routers/files.py`：

```python
import os

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import FileResponse

router = APIRouter(prefix="/api/files")

MEDIA = {"images": "image/png", "audio": "audio/mpeg"}


@router.get("/{sub}/{name}")
def get_file(sub: str, name: str, request: Request):
    if sub not in MEDIA:
        raise HTTPException(400, "非法目录")
    if "/" in name or "\\" in name or ".." in name:
        raise HTTPException(400, "非法文件名")
    path = os.path.join(request.app.state.data_dir, sub, name)
    if not os.path.isfile(path):
        raise HTTPException(404, "文件不存在")
    return FileResponse(path, media_type=MEDIA[sub])
```

`server/app/routers/phone.py`：

```python
import json
import uuid

from fastapi import APIRouter, Form, HTTPException, Request, UploadFile
from sqlalchemy.orm import Session

from app.engine.errors import ConfigError
from app.engine.tts import synthesize
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError
from app.models import Turn
from app.routers.admin_voice import load_voice_config

router = APIRouter(prefix="/api/phone")


@router.post("/voice-turn")
async def voice_turn(request: Request, audio: UploadFile, history: str = Form("[]")):
    pairs = json.loads(history)  # [["user","assistant"], ...]
    msgs: list[dict] = []
    for u, a in pairs:
        msgs.append({"role": "user", "content": u})
        msgs.append({"role": "assistant", "content": a})

    data = await audio.read()
    tin = TurnInput(source="phone", audio=data,
                    audio_filename=audio.filename or "audio.webm",
                    history=msgs, use_voice_hint=True)
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError as e:
        raise HTTPException(502, f"模型服务出错（{e.status_code}）")

    with request.app.state.sessionmaker() as db:  # type: Session
        try:
            _, tts_cfg = load_voice_config(db)
            audio_bytes = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"],
                                           tts_cfg["model"], tts_cfg["voice"],
                                           runner.reply_text, tts_cfg["speed"])
            rel = f"audio/{uuid.uuid4().hex}.mp3"
            with open(f"{request.app.state.data_dir}/{rel}", "wb") as f:
                f.write(audio_bytes)
            turn = db.get(Turn, runner.turn_id)
            turn.reply_audio_path = rel
            db.commit()
            audio_url = f"/api/files/{rel}"
        except (ConfigError, UpstreamError):
            audio_url = ""  # TTS 失败不阻塞文字回复

    return {"turn_id": runner.turn_id, "transcript": runner.transcript,
            "reply_text": runner.reply_text, "audio_url": audio_url}
```

`server/app/main.py` 路由注册处改为：

```python
    from app.routers import (admin_profiles, admin_providers, admin_voice,
                             files, openai_compat, phone)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(files.router)
```

- [ ] **Step 4: 运行确认通过**

Run: `cd server && uv run pytest tests/test_phone.py`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/phone.py server/app/routers/files.py server/app/main.py server/tests/test_phone.py
git commit -m "feat(server): phone voice-turn endpoint and data file serving"
```

---

### Task 10: 测试台接口 + 对话记录接口

**Files:**
- Create: `server/app/routers/admin_test.py`
- Create: `server/app/routers/admin_turns.py`
- Modify: `server/app/main.py`（注册路由）
- Test: `server/tests/test_admin_test_and_turns.py`

**Interfaces:**
- Consumes: Task 7 的 `TurnRunner`
- Produces: `POST /api/admin/test-turn`（JSON `{text, image_base64?, history?: [["u","a"],...], voice_mode?: bool}`）→ SSE：每段 `data: {"delta": "..."}`，结尾 `data: {"done": true, "turn_id": N, "transcript": "...", "system_prompt": "..."}`，错误 `data: {"error": "..."}`；`GET /api/admin/turns?limit=&offset=` → `{"total": N, "items": [...]}`（按时间倒序，含全部 Turn 字段）；`GET /api/admin/turns/{id}` → 单条

- [ ] **Step 1: 写失败测试**

`server/tests/test_admin_test_and_turns.py`：

```python
import base64
import json

import httpx
import respx

SSE = (
    'data: {"choices":[{"delta":{"content":"好呀。"}}]}\n\n'
    "data: [DONE]\n\n"
)


def setup(client):
    p = client.post("/api/admin/providers",
                    json={"name": "up", "base_url": "https://up.test/v1", "api_key": "sk"}).json()
    prof = client.post("/api/admin/profiles", json={
        "name": "生效", "persona_text": "你是 DouDou。", "provider_id": p["id"], "model": "m",
    }).json()
    client.post(f"/api/admin/profiles/{prof['id']}/activate")


@respx.mock
def test_test_turn_sse_and_history(client):
    setup(client)
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    img = base64.b64encode(b"\x89PNG-fake").decode()
    r = client.post("/api/admin/test-turn", json={
        "text": "你好", "image_base64": img, "history": [["早", "早呀"]],
    })
    assert r.status_code == 200
    lines = [json.loads(l[6:]) for l in r.text.splitlines() if l.startswith("data: ")]
    assert {"delta": "好呀。"} in lines
    done = lines[-1]
    assert done["done"] is True and done["turn_id"] > 0
    assert done["system_prompt"].startswith("你是 DouDou。")

    sent = json.loads(chat.calls[0].request.content)
    assert sent["messages"][1] == {"role": "user", "content": "早"}
    assert sent["messages"][-1]["content"][1]["type"] == "image_url"


def test_test_turn_error_event(client):
    r = client.post("/api/admin/test-turn", json={"text": "hi"})
    lines = [json.loads(l[6:]) for l in r.text.splitlines() if l.startswith("data: ")]
    assert any("后台" in l.get("error", "") for l in lines)


@respx.mock
def test_turns_list_and_detail(client):
    setup(client)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    client.post("/api/admin/test-turn", json={"text": "你好"})
    listing = client.get("/api/admin/turns").json()
    assert listing["total"] == 1
    item = listing["items"][0]
    assert item["source"] == "test" and item["reply_text"] == "好呀。"
    detail = client.get(f"/api/admin/turns/{item['id']}").json()
    assert detail["system_prompt"].startswith("你是 DouDou。")
    assert client.get("/api/admin/turns/999").status_code == 404
```

- [ ] **Step 2: 运行确认失败**

Run: `cd server && uv run pytest tests/test_admin_test_and_turns.py`
Expected: FAIL（404）

- [ ] **Step 3: 实现**

`server/app/routers/admin_test.py`：

```python
import base64
import json

from fastapi import APIRouter, Request
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

from app.engine.errors import ConfigError
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

router = APIRouter(prefix="/api/admin")


class TestTurnIn(BaseModel):
    text: str = ""
    image_base64: str | None = None
    history: list[list[str]] = []
    voice_mode: bool = False


@router.post("/test-turn")
async def test_turn(body: TestTurnIn, request: Request):
    msgs: list[dict] = []
    for u, a in body.history:
        msgs.append({"role": "user", "content": u})
        msgs.append({"role": "assistant", "content": a})
    tin = TurnInput(
        source="test", text=body.text,
        image_png=base64.b64decode(body.image_base64) if body.image_base64 else None,
        history=msgs, use_voice_hint=body.voice_mode,
    )
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)

    async def sse():
        def ev(obj: dict) -> str:
            return f"data: {json.dumps(obj, ensure_ascii=False)}\n\n"
        try:
            async for delta in runner.stream():
                yield ev({"delta": delta})
            yield ev({"done": True, "turn_id": runner.turn_id,
                      "transcript": runner.transcript, "system_prompt": runner.system_prompt})
        except ConfigError as e:
            yield ev({"error": e.message})
        except UpstreamError as e:
            yield ev({"error": f"模型服务出错（{e.status_code}）：{e.detail[:200]}"})

    return StreamingResponse(sse(), media_type="text/event-stream")
```

`server/app/routers/admin_turns.py`：

```python
from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Turn

router = APIRouter(prefix="/api/admin/turns")

FIELDS = ("id", "source", "profile_id", "profile_name", "model", "system_prompt",
          "input_text", "input_image_path", "input_audio_path", "transcript",
          "reply_text", "reply_audio_path", "latency_ms", "status", "error")


def to_json(t: Turn) -> dict:
    return {f: getattr(t, f) for f in FIELDS} | {"ts": t.ts.isoformat()}


@router.get("")
def list_turns(limit: int = 50, offset: int = 0, db: Session = Depends(get_db)):
    q = db.query(Turn).order_by(Turn.id.desc())
    return {"total": q.count(), "items": [to_json(t) for t in q.offset(offset).limit(limit)]}


@router.get("/{tid}")
def get_turn(tid: int, db: Session = Depends(get_db)):
    t = db.get(Turn, tid)
    if t is None:
        raise HTTPException(404, "记录不存在")
    return to_json(t)
```

`server/app/main.py` 路由注册处改为：

```python
    from app.routers import (admin_profiles, admin_providers, admin_test,
                             admin_turns, admin_voice, files, openai_compat, phone)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
    app.include_router(admin_test.router)
    app.include_router(admin_turns.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(files.router)
```

- [ ] **Step 4: 运行确认通过 + 全量回归**

Run: `cd server && uv run pytest`
Expected: 全部通过

- [ ] **Step 5: Commit**

```bash
git add server/app/routers/admin_test.py server/app/routers/admin_turns.py server/app/main.py server/tests/test_admin_test_and_turns.py
git commit -m "feat(server): test bench SSE endpoint and turn history API"
```

---

### Task 11: 静态托管 + 启动脚本 + README

**Files:**
- Modify: `server/app/main.py`（SPA 托管）
- Create: `server/run.sh`
- Create: `server/README.md`

**Interfaces:**
- Consumes: 全部后端路由
- Produces: `server/web/dist` 存在时，非 `/api`、`/v1` 的 GET 均回 `index.html`（SPA 路由）或静态资源；`./run.sh` 启动 http 8787（+ 证书存在时 https 8788）

- [ ] **Step 1: SPA 托管实现**

`server/app/main.py` 的 `create_app` 中、`include_router` 之后追加：

```python
    import os

    from fastapi.responses import FileResponse

    dist = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "web", "dist")
    if os.path.isdir(dist):
        @app.get("/{path:path}")
        def spa(path: str):
            full = os.path.normpath(os.path.join(dist, path))
            if full.startswith(dist) and os.path.isfile(full):
                return FileResponse(full)
            return FileResponse(os.path.join(dist, "index.html"))
```

- [ ] **Step 2: 启动脚本**

`server/run.sh`（`chmod +x`）：

```bash
#!/bin/bash
# DouDou Server：http 8787（平板+管理）；server/certs/ 有 mkcert 证书时加开 https 8788（手机页）
cd "$(dirname "$0")"
set -e

PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
trap cleanup EXIT

uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8787 &
PIDS+=($!)

if [[ -f certs/cert.pem && -f certs/key.pem ]]; then
  uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8788 \
    --ssl-certfile certs/cert.pem --ssl-keyfile certs/key.pem &
  PIDS+=($!)
  echo "https（手机页）: https://$(ipconfig getifaddr en0 2>/dev/null || echo localhost):8788/phone"
else
  echo "未发现 certs/，仅启动 http。手机页需 https（麦克风权限），配置方法见 README。"
fi
echo "http（平板+管理）: http://$(ipconfig getifaddr en0 2>/dev/null || echo localhost):8787/admin"
wait
```

注意：`create_app` 需支持无参调用（已有默认 `data_dir=None`），`--factory` 直接可用。

- [ ] **Step 3: README**

`server/README.md`：

````markdown
# DouDou Server（一期）

本地 Mac 上的 DouDou 大脑：OpenAI 兼容门面（平板零改动接入）+ 中文管理后台 + 手机按住说话页。
设计文档：`docs/superpowers/specs/2026-07-22-doudou-server-phase1-design.md`。

## 运行

```sh
cd server
uv sync
(cd web && npm install && npm run build)   # 构建管理界面
./run.sh
```

- 管理后台：`http://<Mac IP>:8787/admin`
- 手机页：`https://<Mac IP>:8788/phone`（需先配好 https，见下）

## 平板接入（reMarkable / riddle）

改设备上 riddle 目录里的 `oracle.env` 两行，Rust 不用动：

```sh
RIDDLE_OPENAI_KEY="doudou"                      # 非空即可，服务器不校验
RIDDLE_OPENAI_BASE="http://<Mac IP>:8787/v1"
```

先在后台配好 provider 和生效 profile，再在平板上写一笔验证。

## 手机页的 https（麦克风权限要求）

手机浏览器只在 https 下允许录音。用 mkcert 给局域网地址签本地证书：

```sh
brew install mkcert && mkcert -install
mkdir -p certs
mkcert -cert-file certs/cert.pem -key-file certs/key.pem "$(ipconfig getifaddr en0)" localhost
```

手机需安装 mkcert 的根证书（`mkcert -CAROOT` 目录下的 `rootCA.pem` 发到手机安装并信任），
然后访问 `https://<Mac IP>:8788/phone`。

## 安全前提

仅限局域网个人部署：无登录鉴权，API key 明文存 `server/data/doudou.db`。
不要将 8787/8788 暴露到公网。

## 测试

```sh
cd server && uv run pytest
```
````

- [ ] **Step 4: 手动验证**

Run（在 server/ 下）：

```bash
uv run uvicorn --factory app.main:create_app --port 8787 &
sleep 2
curl -s http://127.0.0.1:8787/api/health
kill %1
```

Expected: `{"ok":true}`

- [ ] **Step 5: Commit**

```bash
chmod +x server/run.sh
git add server/app/main.py server/run.sh server/README.md
git commit -m "feat(server): SPA hosting, launch script, README"
```

---

### Task 12: 前端脚手架 + 布局 + API 客户端

**Files:**
- Create: `server/web/`（Vite react-ts 模板：`package.json`、`vite.config.ts`、`tsconfig.json`、`index.html` 等）
- Create: `server/web/src/main.tsx`
- Create: `server/web/src/App.tsx`
- Create: `server/web/src/api.ts`
- Create: 占位页 `server/web/src/pages/{Providers,Profiles,TestBench,VoiceSettings,Turns,Phone}.tsx`

**Interfaces:**
- Produces: `api.ts` 导出 `get/post/put/del(url, body?) -> Promise<any>`（非 2xx 抛 `Error(detail)`）和 `sse(url, body, onEvent: (obj: any) => void) -> Promise<void>`（POST + 读流解析 `data: ` 行，每行 JSON.parse 后回调）；路由 `/admin/*`（侧栏五页）与 `/phone`；后续任务只替换各页组件内容
- Consumes: Task 4-10 的 API

- [ ] **Step 1: 脚手架**

```bash
cd server && npm create vite@latest web -- --template react-ts
cd web && npm install && npm install antd react-router-dom
```

`server/web/vite.config.ts` 覆盖为：

```ts
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': 'http://localhost:8787',
      '/v1': 'http://localhost:8787',
    },
  },
})
```

删除模板自带的 `src/App.css`、`src/index.css`、`src/assets/`，并在 `index.html` 把 `<title>` 改为 `DouDou 后台`。

- [ ] **Step 2: API 客户端**

`server/web/src/api.ts`：

```ts
async function handle(resp: Response) {
  if (!resp.ok) {
    let detail = `HTTP ${resp.status}`
    try {
      const j = await resp.json()
      detail = j.detail ?? JSON.stringify(j)
    } catch { /* keep default */ }
    throw new Error(detail)
  }
  return resp.json()
}

export const get = (url: string) => fetch(url).then(handle)
export const post = (url: string, body?: unknown) =>
  fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  }).then(handle)
export const put = (url: string, body: unknown) =>
  fetch(url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }).then(handle)
export const del = (url: string) => fetch(url, { method: 'DELETE' }).then(handle)

export async function postForm(url: string, form: FormData) {
  return fetch(url, { method: 'POST', body: form }).then(handle)
}

/** POST 后逐条解析 SSE `data: ` 行，每条 JSON.parse 后回调 */
export async function sse(url: string, body: unknown, onEvent: (obj: any) => void) {
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!resp.ok || !resp.body) throw new Error(`HTTP ${resp.status}`)
  const reader = resp.body.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    buf += decoder.decode(value, { stream: true })
    const lines = buf.split('\n')
    buf = lines.pop() ?? ''
    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const data = line.slice(6).trim()
      if (!data || data === '[DONE]') continue
      try { onEvent(JSON.parse(data)) } catch { /* 忽略非 JSON 行 */ }
    }
  }
}
```

- [ ] **Step 3: 路由与布局**

`server/web/src/main.tsx`：

```tsx
import { ConfigProvider } from 'antd'
import zhCN from 'antd/locale/zh_CN'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import App from './App'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ConfigProvider locale={zhCN}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </ConfigProvider>
  </React.StrictMode>,
)
```

`server/web/src/App.tsx`：

```tsx
import { Layout, Menu } from 'antd'
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom'
import Phone from './pages/Phone'
import Profiles from './pages/Profiles'
import Providers from './pages/Providers'
import TestBench from './pages/TestBench'
import Turns from './pages/Turns'
import VoiceSettings from './pages/VoiceSettings'

const MENU = [
  { key: '/admin/providers', label: '模型配置' },
  { key: '/admin/profiles', label: '人设 Profile' },
  { key: '/admin/test', label: '测试台' },
  { key: '/admin/voice', label: '语音配置' },
  { key: '/admin/turns', label: '对话记录' },
]

function AdminShell() {
  const nav = useNavigate()
  const loc = useLocation()
  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Layout.Sider theme="light">
        <div style={{ padding: 16, fontWeight: 700, fontSize: 18 }}>DouDou 后台</div>
        <Menu
          items={MENU}
          selectedKeys={[loc.pathname]}
          onClick={(e) => nav(e.key)}
        />
      </Layout.Sider>
      <Layout.Content style={{ padding: 24 }}>
        <Routes>
          <Route path="providers" element={<Providers />} />
          <Route path="profiles" element={<Profiles />} />
          <Route path="test" element={<TestBench />} />
          <Route path="voice" element={<VoiceSettings />} />
          <Route path="turns" element={<Turns />} />
          <Route path="*" element={<Navigate to="providers" replace />} />
        </Routes>
      </Layout.Content>
    </Layout>
  )
}

export default function App() {
  return (
    <Routes>
      <Route path="/phone" element={<Phone />} />
      <Route path="/admin/*" element={<AdminShell />} />
      <Route path="*" element={<Navigate to="/admin/providers" replace />} />
    </Routes>
  )
}
```

六个占位页（每个文件同样式，改组件名与文字），如 `server/web/src/pages/Providers.tsx`：

```tsx
export default function Providers() {
  return <div>模型配置（待实现）</div>
}
```

（`Profiles.tsx`、`TestBench.tsx`、`VoiceSettings.tsx`、`Turns.tsx`、`Phone.tsx` 同理。）

- [ ] **Step 4: 构建验证**

Run: `cd server/web && npm run build`
Expected: `tsc` 无错误，`dist/` 生成

再验证托管：`cd server && uv run uvicorn --factory app.main:create_app --port 8787 &`，
`curl -s http://127.0.0.1:8787/admin/providers | grep -o "DouDou 后台" | head -1`，Expected 输出 `DouDou 后台`（实为 index.html 注入的 title 文本，能匹配即托管成功），然后 `kill %1`。

- [ ] **Step 5: Commit**

```bash
git add server/web
git commit -m "feat(web): admin scaffold with routing and API client"
```

---

### Task 13: 模型配置页 + 人设 Profile 页

**Files:**
- Modify: `server/web/src/pages/Providers.tsx`（完整实现）
- Modify: `server/web/src/pages/Profiles.tsx`（完整实现）

**Interfaces:**
- Consumes: `api.ts`；`/api/admin/providers*`、`/api/admin/profiles*`

- [ ] **Step 1: 模型配置页**

`server/web/src/pages/Providers.tsx` 覆盖为：

```tsx
import { Button, Form, Input, Modal, Space, Switch, Table, message } from 'antd'
import { useEffect, useState } from 'react'
import { del, get, post, put } from '../api'

type Provider = {
  id: number; name: string; base_url: string; api_key: string
  enabled: boolean; notes: string
}

export default function Providers() {
  const [rows, setRows] = useState<Provider[]>([])
  const [editing, setEditing] = useState<Partial<Provider> | null>(null)
  const [testing, setTesting] = useState<number | null>(null)
  const [form] = Form.useForm()

  const reload = () => get('/api/admin/providers').then(setRows)
  useEffect(() => { reload() }, [])

  const save = async () => {
    const v = await form.validateFields()
    if (editing?.id) await put(`/api/admin/providers/${editing.id}`, v)
    else await post('/api/admin/providers', v)
    setEditing(null)
    reload()
  }

  const test = async (id: number) => {
    setTesting(id)
    try {
      const r = await post(`/api/admin/providers/${id}/test`)
      if (r.ok) message.success(`连通正常，${r.latency_ms}ms，${r.models.length} 个模型`)
      else message.error(`连通失败：${r.error}`)
    } finally { setTesting(null) }
  }

  return (
    <>
      <Space style={{ marginBottom: 16 }}>
        <Button type="primary" onClick={() => { form.resetFields(); setEditing({}) }}>
          新增 Provider
        </Button>
      </Space>
      <Table rowKey="id" dataSource={rows} pagination={false} columns={[
        { title: '名称', dataIndex: 'name' },
        { title: 'Base URL', dataIndex: 'base_url' },
        { title: '启用', dataIndex: 'enabled', render: (v: boolean) => (v ? '是' : '否') },
        { title: '备注', dataIndex: 'notes' },
        {
          title: '操作',
          render: (_, r) => (
            <Space>
              <Button size="small" loading={testing === r.id} onClick={() => test(r.id)}>测试连通</Button>
              <Button size="small" onClick={() => { form.setFieldsValue(r); setEditing(r) }}>编辑</Button>
              <Button size="small" danger onClick={async () => { await del(`/api/admin/providers/${r.id}`); reload() }}>删除</Button>
            </Space>
          ),
        },
      ]} />
      <Modal open={!!editing} title={editing?.id ? '编辑 Provider' : '新增 Provider'}
             onOk={save} onCancel={() => setEditing(null)} destroyOnClose>
        <Form form={form} layout="vertical" initialValues={{ enabled: true }}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="base_url" label="Base URL（如 https://api.openai.com/v1）"
                     rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="api_key" label="API Key"><Input.Password /></Form.Item>
          <Form.Item name="enabled" label="启用" valuePropName="checked"><Switch /></Form.Item>
          <Form.Item name="notes" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </>
  )
}
```

- [ ] **Step 2: 人设 Profile 页**

`server/web/src/pages/Profiles.tsx` 覆盖为：

```tsx
import { AutoComplete, Button, Form, Input, InputNumber, Modal, Select, Space, Table, Tag, message } from 'antd'
import { useEffect, useState } from 'react'
import { del, get, post, put } from '../api'

type Profile = {
  id: number; name: string; age_band: string; persona_text: string; voice_hint: string
  provider_id: number | null; model: string; temperature: number | null
  max_tokens: number; reasoning_effort: string; is_active: boolean
}

export default function Profiles() {
  const [rows, setRows] = useState<Profile[]>([])
  const [providers, setProviders] = useState<{ id: number; name: string }[]>([])
  const [models, setModels] = useState<string[]>([])
  const [editing, setEditing] = useState<Partial<Profile> | null>(null)
  const [form] = Form.useForm()

  const reload = () => get('/api/admin/profiles').then(setRows)
  useEffect(() => {
    reload()
    get('/api/admin/providers').then(setProviders)
  }, [])

  const fetchModels = async (pid?: number) => {
    setModels([])
    if (!pid) return
    try {
      const r = await post(`/api/admin/providers/${pid}/test`)
      if (r.ok) setModels(r.models)
    } catch { /* 拉不到就手填 */ }
  }

  const save = async () => {
    const v = await form.validateFields()
    if (editing?.id) await put(`/api/admin/profiles/${editing.id}`, v)
    else await post('/api/admin/profiles', v)
    setEditing(null)
    reload()
  }

  return (
    <>
      <Space style={{ marginBottom: 16 }}>
        <Button type="primary" onClick={() => { form.resetFields(); setEditing({}) }}>新增 Profile</Button>
      </Space>
      <Table rowKey="id" dataSource={rows} pagination={false} columns={[
        {
          title: '名称', dataIndex: 'name',
          render: (v, r) => <>{v} {r.is_active && <Tag color="green">生效中</Tag>}</>,
        },
        { title: '年龄段', dataIndex: 'age_band' },
        { title: '模型', dataIndex: 'model' },
        {
          title: '操作',
          render: (_, r) => (
            <Space>
              {!r.is_active && (
                <Button size="small" type="primary" onClick={async () => {
                  await post(`/api/admin/profiles/${r.id}/activate`)
                  message.success(`「${r.name}」已生效，平板与手机立即使用`)
                  reload()
                }}>设为生效</Button>
              )}
              <Button size="small" onClick={() => {
                form.setFieldsValue(r); setEditing(r); fetchModels(r.provider_id ?? undefined)
              }}>编辑</Button>
              <Button size="small" danger onClick={async () => { await del(`/api/admin/profiles/${r.id}`); reload() }}>删除</Button>
            </Space>
          ),
        },
      ]} />
      <Modal open={!!editing} width={720} title={editing?.id ? '编辑 Profile' : '新增 Profile'}
             onOk={save} onCancel={() => setEditing(null)} destroyOnClose>
        <Form form={form} layout="vertical" initialValues={{ max_tokens: 2000, reasoning_effort: '' }}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="age_band" label="年龄段">
            <Select options={['3-4', '5-6', '6-7'].map(v => ({ value: v, label: `${v} 岁` }))} allowClear />
          </Form.Item>
          <Form.Item name="persona_text" label="人设提示词" rules={[{ required: true }]}>
            <Input.TextArea rows={10} placeholder="你是 DouDou……" />
          </Form.Item>
          <Form.Item name="voice_hint" label="语音补充提示词（语音对话时追加）">
            <Input.TextArea rows={3} placeholder="这是语音对话，回复要口语化、更短……" />
          </Form.Item>
          <Form.Item name="provider_id" label="模型服务" rules={[{ required: true }]}>
            <Select options={providers.map(p => ({ value: p.id, label: p.name }))}
                    onChange={(v) => fetchModels(v)} />
          </Form.Item>
          <Form.Item name="model" label="模型（可从候选选择，也可直接手填）" rules={[{ required: true }]}>
            <AutoComplete options={models.map(m => ({ value: m }))}
                          filterOption={(i, o) => ((o?.value as string) ?? '').includes(i)}
                          placeholder="gpt-4o-mini" />
          </Form.Item>
          <Form.Item name="temperature" label="temperature（留空用默认）"><InputNumber min={0} max={2} step={0.1} /></Form.Item>
          <Form.Item name="max_tokens" label="max_tokens"><InputNumber min={100} max={20000} /></Form.Item>
          <Form.Item name="reasoning_effort" label="思考力度（仅思考模型）">
            <Select options={[{ value: '', label: '不设置' }, { value: 'low', label: 'low' },
                              { value: 'medium', label: 'medium' }, { value: 'high', label: 'high' }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  )
}
```

验收标准：模型一项既能从候选下拉选择，也能手输任意模型名。

- [ ] **Step 3: 构建验证**

Run: `cd server/web && npm run build`
Expected: 无 TS 错误

- [ ] **Step 4: Commit**

```bash
git add server/web/src/pages/Providers.tsx server/web/src/pages/Profiles.tsx
git commit -m "feat(web): providers and profiles pages"
```

---

### Task 14: 语音配置页 + 测试台页

**Files:**
- Modify: `server/web/src/pages/VoiceSettings.tsx`（完整实现）
- Modify: `server/web/src/pages/TestBench.tsx`（完整实现）

**Interfaces:**
- Consumes: `api.ts`（含 `sse`、`postForm`）；`/api/admin/voice-settings`、`/api/admin/voice/*-test`、`/api/admin/test-turn`

- [ ] **Step 1: 语音配置页**

`server/web/src/pages/VoiceSettings.tsx` 覆盖为：

```tsx
import { Button, Card, Form, Input, InputNumber, Select, Space, message } from 'antd'
import { useEffect, useRef, useState } from 'react'
import { get, post, postForm, put } from '../api'

export default function VoiceSettings() {
  const [providers, setProviders] = useState<{ id: number; name: string }[]>([])
  const [recording, setRecording] = useState(false)
  const [sttResult, setSttResult] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const [form] = Form.useForm()

  useEffect(() => {
    get('/api/admin/providers').then(setProviders)
    get('/api/admin/voice-settings').then(v => form.setFieldsValue(v))
  }, [form])

  const save = async () => {
    await put('/api/admin/voice-settings', await form.validateFields())
    message.success('已保存')
  }

  const recordTest = async () => {
    if (recording) { recRef.current?.stop(); return }
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    const rec = new MediaRecorder(stream)
    const chunks: Blob[] = []
    rec.ondataavailable = e => chunks.push(e.data)
    rec.onstop = async () => {
      stream.getTracks().forEach(t => t.stop())
      setRecording(false)
      const fd = new FormData()
      fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'test.webm')
      try {
        const r = await postForm('/api/admin/voice/stt-test', fd)
        setSttResult(r.text)
      } catch (e) { message.error(String(e)) }
    }
    recRef.current = rec
    rec.start()
    setRecording(true)
  }

  const ttsTest = async () => {
    const text = form.getFieldValue('tts_test_text') || '你好，我是豆豆。'
    const resp = await fetch('/api/admin/voice/tts-test', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text }),
    })
    if (!resp.ok) { message.error(await resp.text()); return }
    new Audio(URL.createObjectURL(await resp.blob())).play()
  }

  const providerOpts = providers.map(p => ({ value: p.id, label: p.name }))
  return (
    <Form form={form} layout="vertical" style={{ maxWidth: 560 }}>
      <Card title="语音识别（STT）" style={{ marginBottom: 16 }}>
        <Form.Item name="stt_provider_id" label="Provider"><Select options={providerOpts} allowClear /></Form.Item>
        <Form.Item name="stt_model" label="模型（如 whisper-1）"><Input /></Form.Item>
        <Space>
          <Button onClick={recordTest} danger={recording}>
            {recording ? '停止并转写' : '录一句测转写'}
          </Button>
          {sttResult && <span>转写结果：{sttResult}</span>}
        </Space>
      </Card>
      <Card title="语音合成（TTS）" style={{ marginBottom: 16 }}>
        <Form.Item name="tts_provider_id" label="Provider"><Select options={providerOpts} allowClear /></Form.Item>
        <Form.Item name="tts_model" label="模型（如 tts-1）"><Input /></Form.Item>
        <Form.Item name="tts_voice" label="音色（如 alloy）"><Input /></Form.Item>
        <Form.Item name="tts_speed" label="语速"><InputNumber min={0.5} max={2} step={0.1} /></Form.Item>
        <Form.Item name="tts_test_text" label="试听文本"><Input placeholder="你好，我是豆豆。" /></Form.Item>
        <Button onClick={ttsTest}>试听音色</Button>
      </Card>
      <Button type="primary" onClick={save}>保存</Button>
    </Form>
  )
}
```

- [ ] **Step 2: 测试台页**

`server/web/src/pages/TestBench.tsx` 覆盖为：

```tsx
import { Button, Card, Checkbox, Collapse, Input, Space, Upload, message } from 'antd'
import { useRef, useState } from 'react'
import { postForm, sse } from '../api'

type Msg = { role: 'user' | 'assistant'; text: string }

export default function TestBench() {
  const [text, setText] = useState('')
  const [imageB64, setImageB64] = useState<string | null>(null)
  const [voiceMode, setVoiceMode] = useState(false)
  const [autoRead, setAutoRead] = useState(false)
  const [busy, setBusy] = useState(false)
  const [recording, setRecording] = useState(false)
  const recRef = useRef<MediaRecorder | null>(null)
  const [msgs, setMsgs] = useState<Msg[]>([])
  const [sysPrompt, setSysPrompt] = useState('')

  // 麦克风输入：录音 → STT 转写 → 填入输入框（复用语音配置的 stt-test 接口）
  const recordToText = async () => {
    if (recording) { recRef.current?.stop(); return }
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    const rec = new MediaRecorder(stream)
    const chunks: Blob[] = []
    rec.ondataavailable = e => chunks.push(e.data)
    rec.onstop = async () => {
      stream.getTracks().forEach(t => t.stop())
      setRecording(false)
      const fd = new FormData()
      fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'bench.webm')
      try {
        const r = await postForm('/api/admin/voice/stt-test', fd)
        setText(t => (t ? t + ' ' : '') + r.text)
      } catch (e) { message.error(String(e)) }
    }
    recRef.current = rec
    rec.start()
    setRecording(true)
  }

  const send = async () => {
    if (!text && !imageB64) return
    setBusy(true)
    const history: [string, string][] = []
    for (let i = 0; i + 1 < msgs.length; i += 2) history.push([msgs[i].text, msgs[i + 1].text])
    const userText = text || '（仅图片）'
    setMsgs(m => [...m, { role: 'user', text: userText }, { role: 'assistant', text: '' }])
    setText('')
    let reply = ''
    try {
      await sse('/api/admin/test-turn',
        { text, image_base64: imageB64, history, voice_mode: voiceMode },
        (ev) => {
          if (ev.delta) {
            reply += ev.delta
            setMsgs(m => [...m.slice(0, -1), { role: 'assistant', text: reply }])
          } else if (ev.error) {
            message.error(ev.error)
            setMsgs(m => [...m.slice(0, -1), { role: 'assistant', text: `⚠️ ${ev.error}` }])
          } else if (ev.done) {
            setSysPrompt(ev.system_prompt)
            if (autoRead && reply) readAloud(reply)
          }
        })
    } finally {
      setBusy(false)
      setImageB64(null)
    }
  }

  const readAloud = async (t: string) => {
    const resp = await fetch('/api/admin/voice/tts-test', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: t }),
    })
    if (resp.ok) new Audio(URL.createObjectURL(await resp.blob())).play()
  }

  return (
    <div style={{ maxWidth: 720 }}>
      <Card style={{ marginBottom: 16, minHeight: 320 }}>
        {msgs.map((m, i) => (
          <p key={i} style={{ textAlign: m.role === 'user' ? 'right' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '8px 12px', borderRadius: 8,
              background: m.role === 'user' ? '#e6f4ff' : '#f5f5f5', whiteSpace: 'pre-wrap',
            }}>{m.text || '…'}</span>
          </p>
        ))}
      </Card>
      <Space.Compact style={{ width: '100%' }}>
        <Input.TextArea rows={2} value={text} onChange={e => setText(e.target.value)}
                        placeholder="输入文字，或只传手写图片" onPressEnter={e => { e.preventDefault(); send() }} />
        <Button type="primary" onClick={send} loading={busy}>发送</Button>
      </Space.Compact>
      <Space style={{ marginTop: 8 }}>
        <Upload beforeUpload={(f) => {
          const reader = new FileReader()
          reader.onload = () => setImageB64((reader.result as string).split(',')[1])
          reader.readAsDataURL(f)
          return false
        }} maxCount={1} accept="image/*">
          <Button>{imageB64 ? '已选图片 ✓' : '上传手写图片'}</Button>
        </Upload>
        <Button onClick={recordToText} danger={recording}>{recording ? '停止并转写' : '🎤 语音输入'}</Button>
        <Checkbox checked={voiceMode} onChange={e => setVoiceMode(e.target.checked)}>语音语域（追加 voice_hint）</Checkbox>
        <Checkbox checked={autoRead} onChange={e => setAutoRead(e.target.checked)}>自动朗读回复</Checkbox>
        <Button onClick={() => setMsgs([])}>清空会话</Button>
      </Space>
      {sysPrompt && (
        <Collapse style={{ marginTop: 16 }} items={[{
          key: 'sys', label: '查看实际 system prompt',
          children: <pre style={{ whiteSpace: 'pre-wrap' }}>{sysPrompt}</pre>,
        }]} />
      )}
    </div>
  )
}
```

- [ ] **Step 3: 构建验证**

Run: `cd server/web && npm run build`
Expected: 无 TS 错误

- [ ] **Step 4: Commit**

```bash
git add server/web/src/pages/VoiceSettings.tsx server/web/src/pages/TestBench.tsx
git commit -m "feat(web): voice settings and test bench pages"
```

---

### Task 15: 对话记录页 + 手机页

**Files:**
- Modify: `server/web/src/pages/Turns.tsx`（完整实现）
- Modify: `server/web/src/pages/Phone.tsx`（完整实现）

**Interfaces:**
- Consumes: `/api/admin/turns*`、`/api/phone/voice-turn`、`/api/files/*`

- [ ] **Step 1: 对话记录页**

`server/web/src/pages/Turns.tsx` 覆盖为：

```tsx
import { Button, Drawer, Table, Tag } from 'antd'
import { useEffect, useState } from 'react'
import { get } from '../api'

type Turn = {
  id: number; ts: string; source: string; profile_name: string; model: string
  system_prompt: string; input_text: string; input_image_path: string
  input_audio_path: string; transcript: string; reply_text: string
  reply_audio_path: string; latency_ms: number; status: string; error: string
}

const SOURCE = { tablet: '平板', test: '测试台', phone: '手机' } as Record<string, string>

export default function Turns() {
  const [rows, setRows] = useState<Turn[]>([])
  const [detail, setDetail] = useState<Turn | null>(null)

  useEffect(() => {
    get('/api/admin/turns?limit=100').then(r => setRows(r.items))
  }, [])

  return (
    <>
      <Table rowKey="id" dataSource={rows} columns={[
        { title: '时间', dataIndex: 'ts', render: (v: string) => new Date(v + 'Z').toLocaleString('zh-CN') },
        { title: '来源', dataIndex: 'source', render: (v: string) => SOURCE[v] ?? v },
        { title: 'Profile', dataIndex: 'profile_name' },
        {
          title: '输入', render: (_, r) => (
            <span>
              {r.input_image_path && (
                <img src={`/api/files/${r.input_image_path}`} alt="" style={{ height: 40, marginRight: 8 }} />
              )}
              {r.transcript || r.input_text}
            </span>
          ),
        },
        { title: '回复', dataIndex: 'reply_text', ellipsis: true },
        { title: '延迟', dataIndex: 'latency_ms', render: (v: number) => `${v}ms` },
        {
          title: '状态', dataIndex: 'status',
          render: (v: string) => (v === 'ok' ? <Tag color="green">成功</Tag> : <Tag color="red">失败</Tag>),
        },
        { title: '', render: (_, r) => <Button size="small" onClick={() => setDetail(r)}>详情</Button> },
      ]} />
      <Drawer open={!!detail} width={640} title={`第 ${detail?.id} 轮`} onClose={() => setDetail(null)}>
        {detail && (
          <div style={{ display: 'grid', gap: 12 }}>
            {detail.input_image_path && <img src={`/api/files/${detail.input_image_path}`} alt="" style={{ maxWidth: '100%' }} />}
            {detail.input_audio_path && <audio controls src={`/api/files/${detail.input_audio_path}`} />}
            {detail.transcript && <p><b>转写：</b>{detail.transcript}</p>}
            <p style={{ whiteSpace: 'pre-wrap' }}><b>回复：</b>{detail.reply_text}</p>
            {detail.reply_audio_path && <audio controls src={`/api/files/${detail.reply_audio_path}`} />}
            {detail.error && <p style={{ color: 'red' }}><b>错误：</b>{detail.error}</p>}
            <p><b>模型：</b>{detail.model}　<b>延迟：</b>{detail.latency_ms}ms</p>
            <details><summary>system prompt</summary>
              <pre style={{ whiteSpace: 'pre-wrap' }}>{detail.system_prompt}</pre>
            </details>
          </div>
        )}
      </Drawer>
    </>
  )
}
```

- [ ] **Step 2: 手机页**

`server/web/src/pages/Phone.tsx` 覆盖为：

```tsx
import { useRef, useState } from 'react'

type Bubble = { role: 'user' | 'assistant'; text: string }

export default function Phone() {
  const [bubbles, setBubbles] = useState<Bubble[]>([])
  const [state, setState] = useState<'idle' | 'recording' | 'thinking'>('idle')
  const [error, setError] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const historyRef = useRef<[string, string][]>([])

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
        try {
          const resp = await fetch('/api/phone/voice-turn', { method: 'POST', body: fd })
          if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
          const j = await resp.json()
          historyRef.current.push([j.transcript, j.reply_text])
          setBubbles(b => [...b, { role: 'user', text: j.transcript }, { role: 'assistant', text: j.reply_text }])
          if (j.audio_url) new Audio(j.audio_url).play()
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
      <div style={{ flex: 1, overflowY: 'auto', padding: '0 16px' }}>
        {bubbles.map((b, i) => (
          <p key={i} style={{ textAlign: b.role === 'user' ? 'right' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '10px 14px', borderRadius: 16, fontSize: 17,
              maxWidth: '80%', background: b.role === 'user' ? '#bae0ff' : '#fff',
              boxShadow: '0 1px 2px rgba(0,0,0,.1)', whiteSpace: 'pre-wrap', textAlign: 'left',
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

- [ ] **Step 3: 构建验证**

Run: `cd server/web && npm run build`
Expected: 无 TS 错误

- [ ] **Step 4: Commit**

```bash
git add server/web/src/pages/Turns.tsx server/web/src/pages/Phone.tsx
git commit -m "feat(web): turn history and phone push-to-talk pages"
```

---

### Task 16: 端到端验收（人工步骤，需真实 API key）

**Files:** 无新文件（发现问题回相应任务修）

- [ ] **Step 1: 全量测试 + 构建**

```bash
cd server && uv run pytest && (cd web && npm run build)
```

Expected: 测试全过、构建成功

- [ ] **Step 2: 起服务并配置**

`./run.sh` 后浏览器开 `http://localhost:8787/admin`：新增真实 provider（如 OpenAI/SiliconFlow）→ 测试连通 → 新建 profile（可复制 `device/riddle/persona.txt` 内容）→ 设为生效。

- [ ] **Step 3: 测试台走查**

文字问答一轮、传一张手写照片一轮；打开"查看实际 system prompt"确认人设正确；对话记录页出现这两轮且缩略图/延迟正常。

- [ ] **Step 4: 模拟 riddle 请求**

```bash
curl -sN http://localhost:8787/v1/chat/completions \
  -H 'Content-Type: application/json' \
  --data @server/tests/fixtures/riddle_body.json | head -20
```

Expected: SSE 流（`data: {"choices":…}`），对话记录出现一条「平板」来源。
（可选更真：本机 `cargo run -- --oracle-test 手写图.png`，`RIDDLE_OPENAI_BASE=http://127.0.0.1:8787/v1`。）

- [ ] **Step 5: 语音链路**

语音配置页配好 STT/TTS → 录一句测转写 → 试听音色；按 README 配 mkcert，手机开 `https://<Mac IP>:8788/phone` 按住说话完整对话一轮；对话记录里能回放录音与回复音频。

- [ ] **Step 6: 真机（可选，需平板在手边）**

平板 `oracle.env` 按 README 修改 → 写一笔 → 纸面出现服务器人设的回复 → 后台对话记录出现该轮（含手写页缩略图）。

- [ ] **Step 7: Commit（如有修复）**

```bash
git add -A && git commit -m "fix(server): e2e acceptance fixes"
```
