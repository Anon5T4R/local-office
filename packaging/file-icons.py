#!/usr/bin/env python3
"""Regenerate the Local Writer file-type icons in assets/file-icons/.

Writes one master SVG per format (`docx.svg`), two pixel-fitted small variants (`docx-32.svg`, `docx-16.svg`) and the
proof sheet `preview.png`. The SVGs are committed, so builds and packaging never need this script.

Needs: Python 3 with fontTools and Pillow, and `resvg` on PATH (only for the proof sheet).
The labels are outlined from the fonts in assets/fonts (OFL), so the SVGs carry no <text>.

    python packaging/file-icons.py            # SVGs + preview.png
    python packaging/file-icons.py --no-preview
"""

from __future__ import annotations

import io
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "file-icons"
FONTS = ROOT / "assets" / "fonts"

INK = "#15171d"
WHITE = "#ffffff"
BRAND_HUE = 216  # Local Writer blue, the hue of the app icon


@dataclass(frozen=True)
class Format:
    ext: str  # file name and label
    hue: int
    sat: int  # 82 = the suite's accent; lower = legacy or neutral
    detail: str  # which body drawing
    template: bool = False  # outlined label instead of a filled one
    what: str = ""

    def mid(self) -> str:
        return f"hsl({self.hue} {self.sat}% 62%)"

    def light(self) -> str:
        return f"hsl({self.hue} {min(95, self.sat + 13)}% 82%)"

    def deep(self) -> str:
        return f"hsl({self.hue} {max(0, self.sat - 12)}% 46%)"


FORMATS = [
    Format("docx", 216, 82, "caret", what="Word document"),
    Format("docm", 216, 82, "macro", what="Word macro-enabled document"),
    Format("dotx", 216, 82, "caret", template=True, what="Word template"),
    Format("dotm", 216, 82, "macro", template=True, what="Word macro-enabled template"),
    Format("doc", 216, 26, "caret", what="Legacy Word document"),
    Format("dot", 216, 26, "caret", template=True, what="Legacy Word template"),
    Format("odt", 172, 82, "list", what="OpenDocument Text"),
    Format("rtf", 268, 82, "rich", what="Rich Text document"),
    Format("md", 40, 82, "hash", what="Markdown document"),
    Format("html", 18, 82, "tag", what="Web page"),
    Format("tex", 96, 70, "tex", what="LaTeX document"),
    Format("txt", 220, 8, "plain", what="Plain text document"),
]


