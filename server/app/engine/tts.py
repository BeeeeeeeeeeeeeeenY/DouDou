import httpx

from app.engine.upstream import UpstreamError


async def synthesize(
    base_url: str, api_key: str, model: str, voice: str, text: str, speed: float = 1.0
) -> bytes:
    async with httpx.AsyncClient(timeout=60) as client:
        resp = await client.post(
            f"{base_url.rstrip('/')}/audio/speech",
            headers={"Authorization": f"Bearer {api_key}"},
            json={"model": model, "voice": voice, "input": text, "speed": speed},
        )
    if resp.status_code != 200:
        raise UpstreamError(resp.status_code, resp.text[:300])
    return resp.content
