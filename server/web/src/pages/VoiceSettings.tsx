import { Button, Card, Form, Input, InputNumber, Select, Space, message } from 'antd'
import { useEffect, useRef, useState } from 'react'
import { get, postForm, put } from '../api'

export default function VoiceSettings() {
  const [providers, setProviders] = useState<{ id: number; name: string }[]>([])
  const [recording, setRecording] = useState(false)
  const [sttResult, setSttResult] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const [form] = Form.useForm()

  useEffect(() => {
    get('/api/admin/providers').then(setProviders)
    get('/api/admin/voice-settings').then(v => form.setFieldsValue(v))
  }, [form])

  const save = async () => {
    const v = await form.validateFields()
    try {
      await put('/api/admin/voice-settings', v)
      message.success('已保存')
    } catch (e) { message.error(String(e)) }
  }

  const recordTest = async () => {
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
      fd.append('audio', new Blob(chunks, { type: 'audio/webm' }), 'test.webm')
      try {
        const r = await postForm('/api/admin/voice/stt-test', fd)
        setSttResult(r.text)
      } catch (e) { message.error(String(e)) }
    }
    recRef.current = rec
    rec.start()
    setRecording(true)
  }

  const ttsTest = async () => {
    const text = form.getFieldValue('tts_test_text') || '你好，我是豆豆。'
    const resp = await fetch('/api/admin/voice/tts-test', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text }),
    })
    if (!resp.ok) { message.error(await resp.text()); return }
    new Audio(URL.createObjectURL(await resp.blob())).play()
  }

  const providerOpts = providers.map(p => ({ value: p.id, label: p.name }))
  return (
    <Form form={form} layout="vertical" style={{ maxWidth: 560 }}>
      <Card title="语音识别（STT）" style={{ marginBottom: 16 }}>
        <Form.Item name="stt_provider_id" label="Provider"><Select options={providerOpts} allowClear /></Form.Item>
        <Form.Item name="stt_model" label="模型（如 whisper-1）"><Input /></Form.Item>
        <Space>
          <Button onClick={recordTest} danger={recording}>
            {recording ? '停止并转写' : '录一句测转写'}
          </Button>
          {sttResult && <span>转写结果：{sttResult}</span>}
        </Space>
      </Card>
      <Card title="语音合成（TTS）" style={{ marginBottom: 16 }}>
        <Form.Item name="tts_provider_id" label="Provider"><Select options={providerOpts} allowClear /></Form.Item>
        <Form.Item name="tts_model" label="模型（如 tts-1）"><Input /></Form.Item>
        <Form.Item name="tts_voice" label="音色（如 alloy）"><Input /></Form.Item>
        <Form.Item name="tts_speed" label="语速"><InputNumber min={0.5} max={2} step={0.1} /></Form.Item>
        <Form.Item name="tts_test_text" label="试听文本"><Input placeholder="你好，我是豆豆。" /></Form.Item>
        <Button onClick={ttsTest}>试听音色</Button>
      </Card>
      <Button type="primary" onClick={save}>保存</Button>
    </Form>
  )
}
