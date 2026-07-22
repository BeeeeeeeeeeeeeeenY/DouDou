import json
import uuid

from fastapi import APIRouter, Form, HTTPException, Request, UploadFile
from sqlalchemy.orm import Session

from app.engine.errors import ConfigError
from app.engine.tts import synthesize
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError
from app.models import Turn
from app.routers.admin_voice import load_voice_config

router = APIRouter(prefix="/api/phone")


@router.post("/voice-turn")
async def voice_turn(request: Request, audio: UploadFile, history: str = Form("[]")):
    try:
        pairs = json.loads(history)  # [["user","assistant"], ...]
        msgs: list[dict] = []
        for u, a in pairs:
            msgs.append({"role": "user", "content": str(u)})
            msgs.append({"role": "assistant", "content": str(a)})
    except (ValueError, TypeError) as e:
        raise HTTPException(400, f"history 参数格式错误：{e}")

    data = await audio.read()
    tin = TurnInput(source="phone", audio=data,
                    audio_filename=audio.filename or "audio.webm",
                    history=msgs, use_voice_hint=True)
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError as e:
        raise HTTPException(502, f"模型服务出错（{e.status_code}）")

    with request.app.state.sessionmaker() as db:  # type: Session
        try:
            _, tts_cfg = load_voice_config(db)
            audio_bytes = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"],
                                           tts_cfg["model"], tts_cfg["voice"],
                                           runner.reply_text, tts_cfg["speed"])
            rel = f"audio/{uuid.uuid4().hex}.mp3"
            with open(f"{request.app.state.data_dir}/{rel}", "wb") as f:
                f.write(audio_bytes)
            turn = db.get(Turn, runner.turn_id)
            turn.reply_audio_path = rel
            db.commit()
            audio_url = f"/api/files/{rel}"
        except (ConfigError, UpstreamError):
            audio_url = ""  # TTS 失败不阻塞文字回复

    return {"turn_id": runner.turn_id, "transcript": runner.transcript,
            "reply_text": runner.reply_text, "audio_url": audio_url}
