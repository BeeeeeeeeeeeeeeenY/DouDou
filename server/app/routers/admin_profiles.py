from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Profile

router = APIRouter(prefix="/api/admin/profiles")

FIELDS = ("name", "age_band", "persona_text", "voice_hint", "provider_id",
          "model", "temperature", "max_tokens", "reasoning_effort")


class ProfileIn(BaseModel):
    name: str | None = None
    age_band: str | None = None
    persona_text: str | None = None
    voice_hint: str | None = None
    provider_id: int | None = None
    model: str | None = None
    temperature: float | None = None
    max_tokens: int | None = None
    reasoning_effort: str | None = None


def to_json(p: Profile) -> dict:
    return {f: getattr(p, f) for f in FIELDS} | {"id": p.id, "is_active": p.is_active}


@router.get("")
def list_profiles(db: Session = Depends(get_db)):
    return [to_json(p) for p in db.query(Profile).order_by(Profile.id).all()]


@router.post("")
def create_profile(body: ProfileIn, db: Session = Depends(get_db)):
    p = Profile()
    for f in FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(p, f, v)
    db.add(p)
    db.commit()
    return to_json(p)


def _get_or_404(db: Session, pid: int) -> Profile:
    p = db.get(Profile, pid)
    if p is None:
        raise HTTPException(404, "profile 不存在")
    return p


@router.put("/{pid}")
def update_profile(pid: int, body: ProfileIn, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    for f in FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(p, f, v)
    db.commit()
    return to_json(p)


@router.delete("/{pid}")
def delete_profile(pid: int, db: Session = Depends(get_db)):
    db.delete(_get_or_404(db, pid))
    db.commit()
    return {"ok": True}


@router.post("/{pid}/activate")
def activate_profile(pid: int, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    db.query(Profile).update({Profile.is_active: False})
    p.is_active = True
    db.commit()
    return to_json(p)
