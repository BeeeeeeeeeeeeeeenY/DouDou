import pytest
from fastapi.testclient import TestClient

from app.main import create_app


@pytest.fixture()
def app(tmp_path):
    return create_app(data_dir=str(tmp_path))


@pytest.fixture()
def client(app):
    with TestClient(app) as c:
        yield c


@pytest.fixture()
def db(app):
    with app.state.sessionmaker() as session:
        yield session
