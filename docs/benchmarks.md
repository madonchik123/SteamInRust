# SteamInRust vs Steam: benchmarks

Measured on 8 November 2026 on one PC, signed in to the same account, on the
same network. Every number is the **median of 5 runs**, with the range of the
runs in brackets. The measuring scripts and raw results were removed from the
repo to keep it all Rust; they're kept in its history at
[`tools/bench/` (commit 29d9fa1)](https://github.com/madonchik123/SteamInRust/tree/29d9fa1/tools/bench).

| Metric | Steam | SteamInRust |
|---|---|---|
| Cold start → library settled | **20.5 s** (18.1–20.6) | **8.1 s** (7.1–11.3) |
| Cold start → main window | 10.6 s (8.4–10.9) | 2.4 s (2.4–4.2) |
| Warm start → library settled | **20.1 s** (18.9–20.9) | **10.9 s** (8.1–11.0) |
| Warm start → main window | 10.3 s (9.0–11.1) | 2.4 s (2.4–4.6) |
| RAM, private bytes (60 s idle on the library) | **1130 MB** (1123–1142) | **245 MB** (243–249) |
| RAM, working set (same 60 s) | 1072 MB (1059–1074) | 206 MB (205–208) |
| CPU while idle, % of one core | **8.0 %** (7.9–8.8) | **0.03 %** (0.00–0.05) |
| Processes | 8 (steam.exe + 7 × steamwebhelper.exe) | 1 (steam-rs.exe) |
| Install size (program files) | 1083 MB, plus ~0.6 GB web cache in `%LOCALAPPDATA%\Steam` | 25.5 MB (steam-rs.exe) |
| Open the Store (click → settled) | 1.19 s (1.16–1.84) | 1.01 s (0.92–3.09) |
| Open a game page in the library | 0.89 s (0.84–1.45) | 0.03 s (0.02–1.22) |
| Open Friends & Chat | 1.31 s (1.24–1.39) | 0.06 s (0.05–0.43) |
| First visible reaction to a click: Store / game page / Friends | 0.07 / 0.10 / 0.72 s | 0.03 / 0.03 / 0.06 s |

## Where Steam holds its own

- **Opening the Store takes about the same time in both** (1.19 s vs 1.01 s).
  Both wait on the same Steam servers and image CDN, so the network sets the
  pace, not the UI.
- **SteamInRust's "settled" time is mostly image downloads.** Its window is up
  and usable at 2.4 s; the remaining 6–9 s is library art arriving through the
  network. That's also why its warm start (10.9 s) came out slower than its cold
  start (8.1 s) on this day: a warm file cache doesn't speed up the network.
- **Steam's cold and warm starts are almost the same** (20.5 s vs 20.1 s). On
  this machine its startup is bound by CPU and network, not disk.
- The first time a game page opens, SteamInRust has to fetch its art too (up to
  1.2 s). The 0.03 s median is for pages whose art was already loaded.

## Setup

| | |
|---|---|
| PC | AMD Ryzen 5 5600 (6 cores / 12 threads), 16 GB RAM, NVIDIA RTX 3060, one 1920×1080 165 Hz display |
| OS | Windows 11 Home 10.0.26300 |
| Steam | steam.exe 11.15.70.27, client package `1792618285`. Millennium (a third-party Steam mod) was uninstalled before testing, so this is stock Steam. |
| SteamInRust | release build of commit `af8a0ac` (v0.1.0) |
| Network | home connection; all HTTP(S) goes through a local proxy on this PC (same for both clients) |

The PC was in everyday use: Roblox Studio, Discord, a browser and the Claude app
were open. Each trial waited until total CPU load was under 25 % for 4 seconds
before starting, and the order of the two clients alternated every round, so
both saw the same conditions.

## Method

**Same starting page.** Both clients open on the library: Steam is launched as
`steam.exe steam://open/games`, SteamInRust as `steam-rs.exe --page=library`.
Both sign in automatically with the saved account. Steam's "always show the
account picker" option was switched off for the runs and switched back on
afterwards.

**Start-up time.** The harness ([`bench.py`](https://github.com/madonchik123/SteamInRust/blob/29d9fa1/tools/bench/bench.py)) places a
black backdrop at a fixed 1280×800 spot and starts the client. Every client
window that appears is moved into that spot and kept on top, with the main
window resized to 1280×800. It then grabs that region about 30 times a second
and records three moments:

- *main window*: the client's full-size window appears;
- *library settled*: the last moment the picture changed by more than 0.5 % of
  its pixels, followed by 5 s (SteamInRust) or 6 s (Steam) without change.
  This is the "visually complete" time: every image is in and nothing moves any
  more.

Notification pop-ups that a client shows after its main window is up are left
where the client puts them, outside the measured region.

**Cold vs warm.** There's no admin access on this PC, so a cold start can't
come from a reboot or RAMMap. Instead, before each cold run the harness fills
all free and standby memory (about 7.5 GB) and frees it again, which makes
Windows drop its file cache. A warm run starts right after the previous run
closed. Before each run both clients are fully closed: Steam with
`steam.exe -shutdown`, SteamInRust by ending its process.

**Idle RAM and CPU.** On warm runs the harness waits 30 s after the library
settles, then samples every second for 60 s, summed over all the client's
processes:

- private bytes, the memory that belongs to the client alone (the main RAM
  number, because Windows can trim working sets when RAM gets tight);
- working set;
- the process count;
- CPU time used during the 60 s.

Steam's processes are steam.exe and steamwebhelper.exe. SteamService.exe runs
as a Windows service and can't be read without admin rights, so it isn't
counted. SteamInRust is the single steam-rs.exe.

**Opening pages** ([`nav.py`](https://github.com/madonchik123/SteamInRust/blob/29d9fa1/tools/bench/nav.py)). With the client idle on
the library, the harness clicks the Store tab, a game in the library list
(*7 Days to Die*, first in both lists) and the Friends & Chat button. The clock
starts on the mouse-button release, since both clients act on release. Both
clients open Friends & Chat as a separate window, and that window is what gets
timed. "First reaction" is the first frame that differs; "settled" uses the same
rule as above with a 3 s quiet period. Between clicks the harness goes back to
the library and waits for it to settle.

**Disturbances.** If the mouse moved, or another window covered the measured
region during a trial, the trial was thrown out and run again. The final set
needed no re-runs.

**Disk size.** For Steam, everything in `C:\Program Files (x86)\Steam` except
games (`steamapps`), user data, config, logs, dumps and caches; this includes
246 MB of update archives in `package/`. For SteamInRust, the release
`steam-rs.exe`; the installer also ships the steamclient.dll replacement, which
isn't counted here.

### A mistake caught along the way

In the first full run, the harness also pulled Steam's notification toast (a
"9 new friend invites" pop-up, a separate window) into the middle of the
measured region. Its disappearance about 5 s later became the "last change",
which made Steam look about 5 s slower than it is. That run was thrown away,
the harness was fixed to leave windows that appear after start-up alone, and
everything was measured again. The numbers above come from that second run.

## Videos

Recorded after the benchmark runs, at 1080p and 30 fps, with English and
Russian captions. The full videos are kept out of git in `media/`:

| File | What it shows |
|---|---|
| `steaminrust-vs-steam-race-{en,ru}.mp4` / `.webm` | One real-time cold start of each client side by side, with timers that stop at "library settled", then the 5-run medians |
| `steaminrust-vs-steam-idle-{en,ru}.mp4` | Both clients idling on the library with live RAM, CPU and process counters from that take, then the medians and install size |
| `steaminrust-tour-{en,ru}.mp4` | SteamInRust's store, a trailer, the library, the Shift+Tab overlay, inventory and market, listings, chat and profile, in `--demo` mode with sample data |

[`docs/media/en/startup-race.gif`](media/en/startup-race.gif) and
[`docs/media/ru/startup-race.gif`](media/ru/startup-race.gif) are 760 px cuts of
the race for the README.

Each client was recorded on its own, never both at once. In the race video the
two launches are lined up using a marker the harness shows at the moment of
launch. A recorded take is a single run, so its time can differ from the median.
The SteamInRust take in the race video took 12.2 s, against a median of 8.1 s;
the end card shows the medians.

Privacy: the account name, avatar, wallet balance and profile tab are blurred in
both clients, and so is the whole frame until each client's main window is up,
because both sign-in screens show the account name. The feature tour uses
`--demo` mode, so no real account data appears in it.

## Reproduce

The scripts live in the repo's history: check out
[`tools/bench/` at commit 29d9fa1](https://github.com/madonchik123/SteamInRust/tree/29d9fa1/tools/bench), build the app with
`cargo build --release -p steam-rs`, and run `bash tools/bench/full.sh`. It runs
the 5 start-up rounds, the click tests and the recordings, and needs Python 3.
`python -I tools/bench/analyze.py` then prints the table. Steam has to sign in
without the account picker, and the mouse and screen must be left alone while
it runs (about 50 minutes).
