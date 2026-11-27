//! Steam Play on a Mac, the parts the player sees outside Settings and
//! Properties: the offer to install Wine on a Windows-only game's page
//! when there's no CrossOver or Wine, and installing it (Rosetta first on
//! Apple Silicon). Big Picture uses the same through `MainAction::GetWine`.

use super::Library;
use egui::{Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, OwnedApp};
use std::sync::Arc;
use ui::{Theme, icons, widgets as w};

/// Would installing Wine let this game play here? (Windows-only, no
/// CrossOver or Wine yet, Steam Play on for such games, no anti-cheat.)
pub(crate) fn wine_would_help(o: &OwnedApp) -> bool {
    client::compat::SUPPORTED
        && client::compat::runners().is_empty()
        && client::compat::install_blocker(o, &client::compat::load(), &[]) == Some(client::compat::Blocker::NoRunner)
        && client::compat::load().other_titles
}

impl Library {
    /// The Wine install under way (or just over), if any.
    pub(crate) fn wine_progress(&self) -> Option<f32> {
        let w = self.wine_install.as_ref()?;
        w.finished().is_none().then(|| w.fraction())
    }

    /// Install Wine (and Rosetta before it, if missing) in the background.
    pub(crate) fn start_wine_install(&mut self, ctx: &mut Ctx) {
        if self.wine_progress().is_some() {
            return;
        }
        let Some(rt) = self.rt.clone() else { return };
        let p = Arc::new(client::compat::WineInstall::default());
        self.wine_install = Some(p.clone());
        let egui = self.egui.clone();
        rt.spawn(async move {
            if !client::compat::rosetta_ready() {
                p.rosetta.store(true, std::sync::atomic::Ordering::Relaxed);
                let r = tokio::task::spawn_blocking(client::compat::install_rosetta).await.map_err(|e| e.to_string()).and_then(|r| r);
                p.rosetta.store(false, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = r {
                    *p.result.lock().unwrap() = Some(Err(e));
                    if let Some(c) = &egui {
                        c.request_repaint();
                    }
                    return;
                }
            }
            let _ = client::compat::install_wine(p).await;
            if let Some(c) = &egui {
                c.request_repaint();
            }
        });
        ctx.toast(t!("Installing Wine…"));
    }

    /// Say how the Wine install ended, once; keep the screen moving while it runs.
    pub(crate) fn wine_install_tick(&mut self, ui: &Ui, ctx: &mut Ctx) {
        let Some(w) = self.wine_install.clone() else { return };
        match w.finished() {
            None => ui.ctx().request_repaint_after(std::time::Duration::from_millis(250)),
            Some(Ok(())) => {
                ctx.toast(t!("Wine is installed. Windows games can be installed now."));
                self.wine_install = None;
            }
            Some(Err(e)) if e == "cancelled" => self.wine_install = None,
            Some(Err(_)) => {}
        }
    }

    /// The offer on a Windows-only game's page (desktop library).
    pub(super) fn wine_offer(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, name: &str, owned: Option<&OwnedApp>) {
        let Some(o) = owned else { return };
        if ctx.local.games.iter().any(|g| g.app_id == id) || !wine_would_help(o) {
            return;
        }
        let busy = self.wine_install.clone();
        let failed = busy.as_ref().and_then(|b| b.finished()).and_then(|r| r.err());
        if busy.is_none() && (client::compat::load().no_wine_offer || self.wine_offer_hidden.contains(&id)) {
            return;
        }
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::heading_s(t, t!("Play this Windows game on your Mac"), t.text));
            ui.add_space(t.spacing.xs);
            ui.label(w::body_m(
                t,
                t!("{game} is made for Windows. SteamInRust can run it through Wine, free software that runs Windows games on a Mac (a 225 MB download). Many games work, some don't, and games with anti-cheat can't.", game = name),
                t.text_dim,
            ));
            if !client::compat::rosetta_ready() {
                ui.label(w::body_s(t, t!("Wine also needs Apple's Rosetta 2. It's installed first; macOS asks for your password."), t.text_dim));
            }
            ui.add_space(t.spacing.sm);
            match (&busy, &failed) {
                (Some(b), None) => {
                    let label = if b.rosetta.load(std::sync::atomic::Ordering::Relaxed) {
                        t!("Installing Rosetta 2…").to_string()
                    } else if b.unpacking.load(std::sync::atomic::Ordering::Relaxed) {
                        t!("Unpacking Wine…").to_string()
                    } else {
                        t!("Downloading Wine… {pct}%", pct = (b.fraction() * 100.0) as u32).to_string()
                    };
                    ui.horizontal(|ui| {
                        ui.label(w::body_m(t, label, t.text));
                        w::progress(ui, t, b.fraction(), 260.0);
                        if w::button(ui, t, Some(icons::X), t!("Cancel"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            b.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    });
                }
                _ => {
                    if let Some(e) = &failed {
                        ui.label(w::body_s(t, t!("Wine wasn't installed: {why}", why = e), t.danger));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                        let go = if failed.is_some() { t!("Try again") } else { t!("Install Wine") };
                        if w::button(ui, t, Some(icons::DOWNLOAD), go, w::Kind::Primary, w::Size::Md).clicked() {
                            self.start_wine_install(ctx);
                        }
                        if w::button(ui, t, None, t!("Get CrossOver"), w::Kind::Secondary, w::Size::Md).on_hover_text(t!("A paid app from CodeWeavers, with a free trial")).clicked() {
                            crate::links::open_http("https://www.codeweavers.com/crossover");
                        }
                        if w::button(ui, t, None, t!("Not now"), w::Kind::Secondary, w::Size::Md).clicked() {
                            self.wine_offer_hidden.insert(id);
                            self.wine_install = None;
                        }
                        if w::button(ui, t, None, t!("Don't ask again"), w::Kind::Secondary, w::Size::Md).clicked() {
                            let mut cfg = client::compat::load();
                            cfg.no_wine_offer = true;
                            let _ = client::compat::save(&cfg);
                            self.wine_install = None;
                        }
                    });
                }
            }
        });
        ui.add_space(t.spacing.lg);
    }
}
