# DouDou Server（一期）

本地 Mac 上的 DouDou 大脑：OpenAI 兼容门面（平板零改动接入）+ 中文管理后台 + 手机按住说话页。
设计文档：`docs/superpowers/specs/2026-07-22-doudou-server-phase1-design.md`。

## 运行

```sh
cd server
uv sync
(cd web && npm install && npm run build)   # 构建管理界面
./run.sh
```

- 管理后台：`http://<Mac IP>:8787/admin`
- 手机页：`https://<Mac IP>:8788/phone`（需先配好 https，见下）

## 平板接入（reMarkable / riddle）

改设备上 riddle 目录里的 `oracle.env` 两行，Rust 不用动：

```sh
RIDDLE_OPENAI_KEY="doudou"                      # 非空即可，服务器不校验
RIDDLE_OPENAI_BASE="http://<Mac IP>:8787/v1"
```

先在后台配好 provider 和生效 profile，再在平板上写一笔验证。

## 手机页的 https（麦克风权限要求）

手机浏览器只在 https 下允许录音。用 mkcert 给局域网地址签本地证书：

```sh
brew install mkcert && mkcert -install
mkdir -p certs
mkcert -cert-file certs/cert.pem -key-file certs/key.pem "$(ipconfig getifaddr en0)" localhost
```

手机需安装 mkcert 的根证书（`mkcert -CAROOT` 目录下的 `rootCA.pem` 发到手机安装并信任），
然后访问 `https://<Mac IP>:8788/phone`。

## 安全前提

仅限局域网个人部署：无登录鉴权，API key 明文存 `server/data/doudou.db`。
不要将 8787/8788 暴露到公网。

## 测试

```sh
cd server && uv run pytest
```

## 课程（形状小画家）

1. 后台「课程」页点「一键导入示范课程」，再点「设为生效」。
2. 把页面顶部的「平板夸奖规则」复制进 3-4 岁 profile 的人设文本末尾（平板提交轮的手写夸奖靠它）。
3. 手机页出现「开始上课」按钮：点开始 → 按住说话跟着豆豆走五环节 →
   孩子画完后在平板上把画发给豆豆（作品会按时间自动挂到本课记录）→
   课程收尾后手机页出现小结气泡，后台「课程」页可看每课的亮点、在家延伸与作品。
4. 中途退出：点课程条上的「结束」（记录状态为「未收尾」，家长可在后台修正）。

设计文档：`docs/superpowers/specs/2026-07-22-mini-curriculum-design.md`
