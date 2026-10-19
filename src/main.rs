//! Ajar — un espace d'écriture zen pour la littérature.
//!
//! Usage : `ajar [DOSSIER_DU_LIVRE]`. Sans argument, rouvre le dernier
//! livre, ou crée « Mon livre » dans ~/Documents/Ajar.

mod app;
mod config;
mod editor;
mod historique;
mod manuscrit;
mod project;
mod typography;

use std::path::PathBuf;

use gtk::glib;

fn main() -> glib::ExitCode {
    let config = config::Config::load();
    let state = config::State::load();

    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| state.dernier_livre.clone().filter(|p| p.join("plan.toml").exists()))
        .unwrap_or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Documents)
                .unwrap_or_else(glib::home_dir)
                .join("Ajar")
                .join("Mon livre")
        });

    let project = match project::Project::open_or_init(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ajar : impossible d’ouvrir {} : {e:#}", root.display());
            return glib::ExitCode::FAILURE;
        }
    };
    // Lu avant d’ouvrir la fenêtre : un livre illisible ne doit jamais être
    // réenregistré à moitié.
    let chapitres = match project.lire_chapitres() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ajar : impossible de lire {} : {e:#}", root.display());
            return glib::ExitCode::FAILURE;
        }
    };

    relm4::RelmApp::new("fr.ciph3r.Ajar")
        .with_args(Vec::new())
        .run::<app::App>(app::Init { project, chapitres, config, state });
    glib::ExitCode::SUCCESS
}
