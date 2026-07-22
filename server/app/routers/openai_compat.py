import base64
import json

from fastapi import APIRouter, Request
from fastapi.responses import PlainTextResponse, StreamingResponse

from app.engine.errors import ConfigError
from app.engine.prompt import split_protocol_suffix
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

router = APIRouter()


def _chunk(delta: str) -> str:
    payload = {"choices": [{"delta": {"content": delta}, "index": 0, "finish_reason": None}]}
    return f"data: {json.dumps(payload, ensure_ascii=False, separators=(',', ':'))}\n\n"


def parse_riddle_body(body: dict) -> TurnInput:
    messages: list[dict] = body.get("messages", [])
    protocol_suffix = ""
    history: list[dict] = []
    text, image_png = "", None

    if messages and messages[0].get("role") == "system":
        _, protocol_suffix = split_protocol_suffix(str(messages[0].get("content", "")))
        middle = messages[1:-1]
    else:
        middle = messages[:-1]
    history = middle

    if messages:
        content = messages[-1].get("content", "")
        if isinstance(content, str):
            text = content
        else:  # [{type:text},{type:image_url}]
            for part in content:
                if part.get("type") == "text":
                    text = part.get("text", "")
                elif part.get("type") == "image_url":
                    url = part.get("image_url", {}).get("url", "")
                    if url.startswith("data:image/png;base64,"):
                        image_png = base64.b64decode(url.split(",", 1)[1])
    return TurnInput(source="tablet", text=text, image_png=image_png,
                     history=history, device_protocol_suffix=protocol_suffix)


@router.post("/v1/chat/completions")
async def chat_completions(request: Request):
    tin = parse_riddle_body(await request.json())
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    agen = runner.stream()
    try:
        first = await anext(agen)
    except StopAsyncIteration:
        first = None
    except ConfigError as e:
        return PlainTextResponse(e.message, status_code=400)
    except UpstreamError as e:
        return PlainTextResponse(f"模型服务出错（{e.status_code}），请在后台检查配置", status_code=502)

    async def sse():
        if first is not None:
            yield _chunk(first)
        try:
            async for delta in agen:
                yield _chunk(delta)
        except UpstreamError:
            pass  # 中途断流：结束响应，riddle 端有读超时兜底
        payload = {"choices": [{"delta": {}, "index": 0, "finish_reason": "stop"}]}
        yield f"data: {json.dumps(payload, separators=(',', ':'))}\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(sse(), media_type="text/event-stream")
