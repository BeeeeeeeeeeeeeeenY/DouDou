import { useEffect, useRef, useState } from 'react'

type Bubble = { role: 'user' | 'assistant' | 'system'; text: string }
type CurrentLesson = { available: boolean; lesson_seq?: number; lesson_title?: string; curriculum_title?: string }

const REPORT_LABEL = { completed: '完成', partial: '部分完成', skipped: '未参与' } as Record<string, string>

// VAD 阈值（真机需微调）：START 说话起判、STOP 静音判、SILENCE_MS 说完一句静默多久发、
// MAX_MS 单句上限防跑飞、MIN_MS 太短丢弃（避免咳嗽/噪声误触）。
const START_THRESH = 0.045
const STOP_THRESH = 0.025
const SILENCE_MS = 1200
const MAX_MS = 12000
const MIN_MS = 350

export default function Phone() {
  const [bubbles, setBubbles] = useState<Bubble[]>([])
  // idle=在听 / thinking=豆豆想 / speaking=豆豆在说；录音是 VAD 自动的，不再有"按住"态。
  const [status, setStatus] = useState<'idle' | 'thinking' | 'speaking'>('idle')
  const [muted, setMuted] = useState(false)
  const [error, setError] = useState('')
  const [lesson, setLesson] = useState<CurrentLesson>({ available: false })
  const [runId, setRunId] = useState<number | null>(null)
  const [runTitle, setRunTitle] = useState('')

  const historyRef = useRef<[string, string][]>([])
  const runRef = useRef<number | null>(null); runRef.current = runId
  const mutedRef = useRef(muted); mutedRef.current = muted
  // 豆豆在说话或正在想时，绝不收音（否则会把豆豆自己的声音/回声当成孩子说话）。
  const busyRef = useRef(false)
  const statusRef = useRef(status); statusRef.current = status

  useEffect(() => {
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }, [])

  // 播放豆豆语音：播放期间挂 busy（暂停收音），播完/失败恢复。
  const speak = (url?: string) => {
    if (!url) return
    busyRef.current = true
    setStatus('speaking')
    const a = new Audio(url)
    const done = () => { busyRef.current = false; if (statusRef.current === 'speaking') setStatus('idle') }
    a.onended = done
    a.onerror = done
    a.play().catch(done)
  }

  // 一句录好（VAD 判定说完）后发给服务器，播豆豆回复。
  const sendUtterance = async (blob: Blob, ext: string) => {
    busyRef.current = true
    setStatus('thinking')
    try {
      const fd = new FormData()
      fd.append('audio', blob, `say.${ext}`)
      fd.append('history', JSON.stringify(historyRef.current.slice(-5)))
      if (runRef.current != null) fd.append('lesson_run_id', String(runRef.current))
      const resp = await fetch('/api/phone/voice-turn', { method: 'POST', body: fd })
      if (!resp.ok) throw new Error((await resp.json()).detail ?? `HTTP ${resp.status}`)
      const j = await resp.json()
      historyRef.current.push([j.transcript, j.reply_text])
      setBubbles(b => [...b, { role: 'user', text: j.transcript }, { role: 'assistant', text: j.reply_text }])
      if (j.lesson_report) {
        const label = REPORT_LABEL[j.lesson_report.status] ?? j.lesson_report.status
        setBubbles(b => [...b, {
          role: 'system',
          text: `⭐ 今天的课${label}啦！\n亮点：${j.lesson_report.highlights}\n在家可以试试：${j.lesson_report.parent_tip}`,
        }])
        setRunId(null); setRunTitle('')
        fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
      }
      if (j.audio_url) speak(j.audio_url)     // 播完在 speak 里恢复 busy/idle
      else { busyRef.current = false; setStatus('idle') }
    } catch (e) {
      setError(String(e)); busyRef.current = false; setStatus('idle')
    }
  }

  // 双通道联动：上课中后台轮询 /api/phone/next——孩子在平板上画完，服务器把豆豆
  // 的下一句入队，这里取到自动播报+续气泡。只在空闲(不在说/想/收音)时取播。
  useEffect(() => {
    if (runId == null) return
    const id = setInterval(async () => {
      if (busyRef.current || statusRef.current !== 'idle') return
      try {
        const r = await fetch('/api/phone/next')
        if (!r.ok) return
        const u = (await r.json()).utterance
        if (u && u.text) {
          historyRef.current.push(['（孩子在平板上画完了）', u.text])
          setBubbles(b => [...b, { role: 'assistant', text: u.text }])
          speak(u.audio_url)
        }
      } catch { /* 网络抖动：忽略 */ }
    }, 1200)
    return () => clearInterval(id)
  }, [runId])

  // 持续收音 + VAD：上课中且未静音时常开麦，自动判断说完一句就发。豆豆说话/想时
  // (busyRef) 不收。静音或下课时彻底关麦。真机阈值可调（见文件顶常量）。
  useEffect(() => {
    if (runId == null || muted) return
    let stream: MediaStream | null = null
    let ctx: AudioContext | null = null
    let raf = 0
    let cancelled = false
    let rec: MediaRecorder | null = null
    let recording = false
    let chunks: Blob[] = []
    let recStart = 0
    let silenceStart = 0
    let ext = 'webm'

    const setup = async () => {
      try {
        stream = await navigator.mediaDevices.getUserMedia({ audio: true })
        if (cancelled) { stream.getTracks().forEach(t => t.stop()); return }
        rec = new MediaRecorder(stream)
        const mime = rec.mimeType || 'audio/webm'
        ext = mime.includes('mp4') ? 'm4a' : mime.includes('ogg') ? 'ogg' : 'webm'
        rec.ondataavailable = e => chunks.push(e.data)
        rec.onstop = () => {
          const dur = performance.now() - recStart
          const blob = new Blob(chunks, { type: mime })
          chunks = []
          if (dur >= MIN_MS && blob.size > 0 && !mutedRef.current) sendUtterance(blob, ext)
        }
        ctx = new AudioContext()
        const src = ctx.createMediaStreamSource(stream)
        const analyser = ctx.createAnalyser()
        analyser.fftSize = 1024
        src.connect(analyser)
        const buf = new Uint8Array(analyser.fftSize)
        const tick = () => {
          if (cancelled) return
          analyser.getByteTimeDomainData(buf)
          let sum = 0
          for (let i = 0; i < buf.length; i++) { const x = (buf[i] - 128) / 128; sum += x * x }
          const rms = Math.sqrt(sum / buf.length)
          const now = performance.now()
          if (busyRef.current) {
            // 豆豆在说/想：若正录着就丢弃这段（大概率是回声）
            if (recording && rec && rec.state === 'recording') { chunks = []; recStart = now; }
          } else if (!recording) {
            if (rms > START_THRESH && rec && rec.state === 'inactive') {
              chunks = []; recStart = now; silenceStart = 0; recording = true; rec.start()
            }
          } else {
            if (rms < STOP_THRESH) {
              if (!silenceStart) silenceStart = now
              else if (now - silenceStart > SILENCE_MS) { recording = false; rec?.stop() }
            } else silenceStart = 0
            if (now - recStart > MAX_MS) { recording = false; rec?.stop() }
          }
          raf = requestAnimationFrame(tick)
        }
        raf = requestAnimationFrame(tick)
      } catch {
        setError('无法使用麦克风：请确认已用 https 打开本页并允许麦克风权限')
      }
    }
    setup()
    return () => {
      cancelled = true
      cancelAnimationFrame(raf)
      try { if (rec && rec.state === 'recording') rec.stop() } catch { /* ignore */ }
      stream?.getTracks().forEach(t => t.stop())
      ctx?.close().catch(() => {})
    }
  }, [runId, muted])

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
      speak(j.greeting_audio_url)   // 开课先开口，播完自动进入"在听"
    } catch (e) { setError(String(e)) }
  }

  const endLesson = async () => {
    if (runRef.current != null) {
      await fetch(`/api/phone/lesson-runs/${runRef.current}/end`, { method: 'POST' }).catch(() => {})
    }
    setRunId(null); setRunTitle(''); setStatus('idle'); busyRef.current = false
    fetch('/api/phone/current-lesson').then(r => r.json()).then(setLesson).catch(() => {})
  }

  const statusText = muted ? '已静音' : status === 'speaking' ? '豆豆在说…' : status === 'thinking' ? '豆豆在想…' : '在听你说呀'

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
        <div style={{ marginBottom: 10, fontSize: 15, color: '#888' }}>
          {runId != null ? statusText : '按上面绿色按钮开始上课'}
        </div>
        <button
          onClick={() => setMuted(m => !m)}
          disabled={runId == null}
          style={{
            width: 120, height: 120, borderRadius: '50%', border: 'none', fontSize: 17,
            color: '#fff', userSelect: 'none', WebkitUserSelect: 'none',
            background: runId == null ? '#d9d9d9' : muted ? '#ff4d4f' : '#52c41a',
          }}>
          {muted ? '🔇\n已静音' : '🎙️\n听着呢'}
        </button>
      </div>
    </div>
  )
}
