#!/bin/bash
# 保持手机的 adb 反向转发常在（手机 localhost:PORT -> Mac:PORT）。
# 用法：./keep-phone-link.sh [设备序列号] [端口]
# USB 拔插 / adb 重启 / 手机重启都会清掉 reverse，本脚本每 5 秒自检自愈。
SERIAL="${1:-59160DLCQ007QY}"
PORT="${2:-8789}"
echo "守护中：$SERIAL localhost:$PORT → Mac:$PORT（Ctrl+C 退出）"
while true; do
  if adb -s "$SERIAL" get-state >/dev/null 2>&1; then
    if ! adb -s "$SERIAL" reverse --list 2>/dev/null | grep -q "tcp:$PORT"; then
      adb -s "$SERIAL" reverse "tcp:$PORT" "tcp:$PORT" >/dev/null 2>&1 \
        && echo "$(date +%T) 转发已恢复"
    fi
  fi
  sleep 5
done
