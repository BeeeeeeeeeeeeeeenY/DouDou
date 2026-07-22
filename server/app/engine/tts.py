import httpx

from app.engine.upstream import UpstreamError


async def synthesize(
    base_url: str, api_key: str, model: str, voice: str, text: str, speed: float = 1.0
) -> bytes:
    if "minimax" in base_url:
        return await _synthesize_minimax(base_url, api_key, model, voice, text, speed)
    try:
        async with httpx.AsyncClient(timeout=60) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/audio/speech",
                headers={"Authorization": f"Bearer {api_key}"},
                json={"model": model, "voice": voice, "input": text, "speed": speed},
            )
    except httpx.HTTPError as e:
        raise UpstreamError(599, f"{type(e).__name__}: {e}") from e
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    return resp.content


async def _synthesize_minimax(
    base_url: str, api_key: str, model: str, voice: str, text: str, speed: float
) -> bytes:
    """MiniMax 无 OpenAI 兼容 /audio/speech，走原生 t2a_v2（音频为 hex 编码）。"""
    body = {
        "model": model,
        "text": text,
        "stream": False,
        "voice_setting": {"voice_id": voice, "speed": speed},
        "audio_setting": {"format": "mp3"},
    }
    try:
        async with httpx.AsyncClient(timeout=60) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/t2a_v2",
                headers={"Authorization": f"Bearer {api_key}"},
                json=body,
            )
    except httpx.HTTPError as e:
        raise UpstreamError(599, f"{type(e).__name__}: {e}") from e
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    d = resp.json()
    status = (d.get("base_resp") or {}).get("status_code")
    if status not in (0, None):
        raise UpstreamError(502, f"minimax {status}: {(d.get('base_resp') or {}).get('status_msg', '')}")
    audio_hex = (d.get("data") or {}).get("audio") or ""
    if not audio_hex:
        raise UpstreamError(502, "minimax 未返回音频")
    try:
        return bytes.fromhex(audio_hex)
    except ValueError as e:
        raise UpstreamError(502, f"minimax 音频解码失败: {e}") from e
