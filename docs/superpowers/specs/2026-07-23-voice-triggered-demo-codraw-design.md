# DouDou 设计：语音触发 · 平板边讲边画演示（共画增强）

日期：2026-07-23
状态：与产品负责人逐项确认（A / i / T1 / 方案1），待其复核 spec 后转实施计划
范围：让平板在语音教学"讲到那一步"时，**同步一笔一笔演示**对应的形状（如"画圆圆的小脑袋"→角落里逐笔画出一个圆），孩子照着在旁边画自己的。
关联：本设计**增强** [2026-07-23-demo-lesson-color-codraw-design.md](2026-07-23-demo-lesson-color-codraw-design.md)（那份把三部分在一节课里串起来，平板只在孩子提交后**响应**；本份补上"语音教学时平板主动**演示**"这一环）。

---

## 1. 目标与验收画面

**目标**：一节固定课里，DouDou 语音讲到教画的那一步时，平板在空白角落**逐笔画出示范图形**（能看到笔序），孩子在旁边照着画自己的。

**验收画面**：孩子听 DouDou 说"我们先画一个圆圆的小脑袋"，约 1~2 秒内平板右上角空白处，一支看不见的笔一笔一笔画出一个圆（慢速、能看清运笔），**不压孩子已有的画**；孩子随后在旁边画自己的圆。

**四项已确认的产品决定**：
| 维度 | 决定 |
|------|------|
| 范围 | **A · 固定剧本**：这一课的演示时机与图形预先写死，稳、可演示（非模型临场发挥） |
| 孩子动作 | **i · 看完自己画**：DouDou 在角落示范一个样板（实心墨迹 `sketch` 逐笔），孩子在旁边画自己的（**不**留描红骨架） |
| 触发源 | **T1 · 跟着语音回合掐点**：语音教学回合认出"这是教画那一步"，挂起一个待演示 |
| 通道 | **方案 1 · 轮询**：平板空闲时轮询服务器取待演示，非长连接/非推送 |

## 2. 现状与缺口

现有能力（直接复用）：
- 设备 `cardrender.rs` 已有**逐笔动画循环** `RenderPlan { strokes, points_per_frame }`，以及 `shape_strokes("circle"|"square"|…)` 形状几何、`sketch`/`trace` 渲染路径、`pace: slow` 慢速演示语义。
- 设备 `layout.rs` 排版引擎（脏净网格 + 螺旋找空白 + 呼吸边距），演示图自动避让孩子墨迹。
- 服务器 `⟦lesson_report⟧` 标记解析套路（`engine/lesson.py parse_lesson_report` + `engine/turn.py` 剥离），可平移到 demo 标记。
- "房间"模型：以最近一个 `running` 的 `lesson_run` 为当前会话，`/turn` 已按此作用域。

**唯一缺口**：平板是**拉取式、只在 `pen_idle` 提交** `/turn`（`turn.rs` 注释明言"语音触发/commit_now 需另走控制通道，当前不做"）。没有任何服务器→平板的触发/命令通道，孩子听讲不动笔时，平板收不到"该演示了"。本设计**只补这条最小通道 + 空闲轮询**，画本身全复用。

## 3. 数据流

```
手机 voice-turn                        服务器                                平板（turn_mode 空闲轮询）
 孩子聊到该画了 ─POST─► TurnRunner 跑语音回合
                        教学脚本让模型在"教画圆"这步回复里带 ⟦demo:circle⟧
                        引擎解析+剥离（同 parse_lesson_report，不外显/不念）
                        → run.pending_demo = "circle"
 (TTS 播 DouDou 的话)
                                              ◄─ GET /turn/next ─ 每 ~1.5s（仅空闲）
                        本房间有 pending → 返回 {demo:{shape,place,pace}} 并清空
                        ─────────────────► shape_strokes(shape) → RenderPlan 慢速动画
                                            → layout 摆位于空白角落，逐笔画圆（不压孩子墨迹）
```

一节课 = 手机与平板共享一个 `lesson_run`（沿用既有房间约定）。

## 4. 服务器改动

1. **模型字段**：`LessonRun` 增可空列 `pending_demo: str | None`（默认 `None`）。房间作用域的"待演示"状态。SQLite 既有库需一次轻量迁移（`ALTER TABLE lesson_runs ADD COLUMN pending_demo`）；Demo 库本就会重 seed。

2. **demo 标记解析**（`engine/lesson.py`）：新增
   - 常量 `DEMO_MARK = "⟦demo:"`、固定形状词表 `DEMO_SHAPES = ("circle",)`（可扩 square/triangle…）。
   - `parse_demo(text) -> (clean_text, shape|None)`：抽出 `⟦demo:<shape>⟧`，`shape ∈ DEMO_SHAPES` 才认，剥离该标记。与 `parse_lesson_report` 并列、同一剥离处。
   - `engine/turn.py TurnRunner`：在剥 `lesson_report` 之后同样剥 demo 标记，暴露 `runner.demo_shape: str | None`。剥离后的干净文本才落库/念给孩子。

3. **voice-turn 挂 pending**（`routers/phone.py`）：语音回合跑完后，若 `runner.demo_shape` 非空且 `active_run_id` 的 run 仍 `running` → `run.pending_demo = runner.demo_shape`。仅课程模式生效；与"未开画不关课"门槛互不干扰（demo 不是孩子的平板轮，不满足 `run_has_drawing`，符合预期——看演示≠已开画）。

