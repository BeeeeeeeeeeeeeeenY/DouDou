import os

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import FileResponse

router = APIRouter(prefix="/api/files")

SUBS = {"images", "audio"}
EXT_CONTENT_TYPE = {
    "png": "image/png",
    "mp3": "audio/mpeg",
    "webm": "audio/webm",
    "m4a": "audio/mp4",
    "mp4": "audio/mp4",
    "ogg": "audio/ogg",
}


def _content_type(name: str) -> str:
    ext = name.rsplit(".", 1)[-1].lower() if "." in name else ""
    return EXT_CONTENT_TYPE.get(ext, "application/octet-stream")


@router.get("/{sub}/{name}")
def get_file(sub: str, name: str, request: Request):
    if sub not in SUBS:
        raise HTTPException(400, "非法目录")
    if "/" in name or "\\" in name or ".." in name:
        raise HTTPException(400, "非法文件名")
    path = os.path.join(request.app.state.data_dir, sub, name)
    if not os.path.isfile(path):
        raise HTTPException(404, "文件不存在")
    return FileResponse(path, media_type=_content_type(name))
