#!/bin/zsh
set -euo pipefail

DEVICE_HOST="10.11.99.1"
DEVICE_USER="root"
SSH_KEY="/Users/ben/.ssh/remarkable_paper_pro"
KNOWN_HOSTS="/Users/ben/.ssh/known_hosts_remarkable"
REMOTE_CONFIG="/home/root/xovi/exthome/appload/riddle/oracle.env"
DEFAULT_API_BASE="https://dashscope.aliyuncs.com/compatible-mode/v1"

finish() {
  unset api_key 2>/dev/null || true
}
trap finish EXIT

echo "Riddle × Qwen3-VL-Plus 配置"
echo ""
echo "请准备百炼创建 API Key 时显示的两项内容："
echo "  1. API Key（输入时不会显示）"
echo "  2. OpenAI 兼容地址 / Base URL（可直接按回车使用北京 DashScope 地址）"
echo ""

printf "OpenAI 兼容地址 / Base URL [%s]: " "$DEFAULT_API_BASE"
IFS= read -r api_base
api_base="${api_base:-$DEFAULT_API_BASE}"
api_base="${api_base%/}"
api_base="${api_base%/chat/completions}"
api_base="${api_base%/}"

if [[ "$api_base" != https://* ]]; then
  echo "错误：OpenAI 兼容地址必须以 https:// 开头。"
  echo "按回车关闭。"
  IFS= read -r
  exit 1
fi

printf "API Key（隐藏输入）: "
IFS= read -r -s api_key
echo ""

if [[ -z "$api_key" ]]; then
  echo "错误：没有输入 API Key。"
  echo "按回车关闭。"
  IFS= read -r
  exit 1
fi

echo "正在把配置安全写入 reMarkable……"
{
  printf 'RIDDLE_OPENAI_KEY=%s\n' "$api_key"
  printf 'RIDDLE_OPENAI_BASE=%s\n' "$api_base"
  printf 'RIDDLE_OPENAI_MODEL=qwen3-vl-plus\n'
  printf 'RIDDLE_OPENAI_MAX_TOKENS=1000\n'
  printf 'RIDDLE_TZ_OFFSET=8\n'
} | ssh \
  -i "$SSH_KEY" \
  -o BatchMode=yes \
  -o ConnectTimeout=8 \
  -o StrictHostKeyChecking=yes \
  -o UserKnownHostsFile="$KNOWN_HOSTS" \
  "$DEVICE_USER@$DEVICE_HOST" \
  'set -eu; tmp="/home/root/xovi/exthome/appload/riddle/oracle.env.new"; umask 077; cat > "$tmp"; chmod 600 "$tmp"; mv "$tmp" "/home/root/xovi/exthome/appload/riddle/oracle.env"'

unset api_key

ssh \
  -i "$SSH_KEY" \
  -o BatchMode=yes \
  -o ConnectTimeout=8 \
  -o StrictHostKeyChecking=yes \
  -o UserKnownHostsFile="$KNOWN_HOSTS" \
  "$DEVICE_USER@$DEVICE_HOST" \
  'test -s "/home/root/xovi/exthome/appload/riddle/oracle.env" && grep -q "^RIDDLE_OPENAI_MODEL=qwen3-vl-plus$" "/home/root/xovi/exthome/appload/riddle/oracle.env"'

echo ""
echo "配置完成：模型为 qwen3-vl-plus，密钥只保存在设备上。"
echo "请回到 Codex 告诉我“配置好了”，我会做连通性测试。"
echo ""
echo "按回车关闭窗口。"
IFS= read -r
