//! Préférences (`~/.config/ajar/config.toml`, éventuellement écrit par
//! Home Manager, donc en lecture seule) et état de la dernière session
//! (`~/.local/state/ajar/etat.toml`).

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
        let path = glib::user_config_dir().join("ajar").join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                eprintln!("ajar : {} illisible ({e}), préférences par défaut", path.display());
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
    /// Position du curseur dans le livre, en caractères.
    pub curseur: Option<i32>,
    /// Thème choisi à la main (`Ctrl+T`), prioritaire sur la configuration.
    pub theme: Option<String>,
    /// Texte justifié sur les deux bords (palette d'options).
    pub justifie: bool,
}

impl State {
    fn path() -> PathBuf {
        glib::user_state_dir().join("ajar").join("etat.toml")
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
            eprintln!("ajar : état non enregistré ({e})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_partielle() {
        // Ce qui manque prend sa valeur par défaut ; l'inconnu est ignoré.
        let c: Config = toml::from_str("taille = 22.0\ntheme = \"encre\"\ninconnu = 1").unwrap();
        assert_eq!(c.taille, 22.0);
        assert_eq!(c.theme, "encre");
        assert_eq!(c.police, "Literata");
        assert!(c.typographie && c.plein_ecran);
    }

    #[test]
    fn configuration_vide() {
        let c: Config = toml::from_str("").unwrap();
        let d = Config::default();
        assert_eq!((c.police, c.taille, c.largeur, c.theme), (d.police, d.taille, d.largeur, d.theme));
    }

    #[test]
    fn etat_aller_retour() {
        let etat = State {
            dernier_livre: Some(PathBuf::from("/livres/Mon livre")),
            curseur: Some(1234),
            theme: Some("papier".into()),
            justifie: true,
        };
        let relu: State = toml::from_str(&toml::to_string_pretty(&etat).unwrap()).unwrap();
        assert_eq!(relu.dernier_livre, etat.dernier_livre);
        assert_eq!(relu.curseur, Some(1234));
        assert_eq!(relu.theme.as_deref(), Some("papier"));
        assert!(relu.justifie);
        // Un état d'une version antérieure, sans tous les champs.
        let ancien: State = toml::from_str("curseur = 5").unwrap();
        assert_eq!((ancien.curseur, ancien.dernier_livre, ancien.justifie), (Some(5), None, false));
    }
}