4. **新端点 `GET /turn/next`**（`routers/turn.py`）：平板轮询用。
   - 取最近一个 `running` 的 `lesson_run`（与 `/turn` 同一房间逻辑）。
   - 有 `run.pending_demo` → 返回 `{"demo": {"shape": <name>, "place": "blank_area", "pace": "slow"}}`，**同一事务清空** `run.pending_demo`（只演示一次）。
   - 无 pending 或无 running run → `{"demo": null}`。
   - 轻量、无副作用（除清 pending）；不写 Turn 记录。

5. **课程脚本**（`seed_shapes.py`）：**被演示的课固定为第 3 课「圆圆的朋友」**（本就整课教画圆，最贴合 circle 演示）。给其**教画那一步**（脚本 ③ 布置"我们来吹泡泡吧，画大大小小的圆泡泡"）加指令：讲到"先画一个圆圆的…"时，在回复末尾单行输出 `⟦demo:circle⟧`（家长孩子都看不到，与 lesson_report 同规格）。仅这一步、只出一次。（首课「想画就画」的小太阳"圆圆的脑袋"节拍可复用同机制，但本次先只做第 3 课。）

## 5. 设备改动（device/riddle）

1. **空闲轮询循环**：`turn_mode` 开启（上课模式）且笔空闲时，每 `RIDDLE_DEMO_POLL_SECS`（默认 1.5s）GET 一次 `next` 端点（新增 env `RIDDLE_TURN_NEXT_URL`，或由 `RIDDLE_TURN_URL` 基址推导 `/turn/next`）。解析 `{demo:{shape,place,pace}}`。
2. **演示渲染**：拿到 shape → 本地 `shape_strokes(shape)` 生成几何 → 构建**实心墨迹 sketch** 的 `RenderPlan`（非淡墨描红骨架；`pace:slow` → 小 `points_per_frame`）→ 交 `layout` 摆位到空白角落 → 现成动画循环逐笔画出。全部复用既有渲染/排版代码，新代码只是"shape 指令 → sketch RenderPlan"这一小段 + 轮询。
3. **只在空闲演示**：正在落笔（孩子在画）时不轮询/不插入演示，避免和孩子的笔抢；若演示到手时孩子已开画，本轮跳过。
4. **抗断**：轮询失败静默、下个周期重试（与既有睡眠/断网韧性一致）。

## 6. 演示内容与摆位

- **形状词表**：本课 `circle`；后续可随形状课扩 `square`/`triangle`（几何设备已有）。
- **样式**：DouDou 的**实心墨迹**逐笔 sketch（非给孩子描的淡墨骨架），慢速，能看清笔序。
- **摆位**：`layout` 引擎选空白角落，**绝不覆盖孩子墨迹**；演示图留在页上当样板；孩子在旁边画自己的，`/turn` 照常响应。
- **不计作品**：演示是 DouDou 主动画的，不落 Turn、不进 `artifact_turn_ids`、不影响 lesson_report/关课判定。

## 7. 边界与非目标

**边界处理**：
- 至多演示一次/教学步：clear-on-fetch + 脚本只出一次标记。
- 无 running lesson_run 时 `/turn/next` 恒 `{demo:null}`（课外不演示）。
- 音画非严格同步：~1.5s 自然时差可接受（沿用交互设计"语音先响、笔迹跟上"）。
- 与"未开画不关课"门槛正交：demo 不满足 `run_has_drawing`。

**非目标**（本设计明确不做）：
- B 通用能力（模型临场决定演示什么/何时）——留待固定剧本跑顺后。
- 严格音画同步、长连接/服务器推送、commit_now 控制通道。
- 描红骨架给孩子描（那是 ii，本次选 i）。
- 全形状演示（先只 circle）。

## 8. 测试

**服务器（pytest）**：
- `parse_demo`：有效 `⟦demo:circle⟧` 抽出并剥离；未知形状 `⟦demo:xyz⟧` 不认（shape=None）、仍剥离不外显；无标记原样返回。
- voice-turn：教学回合带标记 → running run 的 `pending_demo` 被置为 circle；干净文本不含标记。
- `GET /turn/next`：有 pending → 返回 demo 且清空（再次请求得 null）；无 pending / 无 running run → null；房间作用域（别的 run 的 pending 不串）。
- 与关课门槛正交：demo 不使 `run_has_drawing` 为真。

**设备（cargo test）**：
- shape 指令 → 预期 sketch `RenderPlan`（笔画非空、慢 pace）。
- `next` 响应解析（有 demo / null / 坏 JSON 容错）。

**真机（固定剧本走查）**：语音讲到"画圆圆的小脑袋" → 1~2s 内平板角落逐笔画圆、不压孩子画 → 孩子在旁边画自己的 → `/turn` 照常响应。停笔阈值、演示尺寸/位置、轮询间隔真机调参。

## 9. 决策记录

| 问题 | 决策 |
|------|------|
| 范围 | A 固定剧本、单课、只 circle |
| 孩子动作 | i 看完自己画（实心 sketch 样板，不留描红骨架） |
| 触发源 | T1 语音回合掐点，靠 `⟦demo:circle⟧` 标记（非关键词搜索） |
| 通道 | 方案 1 轮询 `GET /turn/next`（非长连接/推送） |
| 待演示状态 | 挂 `lesson_run.pending_demo`（房间作用域，clear-on-fetch） |
| 几何位置 | 设备端 `shape_strokes` 本地生成；服务器只发轻量指令 |
| 与关课门槛 | 正交；demo 不计作品、不满足 `run_has_drawing` |
