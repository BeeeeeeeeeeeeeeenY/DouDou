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


def test_test_turn_malformed_image_400(client):
    r = client.post("/api/admin/test-turn", json={"text": "hi", "image_base64": "data:image/png;base64,xxx"})
    assert r.status_code == 400


def test_test_turn_malformed_history_400(client):
    r = client.post("/api/admin/test-turn", json={"text": "hi", "history": [["only-one"]]})
    assert r.status_code == 400
