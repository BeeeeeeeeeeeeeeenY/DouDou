import time

import httpx
from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.models import Provider

router = APIRouter(prefix="/api/admin/providers")


class ProviderIn(BaseModel):
    name: str | None = None
    base_url: str | None = None
    api_key: str | None = None
    enabled: bool | None = None
    notes: str | None = None


def to_json(p: Provider) -> dict:
    return {
        "id": p.id, "name": p.name, "base_url": p.base_url,
        "api_key": p.api_key, "enabled": p.enabled, "notes": p.notes,
    }


@router.get("")
def list_providers(db: Session = Depends(get_db)):
    return [to_json(p) for p in db.query(Provider).order_by(Provider.id).all()]


@router.post("")
def create_provider(body: ProviderIn, db: Session = Depends(get_db)):
    p = Provider(
        name=body.name or "", base_url=(body.base_url or "").rstrip("/"),
        api_key=body.api_key or "", enabled=body.enabled if body.enabled is not None else True,
        notes=body.notes or "",
    )
    db.add(p)
    db.commit()
    return to_json(p)


def _get_or_404(db: Session, pid: int) -> Provider:
    p = db.get(Provider, pid)
    if p is None:
        raise HTTPException(404, "provider 不存在")
    return p


@router.put("/{pid}")
def update_provider(pid: int, body: ProviderIn, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    for field in ("name", "api_key", "notes", "enabled"):
        v = getattr(body, field)
        if v is not None:
            setattr(p, field, v)
    if body.base_url is not None:
        p.base_url = body.base_url.rstrip("/")
    db.commit()
    return to_json(p)


@router.delete("/{pid}")
def delete_provider(pid: int, db: Session = Depends(get_db)):
    db.delete(_get_or_404(db, pid))
    db.commit()
    return {"ok": True}


@router.post("/{pid}/test")
def test_provider(pid: int, db: Session = Depends(get_db)):
    p = _get_or_404(db, pid)
    t0 = time.monotonic()
    try:
        resp = httpx.get(
            f"{p.base_url}/models",
            headers={"Authorization": f"Bearer {p.api_key}"},
            timeout=8,
        )
        latency = int((time.monotonic() - t0) * 1000)
        if resp.status_code != 200:
            return {"ok": False, "latency_ms": latency, "models": [],
                    "error": f"HTTP {resp.status_code}: {resp.text[:200]}"}
        models = [m.get("id", "") for m in resp.json().get("data", [])]
        return {"ok": True, "latency_ms": latency, "models": models, "error": ""}
    except httpx.HTTPError as e:
        latency = int((time.monotonic() - t0) * 1000)
        return {"ok": False, "latency_ms": latency, "models": [], "error": str(e)}
