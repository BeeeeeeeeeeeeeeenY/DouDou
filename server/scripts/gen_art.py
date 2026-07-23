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
    # 配色按真机色卡标定（--swatch-test，2026-07-23）：这块彩色墨水屏经 mode 5
    # 只有蓝/绿/黄能渲染成干净正色，暖色（红/橙/粉）会偏紫/土——已接受（方案 A）。
    # 蓝/绿/黄主体用标定出的最佳值；暖色主体取合理值，接受其偏色。
    BLUE = (30, 60, 180)     # 标定：干净正蓝
    GREEN = (35, 150, 65)    # 标定：偏青的绿，最饱和
    YELLOW = (255, 225, 20)  # 标定：清爽淡黄
    cx, cy = W // 2, H // 2
    if subject == "circle":
        d.ellipse([cx - 150, cy - 150, cx + 150, cy + 150], fill=(215, 45, 45))  # 暖：偏紫，接受
    elif subject == "square":
        d.rectangle([cx - 140, cy - 140, cx + 140, cy + 140], fill=BLUE)
    elif subject == "triangle":
        d.polygon([(cx, cy - 160), (cx - 160, cy + 130), (cx + 160, cy + 130)], fill=GREEN)
    elif subject == "star":
        d.polygon(_star_points(cx, cy, 170, 70), fill=YELLOW)
    elif subject == "heart":
        for box in ([cx - 150, cy - 120, cx, cy + 30], [cx, cy - 120, cx + 150, cy + 30]):
            d.ellipse(box, fill=(255, 80, 150))  # 暖：偏藕荷，接受
        d.polygon([(cx - 150, cy - 30), (cx + 150, cy - 30), (cx, cy + 160)], fill=(255, 80, 150))
    elif subject == "sun":  # 全黄，渲染干净
        for i in range(12):
            a = i * math.pi / 6
            d.line([cx + 100 * math.cos(a), cy + 100 * math.sin(a),
                    cx + 180 * math.cos(a), cy + 180 * math.sin(a)], fill=(255, 200, 20), width=18)
        d.ellipse([cx - 100, cy - 100, cx + 100, cy + 100], fill=YELLOW)
    elif subject == "flower":
        for i in range(6):
            a = i * math.pi / 3
            px, py = cx + 90 * math.cos(a), cy + 90 * math.sin(a)
            d.ellipse([px - 55, py - 55, px + 55, py + 55], fill=(255, 80, 150))  # 暖：偏藕荷，接受
        d.ellipse([cx - 50, cy - 50, cx + 50, cy + 50], fill=YELLOW)
    elif subject == "tree":
        d.rectangle([cx - 30, cy + 40, cx + 30, cy + 170], fill=(150, 95, 45))
        d.ellipse([cx - 130, cy - 170, cx + 130, cy + 70], fill=GREEN)
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
