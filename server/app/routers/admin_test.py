import base64
import json

from fastapi import APIRouter, Request
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

from app.engine.errors import ConfigError
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

router = APIRouter(prefix="/api/admin")


class TestTurnIn(BaseModel):
    text: str = ""
    image_base64: str | None = None
    history: list[list[str]] = []
    voice_mode: bool = False


@router.post("/test-turn")
async def test_turn(body: TestTurnIn, request: Request):
    msgs: list[dict] = []
    for u, a in body.history:
        msgs.append({"role": "user", "content": u})
        msgs.append({"role": "assistant", "content": a})
    tin = TurnInput(
        source="test", text=body.text,
        image_png=base64.b64decode(body.image_base64) if body.image_base64 else None,
        history=msgs, use_voice_hint=body.voice_mode,
    )
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)

    async def sse():
        def ev(obj: dict) -> str:
            return f"data: {json.dumps(obj, ensure_ascii=False)}\n\n"
        try:
            async for delta in runner.stream():
                yield ev({"delta": delta})
            yield ev({"done": True, "turn_id": runner.turn_id,
                      "transcript": runner.transcript, "system_prompt": runner.system_prompt})
        except ConfigError as e:
            yield ev({"error": e.message})
        except UpstreamError as e:
            yield ev({"error": f"模型服务出错（{e.status_code}）：{e.detail[:200]}"})

    return StreamingResponse(sse(), media_type="text/event-stream")
