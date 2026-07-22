from fastapi import Request
from sqlalchemy import create_engine
from sqlalchemy.exc import IntegrityError, OperationalError
from sqlalchemy.orm import Session, sessionmaker

from app import models


def _ensure_column(engine, table: str, column: str, ddl: str) -> None:
    """SQLite 轻量迁移：create_all 不会给已存在的表加新列。"""
    with engine.connect() as conn:
        cols = [row[1] for row in conn.exec_driver_sql(f"PRAGMA table_info({table})")]
        if column not in cols:
            conn.exec_driver_sql(f"ALTER TABLE {table} ADD COLUMN {ddl}")
            conn.commit()


def make_sessionmaker(data_dir: str):
    engine = create_engine(
        f"sqlite:///{data_dir}/doudou.db", connect_args={"check_same_thread": False}
    )
    try:
        models.Base.metadata.create_all(engine)
    except OperationalError:
        pass  # 双进程首次启动竞争建表，输家忽略即可（表已被另一进程建好）
    _ensure_column(engine, "profiles", "web_search", "web_search BOOLEAN DEFAULT 0")
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
