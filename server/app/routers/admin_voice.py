from fastapi import APIRouter, Depends, HTTPException, UploadFile
from fastapi.responses import Response
from pydantic import BaseModel
from sqlalchemy.orm import Session

from app.db import get_db
from app.engine.errors import ConfigError
from app.engine.stt import transcribe
from app.engine.tts import synthesize
from app.engine.upstream import UpstreamError
from app.models import Provider, VoiceSettings

router = APIRouter(prefix="/api/admin")

VS_FIELDS = ("stt_provider_id", "stt_model", "tts_provider_id", "tts_model", "tts_voice", "tts_speed")


class VoiceIn(BaseModel):
    stt_provider_id: int | None = None
    stt_model: str | None = None
    tts_provider_id: int | None = None
    tts_model: str | None = None
    tts_voice: str | None = None
    tts_speed: float | None = None


def to_json(v: VoiceSettings) -> dict:
    return {f: getattr(v, f) for f in VS_FIELDS}


def load_voice_config(db: Session) -> tuple[dict, dict]:
    """返回 (stt_cfg, tts_cfg)，各含 base_url/api_key/model(/voice/speed)。配置不全抛 ConfigError。"""
    vs = db.get(VoiceSettings, 1)
    stt_p = db.get(Provider, vs.stt_provider_id) if vs.stt_provider_id else None
    tts_p = db.get(Provider, vs.tts_provider_id) if vs.tts_provider_id else None
    if not (stt_p and vs.stt_model and tts_p and vs.tts_model):
        raise ConfigError("请先在 DouDou 后台完成语音配置")
    return (
        {"base_url": stt_p.base_url, "api_key": stt_p.api_key, "model": vs.stt_model},
        {"base_url": tts_p.base_url, "api_key": tts_p.api_key, "model": vs.tts_model,
         "voice": vs.tts_voice, "speed": vs.tts_speed},
    )


@router.get("/voice-settings")
def get_settings(db: Session = Depends(get_db)):
    return to_json(db.get(VoiceSettings, 1))


@router.put("/voice-settings")
def put_settings(body: VoiceIn, db: Session = Depends(get_db)):
    vs = db.get(VoiceSettings, 1)
    for f in VS_FIELDS:
        v = getattr(body, f)
        if v is not None:
            setattr(vs, f, v)
    db.commit()
    return to_json(vs)


@router.post("/voice/stt-test")
async def stt_test(audio: UploadFile, db: Session = Depends(get_db)):
    try:
        stt_cfg, _ = load_voice_config(db)
    except ConfigError as e:
        raise HTTPException(400, e.message)
    data = await audio.read()
    try:
        text = await transcribe(stt_cfg["base_url"], stt_cfg["api_key"], stt_cfg["model"],
                                data, audio.filename or "audio.webm")
    except UpstreamError as e:
        raise HTTPException(502, f"语音服务出错（{e.status_code}）：{e.detail[:200]}")
    return {"text": text}


class TtsIn(BaseModel):
    text: str


@router.post("/voice/tts-test")
async def tts_test(body: TtsIn, db: Session = Depends(get_db)):
    try:
        _, tts_cfg = load_voice_config(db)
    except ConfigError as e:
        raise HTTPException(400, e.message)
    try:
        audio = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"], tts_cfg["model"],
                                 tts_cfg["voice"], body.text, tts_cfg["speed"])
    except UpstreamError as e:
        raise HTTPException(502, f"语音服务出错（{e.status_code}）：{e.detail[:200]}")
    return Response(content=audio, media_type="audio/mpeg")
