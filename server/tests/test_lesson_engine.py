from app import models
from app.engine.lesson import (
    RECAP_TOKEN,
    close_run_with_report,
    format_recap,
    latest_recap,
    parse_lesson_report,
    render_lesson_script,
)

REPORT_LINE = (
    '⟦lesson_report⟧{"lesson_id":"shapes-01-03","status":"completed",'
    '"highlights":"画了5个泡泡","parent_tip":"在家吹泡泡"}'
)


def test_parse_report_strips_and_parses():
    clean, report, raw = parse_lesson_report("今天真棒！\n" + REPORT_LINE)
    assert clean == "今天真棒！"
    assert report["status"] == "completed" and report["highlights"] == "画了5个泡泡"
    assert raw.startswith('{"lesson_id"')


def test_parse_report_absent():
    clean, report, raw = parse_lesson_report("普通回复")
    assert (clean, report, raw) == ("普通回复", None, "")


def test_parse_report_malformed_json():
    clean, report, raw = parse_lesson_report("收尾啦 ⟦lesson_report⟧{oops")
    assert clean == "收尾啦"  # 坏 JSON 也要剥离，不能进 TTS
    assert report is None and raw == "{oops"


def test_render_script_replaces_token():
    out = render_lesson_script(f"回顾：{RECAP_TOKEN}。开始", "上次画了线")
    assert out == "回顾：上次画了线。开始"
    out2 = render_lesson_script(f"回顾：{RECAP_TOKEN}。开始", "")
    assert "没有上次课的记录" in out2
    assert render_lesson_script("没有占位符 {x}", "r") == "没有占位符 {x}"


def test_format_recap():
    assert format_recap("圆圆的朋友", "画了泡泡", "") == "上次上的是《圆圆的朋友》。孩子的表现：画了泡泡"
    assert "延伸建议" in format_recap("圆圆的朋友", "画了泡泡", "吹泡泡")


def _seed_minimal(db):
    cur = models.Curriculum(slug="c", title="课")
    db.add(cur)
    db.flush()
    l1 = models.Lesson(curriculum_id=cur.id, seq=1, title="一")
    l2 = models.Lesson(curriculum_id=cur.id, seq=2, title="二")
    db.add_all([l1, l2])
    db.flush()
    cur.current_lesson_id = l1.id
    db.commit()
    return cur, l1, l2


def test_latest_recap_picks_newest_non_abandoned(db):
    cur, l1, _ = _seed_minimal(db)
    assert latest_recap(db, cur.id) == ""
    db.add(models.LessonRun(lesson_id=l1.id, status="abandoned", highlights="废"))
    db.add(models.LessonRun(lesson_id=l1.id, status="completed", highlights="真棒"))
    db.commit()
    assert "真棒" in latest_recap(db, cur.id) and "废" not in latest_recap(db, cur.id)


def test_close_run_completed_advances_and_attaches(db):
    cur, l1, l2 = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    tablet = models.Turn(source="tablet")  # ts 默认 now，落在 run 起止窗内
    other = models.Turn(source="phone")
    db.add_all([tablet, other])
    db.commit()
    close_run_with_report(db, run, {"status": "completed", "highlights": "亮", "parent_tip": "提"}, "{...}")
    assert run.status == "completed" and run.ended_at is not None
    assert run.highlights == "亮" and run.parent_tip == "提"
    assert run.raw_report["_raw"] == "{...}"
    assert run.artifact_turn_ids == [tablet.id]
    assert db.get(models.Turn, tablet.id).lesson_run_id == run.id
    assert db.get(models.Turn, other.id).lesson_run_id is None
    assert db.get(models.Curriculum, cur.id).current_lesson_id == l2.id


def test_close_run_completed_does_not_steal_moved_pointer(db):
    cur, l1, l2 = _seed_minimal(db)
    cur.current_lesson_id = l2.id  # 家长手动把指针改到了别的课
    db.commit()
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "completed"}, "")
    assert db.get(models.Curriculum, cur.id).current_lesson_id == l2.id  # 不抢


def test_close_run_partial_keeps_pointer(db):
    cur, l1, _ = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "partial", "highlights": "", "parent_tip": ""}, "")
    assert run.status == "partial"
    assert db.get(models.Curriculum, cur.id).current_lesson_id == l1.id


def test_close_run_bad_status_coerced_to_partial(db):
    _, l1, _ = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "great!!"}, "")
    assert run.status == "partial"


def test_last_lesson_completion_clears_pointer(db):
    cur, l1, l2 = _seed_minimal(db)
    cur.current_lesson_id = l2.id
    db.commit()
    run = models.LessonRun(lesson_id=l2.id)
    db.add(run)
    db.commit()
    close_run_with_report(db, run, {"status": "completed"}, "")
    assert db.get(models.Curriculum, cur.id).current_lesson_id is None


def test_close_run_malformed_preserves_raw_and_attaches(db):
    cur, l1, _ = _seed_minimal(db)
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    tablet = models.Turn(source="tablet")
    db.add(tablet)
    db.commit()
    from app.engine.lesson import close_run_malformed
    close_run_malformed(db, run, "{oops")
    assert run.status == "abandoned" and run.ended_at is not None
    assert run.raw_report == {"_raw": "{oops"}
    assert run.artifact_turn_ids == [tablet.id]


def test_attach_artifacts_excludes_after_window(db):
    _, l1, _ = _seed_minimal(db)
    from app.engine.lesson import attach_artifacts
    from app.models import utcnow
    run = models.LessonRun(lesson_id=l1.id)
    db.add(run)
    db.commit()
    run.ended_at = utcnow()
    db.commit()
    late = models.Turn(source="tablet")  # ts 晚于 ended_at
    db.add(late)
    db.commit()
    attach_artifacts(db, run)
    assert run.artifact_turn_ids in (None, [])
    assert db.get(models.Turn, late.id).lesson_run_id is None


from app.engine.lesson import parse_demo


def test_parse_demo_extracts_and_strips_known_shape():
    clean, shape = parse_demo("先画一个圆圆的小脑袋\n⟦demo:circle⟧")
    assert shape == "circle"
    assert "demo" not in clean and clean == "先画一个圆圆的小脑袋"


def test_parse_demo_unknown_shape_stripped_but_not_recognized():
    clean, shape = parse_demo("好呀 ⟦demo:banana⟧")
    assert shape is None
    assert "demo" not in clean and clean == "好呀"


def test_parse_demo_absent_returns_text_unchanged():
    clean, shape = parse_demo("普通一句话")
    assert (clean, shape) == ("普通一句话", None)


def test_parse_demo_unclosed_marker_truncates():
    clean, shape = parse_demo("画个圆 ⟦demo:circle")
    assert shape is None and clean == "画个圆"
