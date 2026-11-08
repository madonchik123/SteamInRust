"""Benchmark harness: official Steam vs SteamInRust.

    python -I bench.py startup steam|ours [--cold] [--idle] [--record out.mp4] [--keep]
    python -I bench.py shot NAME             (save the capture region as PNG)
    python -I bench.py stop steam|ours

Every result is appended to results.jsonl as one JSON object.
"""
import argparse
import json
import os
import subprocess
import sys
import time

import numpy as np
import psutil
from PIL import Image, ImageGrab

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import winutil as wu  # noqa: E402

STEAM_DIR = r"C:\Program Files (x86)\Steam"
REPO = os.path.dirname(os.path.dirname(HERE))
# Build with `cargo build --release -p steam-rs` (or point STEAMINRUST_EXE at another build).
OURS_EXE = os.environ.get("STEAMINRUST_EXE", os.path.join(REPO, "target", "release", "steam-rs.exe"))
# Everything the scripts write goes here (git-ignored).
OUT = os.environ.get("BENCH_OUT", os.path.join(HERE, "out"))
os.makedirs(OUT, exist_ok=True)
RESULTS = os.path.join(OUT, "results.jsonl")
SHOTS = os.path.join(OUT, "shots")

# The window under test always lives here: 1280x800, centred on the 1080p screen.
RX, RY, RW, RH = 320, 140, 1280, 800
PARK = (1915, 1075)  # cursor parking spot, outside the region

APPS = {
    "steam": {"cmd": [os.path.join(STEAM_DIR, "steam.exe"), "steam://open/games"], "names": {"steam.exe", "steamwebhelper.exe"}},
    "ours": {"cmd": [OURS_EXE, "--page=library"], "names": {"steam-rs.exe"}},
    "demo": {"cmd": [OURS_EXE, "--demo", "--separate", "--page=library"], "names": {"steam-rs.exe"}},
}


def procs(app):
    """Every running process belonging to the client (Steam: its own exes, not games)."""
    out = []
    names = APPS[app]["names"]
    for p in psutil.process_iter(["name", "exe"]):
        try:
            n = (p.info["name"] or "").lower()
            exe = (p.info["exe"] or "").lower()
        except Exception:
            continue
        if app in ("ours", "demo"):
            if exe == OURS_EXE.lower():
                out.append(p)  # only our benchmark build, never another session's copy
        elif n in names:
            out.append(p)
        elif app == "steam" and exe.startswith(STEAM_DIR.lower()) and "steamapps" not in exe:
            out.append(p)
    return out


def stop(app, timeout=60):
    ps = procs(app)
    if not ps:
        return
    if app == "steam":
        subprocess.Popen([os.path.join(STEAM_DIR, "steam.exe"), "-shutdown"])
        end = time.time() + timeout
        while procs(app) and time.time() < end:
            time.sleep(0.5)
    for p in procs(app):
        try:
            p.kill()
        except Exception:
            pass
    time.sleep(1.5)


def wait_quiet(limit=25.0, max_wait=900):
    """Wait until the PC is mostly idle (two 2-second samples under `limit` %)."""
    end, ok = time.time() + max_wait, 0
    while time.time() < end:
        ok = ok + 1 if psutil.cpu_percent(interval=2.0) < limit else 0
        if ok >= 2:
            return True
    return False


def evict_file_cache():
    """No admin rights, so no RAMMap: fill free + standby memory with our own
    pages so Windows drops its file cache, then free it all again."""
    avail = psutil.virtual_memory().available
    target = avail - 900 * 2**20
    chunks, got = [], 0
    while got < target:
        n = min(256 * 2**20, target - got)
        chunks.append(np.ones(n, dtype=np.uint8))
        got += n
    del chunks
    time.sleep(3)
    return got


class Guard:
    """Notices other windows over the region and real user input during a trial."""

    def __init__(self, app, extra_pids=()):
        self.app, self.extra = app, set(extra_pids)
        self.tick0 = wu.last_input_tick()
        self.pos0 = wu.cursor()
        self.reasons = set()
        self.n = 0
        self.pids = set()

    def rebase(self):
        """Call after our own synthetic clicks."""
        self.tick0 = wu.last_input_tick()
        self.pos0 = wu.cursor()

    def check(self):
        self.n += 1
        if self.n % 8:
            return
        if self.n % 40 == 8:
            self.pids = {p.pid for p in procs(self.app)} | self.extra
        # Only the mouse: something on this PC bumps GetLastInputInfo every few
        # seconds without any real input, so that signal is useless here.
        if wu.cursor() != self.pos0:
            self.reasons.add("mouse moved")
        for fx in (0.1, 0.5, 0.9):
            for fy in (0.1, 0.5, 0.9):
                pid = wu.owner_pid_at(RX + RW * fx, RY + RH * fy)
                if pid is not None and pid not in self.pids:
                    try:
                        name = psutil.Process(pid).name()
                    except Exception:
                        name = str(pid)
                    self.reasons.add("covered by " + name)


