#!/usr/bin/env python3
"""gen-icons.py — 生成 PWA 应用图标(用系统 python3 + PIL)。

产物(写到 ../web/icons/):
  icon-192.png            普通 192
  icon-512.png            普通 512
  icon-maskable-512.png   全出血背景+中心安全区(Android/iOS 自适应)
  apple-touch-icon.png    苹果 180

用法: python3 gen-icons.py
"""
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw
except ImportError:
    sys.exit("需要 Pillow: pip3 install pillow")

OUT = Path(__file__).resolve().parent.parent / "web" / "icons"
BG = (47, 111, 235, 255)   # 品牌蓝 #2f6feb
FG = (255, 255, 255, 255)
S = 1024                   # 高分辨率母版,再缩放

# 闪电折线(占边长比例,归一化坐标)
BOLT = [
    (0.57, 0.24),
    (0.40, 0.53),
    (0.51, 0.53),
    (0.44, 0.78),
    (0.66, 0.41),
    (0.51, 0.41),
]


def make(size, maskable=False, rounded=False, radius_frac=0.22):
    """画一张图标(圆角蓝底 + 白色闪电)。"""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    r = int(size * (radius_frac if rounded else 0.06))
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=BG)

    # maskable:闪电缩进到中心 80% 安全区,否则占满
    px, py, scale = 0.0, 0.0, 1.0
    if maskable:
        scale, px, py = 0.80, (size * (1 - 0.80)) / 2, (size * (1 - 0.80)) / 2
    pts = [(int(px + scale * size * x), int(py + scale * size * y)) for x, y in BOLT]
    d.polygon(pts, fill=FG)

    if rounded:
        # 抗锯齿圆角 alpha
        mask = Image.new("L", (size, size), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, size - 1, size - 1],
                                               radius=int(size * radius_frac) + 1, fill=255)
        img.putalpha(mask)
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    norm = make(S)                              # 带圆角的普通图标
    norm.resize((512, 512), Image.LANCZOS).save(OUT / "icon-512.png")
    norm.resize((192, 192), Image.LANCZOS).save(OUT / "icon-192.png")
    norm.resize((180, 180), Image.LANCZOS).save(OUT / "apple-touch-icon.png")
    make(S, maskable=True).resize((512, 512), Image.LANCZOS).save(OUT / "icon-maskable-512.png")

    print("生成完成 →", OUT)
    for p in sorted(OUT.glob("*.png")):
        print("  ", p.name, f"{p.stat().st_size // 1024}KiB")


if __name__ == "__main__":
    main()