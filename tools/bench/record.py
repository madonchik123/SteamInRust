"""Recordings for the videos (not used for the benchmark numbers).

    python -I record.py race   -> rec/race_{steam,ours}_{cold,warm}.mp4 + .json
    python -I record.py idle   -> rec/idle_{steam,ours}.mp4 + .json (live RAM/CPU samples)
"""
import collections
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402
from bench import RX, RY, RW, RH  # noqa: E402

REC = os.path.join(bench.OUT, "rec")
os.makedirs(REC, exist_ok=True)


def ffmpeg_region(path, x=RX, y=RY, w=RW, h=RH, fps=60):
    return subprocess.Popen(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
         f"ddagrab=output_idx=0:framerate={fps}:offset_x={x}:offset_y={y}:video_size={w}x{h}:draw_mouse=0",
         "-c:v", "h264_nvenc", "-preset", "p5", "-cq", "16", "-g", str(fps), path],
        stdin=subprocess.PIPE)


def stop_ffmpeg(p):
    p.stdin.write(b"q")
    p.stdin.flush()
    p.wait(30)


def race():
    for cold in (True, False):
        for app in ("steam", "ours"):
            k = "cold" if cold else "warm"
            path = os.path.join(REC, f"race_{app}_{k}.mp4")
            for attempt in range(3):
                res = bench.startup(app, cold=cold, record=path, tag=f"rec-race-{k}")
                if not res["disturbed"]:
                    break
            json.dump(res, open(path[:-4] + ".json", "w"), indent=1)


def idle(seconds=20):
    for app in ("steam", "ours"):
        path = os.path.join(REC, f"idle_{app}.mp4")
        bench.startup(app, keep=True, tag="rec-idle-setup")
        time.sleep(30)
        placer = bench.Placer(app, started=True)
        placer.tick()
        bench.wait_quiet()
        rec = ffmpeg_region(path)
        time.sleep(1.5)  # ffmpeg warm-up; samples start with the video's first second
        samples, prev = [], None
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < seconds:
            ps = bench.procs(app)
            ws = pr = cpu = 0.0
            for p in ps:
                try:
                    m = p.memory_info()
                    c = p.cpu_times()
                    ws += m.wset
                    pr += m.private
                    cpu += c.user + c.system
                except Exception:
                    pass
            t = time.perf_counter() - t0
            if prev is not None:
                samples.append({"t": round(t, 3), "private_mb": pr / 2**20, "ws_mb": ws / 2**20,
                                "cpu_pct_one_core": 100 * (cpu - prev[1]) / (t - prev[0]), "processes": len(ps),
                                "names": dict(collections.Counter(p.name() for p in ps))})
            prev = (t, cpu)
            time.sleep(0.25)
        stop_ffmpeg(rec)
        json.dump({"samples": samples, "video_offset_s": 1.5}, open(path[:-4] + ".json", "w"), indent=1)
        placer.release()
        bench.stop(app)


if __name__ == "__main__":
    {"race": race, "idle": idle}[sys.argv[1]]()
