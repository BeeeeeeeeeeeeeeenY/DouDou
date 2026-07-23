import sqlite3

from sqlalchemy import text

from app import models
from app.db import make_sessionmaker


def test_curriculum_tables_exist(db):
    assert db.query(models.Curriculum).count() == 0
    assert db.query(models.Lesson).count() == 0
    assert db.query(models.LessonRun).count() == 0


def test_defaults(db):
    cur = models.Curriculum(slug="shapes-01", title="形状小画家")
    db.add(cur)
    db.flush()
    lesson = models.Lesson(curriculum_id=cur.id, seq=1)
    db.add(lesson)
    db.flush()
    run = models.LessonRun(lesson_id=lesson.id)
    db.add(run)
    db.commit()
    assert cur.status == "draft" and cur.current_lesson_id is None
    assert lesson.duration_min == 10
    assert run.status == "running" and run.ended_at is None
    assert run.memory_tags is None  # 一期恒空，二期记忆挂载点


def test_turn_has_lesson_run_id(db):
    db.add(models.Turn(source="phone"))
    db.commit()
    assert db.query(models.Turn).one().lesson_run_id is None


def test_legacy_turns_table_gains_column(tmp_path):
    # 模拟一期旧库：turns 表没有 lesson_run_id 列，启动后应被 ALTER 补上
    con = sqlite3.connect(tmp_path / "doudou.db")
    con.execute("CREATE TABLE turns (id INTEGER PRIMARY KEY, source VARCHAR(10))")
    con.commit()
    con.close()
    maker = make_sessionmaker(str(tmp_path))
    with maker() as s:
        s.execute(text("SELECT lesson_run_id FROM turns"))  # 列不存在会抛 OperationalError


def test_migration_idempotent_on_second_startup(tmp_path):
    # 二次启动（或竞态重试）时列已存在，_migrate 应静默通过
    con = sqlite3.connect(tmp_path / "doudou.db")
    con.execute("CREATE TABLE turns (id INTEGER PRIMARY KEY, source VARCHAR(10))")
    con.commit()
    con.close()
    make_sessionmaker(str(tmp_path))
    maker = make_sessionmaker(str(tmp_path))  # 第二次启动不得抛错
    with maker() as s:
        s.execute(text("SELECT lesson_run_id FROM turns"))


def test_lesson_run_pending_fields_default_none_and_settable(db):
    from app import models
    lesson = db.query(models.Lesson).first()
    if lesson is None:
        cur = models.Curriculum(slug="t", title="t")
        db.add(cur); db.flush()
        lesson = models.Lesson(curriculum_id=cur.id, seq=1, slug="t-1", title="t")
        db.add(lesson); db.flush()
    run = models.LessonRun(lesson_id=lesson.id)
    db.add(run); db.commit()
    assert run.pending_demo is None and run.pending_command is None
    run.pending_demo = "circle"
    run.pending_command = "clear"
    db.commit()
    db.expire_all()
    got = db.get(models.LessonRun, run.id)
    assert got.pending_demo == "circle" and got.pending_command == "clear"


def test_lesson_run_redesign_state_columns_default_and_settable(db):
    from app import models
    lesson = db.query(models.Lesson).first()
    if lesson is None:
        cur = models.Curriculum(slug="t2", title="t2")
        db.add(cur); db.flush()
        lesson = models.Lesson(curriculum_id=cur.id, seq=1, slug="t2-1", title="t2")
        db.add(lesson); db.flush()
    run = models.LessonRun(lesson_id=lesson.id)
    db.add(run); db.commit()
    # Test defaults
    assert run.demoed_shapes is None
    assert run.pending_utterance is None
    assert run.tablet_turns == 0
    assert run.last_image_turn == 0
    # Set values
    run.demoed_shapes = ["circle"]
    run.pending_utterance = {"text": "hi"}
    run.tablet_turns = 2
    run.last_image_turn = 1
    db.commit()
    db.expire_all()
    got = db.get(models.LessonRun, run.id)
    assert got.demoed_shapes == ["circle"]
    assert got.pending_utterance == {"text": "hi"}
    assert got.tablet_turns == 2
    assert got.last_image_turn == 1
