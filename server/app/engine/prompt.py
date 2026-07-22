from datetime import datetime

PROTOCOL_MARKER = "\n\n记忆协议："

WEEKDAYS = ("星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日")


def time_context(now: datetime | None = None) -> str:
    """注入当前时间，让模型能直接回答"现在几点/今天几号/星期几"。"""
    n = now or datetime.now().astimezone()
    return (f"当前时间：{n.year}年{n.month}月{n.day}日 {WEEKDAYS[n.weekday()]} "
            f"{n.hour:02d}:{n.minute:02d}。孩子问时间、日期、星期时可直接回答。")


def split_protocol_suffix(device_system: str) -> tuple[str, str]:
    """把设备 system prompt 切成 (设备人设, 记忆协议后缀)。无标记时后缀为空。"""
    idx = device_system.find(PROTOCOL_MARKER)
    if idx == -1:
        return device_system, ""
    return device_system[:idx], device_system[idx:]


def assemble_system_prompt(
    persona: str, *, voice_hint: str = "", time_line: str = "",
    lesson_context: str = "", protocol_suffix: str = ""
) -> str:
    out = persona.strip()
    if voice_hint.strip():
        out += "\n\n" + voice_hint.strip()
    if time_line:
        out += "\n\n" + time_line
    if lesson_context.strip():
        out += "\n\n" + lesson_context.strip()
    if protocol_suffix:
        out += protocol_suffix  # 后缀自带 \n\n 前导
    return out
