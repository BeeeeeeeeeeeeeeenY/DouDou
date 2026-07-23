"""一次性生成扁平大色块彩图（配形状课），输出到 app/art/<subject>.png。
运行：cd server && uv run --with pillow python scripts/gen_art.py
运行期服务器不依赖 Pillow——这些 PNG 提交进仓库，art.py 只读文件。"""
import math
from pathlib import Path
from PIL import Image, ImageDraw

W, H = 500, 400
OUT = Path(__file__).resolve().parent.parent / "app" / "art"
WHITE = (255, 255, 255)


def _star_points(cx, cy, r_out, r_in, n=5):
    pts = []
    for i in range(n * 2):
        r = r_out if i % 2 == 0 else r_in
        a = -math.pi / 2 + i * math.pi / n
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def draw(subject, d):
    # 配色为彩色墨水屏（Spectra 6）调亮：高亮度、高饱和、贴近面板原色
    # （红/黄/蓝/绿），避免暗/浊色在纸上发闷。
    cx, cy = W // 2, H // 2
    if subject == "circle":
        d.ellipse([cx - 150, cy - 150, cx + 150, cy + 150], fill=(255, 45, 45))
    elif subject == "square":
        d.rectangle([cx - 140, cy - 140, cx + 140, cy + 140], fill=(30, 115, 240))
    elif subject == "triangle":
        d.polygon([(cx, cy - 160), (cx - 160, cy + 130), (cx + 160, cy + 130)], fill=(45, 205, 80))
    elif subject == "star":
        d.polygon(_star_points(cx, cy, 170, 70), fill=(255, 225, 20))
    elif subject == "heart":
        d.ellipse([cx - 150, cy - 120, cx, cy + 30], fill=(255, 80, 150))
        d.ellipse([cx, cy - 120, cx + 150, cy + 30], fill=(255, 80, 150))
        d.polygon([(cx - 150, cy - 30), (cx + 150, cy - 30), (cx, cy + 160)], fill=(255, 80, 150))
    elif subject == "sun":
        for i in range(12):
            a = i * math.pi / 6
            d.line([cx + 100 * math.cos(a), cy + 100 * math.sin(a),
                    cx + 180 * math.cos(a), cy + 180 * math.sin(a)], fill=(255, 180, 10), width=18)
        d.ellipse([cx - 100, cy - 100, cx + 100, cy + 100], fill=(255, 220, 20))
    elif subject == "flower":
        for i in range(6):
            a = i * math.pi / 3
            px, py = cx + 90 * math.cos(a), cy + 90 * math.sin(a)
            d.ellipse([px - 55, py - 55, px + 55, py + 55], fill=(255, 95, 180))
        d.ellipse([cx - 50, cy - 50, cx + 50, cy + 50], fill=(255, 225, 20))
    elif subject == "tree":
        d.rectangle([cx - 30, cy + 40, cx + 30, cy + 170], fill=(170, 110, 45))
        d.ellipse([cx - 130, cy - 170, cx + 130, cy + 70], fill=(45, 205, 80))
    else:
        raise ValueError(subject)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    subjects = ["circle", "square", "triangle", "star", "heart", "sun", "flower", "tree"]
    for s in subjects:
        img = Image.new("RGB", (W, H), WHITE)
        draw(s, ImageDraw.Draw(img))
        img.save(OUT / f"{s}.png")
        print("wrote", OUT / f"{s}.png")


if __name__ == "__main__":
    main()
