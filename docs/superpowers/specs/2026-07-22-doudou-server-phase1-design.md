# DouDou Server 一期设计

日期：2026-07-22
状态：已与产品负责人确认方向，待实施

## 1. 背景与目标

DouDou 是面向 3-7 岁中文儿童（涂鸦到识字阶段）的护眼学习伙伴，目标架构为三件套：
手机（听说）+ 服务器（记忆/教学策略）+ reMarkable Paper Pro（纸面交互）。
现状是平板应用 `device/riddle`（Rust）直连模型 API，人设、模型配置都焊死在设备侧。

一期目标：搭建 **DouDou Server** —— 一个跑在本地 Mac 上的 FastAPI 服务 + 中文管理后台，
把"产品大脑"从平板移到服务器：

1. 模型 provider 配置与连通性测试
2. 人设/教学 profile 配置（按年龄段），全局唯一"生效 profile"
3. 网页测试台（文字/图片/语音输入，与真机同一代码路径）
4. 平板零改动接入（OpenAI 兼容门面）
5. 完整语音闭环 + 手机按住说话页（把路线图 M2 的核心提前）
6. 对话记录回看（平板/测试台/手机的每一轮都可回放）

## 2. 范围边界

一期**不做**：

- `/turn` 结构化接口（等平板交互规划的需求清单，二期实施）
- 知识库 / 长期记忆（二期，候选 LightRAG + mem0/Memobase；一期仅在数据模型留挂载点）
- 跨设备联动（手机说话→平板同步显示：riddle 目前只能主动发起，需改平板端，随 `/turn` 二期做）
- 语音唤醒、VAD 连续对话（一期只有按住说话）
- TTS 逐句流式合成（一期整段回复合成后播放，延迟优化留二期）
- 登录鉴权、多用户（仅局域网个人部署）
- 课程配置

## 3. 总体架构

```
仓库新增 server/ 目录
├── app/                FastAPI（Python 3.12）
│   ├── engine/         Turn 引擎（核心抽象：输入 → 生效 profile → 上游模型 → 流式输出 → 落库）
│   ├── routers/
│   │   ├── openai_compat   POST /v1/chat/completions（平板门面，SSE）
│   │   ├── phone           手机语音轮接口
│   │   └── admin_*         providers / profiles / voice / turns / 测试台
│   └── db              SQLite（SQLAlchemy），文件存 server/data/（gitignore）
└── web/                React + Vite + TypeScript + Ant Design（中文界面）
    ├── /admin          管理后台（桌面）
    └── /phone          手机按住说话页（移动端路由）
```

- **监听**：http `0.0.0.0:8787`（平板与局域网管理用）+ https `0.0.0.0:8788`（手机页用，
  手机浏览器的麦克风 API 要求安全上下文）。https 证书用 mkcert 针对 Mac 局域网地址签发，
  手机安装一次 CA 描述文件。riddle 的 rustls 不信任自签 CA，平板继续走 http 8787。
  启动脚本一键拉起两个监听（同一 FastAPI 应用）。
- **平板接入**：仅改设备上 `oracle.env` 两行，Rust 零改动：
  - `RIDDLE_OPENAI_BASE=http://<Mac IP>:8787/v1`
  - `RIDDLE_OPENAI_KEY=doudou`（riddle 需非空 key 才选 HTTP 后端；服务器不校验此值）
- **前端交付**：生产模式由 FastAPI 托管 Vite 构建产物；开发模式 Vite dev server 代理 API。

## 4. 核心抽象：Turn 引擎

一轮对话 = `TurnInput → run_turn() → 流式输出 + TurnRecord 落库`。

```
TurnInput  { source: tablet|test|phone, text?, image_png?, audio?, history?, device_protocol_suffix? }
TurnResult { reply_text, transcript?, reply_audio_path?, latency_ms, model, error? }
```

流程：

