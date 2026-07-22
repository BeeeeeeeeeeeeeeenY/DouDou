from fastapi import FastAPI

from app.config import resolve_data_dir
from app.db import make_sessionmaker


def create_app(data_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="DouDou Server")
    app.state.data_dir = resolve_data_dir(data_dir)
    app.state.sessionmaker = make_sessionmaker(app.state.data_dir)

    @app.get("/api/health")
    def health():
        return {"ok": True}

    return app
