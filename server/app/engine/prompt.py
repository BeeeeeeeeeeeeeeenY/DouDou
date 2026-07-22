PROTOCOL_MARKER = "\n\n记忆协议："


def split_protocol_suffix(device_system: str) -> tuple[str, str]:
    """把设备 system prompt 切成 (设备人设, 记忆协议后缀)。无标记时后缀为空。"""
    idx = device_system.find(PROTOCOL_MARKER)
    if idx == -1:
        return device_system, ""
    return device_system[:idx], device_system[idx:]


def assemble_system_prompt(
    persona: str, *, voice_hint: str = "", lesson_context: str = "", protocol_suffix: str = ""
) -> str:
    out = persona.strip()
    if voice_hint.strip():
        out += "\n\n" + voice_hint.strip()
    if lesson_context.strip():
        out += "\n\n" + lesson_context.strip()
    if protocol_suffix:
        out += protocol_suffix  # 后缀自带 \n\n 前导
    return out
