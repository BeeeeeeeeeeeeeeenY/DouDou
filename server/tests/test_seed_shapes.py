from app import models
from app.engine.lesson import LESSON_REPORT_MARK, RECAP_TOKEN


def test_seed_creates_8_lessons(client, db):
    r = client.post("/api/admin/curricula/seed-shapes01")
    assert r.status_code == 200
    c = r.json()
    assert c["slug"] == "shapes-01" and c["age_band"] == "3-4"
    lessons = client.get(f"/api/admin/curricula/{c['id']}/lessons").json()
    assert len(lessons) == 8
    assert [l["seq"] for l in lessons] == list(range(1, 9))
    assert [l["slug"] for l in lessons] == [f"shapes-01-{i:02d}" for i in range(1, 9)]
    assert lessons[2]["title"] == "圆圆的朋友"


def test_seed_scripts_are_complete(client, db):
    client.post("/api/admin/curricula/seed-shapes01")
    lessons = db.query(models.Lesson).order_by(models.Lesson.seq).all()
    assert RECAP_TOKEN not in lessons[0].script_text        # 第 1 课无复习占位符
    for l in lessons[1:]:
        assert RECAP_TOKEN in l.script_text                 # 第 2-8 课都有
    for l in lessons:
        assert LESSON_REPORT_MARK in l.script_text          # 打标协议写进每课脚本
        assert "五环节" in l.script_text and l.goal_text and l.materials
        assert l.segments and len(l.segments) == 5
        assert l.segments[3]["channel"] == "tablet"         # 第④环节走平板
        # 未开画不收尾护栏：孩子还没在平板上提交画作前，绝不收尾/打标
        assert "还没在平板上画" in l.script_text


def test_seed_idempotent(client, db):
    a = client.post("/api/admin/curricula/seed-shapes01").json()
    b = client.post("/api/admin/curricula/seed-shapes01").json()
    assert a["id"] == b["id"]
    assert db.query(models.Lesson).count() == 8
