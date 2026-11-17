# Changelog

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
