import base64
import json

import httpx
import respx

from app import models


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
    # 无生效人设：端点现在真的会调用模型，先走 400 分支。
    r = client.post("/turn", json=_min_body())
    assert r.status_code == 400


def test_turn_tolerates_missing_optional_fields(client):
    r = client.post("/turn", json={"turn_id": "t-2", "page_png": "QUJD"})
    assert r.status_code == 400


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
    assert j["v"] == 1 and j["turn_id"] == "t-1"  # 信封契约（设备据此解析）
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


@respx.mock
def test_turn_persists_cards_json(client, db):
    # text 卡不上平板（见 test_turn_drops_text_cards），持久化的应是过滤后的
    # 卡片，故这里用 stamp 卡验证持久化本身能正常工作。
    _setup_active_profile(db)
    reply = json.dumps({"spoken_text": "好", "paper_cards": [
        {"type": "stamp", "name": "sun", "count": 1, "size": "L"}]}, ensure_ascii=False)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(reply)))

    client.post("/turn", json=_min_body(page_png="QUJD"))

    from app import models as m
    row = db.query(m.Turn).filter(m.Turn.source == "tablet").order_by(m.Turn.id.desc()).first()
    assert row is not None
    assert row.cards_json is not None
    assert row.cards_json["paper_cards"][0]["name"] == "sun"


def _setup_course(client, db):
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


def test_turn_next_returns_and_clears_pending_demo(client, db):
    from app import models
    _setup_course(client, db)
    run = models.LessonRun(lesson_id=db.query(models.Lesson).first().id,
                           status="running", pending_demo="circle")
    db.add(run); db.commit()
    j = client.get("/turn/next").json()
    assert j["demo"] == {"shape": "circle", "place": "blank_area", "pace": "slow"}
    assert j["command"] is None
    # 取用即清：再请求得 null
    assert client.get("/turn/next").json()["demo"] is None
    # db 会话 expire_on_commit=False，run 对象不会自动感知端点另开会话所做的
    # 提交，需显式 refresh 才能看到最新行数据。
    db.refresh(run)
    assert run.pending_demo is None


def test_turn_next_returns_and_clears_command(client, db):
    from app import models
    _setup_course(client, db)
    run = models.LessonRun(lesson_id=db.query(models.Lesson).first().id,
                           status="running", pending_command="clear")
    db.add(run); db.commit()
    j = client.get("/turn/next").json()
    assert j["command"] == "clear"
    assert client.get("/turn/next").json()["command"] is None


def test_turn_next_no_running_run_is_empty(client, db):
    assert client.get("/turn/next").json() == {"demo": None, "command": None}


@respx.mock
def test_turn_drops_text_cards(client, db):
    # 平板只承载画面：DouDou 的话走手机语音，text 卡应被过滤掉。
    _setup_active_profile(db)
    reply = json.dumps({
        "spoken_text": "今天画得真棒",
        "paper_cards": [
            {"type": "text", "content": "真棒", "place": "blank_area", "size": "L"},
            {"type": "stamp", "name": "star", "count": 1, "place": "near_new_ink"},
        ],
        "page_action": "none", "memory_tags": [],
    }, ensure_ascii=False)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=_sse(reply)))

    r = client.post("/turn", json=_min_body())
    assert r.status_code == 200
    cards = r.json()["paper_cards"]
    assert not any(c["type"] == "text" for c in cards)
    assert any(c["type"] == "stamp" for c in cards)


def _image_reply(n: int = 1) -> str:
    return json.dumps({
        "spoken_text": f"你画了一个圆圈，第{n}次",
        "paper_cards": [{"type": "image", "subject": "circle", "place": "blank_area", "size": "l"}],
        "page_action": "none", "memory_tags": [],
    }, ensure_ascii=False)


@respx.mock
def test_turn_throttles_images(client, db):
    # 彩图节流：每 run 至多每 3 次提交放行 1 张；首张放行，之后需等满 3 轮。
    _setup_course(client, db)
    from app import models
    run = models.LessonRun(lesson_id=db.query(models.Lesson).first().id, status="running")
    db.add(run); db.commit()
    respx.post("https://up.test/v1/chat/completions").mock(
        side_effect=[httpx.Response(200, text=_sse(_image_reply(i))) for i in (1, 2, 3)])

    r1 = client.post("/turn", json=_min_body(turn_id="t-1"))
    r2 = client.post("/turn", json=_min_body(turn_id="t-2"))
    r3 = client.post("/turn", json=_min_body(turn_id="t-3"))

    def has_image(resp):
        return any(c["type"] == "image" for c in resp.json()["paper_cards"])

    assert has_image(r1), "first submission should be allowed through"
    assert not has_image(r2), "second submission within throttle window should be dropped"
    assert not has_image(r3), "third submission within throttle window should be dropped"
