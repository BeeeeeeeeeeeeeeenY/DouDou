import base64

import httpx

from app.engine.upstream import UpstreamError

AUDIO_FORMATS = {"mp3", "wav", "webm", "m4a", "mp4", "ogg", "aac", "amr", "flac"}


def _fmt(filename: str) -> str:
    if "." in filename:
        ext = filename.rsplit(".", 1)[-1].lower()
        if ext in AUDIO_FORMATS:
            return ext
    return "webm"


async def transcribe(base_url: str, api_key: str, model: str, audio: bytes, filename: str) -> str:
    if "aliyuncs.com" in base_url or "dashscope" in base_url:
        return await _transcribe_dashscope(base_url, api_key, model, audio, filename)
    try:
        async with httpx.AsyncClient(timeout=60) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/audio/transcriptions",
                headers={"Authorization": f"Bearer {api_key}"},
                files={"file": (filename, audio, "application/octet-stream")},
                data={"model": model},
            )
    except httpx.HTTPError as e:
        raise UpstreamError(599, f"{type(e).__name__}: {e}") from e
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    return resp.json().get("text", "")


async def _transcribe_dashscope(
    base_url: str, api_key: str, model: str, audio: bytes, filename: str
) -> str:
    """阿里 DashScope 系端点无 /audio/transcriptions；qwen-asr 走 chat completions + input_audio。"""
    fmt = _fmt(filename)
    b64 = base64.b64encode(audio).decode()
    body = {
        "model": model,
        "messages": [{
            "role": "user",
            "content": [{
                "type": "input_audio",
                "input_audio": {"data": f"data:audio/{fmt};base64,{b64}", "format": fmt},
            }],
        }],
    }
    try:
        async with httpx.AsyncClient(timeout=60) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/chat/completions",
                headers={"Authorization": f"Bearer {api_key}"},
                json=body,
            )
    except httpx.HTTPError as e:
        raise UpstreamError(599, f"{type(e).__name__}: {e}") from e
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    try:
        content = resp.json()["choices"][0]["message"]["content"]
    except (ValueError, KeyError, IndexError, TypeError) as e:
        raise UpstreamError(502, f"asr 响应解析失败: {e}") from e
    if isinstance(content, list):  # 分段 content 形状兜底
        content = "".join(p.get("text", "") for p in content if isinstance(p, dict))
    return str(content).strip()
