import base64

import httpx

from app.engine.upstream import UpstreamError

AUDIO_FORMATS = {"mp3", "wav", "webm", "m4a", "mp4", "ogg", "aac", "amr", "flac"}

STT_TIMEOUT = httpx.Timeout(20)  # 几秒的录音没理由等 60 秒；快失败快重试
STT_ATTEMPTS = 2


def _fmt(filename: str) -> str:
    if "." in filename:
        ext = filename.rsplit(".", 1)[-1].lower()
        if ext in AUDIO_FORMATS:
            return ext
    return "webm"


async def _post_with_retry(url: str, headers: dict, **kwargs) -> httpx.Response:
    """转写请求：传输层错误（超时/断连）重试一次，HTTP 错误码不重试（确定性失败）。"""
    last: httpx.HTTPError | None = None
    for _ in range(STT_ATTEMPTS):
        try:
            async with httpx.AsyncClient(timeout=STT_TIMEOUT) as client:
                return await client.post(url, headers=headers, **kwargs)
        except httpx.HTTPError as e:
            last = e
    raise UpstreamError(599, f"{type(last).__name__}: {last}") from last


async def transcribe(base_url: str, api_key: str, model: str, audio: bytes, filename: str) -> str:
    if "aliyuncs.com" in base_url or "dashscope" in base_url:
        return await _transcribe_dashscope(base_url, api_key, model, audio, filename)
    resp = await _post_with_retry(
        f"{base_url.rstrip('/')}/audio/transcriptions",
        headers={"Authorization": f"Bearer {api_key}"},
        files={"file": (filename, audio, "application/octet-stream")},
        data={"model": model},
    )
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
    resp = await _post_with_retry(
        f"{base_url.rstrip('/')}/chat/completions",
        headers={"Authorization": f"Bearer {api_key}"},
        json=body,
    )
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    try:
        content = resp.json()["choices"][0]["message"]["content"]
    except (ValueError, KeyError, IndexError, TypeError) as e:
        raise UpstreamError(502, f"asr 响应解析失败: {e}") from e
    if isinstance(content, list):  # 分段 content 形状兜底
        content = "".join(p.get("text", "") for p in content if isinstance(p, dict))
    return str(content).strip()
