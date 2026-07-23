from app.engine import art


def test_all_subjects_load_as_png():
    assert len(art.IMAGE_SUBJECTS) == 16  # + balloon（蓝色气球，双通道重构）
    for s in art.IMAGE_SUBJECTS:
        b = art.load_art_png(s)
        assert b is not None and b[:4] == b"\x89PNG", f"{s} not a PNG"


def test_unknown_subject_is_none():
    assert art.load_art_png("dragon") is None
    assert art.subject_data_b64("dragon") is None


def test_subject_data_b64_roundtrips_to_png():
    import base64
    b64 = art.subject_data_b64("circle")
    assert b64 and base64.b64decode(b64)[:4] == b"\x89PNG"
