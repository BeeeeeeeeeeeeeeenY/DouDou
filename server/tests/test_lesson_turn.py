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


@respx.mock
async def test_runner_strips_lesson_report_before_persist(app, db):
    _setup_profile(db)
    import json as _json
    delta = _json.dumps(
        {"choices": [{"delta": {"content": '好棒！\n⟦lesson_report⟧{"lesson_id":"x","status":"completed","highlights":"h","parent_tip":"p"}'}}]},
        ensure_ascii=False,
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=f"data: {delta}\n\ndata: [DONE]\n\n")
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="phone", text="上完啦"))
    [_ async for _ in runner.stream()]
    assert runner.reply_text == "好棒！"
    assert runner.lesson_report == {"lesson_id": "x", "status": "completed",
                                    "highlights": "h", "parent_tip": "p"}
    turn = db.query(models.Turn).one()
    assert "lesson_report" not in turn.reply_text  # 引擎落库即干净，无过渡窗口
