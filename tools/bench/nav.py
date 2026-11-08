"""Time to open a page: click, then wait for the picture to settle.

    python -I nav.py steam|ours [rounds] [--only store,game,friends]
The client must already be running at the library (bench.py startup ... --keep).
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import json
import sys
import time

import bench
import winutil as wu
from bench import RX, RY, RW, RH, small, changed, grab
from PIL import ImageGrab

# Click targets, relative to the 1280x800 window.
TARGETS = {
    "steam": {"store": (120, 47), "library": (243, 47), "game": (90, 246), "home": (243, 47), "friends": (1210, 774)},
    "ours": {"store": (318, 30), "library": (424, 30), "game": (94, 344), "home": (118, 93), "friends": (1145, 784)},
}


def at(app, name):
    x, y = TARGETS[app][name]
    return RX + x, RY + y


def press(app, name):
    """A click whose timed part is the button release (both clients act on release)."""
    def prep():
        wu.move_cursor(*at(app, name))
        time.sleep(0.25)
        wu.user32.mouse_event(0x2, 0, 0, 0, 0)  # down
        time.sleep(0.05)

    def go():
        if not getattr(go, "prepped", False):
            prep()
        wu.user32.mouse_event(0x4, 0, 0, 0, 0)  # up: the clock starts here
        wu.move_cursor(*bench.PARK)
        go.prepped = False
    go.prep = prep
    return go


def friends_box(app):
    """Both clients open Friends & Chat as a separate window: time that window."""
    def find():
        pids = {p.pid for p in bench.procs(app)}
        for hwnd, r in wu.windows(pids):
            t = wu.title(hwnd).lower()
            if "друз" in t or "friend" in t:
                if not wu.is_topmost(hwnd):
                    wu.place(hwnd, r[0], r[1])
                return r
        return None
    return find


def close_friends(app):
    pids = {p.pid for p in bench.procs(app)}
    for hwnd, r in wu.windows(pids):
        t = wu.title(hwnd).lower()
        if "друз" in t or "friend" in t:
            wu.close(hwnd)


def timed(action, box_fn=None, quiet=3.0, timeout=30.0, level=0.005, guard=None):
    box = (RX, RY, RX + RW, RY + RH)
    anchor = small(grab()) if box_fn is None else None
    first = last = appeared = None
    n = 0
    if hasattr(action, "prep"):
        action.prep()
        action.prepped = True
        if box_fn is None:
            anchor = small(grab())  # includes the pressed-button look
    t0 = time.perf_counter()
    action()
    if guard is not None:
        guard.rebase()
    while True:
        b = box if box_fn is None else box_fn()
        t = time.perf_counter() - t0
        if b is None:
            if t > timeout:
                break
            time.sleep(0.005)
            continue
        if appeared is None:
            appeared = t
        f = small(ImageGrab.grab(bbox=b, all_screens=True))
        n += 1
        if guard is not None:
            guard.check()
        t = time.perf_counter() - t0
        if changed(f, anchor) > level:
            anchor, last = f, t
            if first is None:
                first = t
        if last is not None and t - last >= quiet:
            break
        if t > timeout:
            break
    out = {"first_change_s": first, "settled_s": last, "window_s": appeared, "frames": n}
    if guard is not None:
        out["disturbed"] = sorted(guard.reasons)
    return out


def settle(seconds=2.0):
    timed(lambda: None, quiet=seconds, timeout=15)


def main():
    app = sys.argv[1]
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2].isdigit() else 5
    only = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else ["store", "game", "friends"]
    placer = bench.Placer(app, started=True)
    for r in range(rounds):
        for what in only:
          for attempt in range(3):
            bench.wait_quiet()
            placer.tick()
            guard = bench.Guard(app)
            if what == "store":
                press(app, "library")()
                settle()
                res = timed(press(app, "store"), guard=guard)
                press(app, "library")()
            elif what == "game":
                press(app, "home")()
                settle()
                res = timed(press(app, "game"), guard=guard)
                press(app, "home")()
            else:
                close_friends(app)
                settle()
                res = timed(press(app, "friends"), friends_box(app), guard=guard)
                close_friends(app)
            settle()
            res.update({"kind": "nav", "app": app, "what": what, "round": r, "attempt": attempt,
                        "time": time.strftime("%Y-%m-%d %H:%M:%S")})
            with open(bench.RESULTS, "a", encoding="utf-8") as f:
                f.write(json.dumps(res) + "\n")
            print(json.dumps(res), flush=True)
            if not res.get("disturbed") and res["settled_s"] is not None:
                break


if __name__ == "__main__":
    main()
