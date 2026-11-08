"""Tiny frame-by-frame video compositor: ffmpeg decodes, PIL draws, ffmpeg encodes."""
import subprocess

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

FONTS = r"C:\SteamInRust\crates\ui\assets\fonts"
W, H, FPS = 1920, 1080, 30

BG = (11, 13, 18)
PANEL = (22, 26, 34)
TEXT = (236, 239, 244)
MUTED = (140, 148, 162)
STEAM = (102, 192, 244)    # Steam blue
OURS = (247, 120, 52)      # Rust orange
GOOD = (80, 210, 120)

_fonts = {}


def font(size, weight="Bold"):
    key = (size, weight)
    if key not in _fonts:
        _fonts[key] = ImageFont.truetype(f"{FONTS}\\Inter-{weight}.ttf", size)
    return _fonts[key]


class Reader:
    """Frames of a video, scaled to (w, h), at FPS. Holds the last frame at the end."""

    def __init__(self, path, w, h, start=0.0, vf_extra=""):
        self.w, self.h = w, h
        vf = f"fps={FPS},scale={w}:{h}:flags=lanczos"
        if vf_extra:
            vf = vf_extra + "," + vf
        self.p = subprocess.Popen(
            ["ffmpeg", "-loglevel", "error", "-ss", f"{start:.3f}", "-i", path, "-vf", vf,
             "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], stdout=subprocess.PIPE)
        self.last = Image.new("RGB", (w, h), (0, 0, 0))
        self.done = False

    def next(self):
        if not self.done:
            buf = self.p.stdout.read(self.w * self.h * 3)
            if len(buf) < self.w * self.h * 3:
                self.done = True
            else:
                self.last = Image.frombuffer("RGB", (self.w, self.h), buf, "raw", "RGB", 0, 1)
        return self.last.copy()


class Writer:
    def __init__(self, path, crf=18):
        self.p = subprocess.Popen(
            ["ffmpeg", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
             "-r", str(FPS), "-i", "-", "-c:v", "libx264", "-preset", "slow", "-crf", str(crf),
             "-pix_fmt", "yuv420p", "-movflags", "+faststart", path], stdin=subprocess.PIPE)

    def put(self, img):
        self.p.stdin.write(img.convert("RGB").tobytes())

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def blur(img, boxes, radius=14):
    """Pixelate + blur personal areas. Boxes are fractions of the image (x0, y0, x1, y1)."""
    w, h = img.size
    for x0, y0, x1, y1 in boxes:
        b = (int(x0 * w), int(y0 * h), int(x1 * w), int(y1 * h))
        if b[2] <= b[0] or b[3] <= b[1]:
            continue
        region = img.crop(b)
        small = region.resize((max(1, region.width // 12), max(1, region.height // 12)), Image.BILINEAR)
        region = small.resize(region.size, Image.NEAREST).filter(ImageFilter.GaussianBlur(radius))
        img.paste(region, b)
    return img


def text(d, xy, s, size, color=TEXT, weight="Bold", anchor="la"):
    d.text(xy, s, font=font(size, weight), fill=color, anchor=anchor)


def rounded(d, box, r, fill, outline=None, width=0):
    d.rounded_rectangle(box, radius=r, fill=fill, outline=outline, width=width)


def ease(t):
    t = max(0.0, min(1.0, t))
    return 1 - (1 - t) ** 3


def fade(img, alpha):
    if alpha >= 1:
        return img
    black = Image.new("RGB", img.size, (0, 0, 0))
    return Image.blend(black, img.convert("RGB"), max(0.0, alpha))


def framed(dst, src, x, y, accent, r=14):
    """Paste a window recording with rounded corners and a thin accent border."""
    mask = Image.new("L", src.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, src.width - 1, src.height - 1), radius=r, fill=255)
    d = ImageDraw.Draw(dst)
    d.rounded_rectangle((x - 3, y - 3, x + src.width + 2, y + src.height + 2), radius=r + 3, fill=accent)
    dst.paste(src, (x, y), mask)
