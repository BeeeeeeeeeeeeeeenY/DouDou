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

    from app.routers import (admin_curricula, admin_profiles, admin_providers,
                             admin_test, admin_turns, admin_voice, files,
                             openai_compat, phone)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_curricula.router)
    app.include_router(admin_voice.router)
    app.include_router(admin_test.router)
    app.include_router(admin_turns.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(files.router)

    return app
