from app import models


def make_curriculum(client, **over):
    body = {"slug": "shapes-01", "title": "形状小画家", "age_band": "3-4", **over}
    r = client.post("/api/admin/curricula", json=body)
    assert r.status_code == 200
    return r.json()


def make_lesson(client, cid, seq=1, **over):
    body = {"seq": seq, "title": f"第{seq}课", "script_text": "脚本", **over}
    r = client.post(f"/api/admin/curricula/{cid}/lessons", json=body)
    assert r.status_code == 200
    return r.json()


def test_curriculum_crud(client):
    c = make_curriculum(client)
    assert c["status"] == "draft" and c["current_lesson_id"] is None
    r = client.put(f"/api/admin/curricula/{c['id']}", json={"title": "改名"})
    assert r.json()["title"] == "改名"
    assert client.get("/api/admin/curricula").json()[0]["title"] == "改名"
    assert client.delete(f"/api/admin/curricula/{c['id']}").status_code == 200
    assert client.get("/api/admin/curricula").json() == []


def test_duplicate_slug_400(client):
    make_curriculum(client)
    r = client.post("/api/admin/curricula", json={"slug": "shapes-01", "title": "重复"})
    assert r.status_code == 400


def test_activate_exclusive_and_archived_untouched(client):
    a = make_curriculum(client, slug="a")
    b = make_curriculum(client, slug="b")
    c = make_curriculum(client, slug="c")
    client.put(f"/api/admin/curricula/{c['id']}", json={"status": "archived"})
    client.post(f"/api/admin/curricula/{a['id']}/activate")
    client.post(f"/api/admin/curricula/{b['id']}/activate")
    by_slug = {x["slug"]: x["status"] for x in client.get("/api/admin/curricula").json()}
    assert by_slug == {"a": "draft", "b": "active", "c": "archived"}


def test_lessons_crud_sorted_by_seq(client):
    c = make_curriculum(client)
    make_lesson(client, c["id"], seq=2)
    l1 = make_lesson(client, c["id"], seq=1)
    lessons = client.get(f"/api/admin/curricula/{c['id']}/lessons").json()
    assert [x["seq"] for x in lessons] == [1, 2]
    r = client.put(f"/api/admin/lessons/{l1['id']}", json={"goal_text": "目标"})
    assert r.json()["goal_text"] == "目标"
    assert client.delete(f"/api/admin/lessons/{l1['id']}").status_code == 200


def test_pointer_set_and_validation(client):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    r = client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": l1["id"]})
    assert r.json()["current_lesson_id"] == l1["id"]
    r = client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": None})
    assert r.json()["current_lesson_id"] is None
    assert client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": 999}).status_code == 400


def test_delete_curriculum_cascades(client, db):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    db.add(models.LessonRun(lesson_id=l1["id"]))
    db.commit()
    client.delete(f"/api/admin/curricula/{c['id']}")
    assert db.query(models.Lesson).count() == 0
    assert db.query(models.LessonRun).count() == 0


def test_runs_list_and_correction(client, db):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1, title="圆圆的朋友")
    t = models.Turn(source="tablet", input_image_path="images/x.png")
    db.add(t)
    db.flush()
    run = models.LessonRun(lesson_id=l1["id"], status="completed",
                           highlights="亮点", artifact_turn_ids=[t.id])
    db.add(run)
    db.commit()
    items = client.get("/api/admin/lesson-runs").json()["items"]
    assert items[0]["lesson_title"] == "圆圆的朋友"
    assert items[0]["curriculum_title"] == "形状小画家"
    assert items[0]["artifact_images"] == ["images/x.png"]
    r = client.put(f"/api/admin/lesson-runs/{run.id}",
                   json={"status": "skipped", "parent_note": "当天生病"})
    assert r.json()["status"] == "skipped" and r.json()["parent_note"] == "当天生病"


def test_status_cannot_bypass_activate(client):
    a = make_curriculum(client, slug="a")
    b = make_curriculum(client, slug="b")
    client.post(f"/api/admin/curricula/{a['id']}/activate")
    r = client.post("/api/admin/curricula", json={"slug": "x", "title": "直设", "status": "active"})
    assert r.status_code == 400
    r = client.put(f"/api/admin/curricula/{b['id']}", json={"status": "active"})
    assert r.status_code == 400
    r = client.put(f"/api/admin/curricula/{b['id']}", json={"status": "怪值"})
    assert r.status_code == 400
    by_slug = {x["slug"]: x["status"] for x in client.get("/api/admin/curricula").json()}
    assert by_slug == {"a": "active", "b": "draft"}


def test_update_duplicate_slug_400(client):
    make_curriculum(client, slug="a")
    b = make_curriculum(client, slug="b")
    r = client.put(f"/api/admin/curricula/{b['id']}", json={"slug": "a"})
    assert r.status_code == 400
    r = client.put(f"/api/admin/curricula/{b['id']}", json={"slug": "b", "title": "保留自身slug"})
    assert r.status_code == 200


def test_delete_pointed_lesson_clears_pointer(client):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    client.put(f"/api/admin/curricula/{c['id']}/pointer", json={"lesson_id": l1["id"]})
    client.delete(f"/api/admin/lessons/{l1['id']}")
    assert client.get("/api/admin/curricula").json()[0]["current_lesson_id"] is None


def test_run_invalid_status_400(client, db):
    c = make_curriculum(client)
    l1 = make_lesson(client, c["id"], seq=1)
    run = models.LessonRun(lesson_id=l1["id"])
    db.add(run)
    db.commit()
    r = client.put(f"/api/admin/lesson-runs/{run.id}", json={"status": "怪"})
    assert r.status_code == 400
