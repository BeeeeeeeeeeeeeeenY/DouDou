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
