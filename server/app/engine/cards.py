"""模型回复 → 校验后的 paper_cards。服务器信内容不信形状：任何畸形都降级成
「更少的卡」而非报错，与设备 cards.rs 的容错策略一致。"""

import json

from app.engine.art import IMAGE_SUBJECTS, subject_data_b64

STAMP_NAMES = ("star", "flower", "heart", "smiley", "check", "sun", "moon", "balloon")
MAX_CARDS = 3
MAX_STAMP_COUNT = 5
PROFILE_TEXT_CAPS = {"child_3_4": 6}
DEFAULT_TEXT_CAP = 6

CARD_PROTOCOL = (
    "\n\n【纸面卡片协议】你正在通过 DouDou 平板回应孩子。只输出一个 JSON 对象，"
    "不要任何多余文字、不要 Markdown 围栏：\n"
    '{"spoken_text":"给孩子/家长听的一句话","paper_cards":[卡片...],"page_action":"none"}\n'
    "卡片类型：\n"
    '- {"type":"text","content":"手写短句，最多6个字","place":"blank_area","size":"L"}\n'
    '- {"type":"stamp","name":"star|flower|heart|smiley|check|sun|moon|balloon",'
    '"count":1,"place":"near_new_ink","size":"S"}\n'
    "- 特别时刻（孩子画完、值得庆祝）可以放最多 1 张彩图卡：\n"
    '  {"type":"image","subject":"' + "|".join(IMAGE_SUBJECTS) + '",'
    '"place":"blank_area","size":"l"}\n'
    "  只在合适时用，别每回合都出；subject 必须来自上面这些词，别用词表外的（会被丢弃）。\n"
    "规则：最多 3 张卡；text 的 content 不超过 6 个字；stamp 的 name 必须来自上面 8 个；"
    "spoken_text 与卡片讲同一件事；不要输出表情符号。"
)


def text_cap(profile: str) -> int:
    return PROFILE_TEXT_CAPS.get(profile, DEFAULT_TEXT_CAP)


def _truncate(s: str, cap: int) -> str:
    return s[:cap]  # str 切片按 Unicode 标量，中文安全


def extract_json_object(text: str) -> dict | None:
    """抠出回复里第一个平衡的 JSON 对象（容忍 ```json 围栏与前后杂散文字）。"""
    start = text.find("{")
    if start == -1:
        return None
    depth = 0
    in_str = False
    esc = False
    for i in range(start, len(text)):
        c = text[i]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            continue
        if c == '"':
            in_str = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                try:
                    obj = json.loads(text[start:i + 1])
                    return obj if isinstance(obj, dict) else None
                except json.JSONDecodeError:
                    return None
    return None


def _clean_card(raw: dict, cap: int) -> dict | None:
    if not isinstance(raw, dict):
        return None
    ctype = str(raw.get("type", "")).lower()
    common = {k: raw[k] for k in ("place", "anchor_norm", "size", "pace") if k in raw}
    if ctype == "text":
        content = raw.get("content")
        if not isinstance(content, str) or not content:
            return None
        return {"type": "text", "content": _truncate(content, cap), **common}
    if ctype == "stamp":
        name = raw.get("name")
        if name not in STAMP_NAMES:
            return None
        count = raw.get("count", 1)
        count = count if isinstance(count, int) else 1
        return {"type": "stamp", "name": name, "count": max(1, min(count, MAX_STAMP_COUNT)), **common}
    if ctype == "image":
        subject = raw.get("subject")
        if subject not in IMAGE_SUBJECTS:
            return None
        data = subject_data_b64(subject)
        if data is None:
            return None
        return {"type": "image", "data": data, **common}
    return None  # 未知/本期不支持的类型（sketch/count/trace/page）：丢弃


def build_cards(reply_text: str, profile: str) -> tuple[str, list[dict], str, list[str]]:
    """返回 (spoken_text, paper_cards, page_action, memory_tags)。"""
    cap = text_cap(profile)
    obj = extract_json_object(reply_text)
    if obj is None:
        # 降级：模型没按协议出 JSON。语音念完整原文，纸面给一张截断 text 卡。
        return reply_text, [{"type": "text", "content": _truncate(reply_text, cap),
                             "place": "blank_area", "size": "L"}], "none", []

    spoken = str(obj.get("spoken_text", "") or "")
    raw_cards = obj.get("paper_cards")
    raw_cards = raw_cards if isinstance(raw_cards, list) else []
    cards: list[dict] = []
    seen_image = False
    for rc in raw_cards:
        card = _clean_card(rc, cap)
        if card is None:
            continue
        if card["type"] == "image":
            if seen_image:
                continue          # 每回合 ≤1 张 image
            seen_image = True
        cards.append(card)
        if len(cards) >= MAX_CARDS:
            break

    page_action = str(obj.get("page_action", "none") or "none")
    tags = obj.get("memory_tags")
    tags = [str(t) for t in tags] if isinstance(tags, list) else []
    return spoken, cards, page_action, tags
