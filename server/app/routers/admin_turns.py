from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Turn

router = APIRouter(prefix="/api/admin/turns")

FIELDS = ("id", "source", "profile_id", "profile_name", "model", "system_prompt",
          "input_text", "input_image_path", "input_audio_path", "transcript",
          "reply_text", "reply_audio_path", "latency_ms", "status", "error")


def to_json(t: Turn) -> dict:
    return {f: getattr(t, f) for f in FIELDS} | {"ts": t.ts.isoformat()}


@router.get("")
def list_turns(limit: int = 50, offset: int = 0, db: Session = Depends(get_db)):
    q = db.query(Turn).order_by(Turn.id.desc())
    return {"total": q.count(), "items": [to_json(t) for t in q.offset(offset).limit(limit)]}


@router.get("/{tid}")
def get_turn(tid: int, db: Session = Depends(get_db)):
    t = db.get(Turn, tid)
    if t is None:
        raise HTTPException(404, "记录不存在")
    return to_json(t)
