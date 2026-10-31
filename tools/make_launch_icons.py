#!/usr/bin/env python3
"""Draws the icons of the start screen choices (cmd/xljs/icons/*.ico):
a colored circle with a white symbol, at the sizes Windows asks for.

usage: python3 tools/make_launch_icons.py
"""
import math
import os

from PIL import Image, ImageDraw

SIZES = [16, 20, 24, 32, 40, 48, 64]
S = 256  # drawing size; each icon size is scaled down from it
WHITE = (255, 255, 255, 255)
OUT = os.path.join(os.path.dirname(__file__), "..", "cmd", "xljs", "icons")


def base(color):
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.ellipse((8, 8, S - 8, S - 8), fill=color)
    return im, d


def run():
    im, d = base((46, 158, 79, 255))
    d.polygon([(98, 70), (98, 186), (194, 128)], fill=WHITE)
    return im


def debug():
    im, d = base((224, 123, 26, 255))
    w = 14
    # legs and feelers
    for y, dx in ((112, 60), (146, 64), (180, 58)):
        d.line([(128, y), (128 - dx, y - 12 + (y - 146) // 3)], fill=WHITE, width=w)
        d.line([(128, y), (128 + dx, y - 12 + (y - 146) // 3)], fill=WHITE, width=w)
    d.line([(116, 78), (96, 52)], fill=WHITE, width=w - 4)
    d.line([(140, 78), (160, 52)], fill=WHITE, width=w - 4)
    d.ellipse((104, 66, 152, 104), fill=WHITE)  # head
    d.ellipse((90, 92, 166, 206), fill=WHITE)  # body
    d.line([(128, 108), (128, 200)], fill=(224, 123, 26, 255), width=8)
    return im


def pack():
    im, d = base((47, 111, 209, 255))
    # a box seen from the front-top: lid, front and the tape across
    d.polygon([(64, 100), (128, 72), (192, 100), (128, 128)], fill=WHITE)
    d.polygon([(64, 110), (122, 136), (122, 202), (64, 176)], fill=WHITE)
    d.polygon([(134, 136), (192, 110), (192, 176), (134, 202)], fill=WHITE)
    return im


def vba():
    im, d = base((122, 75, 196, 255))
    w = 20
    d.line([(100, 84), (60, 128), (100, 172)], fill=WHITE, width=w, joint="curve")
    d.line([(156, 84), (196, 128), (156, 172)], fill=WHITE, width=w, joint="curve")
    d.line([(140, 72), (116, 184)], fill=WHITE, width=w - 4)
    return im


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, draw in (("run", run), ("debug", debug), ("pack", pack), ("vba", vba)):
        im = draw()
        frames = [im.resize((s, s), Image.LANCZOS) for s in SIZES]
        path = os.path.join(OUT, name + ".ico")
        frames[-1].save(path, sizes=[(s, s) for s in SIZES], append_images=frames[:-1])
        print(path)


if __name__ == "__main__":
    main()
