from app.engine.transcript import split_transcript


def test_split_transcript():
    visible, transcript = split_transcript("答案是三。\n⁂静夜思 三")
    assert visible == "答案是三。"
    assert transcript == "静夜思 三"


def test_no_transcript_marker():
    visible, transcript = split_transcript("普通回复")
    assert visible == "普通回复"
    assert transcript == ""