class Type:
    """Outlines text from a TrueType font as SVG path data."""

    def __init__(self, file: str) -> None:
        self.font = TTFont(FONTS / file)
        self.glyphs = self.font.getGlyphSet()
        self.cmap = self.font.getBestCmap()
        self.upem = self.font["head"].unitsPerEm
        self.cap = self.font["OS/2"].sCapHeight

    def path(self, text: str, cap_height: float, cx: float, baseline: float, tracking: float = 0.0, max_width: float | None = None) -> str:
        """`text` centred on `cx`, sitting on `baseline`, with capitals `cap_height` tall."""
        scale = cap_height / self.cap
        names = [self.cmap[ord(ch)] for ch in text]
        advances = [self.glyphs[n].width * scale + tracking for n in names]
        width = sum(advances) - tracking
        sx = min(1.0, max_width / width) if max_width else 1.0
        x = cx - width * sx / 2
        parts = []
        for name, adv in zip(names, advances):
            pen = SVGPathPen(self.glyphs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
            self.glyphs[name].draw(TransformPen(pen, (scale * sx, 0, 0, -scale, x, baseline)))
            parts.append(pen.getCommands())
            x += adv * sx
        return "".join(parts)


def defs(f: Format) -> str:
    return (
        '<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#262934"/>'
        '<stop offset="1" stop-color="#101116"/></linearGradient>'
        f'<linearGradient id="r" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{f.light()}"/>'
        f'<stop offset=".5" stop-color="{f.mid()}" stop-opacity=".75"/>'
        f'<stop offset="1" stop-color="{f.deep()}" stop-opacity=".6"/></linearGradient></defs>'
    )


def bars(f: Format, rows: list[tuple[float, float, float]], dashed: bool) -> str:
    """Text lines: (x, y, width), 5 tall. A template draws them as empty placeholders."""
    out = []
    for x, y, w in rows:
        if dashed:
            out.append(
                f'<rect x="{x + 0.75}" y="{y + 0.75}" width="{w - 1.5}" height="3.5" rx="1.75" fill="none" '
                f'stroke="{f.light()}" stroke-width="1.5" stroke-dasharray="3.2 2.4"/>'
            )
        else:
            out.append(f'<rect x="{x}" y="{y}" width="{w}" height="5" rx="2.5" fill="{f.light()}" fill-opacity=".85"/>')
    return "".join(out)


def detail(f: Format, mono: Type) -> str:
    """The body of the sheet (y 40..58): text lines plus the one white accent of the format."""
    t = f.template
    if f.detail == "caret":  # the Writer mark's own two lines and caret
        return bars(f, [(24, 41, 52), (24, 51, 30)], t) + f'<rect x="59" y="47.5" width="4" height="12" rx="2" fill="{WHITE}"/>'
    if f.detail == "macro":  # a "run" triangle where the caret would be
        return bars(f, [(24, 41, 52), (24, 51, 30)], t) + f'<path d="M60.5 47.2V59.8L71.5 53.5Z" fill="{WHITE}" stroke="{WHITE}" stroke-width="2" stroke-linejoin="round"/>'
    if f.detail == "list":  # bullets
        dots = "".join(f'<circle cx="26.8" cy="{y + 2.5}" r="2.8" fill="{WHITE}"/>' for y in (41, 51))
        return dots + bars(f, [(34, 41, 42), (34, 51, 28)], t)
    if f.detail == "rich":  # bold, regular and underlined runs
        return (
            f'<rect x="24" y="40" width="22" height="7" rx="3.5" fill="{WHITE}"/>'
            f'<rect x="50" y="42" width="26" height="3" rx="1.5" fill="{f.light()}" fill-opacity=".85"/>'
            f'<rect x="24" y="51" width="34" height="3" rx="1.5" fill="{f.light()}" fill-opacity=".85"/>'
            f'<rect x="24" y="56.5" width="34" height="1.6" rx=".8" fill="{f.mid()}"/>'
        )
    if f.detail == "hash":  # a heading marker
        return f'<path d="{mono.path("#", 13, 30.5, 57.5)}" fill="{WHITE}"/>' + bars(f, [(41, 41, 35), (41, 51, 22)], t)
    if f.detail == "tag":  # an element
        return f'<path d="{mono.path("</>", 12.5, 50, 56.5, tracking=-1.2)}" fill="{WHITE}"/>'
    if f.detail == "tex":  # a control sequence
        return f'<path d="{mono.path(chr(92), 13, 29.5, 57.5)}" fill="{WHITE}"/>' + bars(f, [(38, 41, 26), (38, 51, 38)], t)
    if f.detail == "plain":  # three even lines, no formatting, no accent colour
        return bars(f, [(24, 39, 52), (24, 46.5, 52), (24, 54, 34)], t).replace('height="5" rx="2.5"', 'height="3.6" rx="1.8"')
    raise ValueError(f.detail)


SHEET = "M22 4.5H62L84.5 27V89A6.5 6.5 0 0 1 78 95.5H22A6.5 6.5 0 0 1 15.5 89V11A6.5 6.5 0 0 1 22 4.5Z"
FOLD = "M62 4.5V21.5A5.5 5.5 0 0 0 67.5 27H84.5Z"
# The Local Writer mark: the page of assets/app-icon/localwriter.svg, scaled to 36%.
MARK = (
    '<g transform="translate(15.4 6.2) scale(.36)">'
    f'<path d="M24 18H62L78 34V82H24Z" fill="hsl({BRAND_HUE} 82% 62%)"/><path d="M62 18V34H78Z" fill="hsl({BRAND_HUE} 95% 82%)"/>'
    f'<rect x="33" y="42" width="34" height="5" rx="2.5" fill="{INK}"/><rect x="33" y="53" width="34" height="5" rx="2.5" fill="{INK}"/>'
    f'<rect x="33" y="64" width="22" height="5" rx="2.5" fill="{INK}"/><rect x="59" y="61" width="4" height="13" rx="2" fill="{WHITE}"/></g>'
)


def master(f: Format, label: Type, mono: Type) -> str:
    text = label.path(f.ext.upper(), 13.2, 50, 82.6, tracking=0.9, max_width=47)
    if f.template:
        band = f'<rect x="23" y="64.5" width="54" height="23" rx="5.5" fill="none" stroke="{f.mid()}" stroke-width="3"/>' f'<path d="{text}" fill="{f.light()}"/>'
    else:
        band = f'<rect x="21.5" y="63" width="57" height="26" rx="6" fill="{f.mid()}"/><path d="{text}" fill="{INK}"/>'
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="1024" height="1024">{defs(f)}'
        f'<path d="{SHEET}" fill="url(#g)" stroke="url(#r)" stroke-width="3" stroke-linejoin="round"/>'
        f'<path d="{FOLD}" fill="{f.light()}"/>{MARK}{detail(f, mono)}{band}</svg>\n'
    )


