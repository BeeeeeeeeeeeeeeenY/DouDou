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


@respx.mock
def test_stt_test_upstream_error_502(client):
    p = client.post("/api/admin/providers",
                    json={"name": "v", "base_url": "https://v.test/v1", "api_key": "sk"}).json()
    client.put("/api/admin/voice-settings", json={
        "stt_provider_id": p["id"], "stt_model": "whisper-1",
        "tts_provider_id": p["id"], "tts_model": "tts-1", "tts_voice": "alloy",
    })
    respx.post("https://v.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(401, text='{"error":"bad key"}')
    )
    r = client.post("/api/admin/voice/stt-test", files={"audio": ("a.webm", b"xx", "audio/webm")})
    assert r.status_code == 502
    assert "语音服务出错" in r.json()["detail"]


@respx.mock
async def test_synthesize_minimax_native():
    route = respx.post("https://api.minimax.test/v1/t2a_v2").mock(
        return_value=httpx.Response(200, json={
            "data": {"audio": "4d503320"},  # hex of b"MP3 "
            "base_resp": {"status_code": 0, "status_msg": "success"},
        })
    )
    audio = await synthesize("https://api.minimax.test/v1", "sk", "speech-2.6-hd",
                             "lovely_girl", "你好", speed=1.0)
    assert audio == b"MP3 "
    import json as _json
    sent = _json.loads(route.calls[0].request.content)
    assert sent["voice_setting"]["voice_id"] == "lovely_girl"
    assert sent["model"] == "speech-2.6-hd" and sent["stream"] is False


@respx.mock
async def test_synthesize_minimax_business_error():
    respx.post("https://api.minimax.test/v1/t2a_v2").mock(
        return_value=httpx.Response(200, json={
            "base_resp": {"status_code": 1004, "status_msg": "invalid voice"},
        })
    )
    from app.engine.upstream import UpstreamError
    with pytest.raises(UpstreamError) as ei:
        await synthesize("https://api.minimax.test/v1", "sk", "m", "bad_voice", "你好")
    assert "1004" in ei.value.detail


@respx.mock
def test_provider_voices_minimax(client):
    p = client.post("/api/admin/providers",
                    json={"name": "minimax", "base_url": "https://api.minimax.test/v1",
                          "api_key": "sk"}).json()
    respx.post("https://api.minimax.test/v1/get_voice").mock(
        return_value=httpx.Response(200, json={
            "system_voice": [{"voice_id": "lovely_girl", "voice_name": "萌萌女童"}],
        })
    )
    r = client.get(f"/api/admin/providers/{p['id']}/voices").json()
    assert r["voices"] == [{"id": "lovely_girl", "name": "萌萌女童"}]


def test_provider_voices_non_minimax_empty(client):
    p = client.post("/api/admin/providers",
                    json={"name": "qwen", "base_url": "https://q.test/v1", "api_key": "sk"}).json()
    r = client.get(f"/api/admin/providers/{p['id']}/voices").json()
    assert r["voices"] == []


@respx.mock
async def test_transcribe_dashscope_chat_asr():
    route = respx.post("https://x.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/chat/completions").mock(
        return_value=httpx.Response(200, json={
            "choices": [{"message": {"content": "天上有几颗星星"}}],
        })
    )
    text = await transcribe("https://x.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                            "sk", "qwen3-asr-flash", b"AUDIO", "say.webm")
    assert text == "天上有几颗星星"
    import json as _json
    sent = _json.loads(route.calls[0].request.content)
    part = sent["messages"][0]["content"][0]
    assert part["type"] == "input_audio"
    assert part["input_audio"]["format"] == "webm"
    assert part["input_audio"]["data"].startswith("data:audio/webm;base64,")


@respx.mock
async def test_transcribe_dashscope_error_mapped():
    respx.post("https://x.dashscope.test/v1/chat/completions").mock(
        return_value=httpx.Response(400, text="bad audio")
    )
    from app.engine.upstream import UpstreamError
    with pytest.raises(UpstreamError) as ei:
        await transcribe("https://x.dashscope.test/v1", "sk", "m", b"A", "a.mp3")
    assert ei.value.status_code == 400


@respx.mock
def test_tts_test_uses_request_overrides(client):
    p = client.post("/api/admin/providers",
                    json={"name": "minimax", "base_url": "https://api.minimax.test/v1",
                          "api_key": "sk"}).json()
    route = respx.post("https://api.minimax.test/v1/t2a_v2").mock(
        return_value=httpx.Response(200, json={
            "data": {"audio": "4d503320"}, "base_resp": {"status_code": 0},
        })
    )
    # 不保存 voice_settings，直接在请求里带页面当前值
    r = client.post("/api/admin/voice/tts-test", json={
        "text": "你好", "tts_provider_id": p["id"],
        "tts_model": "speech-2.6-hd", "tts_voice": "clever_boy", "tts_speed": 1.2,
    })
    assert r.status_code == 200 and r.content == b"MP3 "
    import json as _json
    sent = _json.loads(route.calls[0].request.content)
    assert sent["voice_setting"]["voice_id"] == "clever_boy"
    assert sent["voice_setting"]["speed"] == 1.2


@respx.mock
def test_stt_test_uses_form_overrides(client):
    p = client.post("/api/admin/providers",
                    json={"name": "v", "base_url": "https://v.test/v1", "api_key": "sk"}).json()
    respx.post("https://v.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "带覆盖参数"})
    )
    r = client.post("/api/admin/voice/stt-test",
                    files={"audio": ("a.webm", b"xx", "audio/webm")},
                    data={"stt_provider_id": str(p["id"]), "stt_model": "whisper-1"})
    assert r.json() == {"text": "带覆盖参数"}
