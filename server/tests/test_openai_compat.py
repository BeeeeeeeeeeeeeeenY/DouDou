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
