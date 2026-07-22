import { AutoComplete } from 'antd'
import { useState } from 'react'

type Opt = { value: string; label?: string }

const contains = (kw: string, text: string) => text.toLowerCase().includes(kw.toLowerCase())

/**
 * 组合框：点开先显示全部候选，输入才按关键字过滤，也允许手填任意值。
 * （直接用 AutoComplete + filterOption 时，已保存的值会把候选过滤成空，看起来像没有下拉。）
 */
export default function CandidateInput({ options, placeholder, value, onChange }: {
  options: Opt[]
  placeholder?: string
  value?: string
  onChange?: (v: string) => void
}) {
  const [kw, setKw] = useState('')
  const [open, setOpen] = useState(false)
  const shown = kw
    ? options.filter(o => contains(kw, `${o.label ?? ''}${o.value}`))
    : options
  return (
    <AutoComplete
      value={value}
      onChange={v => onChange?.(v)}
      options={shown}
      onSearch={setKw}
      // AutoComplete 默认只在输入时弹面板；受控 open 让「点开就能选」成立
      open={open}
      onOpenChange={setOpen}
      // 受控 open 下 rc-motion 进场动画可能冻结在第一帧（面板 0 宽高），禁用过渡
      transitionName=""
      onFocus={() => { setKw(''); setOpen(true) }}
      onBlur={() => setOpen(false)}
      allowClear
      placeholder={placeholder}
    />
  )
}
