from fastapi import Request
from sqlalchemy import create_engine, inspect, text
from sqlalchemy.exc import IntegrityError, OperationalError
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
    try:
        _migrate(engine)
    except OperationalError:
        pass  # 双进程竞争给旧库加列，输家忽略即可（列已被另一进程补上）
    try:
        models.Base.metadata.create_all(engine)
    except OperationalError:
        pass  # 双进程首次启动竞争建表，输家忽略即可（表已被另一进程建好）
    maker = sessionmaker(bind=engine, expire_on_commit=False)
    with maker() as s:  # voice_settings 单行保底
        if s.get(models.VoiceSettings, 1) is None:
            try:
                s.add(models.VoiceSettings(id=1))
                s.commit()
            except IntegrityError:
                s.rollback()  # 双进程竞争插入单例行，输家回滚即可
    return maker


def get_db(request: Request):
    with request.app.state.sessionmaker() as session:  # type: Session
        yield session
