import { Layout, Menu } from 'antd'
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom'
import Phone from './pages/Phone'
import Profiles from './pages/Profiles'
import Providers from './pages/Providers'
import TestBench from './pages/TestBench'
import Turns from './pages/Turns'
import VoiceSettings from './pages/VoiceSettings'

const MENU = [
  { key: '/admin/providers', label: '模型配置' },
  { key: '/admin/profiles', label: '人设 Profile' },
  { key: '/admin/test', label: '测试台' },
  { key: '/admin/voice', label: '语音配置' },
  { key: '/admin/turns', label: '对话记录' },
]

function AdminShell() {
  const nav = useNavigate()
  const loc = useLocation()
  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Layout.Sider theme="light">
        <div style={{ padding: 16, fontWeight: 700, fontSize: 18 }}>DouDou 后台</div>
        <Menu
          items={MENU}
          selectedKeys={[loc.pathname]}
          onClick={(e) => nav(e.key)}
        />
      </Layout.Sider>
      <Layout.Content style={{ padding: 24 }}>
        <Routes>
          <Route path="providers" element={<Providers />} />
          <Route path="profiles" element={<Profiles />} />
          <Route path="test" element={<TestBench />} />
          <Route path="voice" element={<VoiceSettings />} />
          <Route path="turns" element={<Turns />} />
          <Route path="*" element={<Navigate to="providers" replace />} />
        </Routes>
      </Layout.Content>
    </Layout>
  )
}

export default function App() {
  return (
    <Routes>
      <Route path="/phone" element={<Phone />} />
      <Route path="/admin/*" element={<AdminShell />} />
      <Route path="*" element={<Navigate to="/admin/providers" replace />} />
    </Routes>
  )
}
