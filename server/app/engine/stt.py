import httpx

from app.engine.upstream import UpstreamError


async def transcribe(base_url: str, api_key: str, model: str, audio: bytes, filename: str) -> str:
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
