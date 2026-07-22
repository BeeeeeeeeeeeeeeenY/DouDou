import { Button, Drawer, Table, Tag, message } from 'antd'
import { useEffect, useState } from 'react'
import { get } from '../api'

type Turn = {
  id: number; ts: string; source: string; profile_name: string; model: string
  system_prompt: string; input_text: string; input_image_path: string
  input_audio_path: string; transcript: string; reply_text: string
  reply_audio_path: string; latency_ms: number; status: string; error: string
}

const SOURCE = { tablet: '平板', test: '测试台', phone: '手机' } as Record<string, string>

export default function Turns() {
  const [rows, setRows] = useState<Turn[]>([])
  const [detail, setDetail] = useState<Turn | null>(null)

  useEffect(() => {
    get('/api/admin/turns?limit=100').then(r => setRows(r.items)).catch(e => message.error(String(e)))
  }, [])

  return (
    <>
      <Table rowKey="id" dataSource={rows} columns={[
        { title: '时间', dataIndex: 'ts', render: (v: string) => new Date(v + 'Z').toLocaleString('zh-CN') },
        { title: '来源', dataIndex: 'source', render: (v: string) => SOURCE[v] ?? v },
        { title: 'Profile', dataIndex: 'profile_name' },
        {
          title: '输入', render: (_, r) => (
            <span>
              {r.input_image_path && (
                <img src={`/api/files/${r.input_image_path}`} alt="" style={{ height: 40, marginRight: 8 }} />
              )}
              {r.transcript || r.input_text}
            </span>
          ),
        },
        { title: '回复', dataIndex: 'reply_text', ellipsis: true },
        { title: '延迟', dataIndex: 'latency_ms', render: (v: number) => `${v}ms` },
        {
          title: '状态', dataIndex: 'status',
          render: (v: string) => (v === 'ok' ? <Tag color="green">成功</Tag> : <Tag color="red">失败</Tag>),
        },
        { title: '', render: (_, r) => <Button size="small" onClick={() => setDetail(r)}>详情</Button> },
      ]} />
      <Drawer open={!!detail} width={640} title={`第 ${detail?.id} 轮`} onClose={() => setDetail(null)}>
        {detail && (
          <div style={{ display: 'grid', gap: 12 }}>
            {detail.input_image_path && <img src={`/api/files/${detail.input_image_path}`} alt="" style={{ maxWidth: '100%' }} />}
            {detail.input_audio_path && <audio controls src={`/api/files/${detail.input_audio_path}`} />}
            {detail.transcript && <p><b>转写：</b>{detail.transcript}</p>}
            <p style={{ whiteSpace: 'pre-wrap' }}><b>回复：</b>{detail.reply_text}</p>
            {detail.reply_audio_path && <audio controls src={`/api/files/${detail.reply_audio_path}`} />}
            {detail.error && <p style={{ color: 'red' }}><b>错误：</b>{detail.error}</p>}
            <p><b>模型：</b>{detail.model}　<b>延迟：</b>{detail.latency_ms}ms</p>
            <details><summary>system prompt</summary>
              <pre style={{ whiteSpace: 'pre-wrap' }}>{detail.system_prompt}</pre>
            </details>
          </div>
        )}
      </Drawer>
    </>
  )
}
