#!/usr/bin/env python3
"""Draw two 22x22 symbolic tray icons and write them as raw ARGB32.

Each glyph is opaque white over a 1 px dark halo at alpha 150, so it reads
on a dark panel (white on dark) and on a light panel (the halo carries it).
Pillow is needed only to regenerate; the .argb files are committed.
"""
from PIL import Image, ImageDraw, ImageFilter

S = 22
HALO_ALPHA = 150


def save(mask, name):
    halo = mask.filter(ImageFilter.MaxFilter(3))
    data = bytearray()
    for m, h in zip(mask.getdata(), halo.getdata()):
        if m:
            data += bytes((255, 255, 255, 255))
        elif h:
            data += bytes((HALO_ALPHA, 0, 0, 0))
        else:
            data += bytes((0, 0, 0, 0))
    with open(name, "wb") as f:
        f.write(data)


hp = Image.new("L", (S, S), 0)
d = ImageDraw.Draw(hp)
d.arc((3, 3, 18, 18), start=180, end=0, fill=255, width=2)      # headband
d.rounded_rectangle((2, 12, 7, 19), radius=2, fill=255)           # left cup
d.rounded_rectangle((14, 12, 19, 19), radius=2, fill=255)         # right cup
save(hp, "headphones.argb")

sp = Image.new("L", (S, S), 0)
d = ImageDraw.Draw(sp)
d.rounded_rectangle((6, 2, 15, 19), radius=2, outline=255, width=2)  # cabinet
d.ellipse((8, 11, 13, 16), outline=255, width=2)                     # woofer ring
d.ellipse((9, 5, 12, 8), fill=255)                                   # tweeter
save(sp, "speakers.argb")