def grab():
    return ImageGrab.grab(bbox=(RX, RY, RX + RW, RY + RH), all_screens=True)


def small(img):
    return np.asarray(img.convert("L").resize((320, 200), Image.BILINEAR), dtype=np.int16)


def changed(a, b, level=20):
    if a is None or b is None:
        return 1.0
    return float((np.abs(a - b) > level).mean())


class Backdrop:
    def __init__(self):
        self.p = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "backdrop.py"), str(RX), str(RY), str(RW), str(RH)],
                                  stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True)
        time.sleep(1.0)

    def mark(self, on=True):
        self.p.stdin.write("mark\n" if on else "clear\n")
        self.p.stdin.flush()

    def close(self):
        self.p.kill()


class Placer:
    """Keeps the client's windows inside the region and above the backdrop."""

    def __init__(self, app, started=False):
        self.app = app
        self.pids = set()
        self.last_scan = 0
        self.placed = {}
        self.ignored = set()
        self.main_seen = started

    def tick(self):
        now = time.time()
        if now - self.last_scan > 0.2:
            self.pids = {p.pid for p in procs(self.app)}
            self.last_scan = now
        main = False
        for hwnd, (l, t, r, b) in wu.windows(self.pids):
            w, h = r - l, b - t
            name = wu.title(hwnd).lower()
            if h > w * 1.25 or "friend" in name or "друз" in name:
                continue  # friends list: not part of the main window
            if w >= 880 and h >= 500:
                main = True
                if (l, t, w, h) != (RX, RY, RW, RH) or self.placed.get(hwnd) != "main" or not wu.is_topmost(hwnd):
                    wu.place(hwnd, RX, RY, RW, RH)
                    self.placed[hwnd] = "main"
            elif self.main_seen or hwnd in self.ignored:
                continue  # after start-up, small windows (notifications...) stay where the client puts them
            elif hwnd not in self.placed and wu.is_topmost(hwnd):
                self.ignored.add(hwnd)  # already always-on-top: a notification toast, not a start-up window
            elif hwnd not in self.placed or (l, t) != self.placed[hwnd] or not wu.is_topmost(hwnd):
                x, y = RX + (RW - w) // 2, RY + (RH - h) // 2
                wu.place(hwnd, x, y)
                self.placed[hwnd] = (x, y)
        if main:
            self.main_seen = True
        return main

    def release(self):
        for hwnd in self.placed:
            wu.untop(hwnd)


def watch(t0, placer=None, quiet=5.0, timeout=90.0, min_time=1.0, need_main=True, level=0.005, guard=None):
    """Grab frames until the picture has been still for `quiet` seconds.
    Returns (first_change, last_change, main_seen, frames)."""
    anchor, first, last, main_at = None, None, None, None
    base = small(grab())
    anchor = base
    n = 0
    while True:
        now = time.perf_counter() - t0
        if placer is not None and placer.tick() and main_at is None:
            main_at = now
        f = small(grab())
        n += 1
        if guard is not None:
            guard.check()
        t = time.perf_counter() - t0
        if changed(f, anchor) > level:
            anchor = f
            last = t
            if first is None:
                first = t
        ready = (main_at is not None) or not need_main
        if ready and last is not None and t - last >= quiet and t >= min_time:
            break
        if t > timeout:
            break
    return first, last, main_at, n, f


