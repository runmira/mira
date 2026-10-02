#!/usr/bin/env python3
"""Make the alpha and beta app icons from the stable one.

    python3 scripts/channel-icons.py

Reads apps/desktop/icons/icon.png and writes apps/desktop/icons/<channel>/
with every size Tauri bundles (32, 128, 256, the 512 source, .icns, .ico).
The channel's name sits on a coloured pill across the bottom of the mark, so
side-by-side installs are told apart at a glance in the Dock. Re-run after
changing the logo.

Needs Pillow (`pip install pillow`); `.icns` uses macOS's `iconutil`.
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "apps" / "desktop" / "icons"

# (label, pill colour, text colour)
CHANNELS = {
    "alpha": ("ALPHA", (244, 63, 94), (255, 255, 255)),
    "beta": ("BETA", (251, 191, 36), (26, 27, 38)),
}

FONTS = [
    "/System/Library/Fonts/SFNSRounded.ttf",
    "/System/Library/Fonts/SFNS.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
]


def font(size: int) -> ImageFont.FreeTypeFont:
    for path in FONTS:
        if Path(path).exists():
            f = ImageFont.truetype(path, size)
            try:
                f.set_variation_by_name("Bold")
            except (OSError, ValueError, AttributeError):
                pass
            return f
    return ImageFont.load_default()


def badge(base: Image.Image, label: str, fill, ink) -> Image.Image:
    img = base.convert("RGBA").copy()
    w, h = img.size
    draw = ImageDraw.Draw(img)
    f = font(int(h * 0.105))
    tb = draw.textbbox((0, 0), label, font=f)
    tw, th = tb[2] - tb[0], tb[3] - tb[1]
    pw, ph = tw + int(w * 0.11), th + int(h * 0.075)
    x0, y0 = (w - pw) // 2, int(h * 0.745)
    # A soft shadow under the pill, then the pill, then the word.
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (x0, y0 + int(h * 0.012), x0 + pw, y0 + ph + int(h * 0.012)), radius=ph // 2, fill=(0, 0, 0, 90)
    )
    img = Image.alpha_composite(img, shadow)
    draw = ImageDraw.Draw(img)
    draw.rounded_rectangle((x0, y0, x0 + pw, y0 + ph), radius=ph // 2, fill=fill + (255,))
    draw.text((x0 + (pw - tw) // 2 - tb[0], y0 + (ph - th) // 2 - tb[1]), label, font=f, fill=ink + (255,))
    return img


def main() -> int:
    src = Image.open(ICONS / "icon.png").convert("RGBA")
    # Draw at 1024 so the text stays crisp, then scale down.
    big = src.resize((1024, 1024), Image.LANCZOS)
    for channel, (label, fill, ink) in CHANNELS.items():
        out = ICONS / channel
        out.mkdir(exist_ok=True)
        icon = badge(big, label, fill, ink)
        sizes = {"32x32.png": 32, "64x64.png": 64, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512}
        for name, px in sizes.items():
            icon.resize((px, px), Image.LANCZOS).save(out / name)
        icon.resize((256, 256), Image.LANCZOS).save(
            out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        )
        if shutil.which("iconutil"):
            with tempfile.TemporaryDirectory() as tmp:
                iconset = Path(tmp) / "icon.iconset"
                iconset.mkdir()
                for px in (16, 32, 128, 256, 512):
                    icon.resize((px, px), Image.LANCZOS).save(iconset / f"icon_{px}x{px}.png")
                    icon.resize((px * 2, px * 2), Image.LANCZOS).save(iconset / f"icon_{px}x{px}@2x.png")
                subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(out / "icon.icns")], check=True)
        else:
            print(f"warning: no iconutil, skipped {channel}/icon.icns", file=sys.stderr)
        print(f"wrote {out.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
