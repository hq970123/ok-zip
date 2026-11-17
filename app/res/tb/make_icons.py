# Regenerates the toolbar icons (16-64 px .ico) in this folder. Requires Pillow.
import os
from PIL import Image, ImageDraw
S = 256
OUT = os.path.dirname(os.path.abspath(__file__))


def canvas():
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    return im, ImageDraw.Draw(im)

OUTL = (40, 52, 70, 255)
W = 8  # outline width at 256

def folder(d, open_=False):
    # back
    d.rounded_rectangle((24, 56, 120, 96), 14, fill=(229, 170, 40), outline=OUTL, width=W)
    d.rounded_rectangle((24, 76, 232, 212), 16, fill=(242, 190, 60), outline=OUTL, width=W)
    if open_:
        d.polygon([(52, 124), (248, 124), (220, 212), (24, 212)], fill=(255, 214, 102), outline=OUTL)
        d.line([(52, 124), (248, 124), (220, 212), (24, 212), (52, 124)], fill=OUTL, width=W, joint="curve")

def box(d, color=(70, 130, 220), light=(120, 170, 240)):
    # archive box with zipper
    d.rounded_rectangle((40, 60, 216, 224), 18, fill=color, outline=OUTL, width=W)
    d.rectangle((40 + W, 60 + W, 216 - W, 104), fill=light)
    d.line([(40, 104), (216, 104)], fill=OUTL, width=W)
    # zipper
    for y in range(116, 216, 20):
        d.rectangle((118, y, 138, y + 10), fill=(250, 250, 250))
    d.rounded_rectangle((110, 104, 146, 132), 6, fill=(250, 250, 250), outline=OUTL, width=6)

def arrow_down(d, cx, top, bottom, color=(46, 170, 90)):
    w = 26
    d.rectangle((cx - w, top, cx + w, bottom - 48), fill=color, outline=OUTL, width=W)
    d.polygon([(cx - 60, bottom - 52), (cx + 60, bottom - 52), (cx, bottom)], fill=color, outline=OUTL)
    d.line([(cx - 60, bottom - 52), (cx + 60, bottom - 52), (cx, bottom), (cx - 60, bottom - 52)], fill=OUTL, width=W, joint="curve")
    d.rectangle((cx - w + W, bottom - 60, cx + w - W, bottom - 40), fill=color)

def plus(d, cx, cy, r=44, color=(46, 170, 90)):
    d.ellipse((cx - r - 6, cy - r - 6, cx + r + 6, cy + r + 6), fill=color, outline=OUTL, width=W)
    t = 11
    d.rectangle((cx - r + 18, cy - t, cx + r - 18, cy + t), fill="white")
    d.rectangle((cx - t, cy - r + 18, cx + t, cy + r - 18), fill="white")

def save(im, name):
    sizes = [(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64)]
    im.save(f"{OUT}/{name}.ico", sizes=sizes)


# open: open folder
im, d = canvas(); folder(d, True); save(im, "open")
# extract: box + green down arrow on the right
im, d = canvas(); box(d); 
d2 = d
arrow_down(d2, 186, 112, 250); save(im, "extract")
# new: box + plus
im, d = canvas(); box(d, (150, 100, 200), (190, 150, 230)); plus(d, 190, 190); save(im, "new")
# add: document + plus
im, d = canvas()
d.polygon([(48, 24), (156, 24), (204, 72), (204, 232), (48, 232)], fill="white", outline=OUTL)
d.line([(48, 24), (156, 24), (204, 72), (204, 232), (48, 232), (48, 24)], fill=OUTL, width=W, joint="curve")
d.polygon([(156, 24), (156, 72), (204, 72)], fill=(210, 220, 235), outline=OUTL)
for y in (100, 130, 160): d.rectangle((76, y, 170, y + 10), fill=(150, 165, 190))
plus(d, 186, 190, 46, (60, 120, 220)); save(im, "add")
# delete: red circle with X
im, d = canvas()
d.ellipse((28, 28, 228, 228), fill=(220, 60, 60), outline=OUTL, width=W)
d.line([(84, 84), (172, 172)], fill="white", width=30)
d.line([(172, 84), (84, 172)], fill="white", width=30)
save(im, "delete")
# test: shield with check
im, d = canvas()
d.polygon([(128, 20), (220, 56), (212, 140), (128, 236), (44, 140), (36, 56)], fill=(46, 160, 110), outline=OUTL)
d.line([(128, 20), (220, 56), (212, 140), (128, 236), (44, 140), (36, 56), (128, 20)], fill=OUTL, width=W, joint="curve")
d.line([(82, 128), (116, 162), (178, 94)], fill="white", width=26, joint="curve")
save(im, "test")
# view: magnifier
im, d = canvas()
d.line([(160, 160), (226, 226)], fill=OUTL, width=40)
d.line([(160, 160), (222, 222)], fill=(120, 90, 60), width=24)
d.ellipse((24, 24, 176, 176), fill=(200, 230, 250), outline=OUTL, width=W + 4)
d.arc((52, 52, 148, 148), 200, 260, fill="white", width=12)
save(im, "view")
