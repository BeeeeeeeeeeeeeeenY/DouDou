from datetime import datetime, timezone

from sqlalchemy import JSON, DateTime, Float, ForeignKey, Integer, String, Text
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class Base(DeclarativeBase):
    pass


class Provider(Base):
    __tablename__ = "providers"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(100))
    base_url: Mapped[str] = mapped_column(String(500))  # 不带尾斜杠
    api_key: Mapped[str] = mapped_column(String(500), default="")
    enabled: Mapped[bool] = mapped_column(default=True)
    notes: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)


class Profile(Base):
    __tablename__ = "profiles"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(100))
    age_band: Mapped[str] = mapped_column(String(10), default="")  # "3-4"|"5-6"|"6-7"
    persona_text: Mapped[str] = mapped_column(Text, default="")
    voice_hint: Mapped[str] = mapped_column(Text, default="")
    provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    model: Mapped[str] = mapped_column(String(200), default="")
    temperature: Mapped[float | None] = mapped_column(Float, nullable=True)
    max_tokens: Mapped[int] = mapped_column(Integer, default=2000)
    reasoning_effort: Mapped[str] = mapped_column(String(10), default="")  # ""|low|medium|high
    is_active: Mapped[bool] = mapped_column(default=False)
    knowledge_base: Mapped[dict | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期挂载点
    memory: Mapped[dict | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期挂载点
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class VoiceSettings(Base):
    __tablename__ = "voice_settings"
    id: Mapped[int] = mapped_column(primary_key=True)  # 恒为 1 的单行表
    stt_provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    stt_model: Mapped[str] = mapped_column(String(200), default="")
    tts_provider_id: Mapped[int | None] = mapped_column(ForeignKey("providers.id"), nullable=True)
    tts_model: Mapped[str] = mapped_column(String(200), default="")
    tts_voice: Mapped[str] = mapped_column(String(100), default="")
    tts_speed: Mapped[float] = mapped_column(Float, default=1.0)


class Turn(Base):
    __tablename__ = "turns"
    id: Mapped[int] = mapped_column(primary_key=True)
    ts: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    source: Mapped[str] = mapped_column(String(10))  # tablet|test|phone
    profile_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    profile_name: Mapped[str] = mapped_column(String(100), default="")
    model: Mapped[str] = mapped_column(String(200), default="")
    system_prompt: Mapped[str] = mapped_column(Text, default="")
    input_text: Mapped[str] = mapped_column(Text, default="")
    input_image_path: Mapped[str] = mapped_column(String(500), default="")
    input_audio_path: Mapped[str] = mapped_column(String(500), default="")
    transcript: Mapped[str] = mapped_column(Text, default="")
    reply_text: Mapped[str] = mapped_column(Text, default="")
    reply_audio_path: Mapped[str] = mapped_column(String(500), default="")
    latency_ms: Mapped[int] = mapped_column(Integer, default=0)
    status: Mapped[str] = mapped_column(String(10), default="ok")  # ok|error
    error: Mapped[str] = mapped_column(Text, default="")
    lesson_run_id: Mapped[int | None] = mapped_column(Integer, nullable=True)


class Curriculum(Base):
    __tablename__ = "curricula"
    id: Mapped[int] = mapped_column(primary_key=True)
    slug: Mapped[str] = mapped_column(String(100), unique=True)
    title: Mapped[str] = mapped_column(String(200), default="")
    age_band: Mapped[str] = mapped_column(String(10), default="")  # "3-4"|"5-6"|"6-7"
    description: Mapped[str] = mapped_column(Text, default="")
    status: Mapped[str] = mapped_column(String(10), default="draft")  # draft|active|archived，active 全局唯一
    current_lesson_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class Lesson(Base):
    __tablename__ = "lessons"
    id: Mapped[int] = mapped_column(primary_key=True)
    curriculum_id: Mapped[int] = mapped_column(ForeignKey("curricula.id"))
    seq: Mapped[int] = mapped_column(Integer)
    slug: Mapped[str] = mapped_column(String(100), default="")
    title: Mapped[str] = mapped_column(String(200), default="")
    goal_text: Mapped[str] = mapped_column(Text, default="")
    script_text: Mapped[str] = mapped_column(Text, default="")
    segments: Mapped[list | None] = mapped_column(JSON, nullable=True)
    duration_min: Mapped[int] = mapped_column(Integer, default=10)
    materials: Mapped[str] = mapped_column(Text, default="")
    enhancements: Mapped[list | None] = mapped_column(JSON, nullable=True)
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow, onupdate=utcnow)


class LessonRun(Base):
    __tablename__ = "lesson_runs"
    id: Mapped[int] = mapped_column(primary_key=True)
    lesson_id: Mapped[int] = mapped_column(ForeignKey("lessons.id"))
    started_at: Mapped[datetime] = mapped_column(DateTime, default=utcnow)
    ended_at: Mapped[datetime | None] = mapped_column(DateTime, nullable=True)
    status: Mapped[str] = mapped_column(String(10), default="running")  # running|completed|partial|skipped|abandoned
    highlights: Mapped[str] = mapped_column(Text, default="")
    parent_tip: Mapped[str] = mapped_column(Text, default="")
    raw_report: Mapped[dict | None] = mapped_column(JSON, nullable=True)
    memory_tags: Mapped[list | None] = mapped_column(JSON, nullable=True)  # 一期恒 NULL，二期记忆挂载点
    artifact_turn_ids: Mapped[list | None] = mapped_column(JSON, nullable=True)
    parent_note: Mapped[str] = mapped_column(Text, default="")
