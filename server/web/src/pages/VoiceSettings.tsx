import { Button, Card, Form, Input, InputNumber, Select, Space, message } from 'antd'
import { useEffect, useRef, useState } from 'react'
import { get, post, postForm, put } from '../api'
import CandidateInput from '../components/CandidateInput'

type Voice = { id: string; name: string }

export default function VoiceSettings() {
  const [providers, setProviders] = useState<{ id: number; name: string }[]>([])
  const [sttModels, setSttModels] = useState<string[]>([])
  const [ttsModels, setTtsModels] = useState<string[]>([])
  const [voices, setVoices] = useState<Voice[]>([])
  const [recording, setRecording] = useState(false)
  const [sttResult, setSttResult] = useState('')
  const recRef = useRef<MediaRecorder | null>(null)
  const [form] = Form.useForm()

  const fetchModels = async (pid: number | undefined, set: (m: string[]) => void) => {
    set([])
    if (!pid) return
    try {
      const r = await post(`/api/admin/providers/${pid}/test`)
      if (r.ok) set(r.models)
    } catch { /* 拉不到就手填 */ }
  }

  const fetchVoices = async (pid: number | undefined) => {
    setVoices([])
    if (!pid) return
    try {
      const r = await get(`/api/admin/providers/${pid}/voices`)
      setVoices(r.voices)
    } catch { /* 拉不到就手填 */ }
  }

  useEffect(() => {
    get('/api/admin/providers').then(setProviders).catch(e => message.error(String(e)))
    get('/api/admin/voice-settings').then(v => {
      form.setFieldsValue(v)
      fetchModels(v.stt_provider_id, setSttModels)
      fetchModels(v.tts_provider_id, setTtsModels)
      fetchVoices(v.tts_provider_id)
    }).catch(e => message.error(String(e)))
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
      const mime = rec.mimeType || 'audio/webm'
      const ext = mime.includes('mp4') ? 'm4a' : mime.includes('ogg') ? 'ogg' : 'webm'
      const fd = new FormData()
      fd.append('audio', new Blob(chunks, { type: mime }), `test.${ext}`)
      // 试听即所见：带上页面当前值，改了不用先保存
      const cur = form.getFieldsValue(['stt_provider_id', 'stt_model'])
      if (cur.stt_provider_id != null) fd.append('stt_provider_id', String(cur.stt_provider_id))
      if (cur.stt_model) fd.append('stt_model', cur.stt_model)
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
    try {
      const text = form.getFieldValue('tts_test_text') || '你好，我是豆豆。'
      // 试听即所见：带上页面当前值，改了不用先保存
      const cur = form.getFieldsValue(['tts_provider_id', 'tts_model', 'tts_voice', 'tts_speed'])
      const resp = await fetch('/api/admin/voice/tts-test', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text, ...cur }),
      })
      if (!resp.ok) { message.error(await resp.text()); return }
      new Audio(URL.createObjectURL(await resp.blob())).play()
    } catch (e) {
      message.error(String(e))
    }
  }

  const providerOpts = providers.map(p => ({ value: p.id, label: p.name }))
  return (
    <Form form={form} layout="vertical" style={{ maxWidth: 560 }}>
      <Card title="语音识别（STT）" style={{ marginBottom: 16 }}>
        <Form.Item name="stt_provider_id" label="服务商">
          <Select options={providerOpts} allowClear
                  onChange={v => fetchModels(v, setSttModels)} />
        </Form.Item>
        <Form.Item name="stt_model" label={`模型（候选 ${sttModels.length} 个，点开选择或手填）`}>
          <CandidateInput options={sttModels.map(m => ({ value: m }))}
                          placeholder="如 qwen3-asr-flash-2026-02-10" />
        </Form.Item>
        <Space>
          <Button onClick={recordTest} danger={recording}>
            {recording ? '停止并转写' : '录一句测转写'}
          </Button>
          {sttResult && <span>转写结果：{sttResult}</span>}
        </Space>
      </Card>
      <Card title="语音合成（TTS）" style={{ marginBottom: 16 }}>
        <Form.Item name="tts_provider_id" label="服务商">
          <Select options={providerOpts} allowClear
                  onChange={v => { fetchModels(v, setTtsModels); fetchVoices(v) }} />
        </Form.Item>
        <Form.Item name="tts_model" label={`模型（候选 ${ttsModels.length} 个，点开选择或手填）`}>
          <CandidateInput options={ttsModels.map(m => ({ value: m }))}
                          placeholder="如 speech-2.6-hd" />
        </Form.Item>
        <Form.Item name="tts_voice" label={`音色（候选 ${voices.length} 个，点开选择，可搜索或手填）`}>
          <CandidateInput options={voices.map(v => ({ value: v.id, label: `${v.name}（${v.id}）` }))}
                          placeholder="如 lovely_girl 萌萌女童" />
        </Form.Item>
        <Form.Item name="tts_speed" label="语速"><InputNumber min={0.5} max={2} step={0.1} /></Form.Item>
        <Form.Item name="tts_test_text" label="试听文本"><Input placeholder="你好，我是豆豆。" /></Form.Item>
        <Button onClick={ttsTest}>试听音色</Button>
      </Card>
      <Button type="primary" onClick={save}>保存</Button>
    </Form>
  )
}
