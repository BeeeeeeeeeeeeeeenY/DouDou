import json
from typing import AsyncIterator

import httpx


class UpstreamError(Exception):
    def __init__(self, status_code: int, detail: str):
        self.status_code = status_code
        self.detail = detail
        super().__init__(f"upstream {status_code}: {detail}")


def build_chat_body(
    model: str,
    messages: list[dict],
    *,
    temperature: float | None = None,
    max_tokens: int = 2000,
    reasoning_effort: str = "",
) -> dict:
    body: dict = {"model": model, "stream": True, "max_tokens": max_tokens, "messages": messages}
    if temperature is not None:
        body["temperature"] = temperature
    if reasoning_effort:
        body["reasoning_effort"] = reasoning_effort
    return body


async def stream_chat(base_url: str, api_key: str, body: dict) -> AsyncIterator[str]:
    """流式调用 OpenAI 兼容 /chat/completions，逐段 yield delta 文本。"""
    timeout = httpx.Timeout(10, read=90)
    try:
        async with httpx.AsyncClient(timeout=timeout) as client:
            async with client.stream(
                "POST",
                f"{base_url.rstrip('/')}/chat/completions",
                headers={"Authorization": f"Bearer {api_key}"},
                json=body,
            ) as resp:
                if resp.status_code != 200:
                    raw = (await resp.aread()).decode("utf-8", "replace")
                    raise UpstreamError(resp.status_code, raw)
                async for line in resp.aiter_lines():
                    if not line.startswith("data: "):
                        continue
                    data = line[6:].strip()
                    if data == "[DONE]":
                        return
                    try:
                        delta = json.loads(data)["choices"][0].get("delta", {}).get("content")
                    except (json.JSONDecodeError, KeyError, IndexError, TypeError, AttributeError):
                        continue
                    if delta:
                        yield delta
    except httpx.HTTPError as e:
        raise UpstreamError(599, f"{type(e).__name__}: {e}") from e
