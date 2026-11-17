# SteamInRust vs Steam: benchmarks

Measured on 17 November 2026 on one PC, signed in to the same account, on the
same network. Every number is the **middle result of 3 runs**, with the lowest
and highest run in brackets. Three clients were measured in turn: Steam,
SteamInRust 0.7.0 as released, and SteamInRust 0.7.1, which adds the memory and
CPU fixes described under [What changed in 0.7.1](#what-changed-in-071). The
measuring scripts were removed from the repo to keep it all Rust; they're kept
in its history at
[`tools/bench/` (commit 29d9fa1)](https://github.com/madonchik123/SteamInRust/tree/29d9fa1/tools/bench).

**Short version:** SteamInRust shows its first window about 2.5 times sooner
than Steam, has the library fully drawn about 3 times sooner, and uses about a
sixth of the memory and almost no CPU while idle.

| | Steam | SteamInRust 0.7.0 | SteamInRust 0.7.1 |
|---|---|---|---|
| **Cold start → first window on screen** | 6.4 s (1.7–6.4) | 3.0 s (2.6–3.1) | **2.6 s** (2.5–2.6) |
| Cold start → full-size main window | 9.0 s (8.9–10.5) | 3.0 s (2.9–5.0) | 2.4 s (2.4–4.9) |
| **Cold start → library fully drawn** | 14.7 s (14.3–16.5) | 6.4 s (6.4–6.6) | **4.8 s** (4.8–6.3) |
| Warm start → first window on screen | 5.8 s (5.8–5.8) | 2.5 s (2.5–2.6) | 2.5 s (2.5–2.5) |
| Warm start → full-size main window | 8.5 s (7.9–8.9) | 3.6 s (2.4–4.4) | 4.4 s (2.4–4.9) |
| Warm start → library fully drawn | 14.5 s (13.6–14.5) | 5.0 s (4.7–5.7) | 5.8 s (4.8–6.4) |
| **Memory while idle** (private bytes) | **1019 MB** (1003–1126) | 253 MB (244–257) | **163 MB** (153–163) |
| Memory while idle (working set) | 977 MB (953–1085) | 228 MB (220–231) | 134 MB (126–135) |
| **CPU while idle**, % of one core | **7.6 %** (7.2–9.4) | 0.10 % (0.08–0.21) | **0.00 %** (0.00–0.05) |
| Processes | 8 (steam.exe + 7 × steamwebhelper.exe) | 1 | 1 |
| Program size | 1084 MB, plus ~0.6 GB web cache in `%LOCALAPPDATA%\Steam` | 29.8 MB | 29.8 MB |
| Open the Store (click → settled) | 1.33 s (1.25–2.90) | not measured | 0.75 s (0.59–2.76) |
| Open a game page in the library | 0.70 s (0.69–1.05) | not measured | 0.05 s (0.04–0.85) |
| Open Friends & Chat | 1.20 s (1.19–1.21) | not measured | 0.06 s (0.05–0.08) |
| First visible reaction to a click: Store / game page / Friends | 0.06 / 0.11 / 0.62 s | not measured | 0.05 / 0.02 / 0.05 s |

The click tests were only run for Steam and 0.7.1: the two SteamInRust builds
draw the same pages the same way, and the changes in 0.7.1 don't touch them.

## What changed in 0.7.1

Measuring 0.7.0 against the last benchmark (0.4.0, 8 November) showed that
memory had stayed about the same but **CPU use while idle had gone up from
0.13 % to 0.10–0.57 % of a core**. Three fixes came out of it:

- **Controllers (CPU).** The gamepad library used for Big Picture reads every
  controller 125 times a second on Windows, even with none plugged in, which
  was nearly all of SteamInRust's idle CPU. It now runs only while a controller
  is connected; Windows tells SteamInRust when one is plugged in, so the Guide
  button still opens Big Picture. Idle CPU: 0.10 % → 0.00 %, and 2 fewer
  threads.
- **Fonts (−70 MB).** The Chinese, Japanese and Korean system fonts (72 MB of
  files) were copied whole into memory at start-up. They're now read straight
  from the font files, so only the parts actually drawn are loaded, and those
  count as shared file cache, not SteamInRust's own memory.
- **Pictures (−28 MB on the library).** egui kept a decoded copy of every
  picture (4 bytes a pixel) after it was already on the graphics card, where
  it's never used again. Those copies are now dropped every few seconds.

Together: **253 MB → 163 MB** of private memory on the same library, and the
working set from 228 MB to 134 MB.

## Where Steam holds its own

- **SteamInRust's "fully drawn" time is mostly picture downloads.** Its window
  is up at about 2.5 s; the rest is library art and news arriving over the
  network, so the range of runs is wider than for the window itself.
- **The full-size main window shows up at one of two times** for SteamInRust
  (about 2.4 s or about 4.5 s). If signing in takes a moment longer, the small
  sign-in window appears first and the main window follows when sign-in is
  done. That's why its middle result can land on either side; "first window"
  doesn't have this split.
- **Opening the Store depends on the network** for both clients. SteamInRust's
  middle result was faster (0.75 s vs 1.33 s), but the ranges overlap
  (0.59–2.76 s vs 1.25–2.90 s), because both wait on the same servers.
- The first time a game page opens, SteamInRust has to fetch its art too (up to
  0.85 s). The 0.05 s middle result is for pages whose art was already loaded.
- **Steam itself got faster since 8 November**: its library was fully drawn in
  14.7 s, against 19.2 s then (Steam 11.23.65.88 now, 11.15.70.27 then). Its
  memory went down too (1019 MB vs 1126 MB).

## Setup

| | |
|---|---|
| PC | AMD Ryzen 5 5600 (6 cores / 12 threads), 16 GB RAM, NVIDIA RTX 3060 |
| OS | Windows 11 Home 10.0.26300 |
| Steam | steam.exe 11.23.65.88, unmodified |
| SteamInRust 0.7.0 | release build of tag `v0.7.0` |
| SteamInRust 0.7.1 | release build of the change that adds the fixes above |
| Network | the same connection for all three |

Other apps were open, as on a normal day. Each trial waited until total CPU
load was under 25 % for 4 seconds before starting, and the order of the three
clients rotated every round, so all saw the same conditions.

## Method

**Same starting page.** All clients open on the library: Steam is launched as
`steam.exe steam://open/games`, SteamInRust as `steam-rs.exe --page=library`.
All sign in automatically with the saved account. Steam's "always show the
account picker" option was switched off for the runs and switched back on
afterwards.

**Start-up time.** The harness ([`bench.py`](https://github.com/madonchik123/SteamInRust/blob/29d9fa1/tools/bench/bench.py)) places a
black backdrop at a fixed 1280×800 spot and starts the client. Every client
window that appears is moved into that spot and kept on top, with the main
window resized to 1280×800. It then grabs that region about 30 times a second
and records three moments:

- *first window*: the first frame where anything of the client shows up in
  that spot (for Steam, its updater/sign-in window; for SteamInRust, the
  sign-in window or straight away the main window);
- *main window*: the client's full-size window appears;
- *library settled*: the last moment the picture changed by more than 0.5 % of
  its pixels, followed by 5 s (SteamInRust) or 6 s (Steam) without change.
  This is the "visually complete" time: every image is in and nothing moves any
  more.

Notification pop-ups that a client shows after its main window is up are left
where the client puts them, outside the measured region.

**Cold vs warm.** Cold starts don't come from a reboot. Instead, before each
cold run the harness fills
all free and standby memory (about 7.5 GB) and frees it again, which makes
Windows drop its file cache. A warm run starts right after the previous run
closed. Before each run every client is fully closed: Steam with
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
as a Windows service and isn't counted. SteamInRust is the single steam-rs.exe.

**Opening pages** ([`nav.py`](https://github.com/madonchik123/SteamInRust/blob/29d9fa1/tools/bench/nav.py)). With the client idle on
the library, the harness clicks the Store tab, the first game in the library list (the same game in both) and the Friends & Chat button. The clock
starts on the mouse-button release, since both clients act on release. Both
open Friends & Chat as a separate window, and that window is what gets
timed. "First reaction" is the first frame that differs; "settled" uses the same
rule as above with a 3 s quiet period. Between clicks the harness goes back to
the library and waits for it to settle.

**Disturbances and failed connections.** If the mouse moved, or another
window covered the measured region during a trial, the trial was thrown out and
run again. So was a trial where the client never got to its library: no main
window within 15 s (45 s for Steam), or a final picture that's still the
sign-in screen (almost one flat colour, where a library is a busy picture).
In the final set that happened once, for Steam (the mouse moved); no
SteamInRust run needed redoing.

**Disk size.** For Steam, everything in `C:\Program Files (x86)\Steam` except
games (`steamapps`), user data, config, logs, dumps and caches; this includes
247 MB of update archives in `package/` (the same 1084 MB as on 8 November). For SteamInRust, the release
`steam-rs.exe`; the installer also ships the steamclient.dll replacement, which
isn't counted here.

### Mistakes caught along the way

- **An update prompt in the first 0.7.1 runs.** The first set of 0.7.1 runs
  used a build that still called itself 0.6.0, so it showed the "SteamInRust
  0.7.0 is available" dialog over the library. That made its start-up times
  look worse and blocked every click test. Those results were thrown away; the
  build was remade on top of 0.7.0 and measured again. The numbers above come
  from that second set. (Steam's and 0.7.0's runs weren't affected.)
- **The click-test script had a typo** in the first run and didn't start; it was
  fixed and the click tests were run on their own afterwards.

The previous benchmark (0.4.0, 8 November) also threw away a first run: the
harness had pulled a Steam notification pop-up into the measured region, whose
disappearance made Steam look about 5 s slower than it is.

## Videos

Recorded after the 8 November benchmark (SteamInRust 0.4.0), at 1080p and 30 fps, with English and
Russian captions. The full videos are kept out of git in `media/`:

| File | What it shows |
|---|---|
| `steaminrust-vs-steam-race-{en,ru}.mp4` / `.webm` | One real-time cold start of each client side by side, with timers that stop at "library settled", then the middle results of 5 runs |
| `steaminrust-vs-steam-idle-{en,ru}.mp4` | Both clients idling on the library with live RAM, CPU and process counters from that take, then the middle results and install size |
| `steaminrust-tour-{en,ru}.mp4` | SteamInRust's store, a trailer, the library, the Shift+Tab overlay, inventory and market, listings, chat and profile, in `--demo` mode with sample data |

[`docs/media/en/startup-race.gif`](media/en/startup-race.gif) and
[`docs/media/ru/startup-race.gif`](media/ru/startup-race.gif) are 760 px cuts of
the race for the README.

Each client was recorded on its own, never both at once. In the race video the
two launches are lined up using a marker the harness shows at the moment of
launch. A recorded take is a single run, so its time can differ from the middle result.
The SteamInRust take in the race video took 5.1 s, against a middle result of 10.0 s;
the end card shows the middle results.

Privacy: the account name, avatar, wallet balance and profile tab are blurred in
both clients, and so is the whole frame until each client's main window is up,
because both sign-in screens show the account name. The feature tour uses
`--demo` mode, so no real account data appears in it.

## Reproduce

The scripts live in the repo's history: check out
[`tools/bench/` at commit 29d9fa1](https://github.com/madonchik123/SteamInRust/tree/29d9fa1/tools/bench), build the app with
`cargo build --release -p steam-rs`, and run `bash tools/bench/full.sh`. It runs
the start-up rounds (`python -I run_all.py 3` for 3 rounds), the click tests and
the recordings, and needs Python 3. `python -I tools/bench/analyze.py` then
prints the table. For this run the harness got two small additions that aren't
in that commit: entries for two SteamInRust builds side by side (each matched
by its own exe path, with the order of clients rotating every round), and the
first-window time in the table (it was already recorded as `first_pixels_s`).
Steam has to sign in without the account picker, and the mouse and screen must
be left alone while it runs (about 40 minutes for 3 rounds of three clients).
