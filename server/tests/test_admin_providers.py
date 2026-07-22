import httpx
import respx


def make_provider(client, **over):
    body = {"name": "测试商", "base_url": "https://api.test/v1", "api_key": "sk-1", **over}
    r = client.post("/api/admin/providers", json=body)
    assert r.status_code == 200
    return r.json()


def test_crud_roundtrip(client):
    p = make_provider(client)
    assert p["id"] > 0 and p["enabled"] is True

    r = client.get("/api/admin/providers")
    assert [x["name"] for x in r.json()] == ["测试商"]

    r = client.put(f"/api/admin/providers/{p['id']}", json={"name": "改名", "enabled": False})
    assert r.json()["name"] == "改名" and r.json()["enabled"] is False

    assert client.delete(f"/api/admin/providers/{p['id']}").status_code == 200
    assert client.get("/api/admin/providers").json() == []


def test_update_missing_404(client):
    assert client.put("/api/admin/providers/999", json={"name": "x"}).status_code == 404


@respx.mock
def test_connectivity_ok(client):
    p = make_provider(client)
    respx.get("https://api.test/v1/models").mock(
        return_value=httpx.Response(200, json={"data": [{"id": "gpt-4o-mini"}, {"id": "o4"}]})
    )
    r = client.post(f"/api/admin/providers/{p['id']}/test").json()
    assert r["ok"] is True and r["models"] == ["gpt-4o-mini", "o4"] and r["latency_ms"] >= 0


@respx.mock
def test_connectivity_failure(client):
    p = make_provider(client)
    respx.get("https://api.test/v1/models").mock(return_value=httpx.Response(401, text="denied"))
    r = client.post(f"/api/admin/providers/{p['id']}/test").json()
    assert r["ok"] is False and "401" in r["error"]
