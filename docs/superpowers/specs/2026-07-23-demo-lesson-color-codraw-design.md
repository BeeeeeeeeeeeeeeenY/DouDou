# DouDou Demo 设计：一节课 · 彩色共画

日期：2026-07-23
状态：已与产品负责人逐项确认，待评审后转实施计划
范围：把三部分（手机语音 / 服务器大脑 / reMarkable 平板纸面）在**一节固定课程**里首次有机串起来，做出可对外演示的效果。
不含：P3 控制通道 / commit_now、严格音画同步、记忆系统、任意现场发挥（本 Demo 走精选剧本）。

---

## 1. 目标（北极星）

一场**编排好的固定剧本**演示，跑通一节课（如「圆圆的朋友」）的完整闭环：

- 手机开课 → 语音引导五环节（已有能力）；
- 到「④给 DouDou 看」时，孩子在平板上的画 → 服务器 → **平板干净地画出回应**：一句手写夸 + 简笔画 + **一张真彩位图插画**；
- 回应**不压孩子的画**（当前最大痛点）；
- 手机收尾打标，一节课记为一个 `lesson_run`。

**验收画面**：孩子画一个笑脸太阳 → 手机里 DouDou 夸他 → 平板空白处出现一句手写夸 + 一张暖色调的太阳彩图，边上一只简笔小鸟，全程不重叠。

## 2. 现状诊断（为什么现在做不到）

三部分里手机↔服务器已长在一起（课程系统、语音闭环齐全）；**平板是孤岛**，且有两处硬伤：

1. **重叠根因**：平板有一套扎实、带碰撞避让的排版引擎 `device/riddle/src/layout.rs`（`InkMap` 脏净网格 + `find_spot` 螺旋找空白 + 40px 呼吸边距，测试齐全），**但它只服务 `/turn` 卡片渲染路**。现在平板的散文回复走老 `oracle.rs` 自由手写路，**不经过 layout 引擎**，于是把回复一笔笔压在孩子的画和上一轮回复上。
   → **根治重叠、上简笔画、上彩图，是同一个动作：把平板从"自由手写文字"切到"`/turn` 卡片路"。**
2. **`/turn` 地基缺口**：服务器至今**没有 `/turn` 端点**（`server/app/routers/admin_turns.py` 只是日志查看）。平板 `device/riddle/src/turn.rs` 结构化客户端已写好、带 mock 模式与卡片渲染（cards.rs/cardrender.rs），但只能吃本地 mock 文件，接不上真服务器。
3. **课程到不了平板**：课程注入只写在 `phone.py`；平板走的 OpenAI 兼容门面不注入课时，共画那一下不知道在上什么课。

已经能用、本 Demo 直接复用的：手机开课/结课/语音五环节（phone.py）、统一 Turn 引擎（engine/turn.py）、课程引擎与「圆圆的朋友」种子脚本（engine/lesson.py、seed_shapes.py）、layout 排版引擎与卡片渲染（layout.rs、cardrender.rs、cards.rs）。

## 3. 架构：Demo 的数据流

```
手机（语音·已有）                     服务器                          平板（切到 /turn 卡片路）
  开课→lesson_run ───────────────► 记录生效课程 + 当前课时
  语音引导五环节（STT/TTS 已有）
                                                                   孩子画 → 停笔提交
                                     ◄──── POST /turn ─────────────  page_png + new_strokes + page_id
                                     TurnRunner 跑 vision 回合
                                     + 注入当前课时 lesson_context
                                     + 生成 paper_cards[]:
                                       text / sketch / image(彩图)
                                     ──── TurnResponse ───────────►  layout 引擎摆位 → 逐笔画卡片
                                                                     （text/sketch 逐笔，image 贴彩色位图）
  收尾打标 ⟦lesson_report⟧
```

一节课 = 手机与平板**共享一个 `lesson_run`**：平板 `/turn` 回合携带手机开课时的 run 上下文（Demo 里通过一个约定的"当前生效 run"读取，见 §5 S2）。

## 4. 卡片词汇表增量：新增 `image` 卡

现有词汇（§14.2 平板交互设计）：`text / sketch / stamp / count / trace / page`。本 Demo 新增一种，供彩图：

```json
{ "type": "image", "url": "/api/files/lesson-art/sun-01.png",
  "place": "blank_area", "size": "L", "color": true }
```

- `url`：服务器托管的彩色 PNG（复用现有 `/api/files` 静态路由）。
- 平板拉取后，经 quill 的 image 渲染路径（§16.7 提到 quill 已有 image demo）**整块贴到 layout 引擎选定的矩形**，不做逐笔动画（位图无笔画序列）。
- `color: true`：提示平板走彩色刷新波形。**渲染用 quill `quill_swap` 的 mode 5**（S0a spike 实测：mode 0/3/4/5 都出彩色，mode 5 观感最好、主要赢在绿色；见 §5 S0a）。
- 强约束：`image` 卡每回合 ≤1 张。

`text` 与 `sketch` 沿用 §14.2 契约不变；本 Demo 服务器只产出 `text` / `sketch` / `image` 三类，其余卡型留给后续。

## 5. 建造顺序（每步独立可验收）

### S0 · 先并行两件小的

