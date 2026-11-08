"""(c) Feature tour.  python -I make_tour.py en|ru OUT.mp4"""
import json
import os
import sys

from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vid  # noqa: E402
from vid import W, H, FPS, BG, TEXT, MUTED, OURS, GOOD  # noqa: E402

HERE = os.environ.get("BENCH_OUT", os.path.join(os.path.dirname(os.path.abspath(__file__)), "out"))
REC = os.path.join(HERE, "rec")

# (clip key, start s, length s)
CLIPS = [
    ("store", 3.0, 4.8),
    ("trailer", 0.3, 5.5),
    ("library", 0.3, 4.0),
    ("overlay", 0.3, 4.6),
    ("inventory", 0.3, 5.0),
    ("listings", 0.3, 3.4),
    ("chat", 0.3, 3.6),
    ("profile", 0.3, 3.6),
]

CAP = {
    "en": {
        "intro1": "SteamInRust", "intro2": "A native Steam client, written in Rust",
        "store": ("Native store", "No browser engine inside: Rust + egui"),
        "trailer": ("Trailers play natively", "HLS streaming + Media Foundation, in-app"),
        "library": ("Your library", "Play, install, verify, DLC, launch options"),
        "overlay": ("Shift+Tab overlay", "Friends, chat and store over your game"),
        "inventory": ("Inventory & Market", "Live prices, price history, Steam's real fees"),
        "listings": ("Your listings", "Sell listings and buy orders"),
        "chat": ("Friends & chat", "Live status and chat history"),
        "profile": ("Profiles", "Backgrounds, avatar frames, showcases"),
        "demo": "demo mode · sample data",
        "outro_ram": "MB RAM idle", "outro_proc": "process", "outro_size": "MB app",
        "outro_ready": "s cold start",
    },
    "ru": {
        "intro1": "SteamInRust", "intro2": "Нативный клиент Steam на Rust",
        "store": ("Нативный магазин", "Без браузерного движка: Rust + egui"),
        "trailer": ("Трейлеры — нативно", "HLS-стриминг и Media Foundation прямо в приложении"),
        "library": ("Ваша библиотека", "Запуск, установка, проверка файлов, DLC, параметры"),
        "overlay": ("Оверлей по Shift+Tab", "Друзья, чат и магазин поверх игры"),
        "inventory": ("Инвентарь и Торговая площадка", "Живые цены, история цен, реальные комиссии Steam"),
        "listings": ("Ваши лоты", "Лоты на продажу и заявки на покупку"),
        "chat": ("Друзья и чат", "Статусы в реальном времени и история переписки"),
        "profile": ("Профили", "Фоны, рамки аватара, витрины"),
        "demo": "демо-режим · примерные данные",
        "outro_ram": "МБ ОЗУ в простое", "outro_proc": "процесс", "outro_size": "МБ приложение",
        "outro_ready": "с холодный запуск",
    },
}


def caption(img, title, sub, age):
    """Bottom-left caption that pops in (slide up + fade)."""
    a = vid.ease(age / 0.35)
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    tw = max(d.textlength(title, font=vid.font(64)), d.textlength(sub, font=vid.font(32, "Medium")))
    y = 1080 - 70 - 170 + int((1 - a) * 40)
    x = 70
    d.rounded_rectangle((x, y, x + tw + 80, y + 170), radius=26, fill=(10, 12, 16, int(244 * a)))
    d.rounded_rectangle((x, y, x + 10, y + 170), radius=5, fill=OURS + (int(255 * a),))
    d.text((x + 40, y + 22), title, font=vid.font(64), fill=TEXT + (int(255 * a),))
    d.text((x + 42, y + 108), sub, font=vid.font(32, "Medium"), fill=(200, 205, 215, int(255 * a)))
    img.paste(layer, (0, 0), layer)


def tag(img, s):
    d = ImageDraw.Draw(img, "RGBA")
    w = d.textlength(s, font=vid.font(24, "SemiBold"))
    d.rounded_rectangle((1920 - w - 70, 1080 - 70, 1920 - 30, 1080 - 26), radius=22, fill=(10, 12, 16, 200))
    d.text((1920 - w - 50, 1080 - 62), s, font=vid.font(24, "SemiBold"), fill=(220, 224, 232))


def progress(img, k, frac):
    d = ImageDraw.Draw(img, "RGBA")
    n = len(CLIPS)
    seg = (1920 - 80) / n
    for i in range(n):
        x0 = 40 + i * seg + 3
        x1 = 40 + (i + 1) * seg - 3
        d.rounded_rectangle((x0, 16, x1, 22), radius=3, fill=(255, 255, 255, 70))
        fill = 1.0 if i < k else (frac if i == k else 0.0)
        if fill > 0:
            d.rounded_rectangle((x0, 16, x0 + (x1 - x0) * fill, 22), radius=3, fill=OURS + (255,))


def main():
    lang, out = sys.argv[1], sys.argv[2]
    L = CAP[lang]
    S = json.load(open(os.path.join(HERE, "summary.json")))["ours"]
    disk = json.load(open(os.path.join(HERE, "disk.json")))
    wr = vid.Writer(out)
    # intro
    for i in range(int(1.6 * FPS)):
        t = i / FPS
        img = Image.new("RGB", (W, H), BG)
        d = ImageDraw.Draw(img)
        vid.text(d, (W // 2, 470), L["intro1"], 150, TEXT, anchor="mm")
        vid.text(d, (W // 2, 590), L["intro2"], 50, OURS, "SemiBold", anchor="mm")
        wr.put(vid.fade(img, vid.ease(t / 0.35)))
    for k, (key, start, length) in enumerate(CLIPS):
        r = vid.Reader(os.path.join(REC, f"tour_{lang}_{key}.mp4"), W, H, start=start)
        title, sub = L[key]
        for i in range(int(length * FPS)):
            t = i / FPS
            img = r.next()
            # quick punch-in on each cut
            if t < 0.18:
                z = 1.0 + 0.035 * (1 - t / 0.18)
                zw, zh = int(W / z), int(H / z)
                img = img.crop(((W - zw) // 2, (H - zh) // 2, (W + zw) // 2, (H + zh) // 2)).resize((W, H), Image.BILINEAR)
            caption(img, title, sub, t - 0.12)
            tag(img, L["demo"])
            progress(img, k, t / length)
            wr.put(img)
    # outro
    stats = [
        (f"{disk['ours_mb']:.0f}", L["outro_size"]),
        ("1", L["outro_proc"]),
        (f"{S['private_mb_median'][0]:.0f}", L["outro_ram"]),
        (f"{S['cold_ready'][0]:.1f}", L["outro_ready"]),
    ]
    for i in range(int(3.6 * FPS)):
        t = i / FPS
        img = Image.new("RGB", (W, H), BG)
        d = ImageDraw.Draw(img)
        vid.text(d, (W // 2, 250), "SteamInRust", 110, TEXT, anchor="mm")
        for j, (big, small) in enumerate(stats):
            a = vid.ease((t - 0.15 * j) / 0.4)
            cx = 240 + j * 480
            col = tuple(int(c * a + b * (1 - a)) for c, b in zip(OURS, BG))
            vid.text(d, (cx, 560), big, 120, col, anchor="mm")
            vid.text(d, (cx, 660), small, 34, tuple(int(c * a + b * (1 - a)) for c, b in zip(MUTED, BG)), "SemiBold", anchor="mm")
        alpha = min(vid.ease(t / 0.3), (3.6 - t) / 0.4)
        wr.put(vid.fade(img, alpha))
    wr.close()
    print(out)


if __name__ == "__main__":
    main()
