import { useRef, useState } from 'react'

type Bubble = { role: 'user' | 'assistant'; text: string }

export default function Phone() {
  const [bubbles, setBubbles] = useState<Bubble[]>([])
  const [state, setState] = useState<'idle' | 'recording' | 'thinking'>('idle')
  const [error, setError] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const historyRef = useRef<[string, string][]>([])

  const start = async () => {
    if (state !== 'idle') return
    setError('')
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      const rec = new MediaRecorder(stream)
      const chunks: Blob[] = []
      rec.ondataavailable = e => chunks.push(e.data)
      rec.onstop = async () => {
        stream.getTracks().forEach(t => t.stop())
        setState('thinking')
        const fd = new FormData()
        fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'say.webm')
        fd.append('history', JSON.stringify(historyRef.current.slice(-5)))
        try {
          const resp = await fetch('/api/phone/voice-turn', { method: 'POST', body: fd })
          if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
          const j = await resp.json()
          historyRef.current.push([j.transcript, j.reply_text])
          setBubbles(b => [...b, { role: 'user', text: j.transcript }, { role: 'assistant', text: j.reply_text }])
          if (j.audio_url) new Audio(j.audio_url).play().catch(() => {})
        } catch (e) {
          setError(String(e))
        } finally { setState('idle') }
      }
      recRef.current = rec
      rec.start()
      setState('recording')
    } catch {
      setError('无法使用麦克风：请确认已用 https 打开本页并允许麦克风权限')
    }
  }

  const stop = () => { if (state === 'recording') recRef.current?.stop() }

  return (
    <div style={{
      minHeight: '100vh', display: 'flex', flexDirection: 'column',
      background: '#fffbe6', fontFamily: 'sans-serif',
    }}>
      <div style={{ padding: 16, fontSize: 20, fontWeight: 700, textAlign: 'center' }}>豆豆 🎈</div>
      <div style={{ flex: 1, overflowY: 'auto', padding: '0 16px' }}>
        {bubbles.map((b, i) => (
          <p key={i} style={{ textAlign: b.role === 'user' ? 'right' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '10px 14px', borderRadius: 16, fontSize: 17,
              maxWidth: '80%', background: b.role === 'user' ? '#bae0ff' : '#fff',
              boxShadow: '0 1px 2px rgba(0,0,0,.1)', whiteSpace: 'pre-wrap', textAlign: 'left',
            }}>{b.text}</span>
          </p>
        ))}
        {error && <p style={{ color: 'red', textAlign: 'center' }}>{error}</p>}
      </div>
      <div style={{ padding: 24, textAlign: 'center' }}>
        <button
          onPointerDown={start} onPointerUp={stop} onPointerLeave={stop}
          style={{
            width: 120, height: 120, borderRadius: '50%', border: 'none', fontSize: 18,
            color: '#fff', touchAction: 'none', userSelect: 'none', WebkitUserSelect: 'none',
            background: state === 'recording' ? '#ff4d4f' : state === 'thinking' ? '#d9d9d9' : '#1677ff',
          }}>
          {state === 'recording' ? '松开提问' : state === 'thinking' ? '豆豆想…' : '按住说话'}
        </button>
      </div>
    </div>
  )
}
