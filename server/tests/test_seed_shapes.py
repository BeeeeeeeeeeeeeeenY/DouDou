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
    assert lessons[2]["title"] == "画气球"


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
    # 第 3 课「画气球」双通道：带 demo 触发（只一次）+ 平板不写字；其余课不带 demo
    l3 = next(x for x in lessons if x.slug == "shapes-01-03")
    assert l3.title == "画气球"
    assert l3.script_text.count("⟦demo:circle⟧") == 1          # 只演示一次
    assert "只在这第一次" in l3.script_text                     # 演示去重的话术护栏
    assert "绝不在平板上写字" in l3.script_text                 # 平板零文字
    for l in lessons:
        if l.slug != "shapes-01-03":
            assert "⟦demo:circle⟧" not in l.script_text


def test_seed_idempotent(client, db):
    a = client.post("/api/admin/curricula/seed-shapes01").json()
    b = client.post("/api/admin/curricula/seed-shapes01").json()
    assert a["id"] == b["id"]
    assert db.query(models.Lesson).count() == 8
