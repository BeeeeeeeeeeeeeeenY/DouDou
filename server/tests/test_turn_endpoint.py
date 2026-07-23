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
    r = client.post("/turn", json=_min_body())
    assert r.status_code == 200
    j = r.json()
    assert j["v"] == 1
    assert j["turn_id"] == "t-1"
    assert j["spoken_text"] == ""
    assert j["paper_cards"] == []
    assert j["page_action"] == "none"
    assert j["memory_tags"] == []


def test_turn_tolerates_missing_optional_fields(client):
    r = client.post("/turn", json={"turn_id": "t-2", "page_png": "QUJD"})
    assert r.status_code == 200
    assert r.json()["turn_id"] == "t-2"
