# DouDou 设计：双通道交互重构（手机=对话 / 平板=画布 / 自动联动）

日期：2026-07-23
状态：Ben 全权委托、离场自主实施（"等你完成我回来验收，不需要再做确认"）。基于真机受挫后的逐条需求。
范围：重构一节课里手机语音与平板纸面的**分工与联动**，让两端像一个 DouDou。不含：彩色墨水残影清屏（设备烧板，单列）、记忆、多设备。

---

## 1. 北极星

**手机＝对话，平板＝画布，两端自动同步成一个 DouDou。**

- **手机（语音）**：DouDou 全部话术——问好、引导、夸奖、追问，语音播 + 文字气泡。孩子（家长持机）说话回应。
- **平板（纸面）**：只出**画面**——DouDou 逐笔演示、孩子自己的墨迹、**偶尔**一张彩图。**零文字卡**。
- **联动**：教画时平板演示**一次**；孩子画完→平板提交→**手机立刻自动接话继续引导**（不用家长再按说话）。

**验收画面**：手机开课，DouDou 语音教"画个圆圆的气球身体"→平板角落**一次**逐笔画个圆→孩子照着画→**画完抬笔，几秒内手机自动响起** DouDou"哇圆圆的气球！再给它拉根线好不好？"→孩子继续。平板始终只有画，没有文字；整节课只在收尾那一下平板给一张蓝气球彩图。

## 2. 现状四宗罪（Ben 真机吐槽）与根因

| 罪 | 现象 | 根因 |
|----|------|------|
| 演示乱刷 | 手机每回一句，平板冒个圆 | 模型每轮都吐 `⟦demo:circle⟧`，服务器无去重（voice-turn 每次都设 pending_demo） |
| 画完不联动 | 孩子画完，手机不接话，要家长再按 | 平板 `/turn` 与手机语音是两条断链；平板画完只回平板卡片，手机不知情 |
| 平板出文字 | 平板显示手写文字回复 | `/turn` 生成 text 卡、设备照渲 |
| 图太频 | 每轮都出彩图 | `/turn` 每轮按模型意愿出 image 卡，无节流 |

**关键洞察**：四条**几乎全靠 服务器 + 手机前端**解决，**设备端基本不动**（设备已在孩子停笔时 POST `/turn`＝天然的"画完"信号；"平板不出文字"＝服务器不再下发 text 卡即可，设备无需改）。→ **本重构不烧板**（残影清屏那条设备改留到最后单独烧）。

## 3. 目标架构：统一的一个循环

```
手机（语音·对话）                     服务器（一个 DouDou 大脑）                平板（画布·已有,不改）
 家长按说话/孩子说 ─voice-turn─► 跑对话，生成 DouDou 的话
                                  → 语音+文字回手机（播 TTS+气泡）
                                  → 教画那一步：设 pending_demo（去重·只一次）
        ◄─ GET /api/phone/next ─ 每~1.5s 轮询"DouDou 有没有新话要说"
                                                    ◄── GET /turn/next ── 平板轮询取演示（已有）
                                                    演示笔画 ──► 平板逐笔画圆（一次）

 孩子在平板上画 ……画完抬笔………………………………………► 平板 POST /turn（已有：page_png+strokes）
                                  跑"看图"回合，生成 DouDou 的话
                                  → 话（spoken_text）入队 run.pending_utterance（给手机）
                                  → 平板只回**画面**：偶尔 image（节流）/ 小 stamp，**无 text 卡**
        ◄─ GET /api/phone/next ─ 手机轮询到 pending_utterance → **自动播 TTS + 续气泡**（不用按）
                                                    ◄── /turn 响应 ── 平板画彩图/贴 stamp（或什么都不画）
```

**闭环**：孩子的**两种输入**（说话→voice-turn；画画→/turn）都汇到服务器一个大脑，产出**统一由手机语音说出**，平板只承载画面。手机通过轮询 `/api/phone/next` 实现"平板一画完就自动接话"。

## 4. 通道分工（硬规则）

- **手机**：DouDou 的**全部文字/语音**。孩子说话（voice-turn）与孩子画画（/turn）触发的 DouDou 回应，**都在手机上说出来**。
- **平板**：**只画**。允许的卡：`image`（彩图，节流·偶尔）、`sketch`/演示（DouDou 逐笔）、可选 `stamp`（小奖励符号，视觉非文字）。**禁止 `text` 卡**。`spoken_text` 一律路由到手机，不落平板。

## 5. 三个同步机制的设计

### 5.1 教画→演示（只一次）
- **服务器去重**：`LessonRun` 加 `demoed_shapes`（JSON list）。voice-turn 解析到 `demo_shape` 时，仅当该 shape 不在 `demoed_shapes` 里才设 `pending_demo` 并记入；否则忽略。→ 模型多吐几次也只演示一次/每形状/每 run。
- **提示词收紧**：课脚本明确"整节课只在第一次布置画圆时输出一次 `⟦demo:circle⟧`，之后绝不再输出"。

