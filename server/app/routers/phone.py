import json
import uuid

from fastapi import APIRouter, Depends, Form, HTTPException, Request, UploadFile
from sqlalchemy.orm import Session

from app.db import get_db
from app.engine.errors import ConfigError
from app.engine.lesson import (
    active_current_lesson,
    attach_artifacts,
    close_run_malformed,
    close_run_with_report,
    latest_recap,
    render_lesson_script,
    run_has_drawing,
)
from app.engine.tts import synthesize
from app.engine.turn import TurnInput, TurnRunner
from app.engine.upstream import UpstreamError
from app.models import Lesson, LessonRun, Turn, utcnow
from app.routers.admin_voice import load_voice_config

router = APIRouter(prefix="/api/phone")


# 保留旧名以兼容既有调用点/引用；实现已提到 app.engine.lesson 作公共 helper。
_active_current_lesson = active_current_lesson


@router.get("/current-lesson")
def current_lesson(db: Session = Depends(get_db)):
    found = _active_current_lesson(db)
    if found is None:
        return {"available": False}
    cur, lesson = found
    return {"available": True, "curriculum_title": cur.title, "lesson_id": lesson.id,
            "lesson_seq": lesson.seq, "lesson_title": lesson.title}


@router.post("/lesson-runs")
async def start_lesson_run(request: Request):
    with request.app.state.sessionmaker() as db:  # type: Session
        found = _active_current_lesson(db)
        if found is None:
            raise HTTPException(400, "请先在 DouDou 后台设置生效课程与当前课时")
        _, lesson = found
        for stale in db.query(LessonRun).filter(LessonRun.status == "running").all():
            stale.status = "abandoned"
            stale.ended_at = utcnow()
            # 遗留 running 仅置 abandoned、不挂靠作品：跨天的失联窗口会误挂无关涂鸦
        run = LessonRun(lesson_id=lesson.id)
        db.add(run)
        db.commit()
        run_id, seq, title = run.id, lesson.seq, lesson.title

    # 豆豆开课先开口：固定暖句 + TTS（不走 LLM，快且稳，无 599 风险）。
    # TTS 失败不阻塞开课——greeting_audio_url 留空，前端仍显示文字开场。
    greeting = f"嗨，我是豆豆！今天我们一起玩《{title}》，想画什么就画什么，好不好呀？"
    greeting_url = ""
    with request.app.state.sessionmaker() as db:  # type: Session
        try:
            _, tts_cfg = load_voice_config(db)
            audio_bytes = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"],
                                           tts_cfg["model"], tts_cfg["voice"],
                                           greeting, tts_cfg["speed"])
            rel = f"audio/{uuid.uuid4().hex}.mp3"
            with open(f"{request.app.state.data_dir}/{rel}", "wb") as f:
                f.write(audio_bytes)
            greeting_url = f"/api/files/{rel}"
        except Exception as e:
            # 开场白 TTS 失败绝不阻塞开课：留空 url，前端仍显示文字开场。
            print(f"phone: greeting TTS failed, starting lesson without audio: {e}")
            greeting_url = ""
    return {"lesson_run_id": run_id, "lesson_seq": seq, "lesson_title": title,
            "greeting_text": greeting, "greeting_audio_url": greeting_url}


@router.post("/lesson-runs/{run_id}/end")
def end_lesson_run(run_id: int, db: Session = Depends(get_db)):
    run = db.get(LessonRun, run_id)
    if run is None:
        raise HTTPException(404, "上课记录不存在")
    if run.status == "running":
        run.status = "abandoned"
        run.ended_at = utcnow()
        attach_artifacts(db, run)
        db.commit()
    return {"ok": True, "status": run.status}


@router.get("/next")
def phone_next(db: Session = Depends(get_db)):
    """手机轮询：取当前房间待播报的 DouDou 话（取用即清）。无 running run 或
    无待播报 → null。"""
    run = (db.query(LessonRun).filter(LessonRun.status == "running")
           .order_by(LessonRun.id.desc()).first())
    if run is None or not run.pending_utterance:
        return {"utterance": None}
    u = run.pending_utterance
    run.pending_utterance = None
    db.commit()
    return {"utterance": u}


@router.post("/clear-board")
def clear_board(db: Session = Depends(get_db)):
    """手机「清空画板」按钮：给当前房间挂 clear 命令，平板轮询到即清屏。"""
    run = (db.query(LessonRun).filter(LessonRun.status == "running")
           .order_by(LessonRun.id.desc()).first())
    if run is None:
        return {"ok": False, "reason": "no_running_lesson"}
    run.pending_command = "clear"
    db.commit()
    return {"ok": True}


