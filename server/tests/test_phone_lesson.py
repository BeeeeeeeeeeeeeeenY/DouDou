import json

import httpx
import respx

from app import models


def sse_reply(text: str) -> str:
    delta = json.dumps({"choices": [{"delta": {"content": text}}]}, ensure_ascii=False)
    return f"data: {delta}\n\ndata: [DONE]\n\n"


REPORT = ('⟦lesson_report⟧{"lesson_id":"shapes-01-01","status":"completed",'
          '"highlights":"敢下笔了","parent_tip":"在家一起涂鸦"}')


def setup_course(client):
    p = client.post("/api/admin/providers",
                    json={"name": "up", "base_url": "https://up.test/v1", "api_key": "sk"}).json()
    prof = client.post("/api/admin/profiles", json={
        "name": "生效", "persona_text": "你是 DouDou。", "voice_hint": "口语化",
        "provider_id": p["id"], "model": "m",
    }).json()
    client.post(f"/api/admin/profiles/{prof['id']}/activate")
    client.put("/api/admin/voice-settings", json={
        "stt_provider_id": p["id"], "stt_model": "whisper-1",
        "tts_provider_id": p["id"], "tts_model": "tts-1", "tts_voice": "alloy",
    })
    c = client.post("/api/admin/curricula/seed-shapes01").json()
    client.post(f"/api/admin/curricula/{c['id']}/activate")
    return c


def test_current_lesson_unavailable_by_default(client):
    assert client.get("/api/phone/current-lesson").json() == {"available": False}


def test_current_lesson_and_run_creation(client):
    setup_course(client)
    j = client.get("/api/phone/current-lesson").json()
    assert j["available"] is True and j["lesson_seq"] == 1
    assert j["curriculum_title"] == "形状小画家"

    r = client.post("/api/phone/lesson-runs").json()
    assert r["lesson_run_id"] > 0 and r["lesson_title"] == "认识 DouDou·想画就画"


def test_run_creation_without_course_400(client):
    r = client.post("/api/phone/lesson-runs")
    assert r.status_code == 400 and "课程" in r.json()["detail"]


def test_stale_running_swept_on_new_run(client, db):
    setup_course(client)
    a = client.post("/api/phone/lesson-runs").json()
    b = client.post("/api/phone/lesson-runs").json()
    assert db.get(models.LessonRun, a["lesson_run_id"]).status == "abandoned"
    assert db.get(models.LessonRun, b["lesson_run_id"]).status == "running"


def test_end_endpoint_abandons_running(client, db):
    setup_course(client)
    r = client.post("/api/phone/lesson-runs").json()
    j = client.post(f"/api/phone/lesson-runs/{r['lesson_run_id']}/end").json()
    assert j == {"ok": True, "status": "abandoned"}
    # 幂等：再次 end 不改状态
    j2 = client.post(f"/api/phone/lesson-runs/{r['lesson_run_id']}/end").json()
    assert j2["status"] == "abandoned"


@respx.mock
def test_voice_turn_with_lesson_full_loop(client, db):
    c = setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]

    # 课中一轮平板提交（作品）
    tablet = models.Turn(source="tablet", input_image_path="images/draw.png")
    db.add(tablet)
    db.commit()

    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "我画完啦"})
    )
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("太棒啦，下次见！\n" + REPORT))
    )
    speech = respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    r = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"AUDIO", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    )
    assert r.status_code == 200
    j = r.json()
    # 打标行绝不外泄
    assert j["reply_text"] == "太棒啦，下次见！"
    assert j["lesson_report"] == {"status": "completed", "highlights": "敢下笔了",
                                  "parent_tip": "在家一起涂鸦"}
    # system prompt 注入了课时脚本
    sent = json.loads(chat.calls[0].request.content)
    assert "第 1 课" in sent["messages"][0]["content"]
    # TTS 收到的是干净文本
    tts_body = json.loads(speech.calls[0].request.content)
    assert "lesson_report" not in tts_body["input"]
    # run 关闭、作品挂靠、指针推进
    run = db.get(models.LessonRun, run_id)
    assert run.status == "completed" and run.artifact_turn_ids == [tablet.id]
    cur = db.query(models.Curriculum).filter(models.Curriculum.slug == "shapes-01").one()
    lesson2 = db.query(models.Lesson).filter(
        models.Lesson.curriculum_id == cur.id, models.Lesson.seq == 2).one()
    assert cur.current_lesson_id == lesson2.id
    # 落库 turn 干净且归属本课
    turn = db.query(models.Turn).filter(models.Turn.source == "phone").one()
    assert turn.lesson_run_id == run_id and "lesson_report" not in turn.reply_text


@respx.mock
def test_voice_turn_sets_pending_demo(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "该画了"}))
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("我们先画一个圆圆的\n⟦demo:circle⟧")))
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3"))
    j = client.post("/api/phone/voice-turn",
                    files={"audio": ("a.webm", b"x", "audio/webm")},
                    data={"history": "[]", "lesson_run_id": str(run_id)}).json()
    assert "demo" not in j["reply_text"]                 # 标记不外泄
    assert db.get(models.LessonRun, run_id).pending_demo == "circle"
    assert db.get(models.LessonRun, run_id).status == "running"  # demo 不关课


@respx.mock
def test_voice_turn_report_before_any_drawing_keeps_run_running(client, db):
    """孩子还没在平板上开画，模型就发收尾打标——不能关课。守住房间，
    治「刚打招呼说句『好』就被判未参与关课→房间死→语音豆豆瞎编画」。"""
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "好"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("那今天就到这里啦！\n" + REPORT))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    # 打标行照常剥离、不外显；但因未开画，报告一律忽略、不外传
    assert "lesson_report" not in j["reply_text"]
    assert j["lesson_report"] is None
    # 关键：房间仍 running，指针没被推进
    assert db.get(models.LessonRun, run_id).status == "running"


