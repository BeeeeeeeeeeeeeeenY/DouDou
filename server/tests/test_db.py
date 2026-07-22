from app import models
from app.db import make_sessionmaker


def test_health(client):
    assert client.get("/api/health").json() == {"ok": True}


def test_tables_created_and_voice_singleton(db):
    # create_app 应建好所有表，并保证 voice_settings 有且仅有 id=1 一行
    assert db.query(models.Provider).count() == 0
    assert db.query(models.Profile).count() == 0
    assert db.query(models.Turn).count() == 0
    vs = db.query(models.VoiceSettings).all()
    assert len(vs) == 1 and vs[0].id == 1


def test_data_subdirs_created(app):
    import os
    for sub in ("images", "audio"):
        assert os.path.isdir(os.path.join(app.state.data_dir, sub))


def test_make_sessionmaker_idempotent(tmp_path):
    # 模拟双进程首次启动竞争建表 / 插入单例行：第二次调用不应抛错
    make_sessionmaker(str(tmp_path))
    maker2 = make_sessionmaker(str(tmp_path))
    with maker2() as s:
        assert s.query(models.VoiceSettings).count() == 1