@router.post("/voice-turn")
async def voice_turn(
    request: Request,
    audio: UploadFile,
    history: str = Form("[]"),
    lesson_run_id: int | None = Form(None),
):
    try:
        pairs = json.loads(history)  # [["user","assistant"], ...]
        msgs: list[dict] = []
        for u, a in pairs:
            msgs.append({"role": "user", "content": str(u)})
            msgs.append({"role": "assistant", "content": str(a)})
    except (ValueError, TypeError) as e:
        raise HTTPException(400, f"history 参数格式错误：{e}")

    # 课程模式：run 有效且 running 时注入课时脚本（短事务，取完即关）
    lesson_context = ""
    active_run_id: int | None = None
    if lesson_run_id is not None:
        with request.app.state.sessionmaker() as db:  # type: Session
            run = db.get(LessonRun, lesson_run_id)
            if run is not None and run.status == "running":
                lesson = db.get(Lesson, run.lesson_id)
                if lesson is not None:
                    recap = latest_recap(db, lesson.curriculum_id)
                    lesson_context = render_lesson_script(lesson.script_text, recap)
                    active_run_id = run.id

    # 让语音豆豆"看见"孩子此刻在平板上画的整页（模型有视觉）：只取【本房间】
    # （当前 lesson_run）内的平板画，避免上一节课的脏数据串进来。新课刚开、
    # 还没在平板画东西时，active_run_id 下没有图 → 不送图，豆豆不会瞎说旧画。
    tablet_image: bytes | None = None
    if active_run_id is not None:
        with request.app.state.sessionmaker() as db:  # type: Session
            row = (db.query(Turn)
                   .filter(Turn.source == "tablet", Turn.lesson_run_id == active_run_id,
                           Turn.input_image_path.isnot(None))
                   .order_by(Turn.id.desc()).first())
            if row is not None and row.input_image_path:
                try:
                    with open(f"{request.app.state.data_dir}/{row.input_image_path}", "rb") as f:
                        tablet_image = f.read()
                except OSError:
                    tablet_image = None
    draw_note = (
        "\n\n（下面附了孩子此刻在平板上画的整页。请结合 ta 画的内容和 ta 说的话来回应，"
        "像真的看着 ta 的画一样，可以夸具体的形状/线条/数量。）"
        if tablet_image is not None else ""
    )

    data = await audio.read()
    tin = TurnInput(source="phone", audio=data,
                    audio_filename=audio.filename or "audio.webm",
                    history=msgs, use_voice_hint=True,
                    lesson_context=lesson_context, lesson_run_id=active_run_id,
                    image_png=tablet_image, device_protocol_suffix=draw_note)
    runner = TurnRunner(request.app.state.sessionmaker, request.app.state.data_dir, tin)
    try:
        async for _ in runner.stream():
            pass
    except ConfigError as e:
        raise HTTPException(400, e.message)
    except UpstreamError as e:
        raise HTTPException(502, f"模型服务出错（{e.status_code}）")

    # 打标剥离已在引擎内完成：runner.reply_text 落库前即已干净
    clean_text = runner.reply_text  # 引擎已剥离 ⟦lesson_report⟧
    report = runner.lesson_report
    raw = runner.lesson_report_raw
    lesson_report_out = None

    with request.app.state.sessionmaker() as db:  # type: Session
        if active_run_id is not None:
            run = db.get(LessonRun, active_run_id)
            if run is not None and run.status == "running":
                if runner.demo_shape:
                    done = list(run.demoed_shapes or [])
                    if runner.demo_shape not in done:
                        run.pending_demo = runner.demo_shape
                        run.demoed_shapes = done + [runner.demo_shape]
                        db.commit()
                # 未开画不关课：孩子还没在平板上画东西前，模型的收尾/打标一律
                # 忽略，房间继续 running。守住房间，避免刚打招呼就被判未参与关课
                # →房间死→语音豆豆取不到本房间图开始瞎编画。打标行已在引擎剥离、
                # 不外显；仅在孩子已开画后才认收尾。
                if run_has_drawing(db, run):
                    if report is not None:
                        close_run_with_report(db, run, report, raw)
                        lesson_report_out = {"status": run.status, "highlights": run.highlights,
                                             "parent_tip": run.parent_tip}
                    elif raw:
                        close_run_malformed(db, run, raw)  # 坏 JSON 兜底：保留原文、按未收尾关闭
        try:
            _, tts_cfg = load_voice_config(db)
            audio_bytes = await synthesize(tts_cfg["base_url"], tts_cfg["api_key"],
                                           tts_cfg["model"], tts_cfg["voice"],
                                           clean_text, tts_cfg["speed"])
            rel = f"audio/{uuid.uuid4().hex}.mp3"
            with open(f"{request.app.state.data_dir}/{rel}", "wb") as f:
                f.write(audio_bytes)
            turn = db.get(Turn, runner.turn_id)
            turn.reply_audio_path = rel
            db.commit()
            audio_url = f"/api/files/{rel}"
        except (ConfigError, UpstreamError):
            audio_url = ""  # TTS 失败不阻塞文字回复

    return {"turn_id": runner.turn_id, "transcript": runner.transcript,
            "reply_text": clean_text, "audio_url": audio_url,
            "lesson_report": lesson_report_out}
