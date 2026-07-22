import os

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import FileResponse

router = APIRouter(prefix="/api/files")

MEDIA = {"images": "image/png", "audio": "audio/mpeg"}


@router.get("/{sub}/{name}")
def get_file(sub: str, name: str, request: Request):
    if sub not in MEDIA:
        raise HTTPException(400, "非法目录")
    if "/" in name or "\\" in name or ".." in name:
        raise HTTPException(400, "非法文件名")
    path = os.path.join(request.app.state.data_dir, sub, name)
    if not os.path.isfile(path):
        raise HTTPException(404, "文件不存在")
    return FileResponse(path, media_type=MEDIA[sub])
