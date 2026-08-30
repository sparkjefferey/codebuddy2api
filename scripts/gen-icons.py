#!/usr/bin/env python3
"""gen-icons.py — 生成 API Transmitter 应用图标(系统 python3 + PIL)。

设计:
  蓝紫渐变圆角底 + 白色双向箭头(⇄),表达「API 转发 / 转接」语义。
  图形占画布约 60%(旧版闪电仅 3.6% 像素、宽 25%,缩到菜单栏 20px 基本看不见),
  因此这里把主体放大、线条加粗,保证小尺寸下依然可辨。

产物(写到 ../web/icons/):
  icon-192.png            PWA 普通 192
  icon-512.png            PWA 普通 512
  icon-maskable-512.png   全出血背景 + 中心安全区(Android/iOS 自适应)
  apple-touch-icon.png    苹果 180(iOS 会自加圆角,故不出圆角)
  menubar-template.png    macOS 菜单栏模板图标(单色 + alpha,自动适配明暗)

用法: python3 gen-icons.py
"""
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    sys.exit("需要 Pillow: pip3 install pillow")

OUT = Path(__file__).resolve().parent.parent / "web" / "icons"

# 品牌渐变(斜向:左上亮蓝 → 右下紫)
GRAD_TOP = (56, 132, 255, 255)      # #3884ff
GRAD_BOTTOM = (124, 77, 237, 255)   # #7c4ded
FG = (255, 255, 255, 255)

S = 1024                 # 高分辨率母版,再降采样
RADIUS_FRAC = 0.22       # 圆角比例(macOS 风格超椭圆观感)
GLYPH_FRAC = 0.56        # 图形占画布比例(旧版过小的核心修正)
STROKE_FRAC = 0.085      # 箭头线条粗细
SAFE_FRAC = 0.80         # maskable 安全区


# ---------------------------------------------------------------------------
# 画布
# ---------------------------------------------------------------------------

def rounded_mask(size: int, radius_frac: float) -> Image.Image:
    """圆角 alpha 蒙版(抗锯齿:先 4x 画再缩)。"""
    s4 = size * 4
    m = Image.new("L", (s4, s4), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        [0, 0, s4 - 1, s4 - 1], radius=int(s4 * radius_frac), fill=255)
    return m.resize((size, size), Image.LANCZOS)


def gradient(size: int, top=GRAD_TOP, bottom=GRAD_BOTTOM) -> Image.Image:
    """斜向线性渐变。逐像素太慢,改为画一条渐变带再仿射变换。"""
    band_len = int(size * 1.6)
    band = Image.new("RGB", (1, band_len))
    px = band.load()
    for i in range(band_len):
        t = i / max(band_len - 1, 1)
        px[0, i] = tuple(round(top[c] + (bottom[c] - top[c]) * t) for c in range(3))
    band = band.resize((size, band_len), Image.NEAREST)
    # 旋转 45° 后居中裁切,得到左上→右下的斜向渐变
    rot = band.rotate(45, expand=True, resample=Image.BICUBIC)
    w, h = rot.size
    left, upper = (w - size) // 2, (h - size) // 2
    return rot.crop((left, upper, left + size, upper + size))


# ---------------------------------------------------------------------------
# 双向箭头(⇄)
# ---------------------------------------------------------------------------

def _arrow_pair(size: int, glyph_frac: float, stroke_frac: float) -> Image.Image:
    """两条水平箭头:上指向右、下指向左。返回 L 模式蒙版(255=前景)。"""
    layer = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(layer)

    span = size * glyph_frac              # 整体边长
    x0 = (size - span) / 2
    y0 = (size - span) / 2
    stroke = max(int(size * stroke_frac), 2)
    head = span * 0.32                    # 箭头头部张开
    gap = span * 0.34                     # 两行间距(线宽已含在内)

    # 两条箭头按 gap 对称排布,使包围盒正好落在画布中心。
    # (早先额外叠加了 span*0.10,导致整体偏上、下方留白过多。)
    cy = y0 + span / 2
    y_up = cy - gap / 2 - stroke / 2
    y_dn = cy + gap / 2 + stroke / 2

    # 上箭头(向右)
    d.line([(x0, y_up), (x0 + span, y_up)], fill=255, width=stroke, joint="curve")
    d.polygon([(x0 + span - head, y_up - head * 0.62),
               (x0 + span, y_up),
               (x0 + span - head, y_up + head * 0.62)], fill=255)

    # 下箭头(向左)
    d.line([(x0, y_dn), (x0 + span, y_dn)], fill=255, width=stroke, joint="curve")
    d.polygon([(x0 + head, y_dn - head * 0.62),
               (x0, y_dn),
               (x0 + head, y_dn + head * 0.62)], fill=255)
    return layer


def composite(size: int, *, maskable: bool = False, rounded: bool = True,
              monochrome: bool = False) -> Image.Image:
    """合成一张图标。

    maskable    : 图形缩进到中心安全区(系统可能裁掉边缘)
    rounded     : 圆角;False = 出方形(iOS 自加圆角)
    monochrome  : 纯白单色 + alpha,用于 macOS 菜单栏模板图标
    """
    glyph_frac = GLYPH_FRAC * SAFE_FRAC if maskable else GLYPH_FRAC
    glyph = _arrow_pair(size, glyph_frac, STROKE_FRAC)

    if monochrome:
        img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        img.paste(FG, (0, 0), glyph)
        return img

    if rounded:
        base = gradient(size).convert("RGBA")
        base.putalpha(rounded_mask(size, RADIUS_FRAC))
    else:
        base = gradient(size).convert("RGBA")

    fg = Image.new("RGBA", (size, size), FG)
    base = Image.composite(fg, base, glyph)
    return base


def downscale(img: Image.Image, size: int) -> Image.Image:
    """多步 LANCZOS 降采样:一次性从 1024 缩到 180 会糊,逐级减半更锐。"""
    cur = img
    while cur.width // 2 >= size:
        cur = cur.resize((cur.width // 2, cur.height // 2), Image.LANCZOS)
    if cur.width != size:
        cur = cur.resize((size, size), Image.LANCZOS)
    return cur


# ---------------------------------------------------------------------------

def main():
    OUT.mkdir(parents=True, exist_ok=True)

    norm = composite(S)                       # 圆角渐变 + 双向箭头
    square = composite(S, rounded=False)      # 无圆角(iOS 自加)
    maskable = composite(S, maskable=True)

    downscale(norm, 512).save(OUT / "icon-512.png")
    downscale(norm, 192).save(OUT / "icon-192.png")
    downscale(square, 180).save(OUT / "apple-touch-icon.png")
    downscale(maskable, 512).save(OUT / "icon-maskable-512.png")
    # macOS 菜单栏:模板图标(只有 alpha + 黑色,系统按明暗自动上色)
    downscale(composite(S, monochrome=True), 44).save(OUT / "menubar-template.png")

    print("生成完成 →", OUT)
    for p in sorted(OUT.glob("*.png")):
        print("  ", f"{p.name:24}", f"{p.stat().st_size // 1024:>4}KiB")


if __name__ == "__main__":
    main()
