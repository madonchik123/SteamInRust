# Changelog

## [0.12.1](https://github.com/madonchik123/SteamInRust/compare/v0.12.0...v0.12.1) (2026-11-20)


### Fixes

* free pictures and videos you're no longer looking at ([#113](https://github.com/madonchik123/SteamInRust/issues/113)) ([c23b851](https://github.com/madonchik123/SteamInRust/commit/c23b851b2dc7bade6e8650976a655b994f9256eb))
* in-game overlay blocks all game input, opens with the Guide button, and stops doing needless work ([#112](https://github.com/madonchik123/SteamInRust/issues/112)) ([700d049](https://github.com/madonchik123/SteamInRust/commit/700d049c5aba5e654a9a373c0d85879febe94043))
* show the Steam Cloud question when a game starts from a shortcut ([#111](https://github.com/madonchik123/SteamInRust/issues/111)) ([23a6d15](https://github.com/madonchik123/SteamInRust/commit/23a6d15d1d5353b8430706bc6c173dd511091050))

## [0.12.0](https://github.com/madonchik123/SteamInRust/compare/v0.11.0...v0.12.0) (2026-11-19)


### New

* post a status for your friends from Community's Friends activity ([#107](https://github.com/madonchik123/SteamInRust/issues/107)) ([17a3bc0](https://github.com/madonchik123/SteamInRust/commit/17a3bc039ccb33c7575950338454ee1d20188b4a))
* reorder the wishlist by dragging or typing a place, like Steam ([#108](https://github.com/madonchik123/SteamInRust/issues/108)) ([95c9dcc](https://github.com/madonchik123/SteamInRust/commit/95c9dccb10097a38e8319280ef4ad24409ee0099))


### Fixes

* Big Picture chat fills the screen and scrolls, and the screenshot viewer works with the mouse ([#106](https://github.com/madonchik123/SteamInRust/issues/106)) ([2bf61d4](https://github.com/madonchik123/SteamInRust/commit/2bf61d447cc9381e76a41d671bf2c51039ad5184))
* games installed into a folder Steam doesn't list show as installed ([#104](https://github.com/madonchik123/SteamInRust/issues/104)) ([a1855fa](https://github.com/madonchik123/SteamInRust/commit/a1855faf92c0de27731969d7037c1999209cc9e3))
* name menu opens your inventory and friends; long summaries stay in their box ([#105](https://github.com/madonchik123/SteamInRust/issues/105)) ([a22e77c](https://github.com/madonchik123/SteamInRust/commit/a22e77c3460bf67639b2a0c8e9be40ea91f5794f))

## [0.11.0](https://github.com/madonchik123/SteamInRust/compare/v0.10.0...v0.11.0) (2026-11-19)


### New

* a Community home with friends' activity, Explore and news from developers ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))
* a new look based on Juxtopposed's Steam Redesign, and themes you can make yourself ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))
* game item definitions, F12 screenshots and an in-game FPS counter ([#99](https://github.com/madonchik123/SteamInRust/issues/99)) ([94dcaab](https://github.com/madonchik123/SteamInRust/commit/94dcaab204b69db6c6f39be59a7beca66b544831))
* group chats, thread replies, group pages and market history ([#98](https://github.com/madonchik123/SteamInRust/issues/98)) ([d5e8407](https://github.com/madonchik123/SteamInRust/commit/d5e8407484f8e3645144f9597daad52a014dbb51))
* new SteamInRust logo in the app, icons and tray ([#97](https://github.com/madonchik123/SteamInRust/issues/97)) ([7fa3db0](https://github.com/madonchik123/SteamInRust/commit/7fa3db0d2dc39002b16da57a914b10eef818dbd6))
* one Friends & Chat window with the chat beside your friends list ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))
* Steam-style profile editing, animated profile backgrounds and friend actions on profiles ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))
* the Points Shop and the discovery feed's new picks ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))


### Fixes

* bound GNS ack_frame/received, hub parse_topics, cloud glob, and inline image decoding ([#93](https://github.com/madonchik123/SteamInRust/issues/93)) ([7f05e96](https://github.com/madonchik123/SteamInRust/commit/7f05e96415c272da8ce84658c7f5977be97051fa))
* bound VDF recursion, CM/depot decompression, MP4/HLS/VideoToolbox/range OOM, and P2P reassembly ([#86](https://github.com/madonchik123/SteamInRust/issues/86)) ([4b34829](https://github.com/madonchik123/SteamInRust/commit/4b34829539107f8da8bec819a745498fdda89cce))
* enforce depot path confinement, HTTPS-only CDN, and mandatory SHA-1 checks ([#79](https://github.com/madonchik123/SteamInRust/issues/79)) ([dcadb81](https://github.com/madonchik123/SteamInRust/commit/dcadb8165bfc6b4d208f376fb41ca6c879d7724c))
* enforce folder name validation, symlink unlink, and canonical containment on uninstall ([#83](https://github.com/madonchik123/SteamInRust/issues/83)) ([9461dca](https://github.com/madonchik123/SteamInRust/commit/9461dcae28fdd100f290ffcac6e61fd526bf5747))
* enforce http/https URL validation, remove egui_extras FileLoader, and type rich::Click ([#85](https://github.com/madonchik123/SteamInRust/issues/85)) ([814f1e4](https://github.com/madonchik123/SteamInRust/commit/814f1e41587821805095a111075193cf032f5d4c))
* enforce HTTPS-only transport and SHA-1 integrity in Workshop, Cloud, and PICS ([#90](https://github.com/madonchik123/SteamInRust/issues/90)) ([1ad74fd](https://github.com/madonchik123/SteamInRust/commit/1ad74fd791a2dd78d4e260f057aac581269c0b2b))
* enforce modular 32-bit sequence math and bounds checks in PseudoTcp ([#80](https://github.com/madonchik123/SteamInRust/issues/80)) ([324cb22](https://github.com/madonchik123/SteamInRust/commit/324cb225bcd1fb38fa733cfe5963ade71f33319c))
* enforce SaveRule matching, OS root gating, and symlink checks on Cloud downloads ([#82](https://github.com/madonchik123/SteamInRust/issues/82)) ([16a15b2](https://github.com/madonchik123/SteamInRust/commit/16a15b284e9847498a3678891ef602e02fa7b822))
* enforce W^X protections on 32-bit steamclient trampoline pages ([#92](https://github.com/madonchik123/SteamInRust/issues/92)) ([413e7cf](https://github.com/madonchik123/SteamInRust/commit/413e7cff75f80466d0361c15dc017c86915c9b50))
* harden local link/IPC bounds, CSPRNG tokens, and verified updater staging ([#88](https://github.com/madonchik123/SteamInRust/issues/88)) ([c548d9b](https://github.com/madonchik123/SteamInRust/commit/c548d9b7723b9f59f7b3cd5a0803b5b6a97345cd))
* harden plugin Rhai sandbox, permission consent persistence, and resource caps ([#84](https://github.com/madonchik123/SteamInRust/issues/84)) ([98b19a7](https://github.com/madonchik123/SteamInRust/commit/98b19a71adfa6e4defedc082df356ea190315a89))
* harden Steam web cookies, URL validation, and image cache SSRF/size limits ([#89](https://github.com/madonchik123/SteamInRust/issues/89)) ([0e3bd1b](https://github.com/madonchik123/SteamInRust/commit/0e3bd1b744c066aba65e2df6001b54554d50d772))
* harden steamclient server list FFI filter parsing and list cancellation ([#87](https://github.com/madonchik123/SteamInRust/issues/87)) ([dc0f817](https://github.com/madonchik123/SteamInRust/commit/dc0f817f21ad567d4d576946256f6781aa87562b))
* isolate remote invite connect args from launch options ([#78](https://github.com/madonchik123/SteamInRust/issues/78)) ([942bedd](https://github.com/madonchik123/SteamInRust/commit/942beddc0cc5e0e62e8c2d07ebcd0bef52fb9dea))
* redact secrets from Debug impls and enforce private local file/temp hygiene ([#91](https://github.com/madonchik123/SteamInRust/issues/91)) ([f3fa6de](https://github.com/madonchik123/SteamInRust/commit/f3fa6dedd04a2c7da8ffa528221e6724997e7251))
* the picked logo shows on the taskbar button, and applies at startup ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))


### Faster

* an idle window stops redrawing ([99e7a1b](https://github.com/madonchik123/SteamInRust/commit/99e7a1b439106238348c7a0146b7c9f5315521ad))

## [0.10.0](https://github.com/madonchik123/SteamInRust/compare/v0.9.0...v0.10.0) (2026-11-17)


### New

* **bigpicture:** trailers and reviews on store pages, an install screen, and a "runs here" library filter ([#75](https://github.com/madonchik123/SteamInRust/issues/75)) ([64f2913](https://github.com/madonchik123/SteamInRust/commit/64f29135e17bbbb4cbc9f01f803a63e494e9e3cb))
* profile settings and badges in the app, review filters, store bar redesign ([#74](https://github.com/madonchik123/SteamInRust/issues/74)) ([be8d333](https://github.com/madonchik123/SteamInRust/commit/be8d3334cdcfb5c91c25ae876c83c584387ec496))

## [0.9.0](https://github.com/madonchik123/SteamInRust/compare/v0.8.0...v0.9.0) (2026-11-17)


### New

* first-time setup with Steam's install scripts ([#63](https://github.com/madonchik123/SteamInRust/issues/63)) ([26e6c81](https://github.com/madonchik123/SteamInRust/commit/26e6c8141266201610e7281cd733cd76e76d5996))
* **plugins:** network, timers, game and friend hooks, richer pages ([#65](https://github.com/madonchik123/SteamInRust/issues/65)) ([0be0bee](https://github.com/madonchik123/SteamInRust/commit/0be0bee2dbc3965db452397414d87740df75584d))
* profiles' friends, reviews, groups and badges, and Market search in the app ([#66](https://github.com/madonchik123/SteamInRust/issues/66)) ([12f06d9](https://github.com/madonchik123/SteamInRust/commit/12f06d9520309cdc2c2d4377831e8a987a7db314))
* **store:** reviews, rating bars, languages table and requirement tabs on game pages ([#67](https://github.com/madonchik123/SteamInRust/issues/67)) ([fed42a1](https://github.com/madonchik123/SteamInRust/commit/fed42a14ae5f1f933d32a4737dd51640c2afa8ea))
* **store:** write reviews in the app; microtrailers on hover ([#64](https://github.com/madonchik123/SteamInRust/issues/64)) ([2e6e660](https://github.com/madonchik123/SteamInRust/commit/2e6e660626d894f7a517c16a9ae9e9736828780c))
* trade offers in the app, fast market prices, and a trade plugin API ([#72](https://github.com/madonchik123/SteamInRust/issues/72)) ([d01267d](https://github.com/madonchik123/SteamInRust/commit/d01267d2b98e4f959e109e40ae801172c42a4aa6))


### Fixes

* notifications start with a capital letter ([#71](https://github.com/madonchik123/SteamInRust/issues/71)) ([3fd4e65](https://github.com/madonchik123/SteamInRust/commit/3fd4e65bf9bd5b2551bfa4274831d9c04525c926))
* **store:** wishlist heart, demo tags, release dates and tidier result rows ([#68](https://github.com/madonchik123/SteamInRust/issues/68)) ([55a4da6](https://github.com/madonchik123/SteamInRust/commit/55a4da66775fe9c054af0b5a8ba4b97bc02bc3af))


### Faster

* pictures are decoded at the size they're drawn, on a few threads ([#69](https://github.com/madonchik123/SteamInRust/issues/69)) ([3a54744](https://github.com/madonchik123/SteamInRust/commit/3a547442a3dac05a7bb8b7a9050bbe4186b86d06))
* the game page no longer reads the disk while it draws ([#70](https://github.com/madonchik123/SteamInRust/issues/70)) ([5d89494](https://github.com/madonchik123/SteamInRust/commit/5d89494e9a55f0366d0e51e17b86eae43d28c414))

## [0.8.0](https://github.com/madonchik123/SteamInRust/compare/v0.7.0...v0.8.0) (2026-11-17)


### New

* Big Picture friend pages, steadier update downloads and a new logo ([#61](https://github.com/madonchik123/SteamInRust/issues/61)) ([a6e9336](https://github.com/madonchik123/SteamInRust/commit/a6e933661abb827ef0b1d5fa815be54ec0a718a0))
* download queue controls, taskbar progress, discovery queue and dynamic collections ([#62](https://github.com/madonchik123/SteamInRust/issues/62)) ([731cef2](https://github.com/madonchik123/SteamInRust/commit/731cef24a27610c3d7eede30729e9f36508fd31f))


### Faster

* about 90 MB less memory and no CPU use while idle ([#57](https://github.com/madonchik123/SteamInRust/issues/57)) ([1a3cbf5](https://github.com/madonchik123/SteamInRust/commit/1a3cbf51109e1376399e2c11665401099834b50c))

## [0.7.0](https://github.com/madonchik123/SteamInRust/compare/v0.6.0...v0.7.0) (2026-11-16)


### New

* Add a friend page like Steam's: Quick Invite links and profile search ([#49](https://github.com/madonchik123/SteamInRust/issues/49)) ([232acf7](https://github.com/madonchik123/SteamInRust/commit/232acf7ee54bdb1ee399ab8e59a5b8b09fc85d95))
* library collections stay in step with Steam ([#48](https://github.com/madonchik123/SteamInRust/issues/48)) ([10bc6e8](https://github.com/madonchik123/SteamInRust/commit/10bc6e85eb0f64ceec7c5bc0f9f783f3ed097fdf))
* Simplified Chinese, Spanish, Brazilian Portuguese, German and French ([#55](https://github.com/madonchik123/SteamInRust/issues/55)) ([55fed9f](https://github.com/madonchik123/SteamInRust/commit/55fed9f1d5c2b749c43b70049c2fbde11065f244))


### Fixes

* Away and Do Not Disturb look different ([#56](https://github.com/madonchik123/SteamInRust/issues/56)) ([e0bd11f](https://github.com/madonchik123/SteamInRust/commit/e0bd11f1da0796bcbb4f62ef62d325ba176edd02))
* Downloads page shows the real speed and lists each game once ([#54](https://github.com/madonchik123/SteamInRust/issues/54)) ([31ef4d5](https://github.com/madonchik123/SteamInRust/commit/31ef4d5926b4740df393d7cb9722fd0540caf381))
* downloads resume where they stopped after the app is closed or killed ([#53](https://github.com/madonchik123/SteamInRust/issues/53)) ([5f82688](https://github.com/madonchik123/SteamInRust/commit/5f82688702f8bc09ec438e0830dc8e7cfe47ac93))
* game icons load again, and pictures load sooner ([#51](https://github.com/madonchik123/SteamInRust/issues/51)) ([b10b5cf](https://github.com/madonchik123/SteamInRust/commit/b10b5cfe9dc8f8d4c946ac925b1c4b2bae9f3de3))

## [0.6.0](https://github.com/madonchik123/SteamInRust/compare/v0.5.1...v0.6.0) (2026-11-16)


### New

* Big Picture mode ([#45](https://github.com/madonchik123/SteamInRust/issues/45)) ([b1077e4](https://github.com/madonchik123/SteamInRust/commit/b1077e404741107f8c1f4927f869f86fd3cbb58c))
* macOS support ([#46](https://github.com/madonchik123/SteamInRust/issues/46)) ([27ba783](https://github.com/madonchik123/SteamInRust/commit/27ba783e41f7672cbcfb84e48f12458076016668))


### Fixes

* menus fit their content and every row lines up ([#43](https://github.com/madonchik123/SteamInRust/issues/43)) ([8d60745](https://github.com/madonchik123/SteamInRust/commit/8d607450664e318ad1f22d385597a27e5e3cd169))

## [0.5.1](https://github.com/madonchik123/SteamInRust/compare/v0.5.0...v0.5.1) (2026-11-15)


### Fixes

* keep saved sign-ins in the OS credential store, not in plain text ([#42](https://github.com/madonchik123/SteamInRust/issues/42)) ([79c9ce2](https://github.com/madonchik123/SteamInRust/commit/79c9ce2ffd567aeb5b45876fbbb21c1f343db8dc))
* paste, copy and cut work in every text field ([#40](https://github.com/madonchik123/SteamInRust/issues/40)) ([8964382](https://github.com/madonchik123/SteamInRust/commit/8964382349aba5be1ef4367db5f8298bc217db62))

## [0.5.0](https://github.com/madonchik123/SteamInRust/compare/v0.4.1...v0.5.0) (2026-11-10)


### New

* Steam-style cart: one Add to Cart, an Added to your cart popup, gifts per copy ([#35](https://github.com/madonchik123/SteamInRust/issues/35)) ([0b010ce](https://github.com/madonchik123/SteamInRust/commit/0b010ce1d757acbed42491d17b139fb56417d4df))
* Steam-style discount price tags ([#33](https://github.com/madonchik123/SteamInRust/issues/33)) ([f748477](https://github.com/madonchik123/SteamInRust/commit/f748477237465e2a68bb2e2fd441b94b9f31c302))


### Fixes

* cancelling a download frees its disk space ([#36](https://github.com/madonchik123/SteamInRust/issues/36)) ([b147abe](https://github.com/madonchik123/SteamInRust/commit/b147abe9cc01ee0c5ff4725632b5ad334dfb89b9))
* downloads, updates and verify work like Steam ([#37](https://github.com/madonchik123/SteamInRust/issues/37)) ([7755bb9](https://github.com/madonchik123/SteamInRust/commit/7755bb9df9e67842201a59536b0cfdb717ea69dd))
* library filters fit, cart names, Back in the store, free games, tag search ([#32](https://github.com/madonchik123/SteamInRust/issues/32)) ([42e188b](https://github.com/madonchik123/SteamInRust/commit/42e188b410c131fcf1a08a0c95f455016ce5fe2c))

## [0.4.1](https://github.com/madonchik123/SteamInRust/compare/v0.4.0...v0.4.1) (2026-11-09)


### Fixes

* readable update dialog, hover states in menus, no update prompts for builds from source ([#30](https://github.com/madonchik123/SteamInRust/issues/30)) ([8a60364](https://github.com/madonchik123/SteamInRust/commit/8a60364f8b570e21c5512071e0fa0a7df77066dd))

## [0.4.0](https://github.com/madonchik123/SteamInRust/compare/v0.3.0...v0.4.0) (2026-11-08)


### New

* compact Workshop filters; items download when subscribed ([#17](https://github.com/madonchik123/SteamInRust/issues/17)) ([7562a6d](https://github.com/madonchik123/SteamInRust/commit/7562a6d30e28e0814f6cf5b2d9c45cfe01bd1056))
* ISteamNetworkingSockets / Messages on Valve's protocol (certs, rendezvous, SNP, ICE) ([#22](https://github.com/madonchik123/SteamInRust/issues/22)) ([417a283](https://github.com/madonchik123/SteamInRust/commit/417a283eae3740a51475a6c5d73cb1a9b087424b))
* live player count on game pages; easy 32-bit DLL for dev builds ([#14](https://github.com/madonchik123/SteamInRust/issues/14)) ([19cebea](https://github.com/madonchik123/SteamInRust/commit/19cebea4a5cdf19bc230e2b9650cc37bf679ed96))
* native account details, purchase history, licenses and Steam Family ([#18](https://github.com/madonchik123/SteamInRust/issues/18)) ([1a119b5](https://github.com/madonchik123/SteamInRust/commit/1a119b59390b5773733f87f997377fe090820e86))
* P2P with official-client players (ISteamNetworking over Valve's libjingle protocol) ([#21](https://github.com/madonchik123/SteamInRust/issues/21)) ([57a2f56](https://github.com/madonchik123/SteamInRust/commit/57a2f56fd64b18594f00e8ea1c283c7f655c8ba5))
* real Steam services in our steamclient (sockets, Workshop, inventory, server browser, screenshots, Steam Input) ([#20](https://github.com/madonchik123/SteamInRust/issues/20)) ([4ab1c87](https://github.com/madonchik123/SteamInRust/commit/4ab1c87a2312b247dee2475df014f085ce600514))
* Shift+Tab overlay drawn inside the game; Steam-style invites in chat ([#19](https://github.com/madonchik123/SteamInRust/issues/19)) ([62b93ea](https://github.com/madonchik123/SteamInRust/commit/62b93eaaa61de5624e2eb3e6dcfe860d5b873bd2))


### Fixes

* 32-bit games on older Steamworks SDKs crashed at start (Human Fall Flat) ([#15](https://github.com/madonchik123/SteamInRust/issues/15)) ([4b9dd49](https://github.com/madonchik123/SteamInRust/commit/4b9dd49aa9e2ce9098c9420326f6a7b80e7f2655))
* uninstalling no longer freezes or crashes the app ([#23](https://github.com/madonchik123/SteamInRust/issues/23)) ([cf676e8](https://github.com/madonchik123/SteamInRust/commit/cf676e8103fbf43371bb491114263aa7e3195121))


### Faster

* library images no longer fetch their fallback while loading ([#26](https://github.com/madonchik123/SteamInRust/issues/26)) ([eacdedd](https://github.com/madonchik123/SteamInRust/commit/eacdedd9c1c18e6f71bbab59437f4043f91772ad))
* the library shows at once instead of "Loading your games" ([#27](https://github.com/madonchik123/SteamInRust/issues/27)) ([b2d7170](https://github.com/madonchik123/SteamInRust/commit/b2d71701990a1cfa97a0499ab89df9da4223b307))

## [0.3.0](https://github.com/madonchik123/SteamInRust/compare/v0.2.0...v0.3.0) (2026-11-08)


### New

* install states, download activity, friends window and tray polish ([#10](https://github.com/madonchik123/SteamInRust/issues/10)) ([29d9fa1](https://github.com/madonchik123/SteamInRust/commit/29d9fa114ec9c2a4d1bea441aa8f0fa19f1a705f))
* online play in our steamclient (lobbies, P2P, 32-bit games, leaderboards, HTTP) ([#11](https://github.com/madonchik123/SteamInRust/issues/11)) ([e289050](https://github.com/madonchik123/SteamInRust/commit/e289050fcdf4778fa8c5f07651ba212e0d4043db))
* Steam-style Friends & Chat menu, own title bar; fix stuck downloads ([#9](https://github.com/madonchik123/SteamInRust/issues/9)) ([70f4eba](https://github.com/madonchik123/SteamInRust/commit/70f4eba0348a9dd8445e81ed88f35ee3a3ac04b2))


### Fixes

* crash from a negative progress bar width during downloads ([#7](https://github.com/madonchik123/SteamInRust/issues/7)) ([d2e3a3e](https://github.com/madonchik123/SteamInRust/commit/d2e3a3e55f9b6b4bafbb6b0ad218bdcfe7688cca))

## [0.2.0](https://github.com/madonchik123/SteamInRust/compare/v0.1.1...v0.2.0) (2026-11-08)


### New

* gifts for games you own, picture cache, store polish ([#5](https://github.com/madonchik123/SteamInRust/issues/5)) ([04f4c4c](https://github.com/madonchik123/SteamInRust/commit/04f4c4cc9c1ddddca75ca47b6d2203dfa254e3f9))

## [0.1.1](https://github.com/madonchik123/SteamInRust/compare/v0.1.0...v0.1.1) (2026-11-07)


### Fixes

* single slashes in Steam Cloud file names for Auto-Cloud folders ([af8a0ac](https://github.com/madonchik123/SteamInRust/commit/af8a0ac4656953171c046c48938dbebb24c40d03))
