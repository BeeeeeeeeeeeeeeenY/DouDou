async function handle(resp: Response) {
  if (!resp.ok) {
    let detail = `HTTP ${resp.status}`
    try {
      const j = await resp.json()
      detail = j.detail ?? JSON.stringify(j)
    } catch { /* keep default */ }
    throw new Error(detail)
  }
  return resp.json()
}

export const get = (url: string) => fetch(url).then(handle)
export const post = (url: string, body?: unknown) =>
  fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  }).then(handle)
export const put = (url: string, body: unknown) =>
  fetch(url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }).then(handle)
export const del = (url: string) => fetch(url, { method: 'DELETE' }).then(handle)

export async function postForm(url: string, form: FormData) {
  return fetch(url, { method: 'POST', body: form }).then(handle)
}

/** POST 后逐条解析 SSE `data: ` 行，每条 JSON.parse 后回调 */
export async function sse(url: string, body: unknown, onEvent: (obj: any) => void) {
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!resp.ok || !resp.body) throw new Error(`HTTP ${resp.status}`)
  const reader = resp.body.getReader()
  const decoder = new TextDecoder()
  let buf = ''
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    buf += decoder.decode(value, { stream: true })
    const lines = buf.split('\n')
    buf = lines.pop() ?? ''
    for (const line of lines) {
      if (!line.startsWith('data: ')) continue
      const data = line.slice(6).trim()
      if (!data || data === '[DONE]') continue
      try { onEvent(JSON.parse(data)) } catch { /* 忽略非 JSON 行 */ }
    }
  }
}
