from app.engine.prompt import assemble_system_prompt, split_protocol_suffix

DEVICE_SYSTEM = "你是设备内置人设。\n\n记忆协议：系统会给你一个记忆目录，⟦show:N⟧ 规则等等。"


def test_split_finds_protocol_suffix():
    persona, suffix = split_protocol_suffix(DEVICE_SYSTEM)
    assert persona == "你是设备内置人设。"
    assert suffix.startswith("\n\n记忆协议：")
    assert "⟦show:N⟧" in suffix


def test_split_without_marker():
    persona, suffix = split_protocol_suffix("只有人设没有协议")
    assert persona == "只有人设没有协议"
    assert suffix == ""


def test_assemble_persona_only():
    assert assemble_system_prompt("  你是 DouDou。  ") == "你是 DouDou。"


def test_assemble_with_voice_hint_and_suffix():
    out = assemble_system_prompt(
        "你是 DouDou。", voice_hint="这是语音对话，口语化。", protocol_suffix="\n\n记忆协议：xxx"
    )
    assert out == "你是 DouDou。\n\n这是语音对话，口语化。\n\n记忆协议：xxx"


def test_assemble_blank_voice_hint_ignored():
    assert assemble_system_prompt("你是 DouDou。", voice_hint="   ") == "你是 DouDou。"
