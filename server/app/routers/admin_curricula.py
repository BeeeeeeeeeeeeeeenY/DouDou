from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Curriculum, Lesson, LessonRun, Turn

router = APIRouter(prefix="/api/admin")

CUR_FIELDS = ("slug", "title", "age_band", "description", "status")
LESSON_FIELDS = ("seq", "slug", "title", "goal_text", "script_text", "segments",
                 "duration_min", "materials", "enhancements")
RUN_STATUS = ("running", "completed", "partial", "skipped", "abandoned")
SETTABLE_STATUS = ("draft", "archived")


class CurriculumIn(BaseModel):
    slug: str | None = None
    title: str | None = None
    age_band: str | None = None
    description: str | None = None
    status: str | None = None


class LessonIn(BaseModel):
    seq: int | None = None
    slug: str | None = None
    title: str | None = None
    goal_text: str | None = None
    script_text: str | None = None
    segments: list | None = None
    duration_min: int | None = None
    materials: str | None = None
    enhancements: list | None = None


class PointerIn(BaseModel):
    lesson_id: int | None = None


class RunIn(BaseModel):
    status: str | None = None
    parent_note: str | None = None
    artifact_turn_ids: list | None = None


def cur_json(c: Curriculum) -> dict:
    return {f: getattr(c, f) for f in CUR_FIELDS} | {
        "id": c.id, "current_lesson_id": c.current_lesson_id,
    }


def lesson_json(l: Lesson) -> dict:
    return {f: getattr(l, f) for f in LESSON_FIELDS} | {
        "id": l.id, "curriculum_id": l.curriculum_id,
    }


def _cur_or_404(db: Session, cid: int) -> Curriculum:
    c = db.get(Curriculum, cid)
    if c is None:
        raise HTTPException(404, "课程不存在")
    return c


@router.get("/curricula")
def list_curricula(db: Session = Depends(get_db)):
    return [cur_json(c) for c in db.query(Curriculum).order_by(Curriculum.id).all()]


@router.post("/curricula")
def create_curriculum(body: CurriculumIn, db: Session = Depends(get_db)):
    if not body.slug:
        raise HTTPException(400, "slug 不能为空")
    if db.query(Curriculum).filter(Curriculum.slug == body.slug).first():
        raise HTTPException(400, "slug 已存在")
    if body.status is not None and body.status not in SETTABLE_STATUS:
        raise HTTPException(400, "status 只能设为 draft 或 archived；激活请用「设为生效」")
    c = Curriculum(slug=body.slug)
    for f in CUR_FIELDS[1:]:
        v = getattr(body, f)
        if v is not None:
            setattr(c, f, v)
    db.add(c)
    db.commit()
    return cur_json(c)


