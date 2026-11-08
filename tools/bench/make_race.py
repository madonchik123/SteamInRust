"""(a) Startup race video.  python -I make_race.py cold|warm en|ru OUT.mp4"""
import json
import os
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vid  # noqa: E402
from vid import W, H, FPS, BG, TEXT, MUTED, STEAM, OURS, GOOD  # noqa: E402

HERE = os.environ.get("BENCH_OUT", os.path.join(os.path.dirname(os.path.abspath(__file__)), "out"))
REC = os.path.join(HERE, "rec")

# Personal areas, as fractions of the 1280x800 window.
BLUR = {
    "steam": [(0.715, 0.0, 0.89, 0.045), (0.36, 0.04, 0.53, 0.08)],     # avatar, name, wallet; profile tab
    "ours": [(0.655, 0.005, 0.885, 0.065)],                            # wallet, avatar, name
}

T = {
    "en": {
        "title": "STARTUP RACE",
        "sub_cold": "Cold start to a settled library · same PC, same account",
        "sub_warm": "Warm start to a settled library · same PC, same account",
        "steam": "Steam", "steam_tag": "official client · Chromium web UI",
        "ours": "SteamInRust", "ours_tag": "native · Rust + egui",
        "ready": "READY",
        "realtime": "one take, real time · each client recorded on its own, launches lined up · medians at the end",
        "median": "Median of 5 runs",
        "cold": "Cold start", "warm": "Warm start",
        "faster": "{x:.1f}× faster", "s": "s",
    },
    "ru": {
        "title": "ГОНКА ЗАПУСКА",
        "sub_cold": "Холодный запуск до загруженной библиотеки · тот же ПК и аккаунт",
        "sub_warm": "Тёплый запуск до загруженной библиотеки · тот же ПК и аккаунт",
        "steam": "Steam", "steam_tag": "официальный клиент · веб-интерфейс Chromium",
        "ours": "SteamInRust", "ours_tag": "нативный · Rust + egui",
        "ready": "ГОТОВО",
        "realtime": "один дубль, реальное время · клиенты записаны по отдельности, старт совмещён · медианы в конце",
        "median": "Медиана 5 запусков",
        "cold": "Холодный запуск", "warm": "Тёплый запуск",
        "faster": "в {x:.1f} раза быстрее", "s": "с",
    },
}


def marker_time(path):
    """When the backdrop turned grey (= the moment the client was launched)."""
    p = subprocess.run(["ffmpeg", "-loglevel", "error", "-t", "5", "-i", path, "-vf", "crop=24:24:2:2,scale=4:4",
                        "-f", "rawvideo", "-pix_fmt", "gray", "-"], capture_output=True)
    frames = np.frombuffer(p.stdout, np.uint8).reshape(-1, 16)
    fps = float(eval(subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v", "-show_entries",
                                     "stream=r_frame_rate", "-of", "csv=p=0", path], capture_output=True, text=True).stdout.strip()))
    for i, f in enumerate(frames):
        if f.mean() > 6:
            return i / fps
    raise SystemExit(f"no launch marker in {path}")


def main():
    kind, lang, out = sys.argv[1], sys.argv[2], sys.argv[3]
    L = T[lang]
    summary = json.load(open(os.path.join(HERE, "summary.json")))
    takes = {}
    for app in ("steam", "ours"):
        path = os.path.join(REC, f"race_{app}_{kind}.mp4")
        res = json.load(open(path[:-4] + ".json"))
        takes[app] = {"path": path, "ready": res["visually_complete_s"], "main": res["main_window_s"], "start": marker_time(path)}
    pw, ph = 912, 570
    readers = {a: vid.Reader(t["path"], pw, ph, start=t["start"]) for a, t in takes.items()}
    race_len = max(t["ready"] for t in takes.values()) + 1.8
    intro, outro = 1.4, 4.0
    w = vid.Writer(out)
    total = int((intro + race_len + outro) * FPS)
    med = {a: summary[a][f"{kind}_ready"][0] for a in ("steam", "ours")}
    for i in range(total):
        t = i / FPS
        img = Image.new("RGB", (W, H), BG)
        d = ImageDraw.Draw(img)
        if t < intro:
            a = vid.ease(t / 0.4)
            vid.text(d, (W // 2, 470), L["title"], 130, TEXT, anchor="mm")
            vid.text(d, (W // 2, 580), "Steam  vs  SteamInRust", 54, MUTED, "SemiBold", anchor="mm")
            img = vid.fade(img, a)
        elif t < intro + race_len:
            rt = t - intro
            vid.text(d, (W // 2, 62), L["title"], 58, TEXT, anchor="mm")
            vid.text(d, (W // 2, 118), L["sub_" + kind], 30, MUTED, "Medium", anchor="mm")
            for app, x, col in (("steam", 36, STEAM), ("ours", 972, OURS)):
                frame = readers[app].next()
                if rt < takes[app]["main"] + 0.4:
                    # sign-in screens show the account name and avatar: blur all of it
                    frame = vid.blur(frame, [(0, 0, 1, 1)], radius=10)
                else:
                    frame = vid.blur(frame, BLUR[app])
                vid.framed(img, frame, x, 170, col)
                d = ImageDraw.Draw(img)
                vid.text(d, (x, 770), L[app], 46, col)
                vid.text(d, (x, 826), L[app + "_tag"], 26, MUTED, "Medium")
                ready = takes[app]["ready"]
                done = rt >= ready
                shown = ready if done else rt
                vid.text(d, (x + pw, 760), f"{shown:4.1f} " + L["s"], 120, GOOD if done else TEXT, anchor="ra")
                if done:
                    vid.rounded(d, (x + pw - 190, 900, x + pw, 948), 24, GOOD)
                    vid.text(d, (x + pw - 95, 924), L["ready"], 28, BG, anchor="mm")
            vid.text(d, (W // 2, 1040), L["realtime"], 24, MUTED, "Medium", anchor="mm")
        else:
            ot = t - intro - race_len
            vid.text(d, (W // 2, 250), L["median"] + " · " + L[kind], 52, TEXT, anchor="mm")
            top = max(med.values())
            for k, (app, col) in enumerate((("steam", STEAM), ("ours", OURS))):
                y = 400 + k * 190
                vid.text(d, (470, y + 40), L[app], 50, col, anchor="rm")
                full = 1050 * med[app] / top
                bw = max(8, full * vid.ease(ot / 0.8))
                vid.rounded(d, (510, y, 510 + bw, y + 80), 16, col)
                vid.text(d, (530 + bw, y + 40), f"{med[app]:.1f} " + L["s"], 56, TEXT, anchor="lm")
            fast, slow = sorted(("steam", "ours"), key=lambda a: med[a])
            ratio = med[slow] / med[fast]
            if ot > 0.9 and ratio >= 1.1:
                vid.text(d, (W // 2, 880), f"{L[fast]}: " + L["faster"].format(x=ratio), 60, GOOD, anchor="mm")
            img = vid.fade(img, min(1.0, ot / 0.25))
            if t > intro + race_len + outro - 0.4:
                img = vid.fade(img, (intro + race_len + outro - t) / 0.4)
        w.put(img)
    w.close()
    print(out, f"{total / FPS:.1f}s", {a: (takes[a]["ready"], med[a]) for a in takes})


if __name__ == "__main__":
    main()
