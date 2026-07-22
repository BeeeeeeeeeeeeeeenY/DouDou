import httpx
import pytest
import respx

from app.engine.upstream import UpstreamError, build_chat_body, stream_chat

SSE = (
    'data: {"choices":[{"delta":{"content":"你好"}}]}\n\n'
    'data: {"choices":[{"delta":{"content":"，小朋友"}}]}\n\n'
    'data: {"choices":[{"delta":{}}]}\n\n'
    "data: [DONE]\n\n"
)


def test_build_chat_body_full():
    body = build_chat_body(
        "gpt-4o-mini",
        [{"role": "user", "content": "hi"}],
        temperature=0.7,
        max_tokens=500,
        reasoning_effort="low",
    )
    assert body == {
        "model": "gpt-4o-mini",
        "stream": True,
        "max_tokens": 500,
        "temperature": 0.7,
        "reasoning_effort": "low",
        "messages": [{"role": "user", "content": "hi"}],
    }


def test_build_chat_body_omits_unset():
    body = build_chat_body("m", [])
    assert "temperature" not in body and "reasoning_effort" not in body
    assert body["max_tokens"] == 2000


@respx.mock
async def test_stream_chat_yields_deltas():
    respx.post("https://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, text=SSE)
    )
    chunks = [c async for c in stream_chat("https://api.test/v1", "sk-x", build_chat_body("m", []))]
    assert chunks == ["你好", "，小朋友"]


@respx.mock
async def test_stream_chat_error_raises():
    respx.post("https://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(401, text='{"error":"bad key"}')
    )
    with pytest.raises(UpstreamError) as ei:
        async for _ in stream_chat("https://api.test/v1", "sk-x", build_chat_body("m", [])):
            pass
    assert ei.value.status_code == 401
    assert "bad key" in ei.value.detail
