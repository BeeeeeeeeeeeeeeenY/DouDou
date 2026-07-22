import json

from app.models import Curriculum, Lesson, LessonRun, Turn, utcnow

LESSON_REPORT_MARK = "⟦lesson_report⟧"
RECAP_TOKEN = "{prev_lesson_recap}"
NO_RECAP_TEXT = "（没有上次课的记录，简短问好后直接开始）"
VALID_REPORT_STATUS = ("completed", "partial", "skipped")


def parse_lesson_report(text: str) -> tuple[str, dict | None, str]:
    """从回复中剥离 ⟦lesson_report⟧ 标记。坏 JSON 也剥离（不能念给孩子），报告记 None。"""
    idx = text.find(LESSON_REPORT_MARK)
    if idx == -1:
        return text.strip(), None, ""
    clean = text[:idx].strip()
    raw = text[idx + len(LESSON_REPORT_MARK):].strip()
    try:
        report = json.loads(raw)
        if not isinstance(report, dict):
            report = None
    except json.JSONDecodeError:
        report = None
    return clean, report, raw


def render_lesson_script(script_text: str, prev_recap: str) -> str:
    """把 {prev_lesson_recap} 替换为上次课回顾。用 replace 不用 format（脚本含花括号示例）。"""
    if RECAP_TOKEN not in script_text:
        return script_text
    return script_text.replace(RECAP_TOKEN, prev_recap or NO_RECAP_TEXT)


def format_recap(title: str, highlights: str, parent_tip: str) -> str:
    out = f"上次上的是《{title}》。孩子的表现：{highlights}"
    if parent_tip:
        out += f"（当时给家长的延伸建议：{parent_tip}）"
    return out


def latest_recap(db, curriculum_id: int) -> str:
    row = (
        db.query(LessonRun, Lesson)
        .join(Lesson, LessonRun.lesson_id == Lesson.id)
        .filter(
            Lesson.curriculum_id == curriculum_id,
            LessonRun.status.in_(("completed", "partial")),
        )
        .order_by(LessonRun.started_at.desc(), LessonRun.id.desc())
        .first()
    )
    if row is None:
        return ""
    run, lesson = row
    return format_recap(lesson.title, run.highlights, run.parent_tip)


def attach_artifacts(db, run: LessonRun) -> None:
    """把 run 起止窗内、尚未归属的平板轮挂为本课作品（spec §8 时间窗自动挂靠）。"""
    turns = (
        db.query(Turn)
        .filter(
            Turn.source == "tablet",
            Turn.ts >= run.started_at,
            Turn.lesson_run_id.is_(None),
        )
        .order_by(Turn.id)
        .all()
    )
    ids = list(run.artifact_turn_ids or [])
    for t in turns:
        t.lesson_run_id = run.id
        ids.append(t.id)
    run.artifact_turn_ids = ids


def advance_pointer(db, run: LessonRun) -> None:
    """仅当课程指针仍指向本课时推进；末课完成后指针置空（= 本轮完成）。"""
    lesson = db.get(Lesson, run.lesson_id)
    cur = db.get(Curriculum, lesson.curriculum_id)
    if cur is None or cur.current_lesson_id != lesson.id:
        return  # 家长手动改过指针时不抢
    nxt = (
        db.query(Lesson)
        .filter(Lesson.curriculum_id == cur.id, Lesson.seq > lesson.seq)
        .order_by(Lesson.seq)
        .first()
    )
    cur.current_lesson_id = nxt.id if nxt else None


def close_run_with_report(db, run: LessonRun, report: dict, raw: str) -> None:
    status = report.get("status")
    run.status = status if status in VALID_REPORT_STATUS else "partial"
    run.highlights = str(report.get("highlights", ""))
    run.parent_tip = str(report.get("parent_tip", ""))
    run.raw_report = report
    run.ended_at = utcnow()
    attach_artifacts(db, run)
    if run.status == "completed":
        advance_pointer(db, run)
    db.commit()
