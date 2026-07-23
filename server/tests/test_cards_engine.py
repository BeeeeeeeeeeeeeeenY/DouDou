import json

from app.engine.cards import (
    CARD_PROTOCOL,
    build_cards,
    extract_json_object,
    text_cap,
)


def test_text_cap_by_profile():
    assert text_cap("child_3_4") == 6
    assert text_cap("unknown") == 6


def test_extract_json_from_fenced_reply():
    reply = '好的\n```json\n{"spoken_text":"哇","paper_cards":[]}\n```\n'
    obj = extract_json_object(reply)
    assert obj["spoken_text"] == "哇"


def test_build_cards_full_and_truncates_to_three():
    reply = json.dumps({
        "spoken_text": "哇，三颗星！",
        "paper_cards": [
            {"type": "stamp", "name": "star", "count": 3, "place": "near_new_ink", "size": "S"},
            {"type": "text", "content": "你画得真好看极了", "place": "blank_area", "size": "L"},
            {"type": "image", "url": "/api/files/lesson-art/sun.png", "size": "L"},
            {"type": "text", "content": "多余", "size": "S"},
        ],
        "page_action": "none",
        "memory_tags": ["sun"],
    }, ensure_ascii=False)
    spoken, cards, page_action, tags = build_cards(reply, "child_3_4")
    assert spoken == "哇，三颗星！"
    assert len(cards) == 3            # 第 4 张被 ≤3 丢弃
    assert cards[0]["type"] == "stamp" and cards[0]["count"] == 3
    assert cards[1]["type"] == "text" and cards[1]["content"] == "你画得真好看"  # 6 字截断
    assert cards[2]["type"] == "image" and cards[2]["url"].endswith("sun.png")
    assert tags == ["sun"]


def test_build_cards_drops_unknown_stamp_and_clamps_count():
    reply = json.dumps({
        "spoken_text": "",
        "paper_cards": [
            {"type": "stamp", "name": "dragon", "count": 2},
            {"type": "stamp", "name": "heart", "count": 99},
        ],
    }, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1
    assert cards[0]["name"] == "heart" and cards[0]["count"] == 5  # 未知 stamp 丢弃；count 夹到 5


def test_build_cards_at_most_one_image():
    reply = json.dumps({
        "spoken_text": "",
        "paper_cards": [
            {"type": "image", "url": "a.png"},
            {"type": "image", "url": "b.png"},
        ],
    }, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1 and cards[0]["url"] == "a.png"


def test_build_cards_drops_image_without_url():
    reply = json.dumps({"spoken_text": "", "paper_cards": [{"type": "image"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert cards == []


def test_build_cards_degrades_non_json_to_single_text_card():
    spoken, cards, page_action, tags = build_cards("太阳会发光是因为核聚变呀真棒", "child_3_4")
    assert len(cards) == 1 and cards[0]["type"] == "text"
    assert cards[0]["content"] == "太阳会发光是"   # 原文 6 字截断
    assert spoken == "太阳会发光是因为核聚变呀真棒"  # 语音仍念完整原文
    assert page_action == "none"


def test_card_protocol_mentions_json_and_stamp_names():
    assert "JSON" in CARD_PROTOCOL or "json" in CARD_PROTOCOL
    assert "star" in CARD_PROTOCOL
