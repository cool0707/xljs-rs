#!/usr/bin/env python3
"""Builds assets/xljs.png (transparent) and assets/xljs.ico (multi-size) from
the original logo whose background is a flat dark color.

usage: python3 tools/make_icon.py <logo.jpg|png>
"""
import sys
from collections import deque

import numpy as np
from PIL import Image, ImageFilter

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]


def main(src):
    im = np.asarray(Image.open(src).convert("RGB")).astype(np.float32)
    h, w, _ = im.shape
    bg = np.median(np.concatenate([im[:8, :8].reshape(-1, 3), im[:8, -8:].reshape(-1, 3),
                                   im[-8:, :8].reshape(-1, 3), im[-8:, -8:].reshape(-1, 3)]), axis=0)
    dist = np.sqrt(((im - bg) ** 2).sum(axis=2))

    # Background = pixels close to the background color connected to the border.
    near = dist < 24
    seen = np.zeros((h, w), bool)
    q = deque()
    for x in range(w):
        for y in (0, h - 1):
            if near[y, x] and not seen[y, x]:
                seen[y, x] = True
                q.append((y, x))
    for y in range(h):
        for x in (0, w - 1):
            if near[y, x] and not seen[y, x]:
                seen[y, x] = True
                q.append((y, x))
    while q:
        y, x = q.popleft()
        for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            ny, nx = y + dy, x + dx
            if 0 <= ny < h and 0 <= nx < w and near[ny, nx] and not seen[ny, nx]:
                seen[ny, nx] = True
                q.append((ny, nx))

    # Soft alpha in the anti-aliased band next to the background.
    band = np.asarray(Image.fromarray((seen * 255).astype(np.uint8)).filter(ImageFilter.MaxFilter(9))) > 0
    alpha = np.ones((h, w), np.float32)
    soft = np.clip((dist - 40) / (100 - 40), 0, 1)
    alpha[band] = soft[band]
    alpha[seen] = 0

    # Un-premultiply the background out of the edge colors.
    a = alpha[..., None]
    rgb = np.where(a > 0.01, (im - (1 - a) * bg) / np.maximum(a, 0.01), 0)
    rgba = np.dstack([np.clip(rgb, 0, 255), alpha * 255]).astype(np.uint8)
    img = Image.fromarray(rgba, "RGBA")

    # Crop to the logo and pad to a square with a small margin.
    bbox = img.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    img = img.crop(bbox)
    side = int(max(img.size) * 1.02)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(img, ((side - img.width) // 2, (side - img.height) // 2))

    master = canvas.resize((1024, 1024), Image.LANCZOS)
    master.save("assets/xljs.png", optimize=True)
    frames = []
    for s in SIZES:
        f = canvas.resize((s, s), Image.LANCZOS)
        if s <= 32:
            f = f.filter(ImageFilter.UnsharpMask(radius=0.6, percent=60, threshold=0))
        frames.append(f)
    frames[-1].save("assets/xljs.ico", format="ICO", sizes=[(s, s) for s in SIZES], append_images=frames[:-1])
    print("wrote assets/xljs.png and assets/xljs.ico", SIZES)


if __name__ == "__main__":
    main(sys.argv[1])
