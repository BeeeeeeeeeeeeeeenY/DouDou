import { Alert, Button, Card, Drawer, Form, Input, InputNumber, Select, Space, Table, Tag, Typography, message } from 'antd'
import { useEffect, useState } from 'react'
import { get, post, put } from '../api'

type Curriculum = {
  id: number; slug: string; title: string; age_band: string
  description: string; status: string; current_lesson_id: number | null
}
type Lesson = {
  id: number; curriculum_id: number; seq: number; slug: string; title: string
  goal_text: string; script_text: string; duration_min: number; materials: string
}
type Run = {
  id: number; lesson_seq: number; lesson_title: string; curriculum_title: string
  started_at: string; status: string; highlights: string; parent_tip: string
  parent_note: string; artifact_images: string[]
}

const STATUS = { draft: '草稿', active: '生效中', archived: '已归档' } as Record<string, string>
const RUN_STATUS = {
  running: '进行中', completed: '完成', partial: '部分完成',
  skipped: '未参与', abandoned: '未收尾',
} as Record<string, string>
const RUN_COLOR = {
  running: 'blue', completed: 'green', partial: 'gold', skipped: 'default', abandoned: 'default',
} as Record<string, string>

const TABLET_PRAISE_RULE =
  '收到孩子的涂鸦画作时：回复第一句是具体的夸奖，必须点出画面里至少一个真实元素；' +
  '然后手写画一个小符号奖励（⭐、❤ 或 ☺）；最后可以加一个关于画的小问题。' +
  '全文不超过 3 行，不评价画得像不像，不提「图片/照片」。'

