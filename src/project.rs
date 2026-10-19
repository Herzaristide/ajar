//! Un livre sur disque : un dossier de fichiers Markdown et un `plan.toml`
//! qui donne l'ordre et les titres des chapitres et des scènes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub titre: String,
    #[serde(default)]
    pub chapitres: Vec<Chapitre>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapitre {
    pub id: String,
    pub titre: String,
    #[serde(default = "statut_initial")]
    pub statut: String,
    #[serde(default)]
    pub scenes: Vec<Scene>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub titre: String,
}

fn statut_initial() -> String {
    "brouillon".into()
}

pub struct Project {
    pub root: PathBuf,
    pub plan: Plan,
}

impl Project {
    /// Ouvre le livre, ou le crée avec un premier chapitre s'il n'existe pas.
    pub fn open_or_init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).with_context(|| format!("création de {}", root.display()))?;
        let root = root.canonicalize()?;
        let plan_path = root.join("plan.toml");

        let plan = if plan_path.exists() {
            let texte = fs::read_to_string(&plan_path)?;
            toml::from_str(&texte).with_context(|| format!("lecture de {}", plan_path.display()))?
        } else {
            let titre = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Sans titre".into());
            Plan { titre, chapitres: Vec::new() }
        };

        let mut project = Self { root, plan };
        if project.plan.chapitres.is_empty() {
            project.add_chapter();
        }
        project.save_plan()?;
        Ok(project)
    }

    pub fn save_plan(&self) -> Result<()> {
        write_atomic(&self.root.join("plan.toml"), &toml::to_string_pretty(&self.plan)?)
    }

    pub fn scene_path(&self, scene_id: &str) -> PathBuf {
        self.root.join("chapitres").join(format!("{scene_id}.md"))
    }

    pub fn chapter_notes_path(&self, chapter_id: &str) -> PathBuf {
        self.root.join("notes").join("chapitres").join(format!("{chapter_id}.md"))
    }

    pub fn general_notes_path(&self) -> PathBuf {
        self.root.join("notes").join("generales.md")
    }

    /// Ajoute un chapitre (avec une première scène) et renvoie son index.
    pub fn add_chapter(&mut self) -> usize {
        let n = self
            .plan
            .chapitres
            .iter()
            .filter_map(|c| c.id.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("{n:02}");
        self.plan.chapitres.push(Chapitre {
            scenes: vec![Scene { id: format!("{id}-01"), titre: "Scène 1".into() }],
            titre: format!("Chapitre {}", self.plan.chapitres.len() + 1),
            statut: statut_initial(),
            id,
        });
        self.plan.chapitres.len() - 1
    }

    /// Ajoute une scène à la fin d'un chapitre et renvoie son index.
    pub fn add_scene(&mut self, chapter: usize) -> usize {
        let ch = &mut self.plan.chapitres[chapter];
        let n = ch
            .scenes
            .iter()
            .filter_map(|s| s.id.rsplit('-').next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        ch.scenes.push(Scene {
            id: format!("{}-{n:02}", ch.id),
            titre: format!("Scène {}", ch.scenes.len() + 1),
        });
        ch.scenes.len() - 1
    }

    /// Retrouve la position d'une scène à partir de son identifiant.
    pub fn find_scene(&self, scene_id: &str) -> Option<(usize, usize)> {
        self.plan.chapitres.iter().enumerate().find_map(|(ci, c)| {
            c.scenes.iter().position(|s| s.id == scene_id).map(|si| (ci, si))
        })
    }
}

/// Lit un fichier texte ; un fichier absent est un texte vide.
pub fn read(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("lecture de {}", path.display())),
    }
}

/// Écrit dans un fichier temporaire puis le renomme : une coupure en pleine
/// écriture ne laisse jamais un texte à moitié écrit.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().context("chemin sans dossier parent")?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().context("chemin sans nom")?.to_string_lossy()
    ));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("écriture de {}", path.display()))?;
    Ok(())
}

pub fn roman(mut n: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
        (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in TABLE {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chiffres_romains() {
        assert_eq!(roman(1), "I");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(14), "XIV");
        assert_eq!(roman(1999), "MCMXCIX");
    }

    #[test]
    fn identifiants_croissants() {
        let dir = std::env::temp_dir().join(format!("scriptorium-test-{}", std::process::id()));
        let mut p = Project::open_or_init(&dir).unwrap();
        assert_eq!(p.plan.chapitres[0].id, "01");
        let c = p.add_chapter();
        assert_eq!(p.plan.chapitres[c].id, "02");
        let s = p.add_scene(c);
        assert_eq!(p.plan.chapitres[c].scenes[s].id, "02-02");
        p.save_plan().unwrap();
        let p2 = Project::open_or_init(&dir).unwrap();
        assert_eq!(p2.find_scene("02-02"), Some((1, 1)));
        fs::remove_dir_all(dir).unwrap();
    }
}
