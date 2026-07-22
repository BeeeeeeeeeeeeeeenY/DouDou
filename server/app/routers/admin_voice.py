from fastapi import APIRouter, Depends, Form, HTTPException, UploadFile
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
async def stt_test(
    audio: UploadFile,
    stt_provider_id: int | None = Form(None),
    stt_model: str | None = Form(None),
    db: Session = Depends(get_db),
):
    """测转写「试听即所见」：优先用页面当前值（表单字段），缺省回退已保存配置。"""
    vs = db.get(VoiceSettings, 1)
    pid = stt_provider_id or vs.stt_provider_id
    model = stt_model or vs.stt_model
    p = db.get(Provider, pid) if pid else None
    if not (p and model):
        raise HTTPException(400, "请先在 DouDou 后台完成语音配置")
    data = await audio.read()
    try:
        text = await transcribe(p.base_url, p.api_key, model,
                                data, audio.filename or "audio.webm")
    except UpstreamError as e:
        raise HTTPException(502, f"语音服务出错（{e.status_code}）：{e.detail[:200]}")
    return {"text": text}


class TtsIn(BaseModel):
    text: str
    tts_provider_id: int | None = None
    tts_model: str | None = None
    tts_voice: str | None = None
    tts_speed: float | None = None


@router.post("/voice/tts-test")
async def tts_test(body: TtsIn, db: Session = Depends(get_db)):
    """试听「试听即所见」：优先用请求里带的页面当前值，缺省回退已保存配置。"""
    vs = db.get(VoiceSettings, 1)
    pid = body.tts_provider_id or vs.tts_provider_id
    model = body.tts_model or vs.tts_model
    voice = body.tts_voice if body.tts_voice is not None else vs.tts_voice
    speed = body.tts_speed or vs.tts_speed
    p = db.get(Provider, pid) if pid else None
    if not (p and model):
        raise HTTPException(400, "请先在 DouDou 后台完成语音配置")
    try:
        audio = await synthesize(p.base_url, p.api_key, model, voice, body.text, speed)
    except UpstreamError as e:
        raise HTTPException(502, f"语音服务出错（{e.status_code}）：{e.detail[:200]}")
    return Response(content=audio, media_type="audio/mpeg")