1. 若输入含音频：先经 STT 转文字（见 §6），转写作为本轮用户文字。
2. 取全局生效 profile，组装 messages：
   - system = profile 人设文本（+ 语音轮追加 `voice_hint`；+ 平板轮追加设备记忆协议后缀，见 §5）
   - history 原样透传（平板带来的近期页面对话，或测试台/手机页的会话历史）
   - user = 文字（+ 图片 data URI，如有）
3. 按 profile 绑定的 provider/模型/参数，httpx 流式调用上游 `/chat/completions`。
4. 边收边转发给调用方；结束后解析 `⁂` 转写后缀（riddle 记忆协议产物，见 §5）。
5. 落库 TurnRecord（含输入图/音频文件路径、转写、回复全文、延迟、错误）。

**关键不变量**：门面、测试台、手机页都调用同一个 `run_turn()`——网页上测的 = 真机上跑的。
二期 `/turn` 结构化接口只是给引擎换外壳，内核不动。

## 5. OpenAI 兼容门面（平板入口）

riddle 的 HTTP 后端发送标准 chat-completions 请求：
`messages = [system(persona[+记忆协议]), …history 对话对…, user[text + image_url(data:image/png;base64)]]`，
`stream: true`，SSE 返回，客户端解析 `choices[0].delta.content`。
门面对此的处理：

- **替换系统提示词**：丢弃设备内置 persona，换成服务器生效 profile 的人设。
  但若设备 system 中含记忆协议段（以 `\n\n记忆协议：` 为标记，驱动平板的召回
  `⟦show:N⟧` 指令与 `⁂` 转写后缀），**从标记起原样保留**，追加在服务器人设之后。
  平板的记忆、召回旧页功能因此照常工作。
- **参数以 profile 为准**：上游 provider、模型、temperature、max_tokens、reasoning_effort
  全用后台配置，忽略设备请求里的对应值。请求体中 `max_tokens` /
  `max_completion_tokens` 两种字段名都接受（riddle 收到 400 会换名重试，门面直接兼容即可）。
  服务器调上游时先发 `max_tokens`，若上游 400 且报文含 `max_completion_tokens`
  则换名重试一次（与 riddle 原直连行为一致）。
- **其余透传**：history 与 user 内容（文字+图片）原样转发；上游 SSE 按 OpenAI chunk
  格式转发回平板（riddle 现有解析器直接兼容）。
- 来源标记为 `tablet` 落库；`⁂` 后的转写内容存入 TurnRecord.transcript，
  供对话记录页展示可读的"孩子写了什么"。

## 6. 语音闭环与手机页

**接口标准**：STT/TTS 也走 OpenAI 兼容协议，复用 provider 体系（base_url + key）：

- STT：`POST {base}/audio/transcriptions`（multipart 上传音频，whisper 系）
- TTS：`POST {base}/audio/speech`（model + voice + input → 音频）

OpenAI、Groq、SiliconFlow（SenseVoice/CosyVoice，中文效果好）及本地 whisper 服务均兼容。

**语音一轮**：

```
手机页按住说话（MediaRecorder，webm/opus）→ 上传
→ STT 转文字 → run_turn()（同引擎、同生效 profile，system 追加 profile.voice_hint）
→ 回复文字 → TTS 整段合成 → 返回 { transcript, reply_text, audio_url }
→ 手机自动播放 + 显示文字气泡
```

接口为同步 `POST /api/phone/voice-turn`（一期不做流式语音，延迟预期数秒，可接受）。
多轮上下文由手机页在客户端持有：每次请求附带本会话最近 N 组（转写, 回复）对，
服务端不维护手机会话状态（测试台同理）。

**手机页 `/phone`**：移动端路由，大按钮按住说话 + 会话气泡（转写与回复文字）+
自动播放回复语音。使用全局生效 profile，无需登录。录音与回复音频均落
`server/data/audio/` 并记录在 turns 表，家长可在后台回放。

## 7. 数据模型（SQLite，4 张表）

- **providers**：id, name, base_url, api_key, enabled, notes, created_at
  —— 同一 provider 可同时服务 chat 与 audio 端点
