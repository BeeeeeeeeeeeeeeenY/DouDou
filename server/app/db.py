from fastapi import Request
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker

from app import models


def make_sessionmaker(data_dir: str):
    engine = create_engine(
        f"sqlite:///{data_dir}/doudou.db", connect_args={"check_same_thread": False}
    )
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
