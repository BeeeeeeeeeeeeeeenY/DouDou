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
            {"type": "image", "subject": "sun", "size": "L"},
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
    assert cards[2]["type"] == "image" and "data" in cards[2] and "url" not in cards[2]
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
            {"type": "image", "subject": "circle"},
            {"type": "image", "subject": "star"},
        ],
    }, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1 and cards[0]["type"] == "image" and "data" in cards[0]


def test_build_cards_drops_image_without_subject():
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


def test_image_card_subject_becomes_inline_data():
    import base64
    reply = json.dumps({"spoken_text": "画好啦", "paper_cards": [
        {"type": "image", "subject": "circle", "size": "l"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1
    c = cards[0]
    assert c["type"] == "image" and "url" not in c
    assert base64.b64decode(c["data"])[:4] == b"\x89PNG"


def test_image_card_unknown_subject_dropped():
    reply = json.dumps({"spoken_text": "", "paper_cards": [
        {"type": "image", "subject": "dragon"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert cards == []


def test_at_most_one_image_still_holds_with_subjects():
    reply = json.dumps({"spoken_text": "", "paper_cards": [
        {"type": "image", "subject": "circle"},
        {"type": "image", "subject": "star"}]}, ensure_ascii=False)
    _, cards, _, _ = build_cards(reply, "child_3_4")
    assert len(cards) == 1


def test_card_protocol_mentions_image_subjects():
    assert "image" in CARD_PROTOCOL
    assert "circle" in CARD_PROTOCOL
