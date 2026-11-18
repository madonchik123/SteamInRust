# Ember: an example theme

A complete, small theme package for SteamInRust. Copy this folder to start
your own: change `id` and `name`, then the colors and styles you want.
Everything not in `theme.toml` comes from the default theme.

- `theme.toml`: the theme.
- `images/dots.svg`: the texture tiled behind community pages.

Try it:

```bash
steam-rs --theme-dir docs/examples/minimal-theme --demo --no-local
```

The format is described in [`docs/themes.md`](../../themes.md).
