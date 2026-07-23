# 双通道交互重构 实施计划（服务器 + 手机，不烧板）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`.

**Goal:** 手机=对话/平板=纯画布/画完自动联动/演示只一次/彩图偶尔。全靠服务器+手机前端，设备不改（不烧板）。

**Architecture:** 复用 `/turn/next`(平板轮询)对称地给手机加 `GET /api/phone/next`(手机轮询)。孩子两种输入(说话 voice-turn / 画画 /turn)都汇服务器一个大脑，DouDou 的话统一由手机语音说，平板只承载画面。

## Global Constraints
- 平板**禁 text 卡**；DouDou 的话（spoken_text）一律走手机语音，不落平板。
- 演示每形状每 run **只一次**（服务器 `demoed_shapes` 去重）。
- 彩图**节流**：每 run 至多每 3 次平板提交出 1 张，默认不出。
- 改课用 **PUT /lessons**，绝不 delete+re-seed（防 run-id 重用污染房间）。
- 测试：`cd server && uv run pytest`。房间=最近一个 `status=='running'` 的 LessonRun。

---

### Task 1: LessonRun 状态位 + 迁移
**Files:** `server/app/models.py`(LessonRun 末尾), `server/app/db.py`(_migrate), `server/tests/test_curriculum_models.py`
**Produces:** `LessonRun.demoed_shapes:list|None`, `pending_utterance:dict|None`, `tablet_turns:int`, `last_image_turn:int`

- [ ] Step1 失败测试：建 run，断言 demoed_shapes/pending_utterance 默认 None、tablet_turns/last_image_turn 默认 0，且可写读回（circle 入 demoed_shapes、{"text":"hi"} 入 pending_utterance、tablet_turns=2）。
- [ ] Step2 跑测试失败（AttributeError）。
- [ ] Step3 实现：models.py 加
```python
    demoed_shapes: Mapped[list | None] = mapped_column(JSON, nullable=True)       # 本 run 已演示过的形状，去重用
    pending_utterance: Mapped[dict | None] = mapped_column(JSON, nullable=True)    # 待手机播报 {text, audio_url}
    tablet_turns: Mapped[int] = mapped_column(default=0)                            # 本 run 平板提交计数（图节流）
    last_image_turn: Mapped[int] = mapped_column(default=0)                         # 上次出图时的 tablet_turns
```
db.py `_migrate` 加：
```python
    _ensure_column(engine, "lesson_runs", "demoed_shapes", "demoed_shapes JSON")
    _ensure_column(engine, "lesson_runs", "pending_utterance", "pending_utterance JSON")
    _ensure_column(engine, "lesson_runs", "tablet_turns", "tablet_turns INTEGER DEFAULT 0")
    _ensure_column(engine, "lesson_runs", "last_image_turn", "last_image_turn INTEGER DEFAULT 0")
```
- [ ] Step4 跑测试通过。 Step5 commit `feat(server): LessonRun redesign state columns + migration`.

---

### Task 2: 演示去重（每形状每 run 只一次）
**Files:** `server/app/routers/phone.py`(voice-turn 挂 pending_demo 处), `server/tests/test_phone_lesson.py`
**Consumes:** Task1 `demoed_shapes`; 既有 `runner.demo_shape`, `run.pending_demo`

- [ ] Step1 失败测试 `test_demo_fires_once_per_run`：连发两条都带 `⟦demo:circle⟧` 的 voice-turn；第一条后 run.pending_demo=='circle' 且 demoed_shapes==['circle']；取用清 pending_demo（模拟平板取走：直接 `run.pending_demo=None;commit`）后发第二条 → pending_demo 仍为 None（不再设），demoed_shapes 仍 ['circle']。
- [ ] Step2 跑测试失败。
- [ ] Step3 实现：把 voice-turn 里
```python
                if runner.demo_shape:
                    run.pending_demo = runner.demo_shape
                    db.commit()
```
改为
```python
                if runner.demo_shape:
                    done = list(run.demoed_shapes or [])
                    if runner.demo_shape not in done:
                        run.pending_demo = runner.demo_shape
                        run.demoed_shapes = done + [runner.demo_shape]
                        db.commit()
```
- [ ] Step4 跑测试 + 全 phone_lesson 通过。 Step5 commit `feat(server): demo fires once per shape per run`.

---

### Task 3: /turn 平板去文字卡 + 彩图节流
**Files:** `server/app/routers/turn.py`, `server/tests/test_turn_endpoint.py`
**Consumes:** Task1 tablet_turns/last_image_turn；既有 `cards_engine.build_cards`

- [ ] Step1 失败测试（两条）：
  `test_turn_drops_text_cards`：mock 模型回一个含 text 的回复（如 spoken+一个会被 build_cards 变 text 卡的短句），断言返回 `paper_cards` 里**没有** type=="text" 的卡。
  `test_turn_throttles_images`：连发 3 次 /turn（有 running run），mock 每次都让模型出 image；断言只有**第 1 次**含 image 卡，第 2、3 次不含（节流：`tablet_turns - last_image_turn >= 3` 才放行，首次 last_image_turn=0、tablet_turns=1→1>=3 假… 见实现取"首张放行"）。