- **profiles**：id, name, age_band("3-4"|"5-6"|"6-7"), persona_text, voice_hint(可空),
  provider_id, model, temperature, max_tokens, reasoning_effort, is_active(全局唯一),
  knowledge_base(JSON, 一期恒空), memory(JSON, 一期恒空), updated_at
  —— 两个 JSON 空列是二期挂 LightRAG / mem0(Memobase) 的预留挂载点，届时不改表结构
- **voice_settings**（单行）：stt_provider_id, stt_model, tts_provider_id, tts_model,
  tts_voice, tts_speed
- **turns**：id, ts, source(tablet|test|phone), profile_id, profile_name 快照, model,
  system_prompt 快照, input_text, input_image_path, input_audio_path, transcript,
  reply_text, reply_audio_path, latency_ms, status, error
  —— system_prompt 快照使每一轮"当时用的什么人设"可追溯，也是测试台
  "查看实际 system prompt"的数据来源

API key 明文存本地 SQLite。前提：仅局域网、个人 Mac、无公网暴露；此前提写入 README。

## 8. 管理界面（5 个页面，中文）

1. **模型配置**：providers 增删改；每行"测试连通"按钮（发极小请求，显示延迟/错误）。
2. **人设 Profile**：列表 + 编辑器（名称、年龄段、人设大文本框、语音补充提示词、
   provider/模型选择、参数），一键"设为生效"（互斥）。模型名为自由文本，
   provider 支持 `/v1/models` 时提供候选下拉。
3. **测试台**：选 profile（默认生效者）；文字输入 / 图片上传（可贴手写照片）/
   麦克风录音三种输入；流式显示回复；"自动朗读回复"开关；
   可展开查看实际发出的完整 system prompt（调试用）。
4. **语音配置**：STT/TTS 的 provider、模型、音色、语速；附两个即时测试：
   "录一句测转写"、"输入文字试听音色"。
5. **对话记录**：最近 turns 列表（时间、来源、缩略图、转写、回复摘要、延迟、状态），
   点开详情：原始手写图、录音回放、回复音频、完整回复、报错原因。
   这是调教人设的主要反馈回路——真机上发生的每一轮都能回看。

## 9. 错误处理

- 未配置 provider / 无生效 profile：门面与语音接口返回明确中文错误
  （如"请先在 DouDou 后台配置模型"）。riddle 会把错误文字写在纸面上，行为可接受。
- 上游 401/429/超时：错误信息透传给调用方并落库；对话记录页可见失败原因。
- SSE 中途上游断流：直接断开下游流；riddle 端已有 90 秒读超时兜底。
- STT/TTS 失败：语音接口返回结构化错误，手机页气泡提示；turn 仍落库（status=error）。

## 10. 测试策略

- **pytest 单测**：以真实 riddle 请求体为 fixture，覆盖：门面请求解析、
  记忆协议后缀保留、system prompt 组装（含 voice_hint 分支）、SSE chunk 格式、
  `⁂` 转写提取、profile 生效互斥、上游错误映射。上游一律 respx mock。
- **端到端验收**：Mac 本机运行 `riddle --oracle-test <手写图>.png`，
  `RIDDLE_OPENAI_BASE` 指向本地门面——不碰平板即可全链路验证；
  语音链路用测试台麦克风走通 STT→turn→TTS。
- 最终真机验收：平板改 `oracle.env` 指向 Mac，手写一轮；手机开 `/phone` 说一轮。

## 11. 二期展望（非本期承诺）

- `/turn` 结构化接口（spoken_text / paper_text / paper_cards），输入来自平板交互
  规划产出的数据需求清单；riddle 改造接入。
- 知识库：LightRAG（轻部署）起步，扫描绘本解析成瓶颈则升级 RAGFlow。
- 记忆：mem0（默认）或 Memobase（结构化儿童档案，家长可在后台直接查看修改）。
- 跨设备联动、TTS 逐句流式、课程配置。
