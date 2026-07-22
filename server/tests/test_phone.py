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


def test_voice_turn_malformed_history_400(client):
    r = client.post("/api/phone/voice-turn",
                    files={"audio": ("a.webm", b"x", "audio/webm")},
                    data={"history": "not json"})
    assert r.status_code == 400


@respx.mock
def test_tts_transport_failure_degrades_gracefully(client, db):
    setup(client)
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "你好"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        side_effect=httpx.ConnectError("tts down")
    )
    r = client.post("/api/phone/voice-turn",
                    files={"audio": ("say.webm", b"AUDIO", "audio/webm")},
                    data={"history": "[]"})
    assert r.status_code == 200
    j = r.json()
    assert j["reply_text"] == "我们来数星星呀。" and j["audio_url"] == ""


def test_files_route_rejects_traversal(client):
    assert client.get("/api/files/audio/..%2Fdoudou.db").status_code in (400, 404)
    assert client.get("/api/files/other/x.png").status_code == 400