- [ ] Step2 跑测试失败。
- [ ] Step3 实现：在 turn.py 生成 `cards` 后、返回前，插入过滤：
```python
        # 平板只承载画面：去掉 text 卡（DouDou 的话走手机语音）
        cards = [c for c in cards if c.get("type") != "text"]
        # 彩图节流：每 run 至多每 3 次提交 1 张；首张放行
        if active_run_id is not None:
            with request.app.state.sessionmaker() as db:
                run = db.get(LessonRun, active_run_id)
                if run is not None:
                    run.tablet_turns = (run.tablet_turns or 0) + 1
                    has_img = any(c.get("type") == "image" for c in cards)
                    allow_img = (run.last_image_turn or 0) == 0 or run.tablet_turns - run.last_image_turn >= 3
                    if has_img and allow_img:
                        run.last_image_turn = run.tablet_turns
                    elif has_img:
                        cards = [c for c in cards if c.get("type") != "image"]
                    db.commit()
```
（注：`build_cards` 返回的是 dict 卡列表；确认键名 `type`。若 build_cards 返回对象，调整为属性访问。实现者先读 `cards_engine.build_cards` 返回结构对齐。）
- [ ] Step4 跑测试 + 全 turn_endpoint 通过。 Step5 commit `feat(server): tablet carries no text cards; throttle color images`.

---

### Task 4: /turn 把 DouDou 的话入队给手机 + GET /api/phone/next
**Files:** `server/app/routers/turn.py`(合成 TTS 入队), `server/app/routers/phone.py`(新端点), `server/tests/test_turn_endpoint.py`, `server/tests/test_phone_lesson.py`
**Consumes:** Task1 pending_utterance；`engine/tts.synthesize`, `admin_voice.load_voice_config`

- [ ] Step1 失败测试：
  `test_turn_queues_phone_utterance`（turn_endpoint）：mock 模型回 "圆圆的气球真好看"，mock TTS 200；发 /turn（有 running run）→ 断言 run.pending_utterance 非空、`["text"]` 含 "气球"、有 `audio_url`。
  `test_phone_next_serves_and_clears`（phone_lesson）：running run 上直接设 pending_utterance={"text":"下一步","audio_url":"/x.mp3"}；GET /api/phone/next → 返回该 utterance；再 GET → null；DB 里 pending_utterance 清空。无 running run → null。
- [ ] Step2 跑测试失败。
- [ ] Step3 实现：
  turn.py：`spoken` 生成后（build_cards 之后），若 active_run_id 有 running run → 合成 TTS（复用 phone.py 里的写法：load_voice_config + synthesize + 存 audio/xxx.mp3 → url `/api/files/audio/xxx.mp3`），`run.pending_utterance={"text":spoken,"audio_url":url}`；TTS 失败则 audio_url=""，text 照存（手机可只显示文字）。
  phone.py：
```python
@router.get("/next")
def phone_next(db: Session = Depends(get_db)):
    run = (db.query(LessonRun).filter(LessonRun.status == "running")
           .order_by(LessonRun.id.desc()).first())
    if run is None or not run.pending_utterance:
        return {"utterance": None}
    u = run.pending_utterance
    run.pending_utterance = None
    db.commit()
    return {"utterance": u}
```
- [ ] Step4 跑测试 + 全套通过。 Step5 commit `feat(server): /turn queues DouDou utterance; GET /api/phone/next`.

---

### Task 5: 手机自动续播（前端）
**Files:** `server/web/src/pages/Phone.tsx`
- [ ] Step1 加后台轮询：lesson 运行中（runId!=null）每 1.5s `GET /api/phone/next`；取到 `{utterance:{text,audio_url}}` → 追加一条 DouDou 气泡（text）、若 audio_url 非空则 `new Audio(audio_url).play()`；去重防重复播（clear-on-fetch 已保证服务端只给一次）。不打断正在进行的录音。
- [ ] Step2 `cd server/web && npm run build` 成功；grep dist 含 `/api/phone/next`。
- [ ] Step3 手动/对拍验收：设 pending_utterance→curl /api/phone/next 得内容→清空。
- [ ] Step4 commit `feat(phone): auto-poll /api/phone/next, speak DouDou's continuation`.

---

### Task 6: 气球课脚本重写（PUT /lessons，不重 seed）
**Files:** 通过 admin API PUT `/api/admin/lessons/{id}`（脚本文件放 scratchpad）
- [ ] 用 PUT 把第 3 课 script_text 改成双通道话术：手机全程语音引导；教画气球身体那一步**只输出一次** `⟦demo:circle⟧`、之后绝不再输出；明确"你只用说话（会由手机说给孩子），平板不写字；孩子画完你会收到 ta 的画，立刻热情夸+引导下一步（拉线/再画一个）"。收尾时才值得给一张气球彩图。验证 DB 内含 `⟦demo:circle⟧` 一次、含"只输出一次"字样。
- [ ] 同步把 `server/app/seed_shapes.py` 第 3 课改成同款（保持 code 与 DB 一致，未来 re-seed 不回退）；`test_seed_shapes` 断言第 3 课含 demo 标记且题面是气球。commit `feat(server): rewrite balloon lesson for two-channel flow`.

---

## 全量回归 + 部署 + 真机
- [ ] `cd server && uv run pytest` 全绿；`npm run build`；重启 8789；`curl /api/phone/next`（无 run→null）。
- [ ] 真机（Ben 回来验收）：手机开课→演示一次→画完手机自动接话→平板零文字、偶尔蓝气球。
- [ ] 单列待办：设备端彩色残影清屏波形（烧板）。
