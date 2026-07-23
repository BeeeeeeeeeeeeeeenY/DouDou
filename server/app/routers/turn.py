import base64

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from app.engine import cards as cards_engine
from app.engine.errors import ConfigError
from app.engine.lesson import active_current_lesson, latest_recap, render_lesson_script
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError
from app.models import Turn

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

    lesson_context = ""
    recent_replies: list[str] = []
    recent_voice: list[str] = []
    with request.app.state.sessionmaker() as db:
        found = active_current_lesson(db)
        if found is not None:
            _curriculum, lesson = found
            lesson_context = render_lesson_script(
                lesson.script_text, latest_recap(db, lesson.curriculum_id))
        # 最近几轮 tablet 回复：注入"别重复"上下文（/turn 本身无状态，
        # 否则模型看不到自己已经说过/画过什么，会一直重复同一主题）。
        rows = (db.query(Turn).filter(Turn.source == "tablet")
                .order_by(Turn.id.desc()).limit(4).all())
        for r in reversed(rows):
            cj = r.cards_json if isinstance(r.cards_json, dict) else {}
            sp = cj.get("spoken_text")
            if sp:
                recent_replies.append(sp)
        # 最近的语音对话：让平板豆豆知道手机上正在聊什么（一个豆豆，画面呼应对话）。
        prows = (db.query(Turn).filter(Turn.source == "phone", Turn.transcript.isnot(None))
                 .order_by(Turn.id.desc()).limit(3).all())
        for r in reversed(prows):
            if r.transcript:
                recent_voice.append(f"孩子说「{r.transcript}」，你回「{(r.reply_text or '')[:40]}」")

    user_text = TURN_USER_TEXT
    if recent_voice:
        user_text += (f"\n（孩子刚才和你在语音里聊到：{'；'.join(recent_voice)}。"
                      "你的纸面回应可以呼应这段对话。）")
    if recent_replies:
        joined = "；".join(recent_replies)
        user_text += (f"\n（你最近几轮已经这样回应过：{joined}。这次务必换新说法、"
                      "新主题、新图案，绝不重复上面说过或画过的内容。）")

    tin = TurnInput(
        source="tablet",
        text=user_text,
        image_png=image_png,
        device_protocol_suffix=cards_engine.CARD_PROTOCOL,
        lesson_context=lesson_context,
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
    # 页面写满就换新页：/turn 收到设备算好的 ink_coverage，超阈值时强制
    # new_page（设备端把 new_page 与本地 page-full 合并处理）。模型自己几乎
    # 从不发换页信号，靠服务器兜这一手。
    if req.page_state.ink_coverage >= 0.55:
        page_action = "new_page"
    resp = _response(req.turn_id, spoken, cards, page_action, tags)
    if runner.turn_id is not None:
        with request.app.state.sessionmaker() as db:
            t = db.get(Turn, runner.turn_id)
            if t is not None:
                t.cards_json = {"spoken_text": spoken, "paper_cards": cards,
                                "page_action": page_action, "memory_tags": tags}
                db.commit()
    return resp
