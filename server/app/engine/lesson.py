import json

from sqlalchemy import and_, or_

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


def active_current_lesson(db) -> tuple[Curriculum, Lesson] | None:
    cur = db.query(Curriculum).filter(Curriculum.status == "active").first()
    if cur is None or cur.current_lesson_id is None:
        return None
    lesson = db.get(Lesson, cur.current_lesson_id)
    if lesson is None:
        return None
    return cur, lesson


def render_lesson_script(script_text: str, prev_recap: str) -> str:
    """把 {prev_lesson_recap} 替换为上次课回顾。用 replace 不用 format（脚本含花括号示例）。"""
    if RECAP_TOKEN not in script_text:
        return script_text
    return script_text.replace(RECAP_TOKEN, prev_recap or NO_RECAP_TEXT)


def format_recap(title: str, highlights: str, parent_tip: str) -> str:
    highlights = highlights or ""
    parent_tip = parent_tip or ""
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
    window_end = run.ended_at or utcnow()
    turns = (
        db.query(Turn)
        .filter(
            Turn.source == "tablet",
            Turn.ts >= run.started_at,
            Turn.ts <= window_end,
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


def run_has_drawing(db, run: LessonRun) -> bool:
    """本房间是否已有平板画作提交。收尾门槛：孩子还没在平板上开画，就不让
    模型收尾/打标关课（治「刚打招呼说句『好』就被判未参与关课→房间死→语音
    豆豆取不到本房间图开始瞎编画」）。「属于本房间」的判定与 attach_artifacts
    保持一致：已挂靠本 run，或尚未挂靠但落在本 run 的起止时间窗内。"""
    window_end = run.ended_at or utcnow()
    exists_q = (
        db.query(Turn.id)
        .filter(
            Turn.source == "tablet",
            or_(
                Turn.lesson_run_id == run.id,
                and_(
                    Turn.lesson_run_id.is_(None),
                    Turn.ts >= run.started_at,
                    Turn.ts <= window_end,
                ),
            ),
        )
        .exists()
    )
    return bool(db.query(exists_q).scalar())


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
    run.raw_report = dict(report, _raw=raw) if raw else report
    run.ended_at = utcnow()
    attach_artifacts(db, run)
    if run.status == "completed":
        advance_pointer(db, run)
    db.commit()


def close_run_malformed(db, run: LessonRun, raw: str) -> None:
    """打标 JSON 解析失败的兜底（spec §6.3）：保留原文、按未收尾关闭，作品照常挂靠。"""
    run.raw_report = {"_raw": raw}
    run.status = "abandoned"
    run.ended_at = utcnow()
    attach_artifacts(db, run)
    db.commit()
