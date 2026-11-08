"""Feature-tour clips of SteamInRust in --demo mode, full screen (1920x1080).

    python -I tour.py shot en "--page=store"         screenshot for planning
    python -I tour.py record en|ru                   all clips -> rec/tour_<lang>_<n>.mp4
"""
import ctypes
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402
import winutil as wu  # noqa: E402
from record import ffmpeg_region, stop_ffmpeg, REC  # noqa: E402
from PIL import ImageGrab  # noqa: E402

APP_RON = os.path.expandvars(r"%APPDATA%\SteamInRust\data\app.ron")
PARK = (960, 18)  # empty strip of our title bar: hovering it changes nothing


def set_lang(lang):
    s = open(APP_RON, encoding="utf-8").read()
    s2 = re.sub(r'language:(None|Some\(\\"[a-z]+\\"\))', f'language:Some(\\\\"{lang}\\\\")', s, count=1)
    if s2 == s and f'language:Some(\\"{lang}\\")' not in s:
        raise SystemExit("could not set language")
    open(APP_RON, "w", encoding="utf-8").write(s2)


def launch(args, image=None):
    """Start a demo copy, full screen, above a backdrop (black or a 'game' picture)."""
    bench.stop("demo")
    wu.move_cursor(*PARK)
    bd_args = [sys.executable, "-I", os.path.join(bench.HERE, "backdrop.py"), "0", "0", "1920", "1080"]
    if image:
        bd_args.append(image)
    bd = subprocess.Popen(bd_args, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True)
    time.sleep(1.0)
    subprocess.Popen([bench.OURS_EXE, "--demo", "--separate", *args], cwd=os.path.dirname(bench.OURS_EXE))
    end = time.time() + 30
    main = None
    while time.time() < end:
        pids = {p.pid for p in bench.procs("demo")}
        for h, (l, t, r, b) in wu.windows(pids):
            if wu.title(h) == "SteamInRust" and r - l >= 900:
                main = h
        if main:
            break
        time.sleep(0.1)
    if main and "--overlay" not in args:
        wu.place(main, 0, 0, 1920, 1079)  # not exactly full screen: Windows would treat it as exclusive fullscreen
    time.sleep(0.5)
    return bd, main


def settle(seconds=6.0):
    time.sleep(seconds)


def clip(name, args, actions, seconds, image=None, wait=6.0):
    """Record one clip. `actions` = [(t, fn)], t in seconds from the clip start."""
    bd, main = launch(args, image)
    settle(wait)
    path = os.path.join(REC, name)
    rec = ffmpeg_region(path, 0, 0, 1920, 1080, fps=60)
    time.sleep(1.2)
    t0 = time.perf_counter()
    for t, fn in sorted(actions, key=lambda a: a[0]):
        while time.perf_counter() - t0 < t:
            time.sleep(0.005)
        fn()
    while time.perf_counter() - t0 < seconds:
        time.sleep(0.01)
    stop_ffmpeg(rec)
    bench.stop("demo")
    bd.kill()
    return path


def click(x, y):
    def f():
        wu.click(x, y)
        wu.move_cursor(*PARK)
    return f


def scroll(x, y, notches):
    """Smooth-ish wheel scroll; the pointer stays over the content while egui handles it."""
    def f():
        wu.move_cursor(x, y)
        time.sleep(0.15)
        for _ in range(notches * 2):
            wu.user32.mouse_event(0x800, 0, 0, ctypes.c_uint32(-60 & 0xFFFFFFFF).value, 0)
            time.sleep(0.07)
    return f


def shot(lang, args, name="tour-shot.png", wait=7.0):
    set_lang(lang)
    bd, main = launch(args.split())
    settle(wait)
    ImageGrab.grab(bbox=(0, 0, 1920, 1080), all_screens=True).save(os.path.join(bench.SHOTS, name))
    pids = {p.pid for p in bench.procs("demo")}
    for h, r in wu.windows(pids):
        print(wu.title(h), r, wu.is_topmost(h))
    bench.stop("demo")
    bd.kill()


if __name__ == "__main__":
    os.makedirs(bench.SHOTS, exist_ok=True)
    if sys.argv[1] == "shot":
        shot(sys.argv[2], sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else "tour-shot.png")
    elif sys.argv[1] == "stop":
        bench.stop("demo")


GAME_BG = os.path.join(bench.OUT, "game_bg.png")


def make_game_bg():
    """A blurred, dimmed Hades scene (cropped from the app's own library hero) to sit behind the overlay."""
    from PIL import Image, ImageFilter, ImageEnhance
    src = Image.open(os.path.join(bench.SHOTS, "t_library.png")).crop((281, 60, 1920, 408))
    w, h = src.size
    scale = 1080 / h
    big = src.resize((int(w * scale), 1080), Image.LANCZOS)
    x0 = (big.width - 1920) // 2
    big = big.crop((x0, 0, x0 + 1920, 1080)).filter(ImageFilter.GaussianBlur(3))
    ImageEnhance.Brightness(big).enhance(0.75).save(GAME_BG)


def record_all(lang, only=None):
    set_lang(lang)
    if not os.path.exists(GAME_BG):
        make_game_bg()
    p = lambda k: f"tour_{lang}_{k}.mp4"  # noqa: E731
    if only:
        real_clip = clip
        def clip_(name, *a, **k):  # noqa: E306
            if any(name.endswith(f"_{o}.mp4") for o in only):
                real_clip(name, *a, **k)
        globals()["clip"] = clip_
    clip(p("store"), ["--page=store"], [(2.9, click(40, 700)), (3.6, scroll(960, 640, 3)), (5.0, scroll(960, 640, 3))], 8.0)
    clip(p("trailer"), ["--page=store", "--arg=app:1145360"], [], 7.0, wait=8.0)
    clip(p("library"), ["--page=library", "--arg=1145360"], [(1.6, click(1890, 600)), (2.2, scroll(1100, 700, 3))], 5.0)
    clip(p("overlay"), ["--overlay"], [], 5.5, image=GAME_BG)
    clip(p("inventory"), ["--page=inventory"], [(1.0, click(180, 307)), (3.0, click(285, 307))], 6.0)
    clip(p("listings"), ["--page=inventory", "--arg=listings"], [], 4.5)
    clip(p("chat"), ["--page=community", "--arg=76561197960265828"], [], 4.5)
    clip(p("profile"), ["--page=profile"], [(1.0, click(1890, 600)), (1.6, scroll(960, 700, 4))], 4.5)


if __name__ == "__main__" and sys.argv[1] == "record":
    record_all(sys.argv[2], sys.argv[3].split(",") if len(sys.argv) > 3 else None)
