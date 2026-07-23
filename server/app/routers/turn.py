from fastapi import APIRouter, Request
from pydantic import BaseModel, Field

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


@router.post("/turn")
async def turn(req: TurnRequest, request: Request):
    # 骨架：形状先锁定，模型调用在 Task 3 接线。
    return _response(req.turn_id, "", [])
