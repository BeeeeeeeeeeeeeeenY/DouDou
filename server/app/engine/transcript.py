TRANSCRIPT_MARK = "⁂"  # U+2042，riddle 记忆协议的转写后缀标记


def split_transcript(full_text: str) -> tuple[str, str]:
    if TRANSCRIPT_MARK not in full_text:
        return full_text.strip(), ""
    visible, _, post = full_text.partition(TRANSCRIPT_MARK)
    return visible.strip(), post.strip()
