import { useEffect, useRef, useState } from 'react'

type Bubble = { role: 'user' | 'assistant' | 'system'; text: string }
type CurrentLesson = { available: boolean; lesson_seq?: number; lesson_title?: string; curriculum_title?: string }

const REPORT_LABEL = { completed: '完成', partial: '部分完成', skipped: '未参与' } as Record<string, string>

export default function Phone() {
  const [bubbles, setBubbles] = useState<Bubble[]>([])
  const [state, setState] = useState<'idle' | 'recording' | 'thinking'>('idle')
  const [error, setError] = useState('')
  const [lesson, setLesson] = useState<CurrentLesson>({ available: false })
  const [runId, setRunId] = useState<number | null>(null)
  const [runTitle, setRunTitle] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const historyRef = useRef<[string, string][]>([])
  const runRef = useRef<number | null>(null)
  runRef.current = runId

  useEffect(() => {
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }, [])

  const startLesson = async () => {
    setError('')
    try {
      const resp = await fetch('/api/phone/lesson-runs', { method: 'POST' })
      if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
      const j = await resp.json()
      setRunId(j.lesson_run_id)
      setRunTitle(`第 ${j.lesson_seq} 课 · ${j.lesson_title}`)
      historyRef.current = []
      const opening: Bubble[] = [{ role: 'system', text: `开始上课：第 ${j.lesson_seq} 课《${j.lesson_title}》` }]
      if (j.greeting_text) opening.push({ role: 'assistant', text: j.greeting_text })
      setBubbles(opening)
      // 豆豆开课先开口（固定暖句 + TTS），孩子再按住说话回应
      if (j.greeting_audio_url) new Audio(j.greeting_audio_url).play().catch(() => {})
    } catch (e) { setError(String(e)) }
  }

  const endLesson = async () => {
    if (runRef.current != null) {
      await fetch(`/api/phone/lesson-runs/${runRef.current}/end`, { method: 'POST' }).catch(() => {})
    }
    setRunId(null)
    setRunTitle('')
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }

  const start = async () => {
    if (state !== 'idle') return
    setError('')
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      const rec = new MediaRecorder(stream)
      const mime = rec.mimeType || 'audio/webm'
      const ext = mime.includes('mp4') ? 'm4a' : mime.includes('ogg') ? 'ogg' : 'webm'
      const chunks: Blob[] = []
      rec.ondataavailable = e => chunks.push(e.data)
      rec.onstop = async () => {
        stream.getTracks().forEach(t => t.stop())
        setState('thinking')
        const fd = new FormData()
        fd.append('audio', new Blob(chunks, { type: mime }), `say.${ext}`)
        fd.append('history', JSON.stringify(historyRef.current.slice(-5)))
        if (runRef.current != null) fd.append('lesson_run_id', String(runRef.current))
        try {
          const resp = await fetch('/api/phone/voice-turn', { method: 'POST', body: fd })
          if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
          const j = await resp.json()
          historyRef.current.push([j.transcript, j.reply_text])
          setBubbles(b => [...b, { role: 'user', text: j.transcript }, { role: 'assistant', text: j.reply_text }])
          if (j.audio_url) new Audio(j.audio_url).play().catch(() => {})
          if (j.lesson_report) {
            const label = REPORT_LABEL[j.lesson_report.status] ?? j.lesson_report.status
            setBubbles(b => [...b, {
              role: 'system',
              text: `⭐ 今天的课${label}啦！\n亮点：${j.lesson_report.highlights}\n在家可以试试：${j.lesson_report.parent_tip}`,
            }])
            setRunId(null)
            setRunTitle('')
            fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
          }
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
      {runId == null && lesson.available && (
        <div style={{ padding: '0 16px 8px', textAlign: 'center' }}>
          <button onClick={startLesson} style={{
            padding: '10px 18px', borderRadius: 20, border: 'none', fontSize: 16,
            background: '#52c41a', color: '#fff',
          }}>
            开始上课：第 {lesson.lesson_seq} 课《{lesson.lesson_title}》
          </button>
        </div>
      )}
      {runId != null && (
        <div style={{
          margin: '0 16px 8px', padding: '8px 12px', borderRadius: 12, background: '#f6ffed',
          border: '1px solid #b7eb8f', display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        }}>
          <span>📚 上课中：{runTitle}</span>
          <span style={{ display: 'flex', alignItems: 'center' }}>
            <button
              onClick={() => { fetch('/api/phone/clear-board', { method: 'POST' }).catch(() => {}) }}
              style={{ border: 'none', background: 'transparent', color: '#1677ff', marginRight: 12 }}
            >清空画板</button>
            <button onClick={endLesson} style={{ border: 'none', background: 'transparent', color: '#999' }}>结束</button>
          </span>
        </div>
      )}
      <div style={{ flex: 1, overflowY: 'auto', padding: '0 16px' }}>
        {bubbles.map((b, i) => (
          <p key={i} style={{ textAlign: b.role === 'user' ? 'right' : b.role === 'system' ? 'center' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '10px 14px', borderRadius: 16, fontSize: 17,
              maxWidth: '80%', whiteSpace: 'pre-wrap', textAlign: 'left',
              background: b.role === 'user' ? '#bae0ff' : b.role === 'system' ? '#fff7e6' : '#fff',
              boxShadow: '0 1px 2px rgba(0,0,0,.1)',
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
