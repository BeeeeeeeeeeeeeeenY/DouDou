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