@respx.mock
def test_voice_turn_malformed_report_before_drawing_keeps_running(client, db):
    """未开画时的坏 JSON 打标同样不关课（不 abandon 空房间）。"""
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "好"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("收尾啦 ⟦lesson_report⟧{oops"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["reply_text"] == "收尾啦" and j["lesson_report"] is None
    assert db.get(models.LessonRun, run_id).status == "running"


@respx.mock
def test_voice_turn_mid_lesson_no_report(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "画好了三个泡泡"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("三个泡泡真圆呀"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["lesson_report"] is None
    assert db.get(models.LessonRun, run_id).status == "running"


@respx.mock
def test_voice_turn_with_closed_run_falls_back_to_chat(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    client.post(f"/api/phone/lesson-runs/{run_id}/end")
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "随便聊聊"})
    )
    chat = respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("好呀"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["lesson_report"] is None
    sent = json.loads(chat.calls[0].request.content)
    assert "今天的课" not in sent["messages"][0]["content"]  # 不注入课时脚本


@respx.mock
def test_voice_turn_malformed_report_abandons_run(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    # 已开画（本房间有平板作品）后模型才收尾，坏 JSON 走 abandon 兜底
    db.add(models.Turn(source="tablet", input_image_path="images/d.png",
                       lesson_run_id=run_id))
    db.commit()
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "上完啦"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("收尾啦 ⟦lesson_report⟧{oops"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )
    j = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)},
    ).json()
    assert j["reply_text"] == "收尾啦" and j["lesson_report"] is None
    run = db.get(models.LessonRun, run_id)
    assert run.status == "abandoned"
    assert run.raw_report == {"_raw": "{oops"}


def test_end_run_attaches_artifacts(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    tablet = models.Turn(source="tablet", input_image_path="images/w.png")
    db.add(tablet)
    db.commit()
    client.post(f"/api/phone/lesson-runs/{run_id}/end")
    db.expire_all()
    run = db.get(models.LessonRun, run_id)
    assert run.status == "abandoned"
    assert run.artifact_turn_ids == [tablet.id]
    assert db.get(models.Turn, tablet.id).lesson_run_id == run_id


def test_clear_board_sets_pending_command(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    j = client.post("/api/phone/clear-board").json()
    assert j["ok"] is True
    assert db.get(models.LessonRun, run_id).pending_command == "clear"


def test_clear_board_without_running_lesson(client, db):
    j = client.post("/api/phone/clear-board").json()
    assert j["ok"] is False


@respx.mock
def test_demo_fires_once_per_run(client, db):
    """演示形状每 run 仅触发一次。连发两条都带 ⟦demo:circle⟧ 的 voice-turn；
    第一条后 pending_demo=='circle' 且 demoed_shapes==['circle']；
    取用清 pending_demo（模拟平板取走）后发第二条 → pending_demo 仍为 None，
    demoed_shapes 仍 ['circle']。"""
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]

    # Mock API responses
    respx.post("https://up.test/v1/audio/transcriptions").mock(
        return_value=httpx.Response(200, json={"text": "该画了"})
    )
    respx.post("https://up.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=sse_reply("我们先画一个圆圆的\n⟦demo:circle⟧"))
    )
    respx.post("https://up.test/v1/audio/speech").mock(
        return_value=httpx.Response(200, content=b"MP3")
    )

    # 第一条 voice-turn：带 demo:circle 标记
    j1 = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("a.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)}
    ).json()

    # 第一条后：pending_demo 已设，demoed_shapes 已记
    run1 = db.get(models.LessonRun, run_id)
    assert run1.pending_demo == "circle", f"After first turn, pending_demo should be 'circle', got {run1.pending_demo}"
    assert run1.demoed_shapes == ["circle"], f"After first turn, demoed_shapes should be ['circle'], got {run1.demoed_shapes}"

    # 模拟平板取走 pending_demo：清空挂靠，但 demoed_shapes 保留
    run1.pending_demo = None
    db.commit()

    # 验证清理后的状态
    run_after_clear = db.get(models.LessonRun, run_id)
    assert run_after_clear.pending_demo is None
    assert run_after_clear.demoed_shapes == ["circle"]

    # 第二条 voice-turn：仍带 demo:circle 标记，但不应重复设置
    j2 = client.post(
        "/api/phone/voice-turn",
        files={"audio": ("b.webm", b"x", "audio/webm")},
        data={"history": "[]", "lesson_run_id": str(run_id)}
    ).json()

    # 第二条后：pending_demo 应保持 None（未再设），demoed_shapes 保持不变
    run2 = db.get(models.LessonRun, run_id)
    assert run2.pending_demo is None, f"After second turn, pending_demo should remain None, got {run2.pending_demo}"
    assert run2.demoed_shapes == ["circle"], f"After second turn, demoed_shapes should remain ['circle'], got {run2.demoed_shapes}"


def test_phone_next_serves_and_clears(client, db):
    setup_course(client)
    run_id = client.post("/api/phone/lesson-runs").json()["lesson_run_id"]
    run = db.get(models.LessonRun, run_id)
    run.pending_utterance = {"text": "下一步", "audio_url": "/x.mp3"}
    db.commit()

    j = client.get("/api/phone/next").json()
    assert j["utterance"] == {"text": "下一步", "audio_url": "/x.mp3"}

    j2 = client.get("/api/phone/next").json()
    assert j2["utterance"] is None

    db.refresh(run)
    assert run.pending_utterance is None


def test_phone_next_no_running_run_is_empty(client, db):
    assert client.get("/api/phone/next").json() == {"utterance": None}
