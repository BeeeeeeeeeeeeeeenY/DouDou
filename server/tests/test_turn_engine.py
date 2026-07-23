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

DEMO_SSE = (
    'data: {"choices":[{"delta":{"content":"先画个圆\\n⟦demo:circle⟧"}}]}\n\n'
    "data: [DONE]\n\n"
)

NO_MARK_SSE = (
    'data: {"choices":[{"delta":{"content":"我们来数星星呀。"}}]}\n\n'
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
    sys_prompt = body["messages"][0]["content"]
    assert sys_prompt.startswith("你是 DouDou。")
    assert "当前时间：" in sys_prompt          # 时间注入
    assert sys_prompt.endswith("\n\n记忆协议：xxx")  # 协议后缀保持在末尾
    assert "语音要更短。" not in sys_prompt      # 无 voice_hint
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
    sys_prompt = body["messages"][0]["content"]
    assert sys_prompt.startswith("你是 DouDou。\n\n语音要更短。")
    assert "当前时间：" in sys_prompt


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


@respx.mock
async def test_voice_turn_keeps_stt_transcript_without_mark(app, db):
    setup_active_profile(db)
    p = db.query(models.Provider).first()
    db.get(models.VoiceSettings, 1).stt_provider_id = p.id
    db.get(models.VoiceSettings, 1).stt_model = "whisper-1"
    vs = db.get(models.VoiceSettings, 1)
    vs.tts_provider_id, vs.tts_model = p.id, "tts-1"
    db.commit()
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "天上有几颗星星"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=NO_MARK_SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="phone", audio=b"AUDIO", audio_filename="say.m4a",
                                  use_voice_hint=True))
    [_ async for _ in runner.stream()]
    assert runner.transcript == "天上有几颗星星"
    turn = db.query(models.Turn).one()
    assert turn.transcript == "天上有几颗星星"
    assert turn.input_text == "天上有几颗星星"
    assert turn.input_audio_path
    assert turn.input_audio_path.endswith(".m4a")  # 存盘扩展名随上传文件名，而非硬编码 webm


@respx.mock
async def test_abandoned_stream_logged_as_error(app, db):
    setup_active_profile(db)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="test", text="hi"))
    agen = runner.stream()
    await anext(agen)      # consume one delta
    await agen.aclose()    # abandon mid-stream
    turn = db.query(models.Turn).one()
    assert turn.status == "error" and "aborted" in turn.error


@respx.mock
async def test_weather_line_injected_and_web_search_flag(app, db):
    from app.engine import ambient
    ambient._cache.update(ts=0.0, line="")  # 清缓存
    respx.get("https://api.open-meteo.com/v1/forecast").mock(
        return_value=httpx.Response(200, json={
            "current": {"temperature_2m": 29.3},
            "daily": {"temperature_2m_max": [31.2], "temperature_2m_min": [26.1],
                      "weather_code": [80]},
        })
    )
    prof = setup_active_profile(db)
    db.get(models.Profile, prof.id).web_search = True
    db.commit()
    route = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="test", text="今天天气怎么样"))
    [_ async for _ in runner.stream()]
    import json
    body = json.loads(route.calls[0].request.content)
    assert "今天深圳天气：阵雨，气温 26~31 度，现在 29 度。" in body["messages"][0]["content"]
    assert body["enable_search"] is True
    ambient._cache.update(ts=0.0, line="")


@respx.mock
async def test_turn_runner_exposes_demo_shape(app, db):
    setup_active_profile(db)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=DEMO_SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="tablet", text="hi"))
    [_ async for _ in runner.stream()]
    assert runner.demo_shape == "circle"
    assert "demo" not in runner.reply_text


@respx.mock
async def test_weather_failure_never_blocks_turn(app, db):
    from app.engine import ambient
    ambient._cache.update(ts=0.0, line="")
    respx.get("https://api.open-meteo.com/v1/forecast").mock(
        side_effect=httpx.ConnectError("no net")
    )
    setup_active_profile(db)
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    runner = TurnRunner(app.state.sessionmaker, app.state.data_dir,
                        TurnInput(source="test", text="hi"))
    chunks = [c async for c in runner.stream()]
    assert chunks  # 天气挂了对话照常
    ambient._cache.update(ts=0.0, line="")
