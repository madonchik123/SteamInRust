"""results.jsonl -> summary.json + a markdown table (medians of undisturbed main runs)."""
import json
import os
import statistics as st
import sys
sys.stdout.reconfigure(encoding="utf-8")

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get("BENCH_OUT", os.path.join(HERE, "out"))
os.makedirs(OUT, exist_ok=True)
src = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "results.jsonl")
rows = [json.loads(l) for l in open(src, encoding="utf-8") if l.strip()]


def ok(r):
    return not r.get("disturbed")


def med(xs):
    xs = [x for x in xs if x is not None]
    return (st.median(xs), min(xs), max(xs), len(xs)) if xs else (None, None, None, 0)


out = {}
for app in ("steam", "ours"):
    a = {}
    st_rows = [r for r in rows if r["kind"] == "startup" and r["app"] == app and r["tag"].startswith("main") and ok(r)]
    for cold in (True, False):
        rs = [r for r in st_rows if r["cold"] == cold]
        k = "cold" if cold else "warm"
        a[k + "_ready"] = med([r["visually_complete_s"] for r in rs])
        a[k + "_window"] = med([r["main_window_s"] for r in rs])
        a[k + "_first_pixels"] = med([r["first_pixels_s"] for r in rs])
        a[k + "_cpu_before"] = med([r["cpu_pct_before"] for r in rs])
    idle = [r["idle"] for r in st_rows if "idle" in r]
    for key in ("private_mb_median", "private_mb_max", "ws_mb_median", "ws_mb_max", "cpu_pct_of_one_core", "cpu_pct_of_machine", "processes"):
        a[key] = med([i[key] for i in idle])
    a["process_names"] = sorted({n for i in idle for n in i["process_names"]})
    nav = [r for r in rows if r["kind"] == "nav" and r["app"] == app and ok(r)]
    for what in ("store", "game", "friends"):
        rs = [r for r in nav if r["what"] == what]
        a["nav_" + what] = med([r["settled_s"] for r in rs])
        a["nav_" + what + "_first"] = med([r["first_change_s"] for r in rs])
    a["disturbed_reruns"] = sum(1 for r in rows if r["app"] == app and r.get("disturbed"))
    out[app] = a

json.dump(out, open(os.path.join(OUT, "summary.json"), "w"), indent=1)


def dir_mb(path, skip=()):
    total = 0
    for root, dirs, files in os.walk(path):
        dirs[:] = [x for x in dirs if os.path.join(root, x) not in skip]
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total / 2**20


steam = r"C:\Program Files (x86)\Steam"
not_client = {os.path.join(steam, x) for x in ("steamapps", "userdata", "config", "logs", "dumps", "appcache", "depotcache")}
exe = os.environ.get("STEAMINRUST_EXE", os.path.join(os.path.dirname(os.path.dirname(HERE)), "target", "release", "steam-rs.exe"))
disk = {"steam_client_mb": round(dir_mb(steam, not_client)), "steam_webcache_mb": round(dir_mb(os.path.expandvars(r"%LOCALAPPDATA%\Steam"))),
        "ours_mb": round(os.path.getsize(exe) / 2**20, 1) if os.path.exists(exe) else None}
json.dump(disk, open(os.path.join(OUT, "disk.json"), "w"), indent=1)


def f(v, unit="", nd=2):
    m, lo, hi, n = v
    if m is None:
        return "–"
    return f"{m:.{nd}f}{unit} ({lo:.{nd}f}–{hi:.{nd}f}, n={n})"


S, O = out["steam"], out["ours"]
lines = [
    "| Metric (median, min–max) | Steam | SteamInRust |",
    "|---|---|---|",
    f"| Cold start → library settled | {f(S['cold_ready'], ' s')} | {f(O['cold_ready'], ' s')} |",
    f"| Cold start → main window | {f(S['cold_window'], ' s')} | {f(O['cold_window'], ' s')} |",
    f"| Warm start → library settled | {f(S['warm_ready'], ' s')} | {f(O['warm_ready'], ' s')} |",
    f"| Warm start → main window | {f(S['warm_window'], ' s')} | {f(O['warm_window'], ' s')} |",
    f"| RAM, private bytes (idle 60 s) | {f(S['private_mb_median'], ' MB', 0)} | {f(O['private_mb_median'], ' MB', 0)} |",
    f"| RAM, working set (idle 60 s) | {f(S['ws_mb_median'], ' MB', 0)} | {f(O['ws_mb_median'], ' MB', 0)} |",
    f"| CPU idle, % of one core | {f(S['cpu_pct_of_one_core'], ' %')} | {f(O['cpu_pct_of_one_core'], ' %')} |",
    f"| Processes | {f(S['processes'], '', 0)} | {f(O['processes'], '', 0)} |",
    f"| Open Store (click → settled) | {f(S['nav_store'], ' s')} | {f(O['nav_store'], ' s')} |",
    f"| Open a game page | {f(S['nav_game'], ' s')} | {f(O['nav_game'], ' s')} |",
    f"| Open Friends & Chat | {f(S['nav_friends'], ' s')} | {f(O['nav_friends'], ' s')} |",
    f"| Open Store: first reaction | {f(S['nav_store_first'], ' s', 3)} | {f(O['nav_store_first'], ' s', 3)} |",
    f"| Open a game page: first reaction | {f(S['nav_game_first'], ' s', 3)} | {f(O['nav_game_first'], ' s', 3)} |",
    f"| Open Friends: first reaction | {f(S['nav_friends_first'], ' s', 3)} | {f(O['nav_friends_first'], ' s', 3)} |",
]
table = "\n".join(lines)
open(os.path.join(OUT, "table.md"), "w", encoding="utf-8").write(table + "\n")
print(table)
print("processes:", S["process_names"], O["process_names"])
print("re-runs:", S["disturbed_reruns"], O["disturbed_reruns"])
