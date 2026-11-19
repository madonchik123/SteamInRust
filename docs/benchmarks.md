# SteamInRust vs Steam: benchmarks

Measured on 19 November 2026 on one PC, signed in to the same account, on the
same network. Every number is the **middle result of 3 runs**, with the lowest
and highest run in brackets. Two clients were measured in turn: Steam, and
SteamInRust 0.10.0 with the redesign (the new look, in its default theme). The
last column is the previous run (17 November, SteamInRust 0.7.1, before the
redesign) for comparison; it was a different day, so small differences are
noise. The measuring scripts were removed from the repo to keep it all Rust;
they're kept in its history at
[`tools/bench/` (commit 29d9fa1)](https://github.com/madonchik123/SteamInRust/tree/29d9fa1/tools/bench).

**Short version:** SteamInRust shows its first window about 2.5 times sooner
than Steam, has the library fully drawn about twice as soon, opens pages many
times faster, and uses about a ninth of the memory and almost no CPU while
idle.

| | Steam | SteamInRust 0.10.0 (redesign) | SteamInRust 0.7.1, 17 Nov |
|---|---|---|---|
| **Cold start → first window on screen** | 6.3 s (5.7–8.8) | **2.5 s** (2.5–3.1) | 2.6 s (2.5–2.6) |
| Cold start → full-size main window | 11.4 s (8.1–24.9) | 2.4 s (2.4–4.9) | 2.4 s (2.4–4.9) |
| **Cold start → library fully drawn** | 19.1 s (14.4–32.5) | **9.6 s** (4.7–10.5) | 4.8 s (4.8–6.3) |
| Warm start → first window on screen | 5.7 s (5.3–5.7) | 2.5 s (2.4–2.5) | 2.5 s (2.5–2.5) |
| Warm start → full-size main window | 8.2 s (8.1–9.8) | 2.4 s (2.4–2.4) | 4.4 s (2.4–4.9) |
| Warm start → library fully drawn | 14.5 s (14.3–15.8) | 6.4 s (3.8–8.2) | 5.8 s (4.8–6.4) |
| **Memory while idle** (private bytes) | **1016 MB** (1014–1452) | **115 MB** (113–120) | 163 MB (153–163) |
| Memory while idle (working set) | 956 MB (951–1387) | 185 MB (184–188) | 134 MB (126–135) |
| **CPU while idle**, % of one core | **7.7 %** (7.3–37.8) | **0.08 %** (0.03–0.21) | 0.00 % (0.00–0.05) |
| Processes | 8 (steam.exe + 7 × steamwebhelper.exe) | 1 | 1 |
| Program size | 1084 MB (measured 17 Nov), plus ~0.6 GB web cache in `%LOCALAPPDATA%\Steam` | 34.8 MB | 29.8 MB |
| Open the Store (click → settled) | 1.84 s (0.10–3.39) | 0.81 s (0.50–2.12) | 0.75 s (0.59–2.76) |
| Open a game page in the library | 0.75 s (0.73–1.59) | 0.03 s (0.03–1.68) | 0.05 s (0.04–0.85) |
| Open Friends & Chat | 1.21 s (1.20–1.22) | 0.05 s (0.05–0.08) | 0.06 s (0.05–0.08) |
| First visible reaction to a click: Store / game page / Friends | 0.07 / 0.09 / 0.62 s | 0.05 / 0.03 / 0.05 s | 0.05 / 0.02 / 0.05 s |

## What changed with the redesign

- **Less memory, more of it shared.** SteamInRust's own memory while idle went
  down from 163 MB to 115 MB. Its working set (which also counts memory shared
  with Windows, like the graphics driver's and the font files) went up from
  134 MB to 185 MB.
- **The library takes longer to be fully drawn: 4.8 s → 9.6 s.** The window
  is up just as fast (2.4–2.5 s); the rest is pictures arriving over the
  network. In this run the library opened in the cover grid, about twenty game
  covers at once, all downloaded at start-up. The wide range (4.7–10.5 s) is
  the network. It's still about twice as fast as Steam.
- **Idle CPU: 0.08 % of a core**, against 0.00 % before: both well under a
  tenth of a percent. An idle window doesn't redraw (loading placeholders stop
  pulsing once nothing is loading).
- **The program is 5 MB bigger** (29.8 → 34.8 MB): the theme system, the
  built-in themes and the new pages.

## What changed in 0.7.1 (17 November)

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
- **Steam varies a lot from run to run.** The same Steam version (11.23.65.88)
  had its library fully drawn in 14.7 s on 17 November and 19.1 s now, with one
  cold start this time taking 32.5 s (its main window took 24.9 s to appear).
  Its idle CPU also had one run at 37.8 % of a core.

## Setup

| | |
|---|---|
| PC | AMD Ryzen 5 5600 (6 cores / 12 threads), 16 GB RAM, NVIDIA RTX 3060 |
| OS | Windows 11 Home 10.0.26300 |
| Steam | steam.exe 11.23.65.88, unmodified |
| SteamInRust 0.10.0 (redesign) | release build of the redesign, version 0.10.0 (the latest release, so no update prompt) |
| Network | the same connection for both |

Other apps were open, as on a normal day. Each trial waited until total CPU
load was under 25 % for 4 seconds before starting, and the order of the
clients rotated every round, so both saw the same conditions.

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

On 19 November:

- **The click test clicked the old layout.** Its click spots were made for the
  look before the redesign (the Friends & Chat button used to be in a bar at
  the bottom, which the new look doesn't have), so SteamInRust's first click
  tests opened the wrong pages and never found the Friends window. The spots
  were moved to the new top bar and SteamInRust's click tests were run again;
  the numbers above are from that second set.
- **The race recording got a path Windows couldn't read**, so SteamInRust's
  first race take recorded nothing. It was recorded again.

On 17 November:

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

The full videos were recorded after the 8 November benchmark (SteamInRust
0.4.0, before the redesign), at 1080p and 30 fps, with English and Russian
captions. They're kept out of git in `media/`:

| File | What it shows |
|---|---|
| `steaminrust-vs-steam-race-{en,ru}.mp4` / `.webm` | One real-time cold start of each client side by side, with timers that stop at "library settled", then the middle results of 5 runs |
| `steaminrust-vs-steam-idle-{en,ru}.mp4` | Both clients idling on the library with live RAM, CPU and process counters from that take, then the middle results and install size |
| `steaminrust-tour-{en,ru}.mp4` | SteamInRust's store, a trailer, the library, the Shift+Tab overlay, inventory and market, listings, chat and profile, in `--demo` mode with sample data |

[`docs/media/en/startup-race.gif`](media/en/startup-race.gif) and
[`docs/media/ru/startup-race.gif`](media/ru/startup-race.gif), the race in the
README, were recorded in this run (19 November), with the redesign.

Each client was recorded on its own, never both at once. In the race video the
two launches are lined up using a marker the harness shows at the moment of
launch. A recorded take is a single run, so its time can differ from the middle result.
In the README's race, the SteamInRust take took 4.8 s and Steam's 16.2 s,
against middle results of 9.6 s and 19.1 s; the end card shows the middle
results.

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
prints the table. Since that commit the harness got a few small additions:
entries for SteamInRust builds side by side (each matched by its own exe path,
with the order of clients rotating every round), the first-window time in the
table (it was already recorded as `first_pixels_s`), and the redesign's click
spots in `nav.py` (Store `(162, 24)`, Library `(521, 24)`, first game
`(110, 120)`, Home `(60, 74)`, Friends `(694, 24)` in the 1280×800 window).
Steam has to sign in without the account picker, and the mouse and screen must
be left alone while it runs (about 40 minutes for 3 rounds).
