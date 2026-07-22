import { Button, Form, Input, InputNumber, Modal, Select, Space, Table, Tag, message } from 'antd'
import CandidateInput from '../components/CandidateInput'
import { useEffect, useState } from 'react'
import { del, get, post, put } from '../api'

type Profile = {
  id: number; name: string; age_band: string; persona_text: string; voice_hint: string
  provider_id: number | null; model: string; temperature: number | null
  max_tokens: number; reasoning_effort: string; is_active: boolean
}

export default function Profiles() {
  const [rows, setRows] = useState<Profile[]>([])
  const [providers, setProviders] = useState<{ id: number; name: string }[]>([])
  const [models, setModels] = useState<string[]>([])
  const [editing, setEditing] = useState<Partial<Profile> | null>(null)
  const [form] = Form.useForm()

  const reload = () => get('/api/admin/profiles').then(setRows)
  useEffect(() => {
    reload()
    get('/api/admin/providers').then(setProviders)
  }, [])

  const fetchModels = async (pid?: number) => {
    setModels([])
    if (!pid) return
    try {
      const r = await post(`/api/admin/providers/${pid}/test`)
      if (r.ok) setModels(r.models)
    } catch { /* 拉不到就手填 */ }
  }

  const save = async () => {
    const v = await form.validateFields()
    try {
      if (editing?.id) await put(`/api/admin/profiles/${editing.id}`, v)
      else await post('/api/admin/profiles', v)
      setEditing(null)
      reload()
    } catch (e) {
      message.error(String(e))
    }
  }

  return (
    <>
      <Space style={{ marginBottom: 16 }}>
        <Button type="primary" onClick={() => { form.resetFields(); setEditing({}) }}>新增 Profile</Button>
      </Space>
      <Table rowKey="id" dataSource={rows} pagination={false} columns={[
        {
          title: '名称', dataIndex: 'name',
          render: (v, r) => <>{v} {r.is_active && <Tag color="green">生效中</Tag>}</>,
        },
        { title: '年龄段', dataIndex: 'age_band' },
        { title: '模型', dataIndex: 'model' },
        {
          title: '操作',
          render: (_, r) => (
            <Space>
              {!r.is_active && (
                <Button size="small" type="primary" onClick={async () => {
                  try {
                    await post(`/api/admin/profiles/${r.id}/activate`)
                    message.success(`「${r.name}」已生效，平板与手机立即使用`)
                    reload()
                  } catch (e) {
                    message.error(String(e))
                  }
                }}>设为生效</Button>
              )}
              <Button size="small" onClick={() => {
                form.setFieldsValue(r); setEditing(r); fetchModels(r.provider_id ?? undefined)
              }}>编辑</Button>
              <Button size="small" danger onClick={async () => { try { await del(`/api/admin/profiles/${r.id}`); reload() } catch (e) { message.error(String(e)) } }}>删除</Button>
            </Space>
          ),
        },
      ]} />
      <Modal open={!!editing} width={720} title={editing?.id ? '编辑 Profile' : '新增 Profile'}
             onOk={save} onCancel={() => setEditing(null)} destroyOnClose>
        <Form form={form} layout="vertical" initialValues={{ max_tokens: 2000, reasoning_effort: '' }}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="age_band" label="年龄段">
            <Select options={['3-4', '5-6', '6-7'].map(v => ({ value: v, label: `${v} 岁` }))} allowClear />
          </Form.Item>
          <Form.Item name="persona_text" label="人设提示词" rules={[{ required: true }]}>
            <Input.TextArea rows={10} placeholder="你是 DouDou……" />
          </Form.Item>
          <Form.Item name="voice_hint" label="语音补充提示词（语音对话时追加）">
            <Input.TextArea rows={3} placeholder="这是语音对话，回复要口语化、更短……" />
          </Form.Item>
          <Form.Item name="provider_id" label="模型服务" rules={[{ required: true }]}>
            <Select options={providers.map(p => ({ value: p.id, label: p.name }))}
                    onChange={(v) => fetchModels(v)} />
          </Form.Item>
          <Form.Item name="model" label={`模型（候选 ${models.length} 个，点开选择或手填）`} rules={[{ required: true }]}>
            <CandidateInput options={models.map(m => ({ value: m }))}
                            placeholder="qwen3-vl-plus" />
          </Form.Item>
          <Form.Item name="temperature" label="temperature（留空用默认）"><InputNumber min={0} max={2} step={0.1} /></Form.Item>
          <Form.Item name="max_tokens" label="max_tokens"><InputNumber min={100} max={20000} /></Form.Item>
          <Form.Item name="reasoning_effort" label="思考力度（仅思考模型）">
            <Select options={[{ value: '', label: '不设置' }, { value: 'low', label: 'low' },
                              { value: 'medium', label: 'medium' }, { value: 'high', label: 'high' }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  )
}
