"""Draw the BriskMarkdown icon (assets/icon.ico). Run: python assets/make_icon.py"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Rounded page
d.rounded_rectangle((64, 64, S - 64, S - 64), radius=200, fill=(27, 30, 35, 255), outline=(52, 58, 67, 255), width=18)

# A bold "M" in lavender, drawn as strokes
lav = (181, 160, 222, 255)
w = 118
left, right, top, bottom = 230, 794, 270, 740
mid = (left + right) // 2
d.line([(left, bottom), (left, top)], fill=lav, width=w)
d.line([(left, top), (mid, 560)], fill=lav, width=w)
d.line([(mid, 560), (right, top)], fill=lav, width=w)
d.line([(right, top), (right, bottom)], fill=lav, width=w)
for x, y in [(left, top), (right, top), (mid, 560), (left, bottom), (right, bottom)]:
    d.ellipse((x - w // 2, y - w // 2, x + w // 2, y + w // 2), fill=lav)

# A quick teal underline: "brisk"
teal = (138, 209, 201, 255)
d.rounded_rectangle((left - w // 2, 830, right + w // 2, 880), radius=25, fill=teal)

out = Path(__file__).with_name("icon.ico")
sizes = [(256, 256), (64, 64), (48, 48), (40, 40), (32, 32), (24, 24), (20, 20), (16, 16)]
img.save(out, sizes=sizes)
img.resize((256, 256), Image.LANCZOS).save(Path(__file__).with_name("icon.png"))
print(out)
