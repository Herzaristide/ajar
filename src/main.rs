//! Scriptorium — un espace d'écriture zen pour la littérature.
//!
//! Usage : `scriptorium [DOSSIER_DU_LIVRE]`. Sans argument, rouvre le dernier
//! livre, ou crée « Mon livre » dans ~/Documents/Scriptorium.

mod app;
mod config;
mod editor;
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
                .join("Scriptorium")
                .join("Mon livre")
        });

    let project = match project::Project::open_or_init(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("scriptorium : impossible d’ouvrir {} : {e:#}", root.display());
            return glib::ExitCode::FAILURE;
        }
    };

    relm4::RelmApp::new("fr.ciph3r.Scriptorium")
        .with_args(Vec::new())
        .run::<app::App>(app::Init { project, config, state });
    glib::ExitCode::SUCCESS
}
