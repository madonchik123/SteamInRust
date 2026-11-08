"""(b) Idle RAM / CPU video.  python -I make_idle.py en|ru OUT.mp4"""
import collections
import json
import os
import sys

from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vid  # noqa: E402
from vid import W, H, FPS, BG, PANEL, TEXT, MUTED, STEAM, OURS, GOOD  # noqa: E402
from make_race import BLUR  # noqa: E402

HERE = os.environ.get("BENCH_OUT", os.path.join(os.path.dirname(os.path.abspath(__file__)), "out"))
REC = os.path.join(HERE, "rec")
DISK = json.load(open(os.path.join(HERE, "disk.json")))

T = {
    "en": {"title": "SITTING IDLE", "sub": "Both clients open on the library, doing nothing · same PC, same account",
           "steam": "Steam", "ours": "SteamInRust", "ram": "RAM (private)", "cpu": "CPU", "procs": "Processes",
           "median": "Median of 5 runs, 60 s idle each", "disk": "Install size", "core": "of one core",
           "note": "live values from this take · CPU = 3-second average, % of one core", "mb": "MB"},
    "ru": {"title": "В ПРОСТОЕ", "sub": "Оба клиента открыты на библиотеке и ничего не делают · тот же ПК и аккаунт",
           "steam": "Steam", "ours": "SteamInRust", "ram": "ОЗУ (private)", "cpu": "ЦП", "procs": "Процессы",
           "median": "Медиана 5 запусков по 60 с простоя", "disk": "Размер установки", "core": "одного ядра",
           "note": "живые значения из этой записи · ЦП — среднее за 3 с, % одного ядра", "mb": "МБ"},
}


def at(samples, t):
    best = samples[0]
    for s in samples:
        if s["t"] <= t:
            best = s
    return best


def cpu_avg(samples, t, window=3.0):
    xs = [s["cpu_pct_one_core"] for s in samples if t - window < s["t"] <= t]
    return sum(xs) / len(xs) if xs else 0.0


def proc_summary(names_count):
    return ", ".join(f"{n} ×{c}" if c > 1 else n for n, c in names_count.most_common())


def main():
    lang, out = sys.argv[1], sys.argv[2]
    L = T[lang]
    S = json.load(open(os.path.join(HERE, "summary.json")))
    data = {a: json.load(open(os.path.join(REC, f"idle_{a}.json"))) for a in ("steam", "ours")}
    pw, ph = 912, 570
    readers = {a: vid.Reader(os.path.join(REC, f"idle_{a}.mp4"), pw, ph, start=data[a]["video_offset_s"]) for a in data}
    intro, body, outro = 1.3, 17.0, 5.0
    wr = vid.Writer(out)
    total = int((intro + body + outro) * FPS)
    peak = max(s["private_mb"] for a in data for s in data[a]["samples"])
    for i in range(total):
        t = i / FPS
        img = Image.new("RGB", (W, H), BG)
        d = ImageDraw.Draw(img)
        if t < intro:
            vid.text(d, (W // 2, 480), L["title"], 130, TEXT, anchor="mm")
            vid.text(d, (W // 2, 590), "RAM · CPU · " + L["procs"], 54, MUTED, "SemiBold", anchor="mm")
            img = vid.fade(img, vid.ease(t / 0.4))
        elif t < intro + body:
            bt = t - intro
            vid.text(d, (W // 2, 56), L["title"], 54, TEXT, anchor="mm")
            vid.text(d, (W // 2, 108), L["sub"], 28, MUTED, "Medium", anchor="mm")
            for app, x, col in (("steam", 36, STEAM), ("ours", 972, OURS)):
                frame = vid.blur(readers[app].next(), BLUR[app])
                vid.framed(img, frame, x, 150, col)
                d = ImageDraw.Draw(img)
                s = at(data[app]["samples"], bt)
                vid.text(d, (x, 742), L[app], 44, col)
                # three counters
                for k, (label, value) in enumerate(((L["ram"], f"{s['private_mb']:.0f} " + L["mb"]),
                                                    (L["cpu"], f"{cpu_avg(data[app]['samples'], bt):.1f} %"),
                                                    (L["procs"], f"{s['processes']}"))):
                    cx = x + k * 310
                    vid.text(d, (cx, 808), label, 26, MUTED, "SemiBold")
                    vid.text(d, (cx, 842), value, 72, TEXT)
                vid.text(d, (x, 940), proc_summary(collections.Counter(s["names"])), 24, MUTED, "Medium")
            vid.text(d, (W // 2, 1050), L["note"], 22, MUTED, "Medium", anchor="mm")
        else:
            ot = t - intro - body
            vid.text(d, (W // 2, 150), L["median"], 52, TEXT, anchor="mm")
            rows = [
                (L["ram"], S["steam"]["private_mb_median"][0], S["ours"]["private_mb_median"][0], "{:.0f} " + L["mb"]),
                (L["cpu"] + " (" + L["core"] + ")", S["steam"]["cpu_pct_of_one_core"][0], S["ours"]["cpu_pct_of_one_core"][0], "{:.1f} %"),
                (L["procs"], S["steam"]["processes"][0], S["ours"]["processes"][0], "{:.0f}"),
                (L["disk"], DISK["steam_client_mb"], DISK["ours_mb"], "{:.0f} " + L["mb"]),
            ]
            for k, (label, sv, ov, fmt) in enumerate(rows):
                y = 270 + k * 190
                vid.text(d, (120, y + 10), label, 34, MUTED, "SemiBold")
                top = max(sv, ov, 1e-9)
                for j, (v, col) in enumerate(((sv, STEAM), (ov, OURS))):
                    yy = y + 60 + j * 56
                    bw = max(6, 1150 * v / top * vid.ease((ot - 0.15 * k) / 0.7))
                    vid.rounded(d, (480, yy, 480 + bw, yy + 44), 12, col)
                    vid.text(d, (500 + bw, yy + 22), fmt.format(v), 36, TEXT, anchor="lm")
                vid.text(d, (460, y + 82), "Steam", 26, STEAM, "SemiBold", anchor="rm")
                vid.text(d, (460, y + 138), "SteamInRust", 26, OURS, "SemiBold", anchor="rm")
            img = vid.fade(img, min(1.0, ot / 0.25))
            if t > intro + body + outro - 0.4:
                img = vid.fade(img, (intro + body + outro - t) / 0.4)
        wr.put(img)
    wr.close()
    print(out, f"{total / FPS:.1f}s")


if __name__ == "__main__":
    main()
