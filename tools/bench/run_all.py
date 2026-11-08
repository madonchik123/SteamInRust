"""5 rounds of: cold start, warm start + 60 s idle, for both clients.
Order alternates each round. Disturbed trials are re-run (up to 3 tries)."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sys
import bench

ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 5


def trial(app, cold, idle, r):
    for attempt in range(3):
        res = bench.startup(app, cold=cold, idle=idle, tag=f"main-r{r}-a{attempt}")
        if not res["disturbed"] and res["visually_complete_s"] is not None and res["main_window_s"] is not None:
            return res
        print(f"  re-running ({res['disturbed']})", flush=True)
    return res


for r in range(ROUNDS):
    apps = ["steam", "ours"] if r % 2 == 0 else ["ours", "steam"]
    for app in apps:
        trial(app, cold=True, idle=False, r=r)
        trial(app, cold=False, idle=True, r=r)
print("done", flush=True)
