def make_profile(client, **over):
    body = {"name": "小班", "age_band": "3-4", "persona_text": "你是 DouDou。", **over}
    r = client.post("/api/admin/profiles", json=body)
    assert r.status_code == 200
    return r.json()


def test_crud_roundtrip(client):
    p = make_profile(client)
    assert p["is_active"] is False and p["max_tokens"] == 2000

    r = client.put(f"/api/admin/profiles/{p['id']}",
                   json={"voice_hint": "口语化", "temperature": 0.6, "model": "gpt-4o-mini"})
    j = r.json()
    assert j["voice_hint"] == "口语化" and j["temperature"] == 0.6

    assert client.delete(f"/api/admin/profiles/{p['id']}").status_code == 200
    assert client.get("/api/admin/profiles").json() == []


def test_activate_is_exclusive(client):
    a = make_profile(client, name="A")
    b = make_profile(client, name="B")
    client.post(f"/api/admin/profiles/{a['id']}/activate")
    client.post(f"/api/admin/profiles/{b['id']}/activate")
    by_name = {x["name"]: x["is_active"] for x in client.get("/api/admin/profiles").json()}
    assert by_name == {"A": False, "B": True}


def test_activate_missing_404(client):
    assert client.post("/api/admin/profiles/999/activate").status_code == 404
