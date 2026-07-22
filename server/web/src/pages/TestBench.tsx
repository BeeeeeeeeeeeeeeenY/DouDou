import { Button, Card, Checkbox, Collapse, Input, Space, Upload, message } from 'antd'
import { useRef, useState } from 'react'
import { postForm, sse } from '../api'

type Msg = { role: 'user' | 'assistant'; text: string }

export default function TestBench() {
  const [text, setText] = useState('')
  const [imageB64, setImageB64] = useState<string | null>(null)
  const [voiceMode, setVoiceMode] = useState(false)
  const [autoRead, setAutoRead] = useState(false)
  const [busy, setBusy] = useState(false)
  const [recording, setRecording] = useState(false)
  const recRef = useRef<MediaRecorder | null>(null)
  const [msgs, setMsgs] = useState<Msg[]>([])
  const [sysPrompt, setSysPrompt] = useState('')

  // 麦克风输入：录音 → STT 转写 → 填入输入框（复用语音配置的 stt-test 接口）
  const recordToText = async () => {
    if (recording) { recRef.current?.stop(); return }
    let stream: MediaStream
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    } catch (e) { message.error(String(e)); return }
    const rec = new MediaRecorder(stream)
    const chunks: Blob[] = []
    rec.ondataavailable = e => chunks.push(e.data)
    rec.onstop = async () => {
      stream.getTracks().forEach(t => t.stop())
      setRecording(false)
      const fd = new FormData()
      fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'bench.webm')
      try {
        const r = await postForm('/api/admin/voice/stt-test', fd)
        setText(t => (t ? t + ' ' : '') + r.text)
      } catch (e) { message.error(String(e)) }
    }
    recRef.current = rec
    rec.start()
    setRecording(true)
  }

  const send = async () => {
    if (!text && !imageB64) return
    setBusy(true)
    const history: [string, string][] = []
    for (let i = 0; i + 1 < msgs.length; i += 2) history.push([msgs[i].text, msgs[i + 1].text])
    const userText = text || '（仅图片）'
    setMsgs(m => [...m, { role: 'user', text: userText }, { role: 'assistant', text: '' }])
    setText('')
    let reply = ''
    try {
      await sse('/api/admin/test-turn',
        { text, image_base64: imageB64, history, voice_mode: voiceMode },
        (ev) => {
          if (ev.delta) {
            reply += ev.delta
            setMsgs(m => [...m.slice(0, -1), { role: 'assistant', text: reply }])
          } else if (ev.error) {
            message.error(ev.error)
            setMsgs(m => [...m.slice(0, -1), { role: 'assistant', text: `⚠️ ${ev.error}` }])
          } else if (ev.done) {
            setSysPrompt(ev.system_prompt)
            if (autoRead && reply) readAloud(reply)
          }
        })
    } finally {
      setBusy(false)
      setImageB64(null)
    }
  }

  const readAloud = async (t: string) => {
    const resp = await fetch('/api/admin/voice/tts-test', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: t }),
    })
    if (resp.ok) new Audio(URL.createObjectURL(await resp.blob())).play()
    else message.error(await resp.text())
  }

  return (
    <div style={{ maxWidth: 720 }}>
      <Card style={{ marginBottom: 16, minHeight: 320 }}>
        {msgs.map((m, i) => (
          <p key={i} style={{ textAlign: m.role === 'user' ? 'right' : 'left' }}>
            <span style={{
              display: 'inline-block', padding: '8px 12px', borderRadius: 8,
              background: m.role === 'user' ? '#e6f4ff' : '#f5f5f5', whiteSpace: 'pre-wrap',
            }}>{m.text || '…'}</span>
          </p>
        ))}
      </Card>
      <Space.Compact style={{ width: '100%' }}>
        <Input.TextArea rows={2} value={text} onChange={e => setText(e.target.value)}
                        placeholder="输入文字，或只传手写图片" onPressEnter={e => { e.preventDefault(); send() }} />
        <Button type="primary" onClick={send} loading={busy}>发送</Button>
      </Space.Compact>
      <Space style={{ marginTop: 8 }}>
        <Upload beforeUpload={(f) => {
          const reader = new FileReader()
          reader.onload = () => setImageB64((reader.result as string).split(',')[1])
          reader.readAsDataURL(f)
          return false
        }} maxCount={1} accept="image/*">
          <Button>{imageB64 ? '已选图片 ✓' : '上传手写图片'}</Button>
        </Upload>
        <Button onClick={recordToText} danger={recording}>{recording ? '停止并转写' : '🎤 语音输入'}</Button>
        <Checkbox checked={voiceMode} onChange={e => setVoiceMode(e.target.checked)}>语音语域（追加 voice_hint）</Checkbox>
        <Checkbox checked={autoRead} onChange={e => setAutoRead(e.target.checked)}>自动朗读回复</Checkbox>
        <Button onClick={() => setMsgs([])}>清空会话</Button>
      </Space>
      {sysPrompt && (
        <Collapse style={{ marginTop: 16 }} items={[{
          key: 'sys', label: '查看实际 system prompt',
          children: <pre style={{ whiteSpace: 'pre-wrap' }}>{sysPrompt}</pre>,
        }]} />
      )}
    </div>
  )
}
