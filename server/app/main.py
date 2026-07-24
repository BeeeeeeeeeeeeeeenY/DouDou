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

    from app.routers import (admin_curricula, admin_profiles, admin_providers,
                             admin_test, admin_turns, admin_voice, files,
                             openai_compat, phone, turn)

    app.include_router(admin_providers.router)
    app.include_router(admin_profiles.router)
    app.include_router(admin_curricula.router)
    app.include_router(admin_voice.router)
    app.include_router(admin_test.router)
    app.include_router(admin_turns.router)
    app.include_router(openai_compat.router)
    app.include_router(phone.router)
    app.include_router(turn.router)
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
                return FileResponse(full)  # 带 hash 的静态资源可长缓存
            # index.html 绝不缓存：手机刷新总能拿到最新页（指向最新 hash 的 JS）。
            # 否则移动端浏览器会一直用缓存的旧 index.html→加载旧 JS→改了看不到。
            return FileResponse(os.path.join(dist, "index.html"),
                                headers={"Cache-Control": "no-cache, no-store, must-revalidate"})

    return app
