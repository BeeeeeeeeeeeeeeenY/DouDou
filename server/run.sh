#!/bin/bash
# DouDou Server：http 8787（平板+管理）；server/certs/ 有 mkcert 证书时加开 https 8788（手机页）
cd "$(dirname "$0")"
set -e

PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
trap cleanup EXIT

uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8787 &
PIDS+=($!)

if [[ -f certs/cert.pem && -f certs/key.pem ]]; then
  uv run uvicorn --factory app.main:create_app --host 0.0.0.0 --port 8788 \
    --ssl-certfile certs/cert.pem --ssl-keyfile certs/key.pem &
  PIDS+=($!)
  echo "https（手机页）: https://$(ipconfig getifaddr en0 2>/dev/null || echo localhost):8788/phone"
else
  echo "未发现 certs/，仅启动 http。手机页需 https（麦克风权限），配置方法见 README。"
fi
echo "http（平板+管理）: http://$(ipconfig getifaddr en0 2>/dev/null || echo localhost):8787/admin"
wait