export default function Curricula() {
  const [curricula, setCurricula] = useState<Curriculum[]>([])
  const [lessons, setLessons] = useState<Lesson[]>([])
  const [selected, setSelected] = useState<Curriculum | null>(null)
  const [runs, setRuns] = useState<Run[]>([])
  const [editing, setEditing] = useState<Lesson | null>(null)
  const [form] = Form.useForm()

  const loadCurricula = async () => {
    try {
      const list: Curriculum[] = await get('/api/admin/curricula')
      setCurricula(list)
      const cur = list.find(c => selected && c.id === selected.id) ?? list.find(c => c.status === 'active') ?? list[0] ?? null
      setSelected(cur)
      if (cur) setLessons(await get(`/api/admin/curricula/${cur.id}/lessons`))
      else setLessons([])
    } catch (e) {
      message.error(String(e))
    }
  }
  const loadRuns = async () => {
    try {
      setRuns((await get('/api/admin/lesson-runs?limit=50')).items)
    } catch (e) {
      message.error(String(e))
    }
  }

  useEffect(() => { loadCurricula(); loadRuns() }, [])

  const seed = async () => {
    try {
      await post('/api/admin/curricula/seed-shapes01')
      message.success('已导入「形状小画家」示范课程')
      loadCurricula()
    } catch (e) {
      message.error(String(e))
    }
  }
  const activate = async (c: Curriculum) => {
    try {
      await post(`/api/admin/curricula/${c.id}/activate`)
      loadCurricula()
    } catch (e) {
      message.error(String(e))
    }
  }
  const setPointer = async (lessonId: number | null) => {
    if (!selected) return
    try {
      await put(`/api/admin/curricula/${selected.id}/pointer`, { lesson_id: lessonId })
      message.success('当前课时已更新')
      loadCurricula()
    } catch (e) {
      message.error(String(e))
    }
  }
  const pickCurriculum = async (c: Curriculum) => {
    try {
      setSelected(c)
      setLessons(await get(`/api/admin/curricula/${c.id}/lessons`))
    } catch (e) {
      message.error(String(e))
    }
  }
  const saveLesson = async () => {
    if (!editing) return
    let values
    try { values = await form.validateFields() } catch { return }
    try {
      await put(`/api/admin/lessons/${editing.id}`, values)
      setEditing(null)
      message.success('课时已保存')
      if (selected) setLessons(await get(`/api/admin/curricula/${selected.id}/lessons`))
    } catch (e) {
      message.error(String(e))
    }
  }
  const fixRun = async (r: Run, patch: object) => {
    try {
      await put(`/api/admin/lesson-runs/${r.id}`, patch)
      loadRuns()
    } catch (e) {
      message.error(String(e))
    }
  }

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Alert type="info" showIcon message="平板夸奖规则（复制到 3-4 岁 profile 的人设文本末尾，让平板提交轮的手写回复符合课程设计）"
        description={<Typography.Paragraph copyable style={{ marginBottom: 0 }}>{TABLET_PRAISE_RULE}</Typography.Paragraph>} />

      <Card title="课程" extra={<Button onClick={seed}>一键导入示范课程「形状小画家」</Button>}>
        <Table rowKey="id" dataSource={curricula} pagination={false}
          onRow={c => ({ onClick: () => pickCurriculum(c) })}
          columns={[
            { title: '名称', dataIndex: 'title' },
            { title: '年龄段', dataIndex: 'age_band' },
            { title: '状态', dataIndex: 'status', render: (v: string) => <Tag color={v === 'active' ? 'green' : 'default'}>{STATUS[v] ?? v}</Tag> },
            {
              title: '当前课时', render: (_, c) => {
                const l = lessons.find(x => x.id === c.current_lesson_id)
                if (c.id !== selected?.id) return c.current_lesson_id ?? '—'
                return l ? `第 ${l.seq} 课 ${l.title}` : '（本轮已完成）'
              },
            },
            {
              title: '操作', render: (_, c) => (
                <Button size="small" disabled={c.status === 'active'} onClick={e => { e.stopPropagation(); activate(c) }}>设为生效</Button>
              ),
            },
          ]} />
      </Card>

      {selected && (
        <Card title={`课时（${selected.title}）`}
          extra={
            <Space>
              <span>当前课时：</span>
              <Select style={{ width: 220 }} value={selected.current_lesson_id}
                onChange={v => setPointer(v)} allowClear placeholder="（本轮已完成）"
                options={lessons.map(l => ({ value: l.id, label: `第 ${l.seq} 课 ${l.title}` }))} />
            </Space>
          }>
          <Table rowKey="id" dataSource={lessons} pagination={false} columns={[
            { title: '#', dataIndex: 'seq', width: 50 },
            { title: '课名', dataIndex: 'title' },
            { title: '目标', dataIndex: 'goal_text', ellipsis: true },
            { title: '时长', dataIndex: 'duration_min', width: 70, render: (v: number) => `${v}'` },
            {
              title: '', width: 80, render: (_, l) => (
                <Button size="small" onClick={() => { setEditing(l); form.setFieldsValue(l) }}>编辑</Button>
              ),
            },
          ]} />
        </Card>
      )}

      <Card title="上课记录">
        <Table rowKey="id" dataSource={runs} columns={[
          { title: '时间', dataIndex: 'started_at', render: (v: string) => (v ? new Date(v + 'Z').toLocaleString('zh-CN') : '') },
          { title: '课时', render: (_, r) => `第 ${r.lesson_seq} 课 ${r.lesson_title}` },
          {
            title: '状态', dataIndex: 'status', render: (v: string, r) => (
              <Select size="small" value={v} style={{ width: 110 }}
                onChange={s => fixRun(r, { status: s })}
                options={Object.entries(RUN_STATUS).map(([value, label]) => ({ value, label }))}
                labelRender={() => <Tag color={RUN_COLOR[v]}>{RUN_STATUS[v] ?? v}</Tag>} />
            ),
          },
          { title: '亮点', dataIndex: 'highlights', ellipsis: true },
          { title: '在家延伸', dataIndex: 'parent_tip', ellipsis: true },
          {
            title: '作品', render: (_, r) => (
              <Space>
                {r.artifact_images.map(p => (
                  <img key={p} src={`/api/files/${p}`} alt="" style={{ height: 40 }} />
                ))}
              </Space>
            ),
          },
          {
            title: '家长补记', dataIndex: 'parent_note', render: (v: string, r) => (
              <Typography.Text editable={{ onChange: t => fixRun(r, { parent_note: t }) }}>{v}</Typography.Text>
            ),
          },
        ]} />
      </Card>

      <Drawer open={!!editing} width={720} title={editing ? `第 ${editing.seq} 课 ${editing.title}` : ''}
        onClose={() => setEditing(null)}
        extra={<Button type="primary" onClick={saveLesson}>保存</Button>}>
        <Form form={form} layout="vertical">
          <Form.Item name="title" label="课名"><Input /></Form.Item>
          <Form.Item name="goal_text" label="教学目标"><Input.TextArea rows={2} /></Form.Item>
          <Form.Item name="script_text" label="课时脚本（注入语音轮 system prompt；{prev_lesson_recap} 会替换为上次课小结）">
            <Input.TextArea rows={16} />
          </Form.Item>
          <Form.Item name="materials" label="课前准备（家长可见）"><Input.TextArea rows={2} /></Form.Item>
          <Form.Item name="duration_min" label="目标时长（分钟）"><InputNumber min={1} max={30} /></Form.Item>
        </Form>
      </Drawer>
    </Space>
  )
}
