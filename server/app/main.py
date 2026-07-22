from fastapi import FastAPI, HTTPException

from app.config import resolve_data_dir
from app.db import make_sessionmaker


def create_app(data_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="DouDou Server")
    app.state.data_dir = resolve_data_dir(data_dir)
    app.state.sessionmaker = make_sessionmaker(app.state.data_dir)

    @app.get("/api/health")
    def health():
        return {"ok": True}

    from app.routers import (admin_profiles, admin_providers, admin_test,
                             admin_turns, admin_voice, files, openai_compat, phone)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_voice.router)
    app.include_router(admin_test.router)
    app.include_router(admin_turns.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(files.router)

    import os

    from fastapi.responses import FileResponse

    dist = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "web", "dist")
    if os.path.isdir(dist):
        @app.get("/{path:path}")
        def spa(path: str):
            if path == "api" or path == "v1" or path.startswith(("api/", "v1/")):
                raise HTTPException(404)
            full = os.path.normpath(os.path.join(dist, path))
            if full.startswith(dist + os.sep) and os.path.isfile(full):
                return FileResponse(full)
            return FileResponse(os.path.join(dist, "index.html"))

    return app
