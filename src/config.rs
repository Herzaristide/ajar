//! Préférences (`~/.config/scriptorium/config.toml`, éventuellement écrit par
//! Home Manager, donc en lecture seule) et état de la dernière session
//! (`~/.local/state/scriptorium/etat.toml`).

use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::project;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Famille de police du texte.
    pub police: String,
    /// Taille du texte, en pixels.
    pub taille: f64,
    /// Largeur de la colonne, en caractères.
    pub largeur: u32,
    /// `papier`, `encre` ou `auto` (suit le thème du système).
    pub theme: String,
    /// Typographie française automatique.
    pub typographie: bool,
    /// Ouvrir l'application en plein écran.
    pub plein_ecran: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            police: "Literata".into(),
            taille: 19.0,
            largeur: 65,
            theme: "auto".into(),
            typographie: true,
            plein_ecran: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = glib::user_config_dir().join("scriptorium").join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                eprintln!("scriptorium : {} illisible ({e}), préférences par défaut", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub dernier_livre: Option<PathBuf>,
    pub derniere_scene: Option<String>,
    /// Thème choisi à la main (`Ctrl+T`), prioritaire sur la configuration.
    pub theme: Option<String>,
}

impl State {
    fn path() -> PathBuf {
        glib::user_state_dir().join("scriptorium").join("etat.toml")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Err(e) = toml::to_string_pretty(self)
            .map_err(anyhow::Error::from)
            .and_then(|t| project::write_atomic(&Self::path(), &t))
        {
            eprintln!("scriptorium : état non enregistré ({e})");
        }
    }
}
