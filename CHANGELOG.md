# Changelog

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
