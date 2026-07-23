import base64

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from app.engine import cards as cards_engine
from app.engine.errors import ConfigError
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError

router = APIRouter()


class PageState(BaseModel):
    ink_coverage: float = 0.0
    page_id: str = ""


class DeviceProfile(BaseModel):
    profile: str = "child_3_4"
    screen: list[int] = Field(default_factory=lambda: [1620, 2160])


class TurnRequest(BaseModel):
    turn_id: str = ""
    trigger: str = "pen_idle"
    page_png: str = ""            # base64 灰度整页
    new_strokes: list = Field(default_factory=list)
    page_state: PageState = Field(default_factory=PageState)
    device_profile: DeviceProfile = Field(default_factory=DeviceProfile)
    page_id: str = ""


def _response(turn_id: str, spoken_text: str, cards: list,
              page_action: str = "none", memory_tags: list | None = None) -> dict:
    return {
        "v": 1,
        "turn_id": turn_id,
        "spoken_text": spoken_text,
        "paper_cards": cards,
        "page_action": page_action,
        "memory_tags": memory_tags or [],
    }


TURN_USER_TEXT = "（这是孩子刚画的整页）请按纸面卡片协议回应。"


@router.post("/turn")
async def turn(req: TurnRequest, request: Request):
    image_png = None
    if req.page_png:
        try:
            image_png = base64.b64decode(req.page_png)
        except (ValueError, TypeError):
            image_png = None

    tin = TurnInput(
        source="tablet",
        text=TURN_USER_TEXT,
        image_png=image_png,
        device_protocol_suffix=cards_engine.CARD_PROTOCOL,
    )
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError:
        raise HTTPException(502, "模型服务出错，请在后台检查配置")

    spoken, cards, page_action, tags = cards_engine.build_cards(
        runner.reply_text, req.device_profile.profile)
    return _response(req.turn_id, spoken, cards, page_action, tags)