def small(f: Format) -> str:
    """The 16 px variant, drawn on the pixel grid: sheet, fold, one line and the format's colour block."""
    if f.template:
        block = f'<rect x="4.5" y="9.5" width="7" height="4" rx=".5" fill="none" stroke="{f.mid()}"/>'
    else:
        block = f'<rect x="4" y="9" width="8" height="5" rx="1" fill="{f.mid()}"/>'
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16">'
        f'<path d="M3.5 .5H10L13.5 4V14.5A1 1 0 0 1 12.5 15.5H3.5A1 1 0 0 1 2.5 14.5V1.5A1 1 0 0 1 3.5 .5Z" fill="#1c1e26" stroke="{f.mid()}"/>'
        f'<path d="M9.5 0V4.5H14Z" fill="{f.light()}"/>'
        f'<rect x="4" y="6" width="5" height="1" fill="{f.light()}"/>{block}</svg>\n'
    )


def medium(f: Format, label: Type) -> str:
    """The 32 px variant, drawn on the pixel grid: no mark and no body detail, and the label grows past the sheet."""
    text = label.path(f.ext.upper(), 6.6, 16, 25.8, tracking=0.5, max_width=25.5)
    if f.template:
        band = f'<rect x="1.75" y="17.75" width="28.5" height="10.5" rx="2.25" fill="{INK}" stroke="{f.mid()}" stroke-width="1.5"/><path d="{text}" fill="{f.light()}"/>'
    else:
        band = f'<rect x="1" y="17" width="30" height="12" rx="3" fill="{f.mid()}"/><path d="{text}" fill="{INK}"/>'
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32">'
        f'<path d="M7.5 1.5H19L26.5 9V28.5A2 2 0 0 1 24.5 30.5H7.5A2 2 0 0 1 5.5 28.5V3.5A2 2 0 0 1 7.5 1.5Z" fill="#1c1e26" stroke="{f.mid()}"/>'
        f'<path d="M18.5 1V9.5H27Z" fill="{f.light()}"/>'
        f'<rect x="9" y="12" width="11" height="2" rx="1" fill="{f.light()}"/>{band}</svg>\n'
    )


def variant(ext: str, size: int) -> Path:
    """Which drawing a given pixel size uses."""
    return OUT / (f"{ext}-16.svg" if size < 32 else f"{ext}-32.svg" if size < 48 else f"{ext}.svg")


def render(svg: Path, size: int):
    from PIL import Image

    png = subprocess.run(["resvg", "-w", str(size), "-h", str(size), str(svg), "-c"], check=True, capture_output=True).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA")


def preview() -> None:
    """All icons at 256, 48, 32 and 16 px, on Explorer's light and dark backgrounds."""
    from PIL import Image, ImageDraw, ImageFont

    cols, cell_w, cell_h, pad = 6, 288, 372, 24
    rows = -(-len(FORMATS) // cols)
    panel_h = pad + rows * cell_h
    sheet = Image.new("RGB", (pad * 2 + cols * cell_w, panel_h * 2), "#ffffff")
    font = ImageFont.truetype(str(FONTS / "Inter-Medium.ttf"), 15)
    for p, (bg, fg) in enumerate((("#ffffff", "#1f1f1f"), ("#191919", "#e6e6e6"))):
        draw = ImageDraw.Draw(sheet)
        draw.rectangle([0, p * panel_h, sheet.width, (p + 1) * panel_h], fill=bg)
        for i, f in enumerate(FORMATS):
            x = pad + (i % cols) * cell_w + (cell_w - 256) // 2
            y = p * panel_h + pad + (i // cols) * cell_h
            sheet.paste(big := render(OUT / f"{f.ext}.svg", 256), (x, y), big)
            sx = x + 16
            for size in (48, 32, 16):
                sheet.paste(im := render(variant(f.ext, size), size), (sx, y + 268 + (48 - size)), im)
                sx += size + 24
            draw.text((sx + 4, y + 268 + 28), f".{f.ext}", fill=fg, font=font)
            draw.text((x + 16, y + 268 + 56), "256 · 48 · 32 · 16 px", fill=fg, font=font)
    sheet.save(OUT / "preview.png", optimize=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    label, mono = Type("SourceSans3-Bold.ttf"), Type("JetBrainsMono-Regular.ttf")
    for f in FORMATS:
        (OUT / f"{f.ext}.svg").write_text(master(f, label, mono), encoding="utf-8", newline="\n")
        (OUT / f"{f.ext}-32.svg").write_text(medium(f, label), encoding="utf-8", newline="\n")
        (OUT / f"{f.ext}-16.svg").write_text(small(f), encoding="utf-8", newline="\n")
    if "--no-preview" not in sys.argv:
        preview()
    print(f"file icons written to {OUT}")


if __name__ == "__main__":
    main()
