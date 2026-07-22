from fastapi import Request
from sqlalchemy import create_engine, inspect, text
from sqlalchemy.orm import Session, sessionmaker

from app import models


def _migrate(engine) -> None:
    """SQLite 轻量迁移：给一期旧库的 turns 表补 lesson_run_id 列。"""
    insp = inspect(engine)
    if "turns" in insp.get_table_names():
        cols = {c["name"] for c in insp.get_columns("turns")}
        if "lesson_run_id" not in cols:
            with engine.begin() as conn:
                conn.execute(text("ALTER TABLE turns ADD COLUMN lesson_run_id INTEGER"))


def make_sessionmaker(data_dir: str):
    engine = create_engine(
        f"sqlite:///{data_dir}/doudou.db", connect_args={"check_same_thread": False}
    )
    _migrate(engine)
    models.Base.metadata.create_all(engine)
    maker = sessionmaker(bind=engine, expire_on_commit=False)
    with maker() as s:  # voice_settings 单行保底
        if s.get(models.VoiceSettings, 1) is None:
            s.add(models.VoiceSettings(id=1))
            s.commit()
    return maker


def get_db(request: Request):
    with request.app.state.sessionmaker() as session:  # type: Session
        yield session