### 5.2 孩子画完→手机自动续（新通道，核心）
- **服务器**：`/turn`（平板提交）跑完模型后，把 DouDou 的 `spoken_text` **合成 TTS**、连同文本入队到 `run.pending_utterance`（`{text, audio_url}`，房间作用域，clear-on-fetch）。**不再把这句话当 text 卡下发平板**。
- **新端点 `GET /api/phone/next`**：手机轮询；有 `pending_utterance` 就返回并清空，否则空。
- **手机前端**：常驻后台轮询 `/api/phone/next`（~1.5s）。取到 utterance → **自动播放 audio + 追加气泡**，无需家长按说话。这样"平板一画完，手机立刻接话继续引导"。
- **麦克风顺序**：自动播报期间不抢录音；播完孩子想说话仍按住说话（voice-turn）照常。

### 5.3 彩图节流（偶尔·特别时刻）
- **服务器**：`/turn` 卡片生成对 `image` 卡加**硬节流**——每个 run 至多每 N=3 次平板提交出 1 张，且**默认不出**，仅当模型判定"完成/里程碑"时出。实现：run 记 `last_image_turn` / `image_count`，超频的 image 卡丢弃（降级为无图）。文字照样走语音。→ 平板绝大多数时候只有孩子的画，偶尔一张干净蓝气球。

## 6. 组件改动清单

**服务器（`server/app`）**：
1. `models.py` `LessonRun` 加 `demoed_shapes`(JSON)、`pending_utterance`(JSON: {text,audio_url})、`image_turns`(JSON/int 节流计数)。`db.py` 迁移。
2. `routers/phone.py` voice-turn：demo 去重（5.1）；新增 `GET /api/phone/next`（5.2）。
3. `routers/turn.py` `/turn`：(a) `spoken_text`→合成 TTS→入队 pending_utterance（5.2）；(b) 生成卡片时**剔除 text 卡**、image 卡节流（5.3、5.4）；(c) demo 去重同步。
4. `engine/cards.py`：`build_cards` 产出**不含 text 卡**（或 turn.py 过滤掉 text 卡）；CARD_PROTOCOL 提示模型"平板不写字，话都用 spoken_text（会由手机说）"。
5. 课脚本（PUT /lessons，不重 seed）：气球课改成"手机对话驱动、演示一次、平板画完自动续"的话术；明确 demo 只一次、平板不写字。

**手机前端（`server/web/src/pages/Phone.tsx`）**：
6. 后台轮询 `GET /api/phone/next`，取到 utterance 自动播报+续气泡（5.2）。与现有按住说话、开课/结课并存。

**设备（`device/riddle`）**：本重构**不改**（已在停笔时 POST /turn；不下发 text 卡即无文字）。唯一设备待办＝彩色残影清屏波形，**单列、最后烧板**。

## 7. 建造顺序（每步独立可测，服务器优先）

- **S1 服务器状态位**：`LessonRun` 加 demoed_shapes/pending_utterance/image 节流 + 迁移（pytest）。
- **S2 演示去重**：voice-turn/turn 用 demoed_shapes 只演示一次（pytest；真机看"每轮冒圆"消失）。
- **S3 平板去文字 + 图节流**：/turn 不出 text 卡、image ≤ 节流；spoken_text 不落平板（pytest 对拍卡片）。
- **S4 平板→手机续播通道**：/turn 合成 TTS 入队 pending_utterance + `GET /api/phone/next`（pytest）。
- **S5 手机自动续播**：Phone.tsx 轮询 + 自动播报（构建、手动/对拍）。
- **S6 课脚本重写**：气球课话术（PUT /lessons）。
- **S7 真机串起来调**：手机开课→演示一次→画完手机自动接话→平板零文字、偶尔蓝气球。

## 8. 非目标 / 单列
- 彩色墨水**残影清屏**：设备端清屏加彩色/翻转波形，要烧板 —— 重构跑通后单独做。
- 严格音画同步（沿用"语音先响、笔迹跟上"的自然时差）。
- 记忆、多设备、通用（非固定剧本）能力。

## 9. 决策记录
| 问题 | 决策 |
|------|------|
| 通道分工 | 手机=全部话术(语音+文字)；平板=纯画面(演示/墨迹/偶尔彩图)，零文字卡 |
| 画完联动 | 平板 /turn 生成的话→TTS 入队 pending_utterance→手机轮询 GET /api/phone/next 自动播 |
| 演示频次 | 每形状每 run 只一次（服务器 demoed_shapes 去重 + 提示词收紧） |
| 彩图频次 | 服务器硬节流，偶尔/里程碑才出，默认不出 |
| 设备是否改 | 本重构不改设备（不烧板）；残影清屏单列最后烧 |
| 改课方式 | PUT /lessons 改 script_text，绝不 delete+re-seed（防 run-id 重用污染房间） |