def sample_idle(app, seconds=60, settle=30):
    time.sleep(settle)
    ps = procs(app)
    cpu0 = {}
    for p in ps:
        try:
            c = p.cpu_times()
            cpu0[p.pid] = c.user + c.system
        except Exception:
            pass
    t0 = time.time()
    ws, priv, counts = [], [], []
    while time.time() - t0 < seconds:
        cur = procs(app)
        w = pr = 0
        for p in cur:
            try:
                m = p.memory_info()
                w += m.wset
                pr += m.private
            except Exception:
                pass
        ws.append(w)
        priv.append(pr)
        counts.append(len(cur))
        time.sleep(1.0)
    elapsed = time.time() - t0
    cpu = 0.0
    for p in procs(app):
        try:
            c = p.cpu_times()
            cpu += c.user + c.system - cpu0.get(p.pid, 0.0)
        except Exception:
            pass
    names = sorted({p.name() for p in procs(app)})
    return {
        "ws_mb_median": float(np.median(ws)) / 2**20,
        "ws_mb_max": max(ws) / 2**20,
        "private_mb_median": float(np.median(priv)) / 2**20,
        "private_mb_max": max(priv) / 2**20,
        "cpu_pct_of_one_core": 100.0 * cpu / elapsed,
        "cpu_pct_of_machine": 100.0 * cpu / elapsed / psutil.cpu_count(),
        "processes": int(np.median(counts)),
        "process_names": names,
        "ws_series_mb": [round(x / 2**20, 1) for x in ws],
        "private_series_mb": [round(x / 2**20, 1) for x in priv],
    }


def save_thumb(img, name):
    os.makedirs(SHOTS, exist_ok=True)
    img.save(os.path.join(SHOTS, name))


def startup(app, cold=False, idle=False, record=None, keep=False, tag=""):
    stop(app)
    if app in ("ours", "demo"):
        stop("demo" if app == "ours" else "ours")
    quiet_ok = wait_quiet()
    evicted = evict_file_cache() if cold else 0
    load = {"cpu_pct_before": psutil.cpu_percent(interval=1.0), "avail_mb_before": psutil.virtual_memory().available // 2**20}
    wu.move_cursor(*PARK)
    bd = Backdrop()
    rec = None
    if record:
        rec = subprocess.Popen(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
             f"ddagrab=output_idx=0:framerate=60:offset_x={RX}:offset_y={RY}:video_size={RW}x{RH}:draw_mouse=0",
             "-c:v", "h264_nvenc", "-preset", "p5", "-cq", "16", "-g", "60", record],
            stdin=subprocess.PIPE)
        time.sleep(2.0)
    placer = Placer(app)
    guard = Guard(app, [bd.p.pid])
    bd.mark()
    t0 = time.perf_counter()
    subprocess.Popen(APPS[app]["cmd"], cwd=os.path.dirname(APPS[app]["cmd"][0]))
    first, last, main_at, n, _ = watch(t0, placer, quiet=6.0 if app == "steam" else 5.0, guard=guard)
    img = grab()
    res = {
        "kind": "startup", "app": app, "cold": cold, "tag": tag, "time": time.strftime("%Y-%m-%d %H:%M:%S"),
        "first_pixels_s": first, "main_window_s": main_at, "visually_complete_s": last, "frames": n,
        "evicted_mb": evicted // 2**20, "quiet_ok": quiet_ok, **load,
        "disturbed": sorted(guard.reasons),
    }
    save_thumb(img.resize((640, 400)), f"{app}-{'cold' if cold else 'warm'}-{int(time.time())}.png")
    if idle:
        res["idle"] = sample_idle(app)
    if rec:
        time.sleep(1.0)
        rec.stdin.write(b"q")
        rec.stdin.flush()
        rec.wait(20)
    if not keep:
        placer.release()
        stop(app)
    bd.close()
    with open(RESULTS, "a", encoding="utf-8") as f:
        f.write(json.dumps(res) + "\n")
    print(json.dumps({k: v for k, v in res.items() if k != "idle"}), flush=True)
    if "idle" in res:
        print(json.dumps({k: v for k, v in res["idle"].items() if not k.endswith("series_mb")}), flush=True)
    return res


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd")
    s = sub.add_parser("startup")
    s.add_argument("app")
    s.add_argument("--cold", action="store_true")
    s.add_argument("--idle", action="store_true")
    s.add_argument("--record")
    s.add_argument("--keep", action="store_true")
    s.add_argument("--tag", default="")
    sh = sub.add_parser("shot")
    sh.add_argument("name")
    st = sub.add_parser("stop")
    st.add_argument("app")
    a = ap.parse_args()
    if a.cmd == "startup":
        startup(a.app, a.cold, a.idle, a.record, a.keep, a.tag)
    elif a.cmd == "shot":
        save_thumb(grab(), a.name + ".png")
    elif a.cmd == "stop":
        stop(a.app)


if __name__ == "__main__":
    main()