@router.put("/curricula/{cid}")
def update_curriculum(cid: int, body: CurriculumIn, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    if body.status is not None and body.status not in SETTABLE_STATUS:
        raise HTTPException(400, "status 只能设为 draft 或 archived；激活请用「设为生效」")
    if body.slug is not None and body.slug != c.slug:
        if db.query(Curriculum).filter(Curriculum.slug == body.slug).first():
            raise HTTPException(400, "slug 已存在")
    for f in CUR_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(c, f, v)
    db.commit()
    return cur_json(c)


@router.delete("/curricula/{cid}")
def delete_curriculum(cid: int, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    lesson_ids = [l.id for l in db.query(Lesson).filter(Lesson.curriculum_id == cid).all()]
    if lesson_ids:
        db.query(LessonRun).filter(LessonRun.lesson_id.in_(lesson_ids)).delete()
        db.query(Lesson).filter(Lesson.id.in_(lesson_ids)).delete()
    db.delete(c)
    db.commit()
    return {"ok": True}


@router.post("/curricula/{cid}/activate")
def activate_curriculum(cid: int, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    db.query(Curriculum).filter(Curriculum.status == "active").update(
        {Curriculum.status: "draft"}
    )
    c.status = "active"
    db.commit()
    return cur_json(c)


@router.put("/curricula/{cid}/pointer")
def set_pointer(cid: int, body: PointerIn, db: Session = Depends(get_db)):
    c = _cur_or_404(db, cid)
    if body.lesson_id is not None:
        lesson = db.get(Lesson, body.lesson_id)
        if lesson is None or lesson.curriculum_id != cid:
            raise HTTPException(400, "课时不属于该课程")
    c.current_lesson_id = body.lesson_id
    db.commit()
    return cur_json(c)


@router.get("/curricula/{cid}/lessons")
def list_lessons(cid: int, db: Session = Depends(get_db)):
    _cur_or_404(db, cid)
    rows = db.query(Lesson).filter(Lesson.curriculum_id == cid).order_by(Lesson.seq).all()
    return [lesson_json(l) for l in rows]


@router.post("/curricula/{cid}/lessons")
def create_lesson(cid: int, body: LessonIn, db: Session = Depends(get_db)):
    _cur_or_404(db, cid)
    l = Lesson(curriculum_id=cid, seq=body.seq or 1)
    for f in LESSON_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(l, f, v)
    db.add(l)
    db.commit()
    return lesson_json(l)


@router.put("/lessons/{lid}")
def update_lesson(lid: int, body: LessonIn, db: Session = Depends(get_db)):
    l = db.get(Lesson, lid)
    if l is None:
        raise HTTPException(404, "课时不存在")
    for f in LESSON_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(l, f, v)
    db.commit()
    return lesson_json(l)


@router.delete("/lessons/{lid}")
def delete_lesson(lid: int, db: Session = Depends(get_db)):
    l = db.get(Lesson, lid)
    if l is None:
        raise HTTPException(404, "课时不存在")
    db.query(LessonRun).filter(LessonRun.lesson_id == lid).delete()
    db.query(Curriculum).filter(Curriculum.current_lesson_id == lid).update(
        {Curriculum.current_lesson_id: None}
    )
    db.delete(l)
    db.commit()
    return {"ok": True}


@router.get("/lesson-runs")
def list_runs(limit: int = 50, db: Session = Depends(get_db)):
    rows = (
        db.query(LessonRun, Lesson, Curriculum)
        .join(Lesson, LessonRun.lesson_id == Lesson.id)
        .join(Curriculum, Lesson.curriculum_id == Curriculum.id)
        .order_by(LessonRun.started_at.desc(), LessonRun.id.desc())
        .limit(limit)
        .all()
    )
    items = []
    for run, lesson, cur in rows:
        images = []
        if run.artifact_turn_ids:
            turns = db.query(Turn).filter(Turn.id.in_(run.artifact_turn_ids)).all()
            images = [t.input_image_path for t in turns if t.input_image_path]
        items.append({
            "id": run.id, "lesson_id": lesson.id, "lesson_seq": lesson.seq,
            "lesson_title": lesson.title, "curriculum_title": cur.title,
            "started_at": run.started_at.isoformat() if run.started_at else "",
            "ended_at": run.ended_at.isoformat() if run.ended_at else "",
            "status": run.status, "highlights": run.highlights,
            "parent_tip": run.parent_tip, "parent_note": run.parent_note,
            "artifact_turn_ids": run.artifact_turn_ids or [],
            "artifact_images": images,
        })
    return {"items": items}


@router.put("/lesson-runs/{rid}")
def update_run(rid: int, body: RunIn, db: Session = Depends(get_db)):
    run = db.get(LessonRun, rid)
    if run is None:
        raise HTTPException(404, "上课记录不存在")
    if body.status is not None:
        if body.status not in RUN_STATUS:
            raise HTTPException(400, "非法状态")
        run.status = body.status
    if body.parent_note is not None:
        run.parent_note = body.parent_note
    if body.artifact_turn_ids is not None:
        run.artifact_turn_ids = body.artifact_turn_ids
    db.commit()
    return {"id": run.id, "status": run.status, "parent_note": run.parent_note,
            "artifact_turn_ids": run.artifact_turn_ids or []}
