import { Button, Form, Input, Modal, Space, Switch, Table, message } from 'antd'
import { useEffect, useState } from 'react'
import { del, get, post, put } from '../api'

type Provider = {
  id: number; name: string; base_url: string; api_key: string
  enabled: boolean; notes: string
}

export default function Providers() {
  const [rows, setRows] = useState<Provider[]>([])
  const [editing, setEditing] = useState<Partial<Provider> | null>(null)
  const [testing, setTesting] = useState<number | null>(null)
  const [form] = Form.useForm()

  const reload = () => get('/api/admin/providers').then(setRows)
  useEffect(() => { reload() }, [])

  const save = async () => {
    const v = await form.validateFields()
    if (editing?.id) await put(`/api/admin/providers/${editing.id}`, v)
    else await post('/api/admin/providers', v)
    setEditing(null)
    reload()
  }

  const test = async (id: number) => {
    setTesting(id)
    try {
      const r = await post(`/api/admin/providers/${id}/test`)
      if (r.ok) message.success(`连通正常，${r.latency_ms}ms，${r.models.length} 个模型`)
      else message.error(`连通失败：${r.error}`)
    } finally { setTesting(null) }
  }

  return (
    <>
      <Space style={{ marginBottom: 16 }}>
        <Button type="primary" onClick={() => { form.resetFields(); setEditing({}) }}>
          新增 Provider
        </Button>
      </Space>
      <Table rowKey="id" dataSource={rows} pagination={false} columns={[
        { title: '名称', dataIndex: 'name' },
        { title: 'Base URL', dataIndex: 'base_url' },
        { title: '启用', dataIndex: 'enabled', render: (v: boolean) => (v ? '是' : '否') },
        { title: '备注', dataIndex: 'notes' },
        {
          title: '操作',
          render: (_, r) => (
            <Space>
              <Button size="small" loading={testing === r.id} onClick={() => test(r.id)}>测试连通</Button>
              <Button size="small" onClick={() => { form.setFieldsValue(r); setEditing(r) }}>编辑</Button>
              <Button size="small" danger onClick={async () => { await del(`/api/admin/providers/${r.id}`); reload() }}>删除</Button>
            </Space>
          ),
        },
      ]} />
      <Modal open={!!editing} title={editing?.id ? '编辑 Provider' : '新增 Provider'}
             onOk={save} onCancel={() => setEditing(null)} destroyOnClose>
        <Form form={form} layout="vertical" initialValues={{ enabled: true }}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="base_url" label="Base URL（如 https://api.openai.com/v1）"
                     rules={[{ required: true }]}><Input /></Form.Item>
          <Form.Item name="api_key" label="API Key"><Input.Password /></Form.Item>
          <Form.Item name="enabled" label="启用" valuePropName="checked"><Switch /></Form.Item>
          <Form.Item name="notes" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </>
  )
}
