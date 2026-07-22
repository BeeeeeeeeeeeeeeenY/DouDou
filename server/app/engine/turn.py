import base64
import time
import uuid
from dataclasses import dataclass, field
from typing import AsyncIterator

from app.engine.errors import ConfigError
from app.engine.prompt import assemble_system_prompt
from app.engine.transcript import split_transcript
from app.engine.upstream import UpstreamError, build_chat_body, stream_chat
from app.models import Profile, Provider, Turn


@dataclass
class TurnInput:
    source: str
    text: str = ""
    image_png: bytes | None = None
    audio: bytes | None = None
    audio_filename: str = "audio.webm"
    history: list[dict] = field(default_factory=list)
    device_protocol_suffix: str = ""
    use_voice_hint: bool = False


class TurnRunner:
    def __init__(self, sessionmaker, data_dir: str, tin: TurnInput):
        self._sm = sessionmaker
        self._data_dir = data_dir
        self._tin = tin
        self.turn_id: int | None = None
        self.reply_text = ""
        self.transcript = ""
        self.system_prompt = ""
        self.input_text = tin.text

    def _save_file(self, sub: str, ext: str, data: bytes) -> str:
        rel = f"{sub}/{uuid.uuid4().hex}.{ext}"
        with open(f"{self._data_dir}/{rel}", "wb") as f:
            f.write(data)
        return rel

    async def stream(self) -> AsyncIterator[str]:
        tin = self._tin
        t0 = time.monotonic()
        full: list[str] = []
        turn = Turn(source=tin.source, input_text=tin.text)
        if tin.image_png:
            turn.input_image_path = self._save_file("images", "png", tin.image_png)
        if tin.audio:
            turn.input_audio_path = self._save_file("audio", "webm", tin.audio)
        try:
            with self._sm() as db:
                profile = db.query(Profile).filter(Profile.is_active.is_(True)).first()
                if profile is None:
                    raise ConfigError("请先在 DouDou 后台配置生效的人设")
                provider = db.get(Provider, profile.provider_id) if profile.provider_id else None
                if provider is None or not provider.enabled or not profile.model:
                    raise ConfigError("请先在 DouDou 后台配置模型")
                turn.profile_id, turn.profile_name, turn.model = profile.id, profile.name, profile.model

                if tin.audio is not None:
                    from app.engine.stt import transcribe
                    from app.routers.admin_voice import load_voice_config
                    stt_cfg, _ = load_voice_config(db)
                    heard = await transcribe(stt_cfg["base_url"], stt_cfg["api_key"],
                                             stt_cfg["model"], tin.audio, tin.audio_filename)
                    self.input_text = heard
                    turn.input_text = heard
                    turn.transcript = heard

                self.system_prompt = assemble_system_prompt(
                    profile.persona_text,
                    voice_hint=profile.voice_hint if tin.use_voice_hint else "",
                    protocol_suffix=tin.device_protocol_suffix,
                )
                turn.system_prompt = self.system_prompt

                user_content: object = self.input_text
                if tin.image_png is not None:
                    b64 = base64.b64encode(tin.image_png).decode()
                    user_content = [
                        {"type": "text", "text": self.input_text},
                        {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
                    ]
                messages = (
                    [{"role": "system", "content": self.system_prompt}]
                    + tin.history
                    + [{"role": "user", "content": user_content}]
                )
                body = build_chat_body(
                    profile.model, messages,
                    temperature=profile.temperature,
                    max_tokens=profile.max_tokens,
                    reasoning_effort=profile.reasoning_effort,
                )
                base_url, api_key = provider.base_url, provider.api_key

            async for delta in stream_chat(base_url, api_key, body):
                full.append(delta)
                yield delta

            visible, post = split_transcript("".join(full))
            self.reply_text = visible
            if post:  # 语音/测试轮无 ⁂ 时保留 STT 转写
                self.transcript = post
            turn.reply_text, turn.transcript = self.reply_text, self.transcript
        except ConfigError as e:
            turn.status, turn.error = "error", e.message
            raise
        except UpstreamError as e:
            turn.status, turn.error = "error", f"{e.status_code}: {e.detail[:500]}"
            raise
        finally:
            turn.latency_ms = int((time.monotonic() - t0) * 1000)
            with self._sm() as db:
                db.add(turn)
                db.commit()
                self.turn_id = turn.id
