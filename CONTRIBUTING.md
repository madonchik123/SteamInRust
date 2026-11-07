# Contributing to SteamInRust

Thanks for helping! A few things keep the project easy to work on.

## Pull requests

- **Title = changelog line.** Start it with a type, then say what changes for
  the user:
  - `feat: show player counts on game pages` (new feature, next minor version)
  - `fix: friends list no longer jumps when someone comes online`
  - `perf:`, `ui:`, `i18n:` (translations), `docs:`, `chore:`, `ci:`, `refactor:`, `test:`
  - Add `!` for breaking changes: `feat!: …`
- PRs are **squash-merged**, so the title is what ends up in the history.
- CI must pass: it builds on Windows and runs `cargo test --workspace`.
- Every merge to `main` produces a test build (setup + portable zip) on the
  workflow run's page, so you can try your change before it's released.

## Releases (maintainers)

Merging PRs doesn't publish anything by itself. A bot keeps an open
"chore(main): release X.Y.Z" PR with the version bump and changelog.
Merge it when you want to ship: the release is built and published, and
installed copies of the app update themselves from it.

## Code

- Rust 2024, GNU toolchain (see `rust-toolchain.toml`). `cargo run -- --demo`
  runs the app with sample data, no sign-in needed.
- Every user-visible string goes through `t!("…")`, with a Russian line in
  `crates/i18n/locales/ru.toml`. `cargo test -p i18n` lists missing ones.
- Never add anything that touches games' anti-cheat. Games with VAC, EAC or
  BattlEye always run through the official Steam client.
- Payments stay on Steam's website.

## Translations

Copy `crates/i18n/locales/ru.toml`, translate the right-hand side, and add a
`Lang` variant in `crates/i18n/src/lib.rs`.
