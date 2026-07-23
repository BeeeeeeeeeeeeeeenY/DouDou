"""精选彩图库：subject → 预生成的 PNG（提交进仓库，见 scripts/gen_art.py）。
运行期只读文件 + base64，无图像库依赖。"""
import base64
from functools import lru_cache
from pathlib import Path

IMAGE_SUBJECTS: tuple[str, ...] = (
    "circle", "square", "triangle", "star", "heart", "sun", "flower", "tree",
    "apple", "fish", "house", "car", "cat", "moon", "butterfly",
)
_ART_DIR = Path(__file__).resolve().parent.parent / "art"


@lru_cache(maxsize=len(IMAGE_SUBJECTS))
def load_art_png(subject: str) -> bytes | None:
    if subject not in IMAGE_SUBJECTS:
        return None
    p = _ART_DIR / f"{subject}.png"
    return p.read_bytes() if p.is_file() else None


def subject_data_b64(subject: str) -> str | None:
    png = load_art_png(subject)
    return base64.b64encode(png).decode() if png is not None else None