- **S0a 彩色位图真机 spike ✅ 已完成（2026-07-23）——彩色可行，风险解除。** 方法：给 riddle 加隐藏子命令 `--color-test`（画彩色带 + 扫 `quill_swap` 波形 mode + 顶部方块数指示器），经 `systemd-run` 瞬态 unit 跑（唯一安全路径，别用 SSH 前台、别手动 juggle xochitl——会触发设备重启），Ben 看屏判读。
  - **结论**：有效 mode 0–5；面板加载 Gallery 3 彩色波形（`GAL3_…eink`）。**mode 0/3/4/5 出彩色，mode 1/2 灰阶**；mode 0 既彩又是最快墨迹模式（~46ms swap），彩色几乎不付慢刷代价。**Ben 复看定：mode 5 彩色最好（赢在绿色）→ `image` 卡用 mode 5**；未见明显闪烁/残影。
  - 原先写的"太慢/太闪则降级彩色线稿/灰度"分叉**不触发**，真彩位图 `image` 卡成立。
- **S0b 设备「一期半」微改**（§13.3）：`IDLE_COMMIT` 2.8s→6s、五指点按→长按≥3s。真小孩要用，零协议依赖。

### S1 · 服务器 `/turn` 端点（地基）

- 新增 `POST /turn`：复用 `TurnRunner` 跑 page_png+text 回合；新增一层"回复→paper_cards"生成器，产出 `text` / `sketch` / `image`；按平板强约束校验（≤3 卡、sketch≤2000 点、image≤1、text 按档位限字）。
- 平板 `turn.rs` 指向真服务器（`RIDDLE_TURN_URL`）→ 卡片路接管 → **重叠消失**（layout 引擎摆位）。
- 验收：真机（或 mock 对拍）拿到并渲染 text+sketch+image，且不压孩子墨迹。

### S2 · 课程贯通 + 手机/平板共享 run

- `/turn` 在有生效课程 + 当前课时时，注入 `lesson_context`（复用 phone.py 的 `render_lesson_script` / `latest_recap`）。
- 平板 `/turn` 回合归属当前 `lesson_run`（Demo 简化：读"最近一个 running 的 run"作为当前会话；沿用 §8 时间窗挂靠作品的思路）。
- 验收：设好生效课程后，平板共画的回应点出"今天学的圆圆的朋友"，作品挂进该 run。

### S3 · 彩色插画来源：精选彩图库

- **每课一个精选彩色插画小库**（可靠、永远好看），按课/关键词取图；不做现场扩散生成（演示最怕临场翻车）。
- 服务器把选中的图落到 `/api/files/lesson-art/…`，`image` 卡带 url 下发。
- 验收：「圆圆的朋友」这一课的候选彩图就位，共画时稳定出图。

### S4 · 真机把这一节课从头到尾串起来调

- 固定剧本走查：手机开课 → 五环节 → 平板共画（text+sketch+image，不重叠）→ 收尾打标 → run 记录含作品与彩图。
- 真机手感调参：停笔阈值、卡片尺寸、彩图刷新时机。

## 6. 风险与非目标

**头号风险 · 真彩位图在电子纸上的观感 → ✅ 已由 S0a spike 解除（2026-07-23）**：实测面板有 Gallery 3 彩色波形，mode 0/3/4/5 均出彩色、观感可用（mode 5 最佳），未见明显闪烁/残影。曾担心的"慢/闪逼降级灰度"未发生。原平板交互设计 §11「v1 全灰度」的顾虑在本 Demo 的低频"特别时刻"用法下被真机推翻。

**次要风险**：
- `image` 卡是对 §14.2 契约的扩展，服务器与设备两端 schema 必须对齐（cards.rs 解析器 + 服务器生成器同步改）。
- 精选彩图库的美术素材需就位（版权干净的儿童彩色插画，或自制）。

**本 Demo 明确不做**（省下换更短路径）：P3 控制通道 / commit_now（平板自行停笔触发、家长陪按即可）、严格音画同步、记忆系统上云、任意现场发挥。

## 7. 决策记录

| 问题 | 决策 |
|------|------|
| Demo 靶心 | 完整一节课（手机+平板+语音），非单回合 |
| 演示保真度 | 精选固定剧本（预调好，不翻车） |
| 彩色程度 | 真彩位图插画（`image` 卡）——S0a spike 已过，彩色可行，`image` 卡渲染用 `quill_swap` mode 5 |
| 重叠根治 | 平板切 `/turn` 卡片路，复用 layout.rs 排版引擎 |
| 彩图来源 | 每课精选彩图库，非现场生成 |
| 共享会话 | 手机与平板共享一个 lesson_run |
| 控制通道/联动 | 本 Demo 不做，平板自停笔触发 |

## 8. 开放问题（留实现期）

1. S0a 结论：真彩位图刷新观感到底可不可用？决定 §4 `image` 卡是否成立。
2. `image` 卡的平板渲染细节：quill image demo 的接入方式、彩色刷新触发、渲染完是否锁一下页面防闪。
3. 「当前 run」在无控制通道下的读取方式（Demo 简化 vs 未来 commit_now 的过渡）。
4. 精选彩图库的素材来源与规模（先够「圆圆的朋友」一课演示即可）。
