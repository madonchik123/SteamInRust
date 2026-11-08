# Changelog

## [0.4.0](https://github.com/madonchik123/SteamInRust/compare/v0.3.0...v0.4.0) (2026-11-08)


### New

* compact Workshop filters; items download when subscribed ([#17](https://github.com/madonchik123/SteamInRust/issues/17)) ([c7d9d46](https://github.com/madonchik123/SteamInRust/commit/c7d9d46585edd2894a587c882108bcb42057df28))
* ISteamNetworkingSockets / Messages on Valve's protocol (certs, rendezvous, SNP, ICE) ([#22](https://github.com/madonchik123/SteamInRust/issues/22)) ([511aa3d](https://github.com/madonchik123/SteamInRust/commit/511aa3d36909561a2e2f872d3114f9cdb1c20cf3))
* live player count on game pages; easy 32-bit DLL for dev builds ([#14](https://github.com/madonchik123/SteamInRust/issues/14)) ([60825be](https://github.com/madonchik123/SteamInRust/commit/60825be9645ac4806a58117ec23f6095fd9c18b5))
* native account details, purchase history, licenses and Steam Family ([#18](https://github.com/madonchik123/SteamInRust/issues/18)) ([31843c9](https://github.com/madonchik123/SteamInRust/commit/31843c943d08e794b4acc0d41fc47b00966e4fbd))
* P2P with official-client players (ISteamNetworking over Valve's libjingle protocol) ([#21](https://github.com/madonchik123/SteamInRust/issues/21)) ([ed03c49](https://github.com/madonchik123/SteamInRust/commit/ed03c497fe53080b9aa00ada16fc015073722fa7))
* real Steam services in our steamclient (sockets, Workshop, inventory, server browser, screenshots, Steam Input) ([#20](https://github.com/madonchik123/SteamInRust/issues/20)) ([6966ea6](https://github.com/madonchik123/SteamInRust/commit/6966ea63b892a2f9c410b298307a60a1dc36e6d6))
* Shift+Tab overlay drawn inside the game; Steam-style invites in chat ([#19](https://github.com/madonchik123/SteamInRust/issues/19)) ([8b8e4b0](https://github.com/madonchik123/SteamInRust/commit/8b8e4b0b3fbfae2a1b6346815f28fb73c5da1d30))


### Fixes

* 32-bit games on older Steamworks SDKs crashed at start (Human Fall Flat) ([#15](https://github.com/madonchik123/SteamInRust/issues/15)) ([bce12f4](https://github.com/madonchik123/SteamInRust/commit/bce12f486728093b47f49906b8dae300622f1b1c))
* uninstalling no longer freezes or crashes the app ([#23](https://github.com/madonchik123/SteamInRust/issues/23)) ([7357847](https://github.com/madonchik123/SteamInRust/commit/7357847ff634b35d36ce73ea9e8079472749acb5))


### Faster

* library images no longer fetch their fallback while loading ([#26](https://github.com/madonchik123/SteamInRust/issues/26)) ([75c84b0](https://github.com/madonchik123/SteamInRust/commit/75c84b0c807186568251f9bf5e9f3caad31e780d))
* the library shows at once instead of "Loading your games" ([#27](https://github.com/madonchik123/SteamInRust/issues/27)) ([bdaa71a](https://github.com/madonchik123/SteamInRust/commit/bdaa71a403f119c45b2f75700ef802133f243139))

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
