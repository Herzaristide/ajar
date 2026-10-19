//! Un livre sur disque : un fichier Markdown par chapitre, et un `plan.toml`
//! qui garde leur ordre. Les titres vivent dans le texte (`# Titre`) ; le plan
//! n'en garde qu'une copie, pour qui lit le dossier.

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
    #[serde(default)]
    pub titre: String,
    #[serde(default = "statut_initial")]
    pub statut: String,
    /// Ancien format : une liste de scènes, chacune dans son fichier. Lue pour
    /// la migration, jamais réécrite.
    #[serde(default, skip_serializing)]
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
    /// Le plan tel qu'il est sur le disque : on ne le réécrit que s'il change.
    plan_ecrit: String,
}

impl Project {
    /// Ouvre le livre, ou le crée vide s'il n'existe pas.
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

        Ok(Self { root, plan, plan_ecrit: String::new() })
    }

    /// Écrit le plan s'il a changé depuis la dernière fois.
    pub fn save_plan(&mut self) -> Result<()> {
        let texte = toml::to_string_pretty(&self.plan)?;
        if texte != self.plan_ecrit {
            write_atomic(&self.root.join("plan.toml"), &texte)?;
            self.plan_ecrit = texte;
        }
        Ok(())
    }

    /// Remplace la liste des chapitres (identifiant, titre), dans l'ordre du
    /// texte, en gardant le statut de ceux qui existaient déjà.
    pub fn set_chapitres(&mut self, chapitres: impl IntoIterator<Item = (String, String)>) {
        let anciens = std::mem::take(&mut self.plan.chapitres);
        self.plan.chapitres = chapitres
            .into_iter()
            .map(|(id, titre)| {
                let statut = anciens
                    .iter()
                    .find(|c| c.id == id)
                    .map_or_else(statut_initial, |c| c.statut.clone());
                Chapitre { id, titre, statut, scenes: Vec::new() }
            })
            .collect();
    }

    pub fn chapter_path(&self, chapter_id: &str) -> PathBuf {
        self.root.join("chapitres").join(format!("{chapter_id}.md"))
    }

    fn scene_path(&self, scene_id: &str) -> PathBuf {
        self.root.join("chapitres").join(format!("{scene_id}.md"))
    }

    pub fn chapter_notes_path(&self, chapter_id: &str) -> PathBuf {
        self.root.join("notes").join("chapitres").join(format!("{chapter_id}.md"))
    }

    pub fn general_notes_path(&self) -> PathBuf {
        self.root.join("notes").join("generales.md")
    }

    /// Le texte de chaque chapitre du plan, dans l'ordre.
    pub fn lire_chapitres(&self) -> Result<Vec<(String, String)>> {
        self.plan
            .chapitres
            .iter()
            .map(|ch| {
                let path = self.chapter_path(&ch.id);
                let texte = if path.exists() || ch.scenes.is_empty() {
                    read(&path)?
                } else {
                    self.migrer(ch)?
                };
                Ok((ch.id.clone(), texte))
            })
            .collect()
    }

    /// Réunit les scènes de l'ancien format dans le fichier du chapitre, séparées
    /// par une ligne « * * * », et l’écrit tout de suite : le plan suivant ne connaîtra
    /// plus les scènes. Les anciens fichiers restent en place.
    fn migrer(&self, ch: &Chapitre) -> Result<String> {
        let mut texte = format!("# {}\n\n", ch.titre);
        let mut corps = Vec::new();
        for scene in &ch.scenes {
            let t = read(&self.scene_path(&scene.id))?;
            if !t.trim().is_empty() {
                corps.push(t.trim_end().to_string());
            }
        }
        if !corps.is_empty() {
            texte.push_str(&corps.join("\n\n* * *\n\n"));
            texte.push_str("\n\n");
        }
        write_atomic(&self.chapter_path(&ch.id), &texte)?;
        Ok(texte)
    }

    /// Un identifiant de chapitre neuf : ni dans `pris`, ni dans le plan, ni
    /// parmi les fichiers déjà présents (ceux d'un chapitre effacé restent).
    pub fn nouvel_id<'a>(&self, pris: impl IntoIterator<Item = &'a str>) -> String {
        let numero = |s: &str| s.split(['-', '.']).next()?.parse::<u32>().ok();
        let fichiers: Vec<String> = fs::read_dir(self.root.join("chapitres"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let n = pris
            .into_iter()
            .filter_map(numero)
            .chain(self.plan.chapitres.iter().filter_map(|c| numero(&c.id)))
            .chain(fichiers.iter().filter_map(|f| numero(f)))
            .max()
            .unwrap_or(0)
            + 1;
        format!("{n:02}")
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
        let dir = std::env::temp_dir().join(format!("ajar-test-{}", std::process::id()));
        let p = Project::open_or_init(&dir).unwrap();
        assert_eq!(p.nouvel_id([]), "01");
        assert_eq!(p.nouvel_id(["01", "03"]), "04");
        // Le fichier d'un chapitre effacé garde son numéro.
        write_atomic(&p.chapter_path("07"), "# Effacé\n").unwrap();
        assert_eq!(p.nouvel_id(["01"]), "08");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_des_scenes() {
        let dir = std::env::temp_dir().join(format!("ajar-migration-{}", std::process::id()));
        fs::create_dir_all(dir.join("chapitres")).unwrap();
        fs::write(
            dir.join("plan.toml"),
            r#"titre = "Livre"
[[chapitres]]
id = "01"
titre = "Le départ"
[[chapitres.scenes]]
id = "01-01"
titre = "Aube"
[[chapitres.scenes]]
id = "01-02"
titre = "Midi"
"#,
        )
        .unwrap();
        fs::write(dir.join("chapitres/01-01.md"), "Il partit.\n").unwrap();
        fs::write(dir.join("chapitres/01-02.md"), "Il arriva.").unwrap();

        let mut p = Project::open_or_init(&dir).unwrap();
        let attendu = "# Le départ\n\nIl partit.\n\n* * *\n\nIl arriva.\n\n";
        assert_eq!(p.lire_chapitres().unwrap(), vec![("01".to_string(), attendu.to_string())]);
        // Écrit tout de suite : le plan réécrit sans les scènes retrouve le texte.
        p.save_plan().unwrap();
        let p2 = Project::open_or_init(&dir).unwrap();
        assert!(p2.plan.chapitres[0].scenes.is_empty());
        assert_eq!(p2.lire_chapitres().unwrap()[0].1, attendu);
        fs::remove_dir_all(dir).unwrap();
    }

    fn dossier(nom: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ajar-{nom}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn livre_neuf_titre_du_dossier() {
        let dir = dossier("titre");
        let p = Project::open_or_init(&dir).unwrap();
        assert_eq!(p.plan.titre, format!("ajar-titre-{}", std::process::id()));
        assert!(p.plan.chapitres.is_empty());
        assert!(p.lire_chapitres().unwrap().is_empty());
        assert_eq!(p.chapter_path("03"), p.root.join("chapitres/03.md"));
        assert_eq!(p.chapter_notes_path("03"), p.root.join("notes/chapitres/03.md"));
        assert_eq!(p.general_notes_path(), p.root.join("notes/generales.md"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chapitres_gardent_leur_statut() {
        let dir = dossier("statut");
        let mut p = Project::open_or_init(&dir).unwrap();
        p.set_chapitres([("01".into(), "Un".into()), ("02".into(), "Deux".into())]);
        assert!(p.plan.chapitres.iter().all(|c| c.statut == "brouillon"));
        p.plan.chapitres[1].statut = "relu".into();

        // Réordonnés et renommés, avec un nouveau venu.
        p.set_chapitres([("02".into(), "Deux bis".into()), ("03".into(), "Trois".into()), ("01".into(), "Un".into())]);
        let vus: Vec<_> = p.plan.chapitres.iter().map(|c| (c.id.as_str(), c.titre.as_str(), c.statut.as_str())).collect();
        assert_eq!(vus, [("02", "Deux bis", "relu"), ("03", "Trois", "brouillon"), ("01", "Un", "brouillon")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn plan_ecrit_seulement_s_il_change() {
        let dir = dossier("plan");
        let mut p = Project::open_or_init(&dir).unwrap();
        p.set_chapitres([("01".into(), "Un".into())]);
        p.save_plan().unwrap();
        let plan = p.root.join("plan.toml");
        assert!(fs::read_to_string(&plan).unwrap().contains("titre = \"Un\""));

        // Inchangé : pas réécrit (le fichier effacé ne revient pas).
        fs::remove_file(&plan).unwrap();
        p.save_plan().unwrap();
        assert!(!plan.exists());

        p.set_chapitres([("01".into(), "Un autre".into())]);
        p.save_plan().unwrap();
        let relu = Project::open_or_init(&dir).unwrap();
        assert_eq!(relu.plan.chapitres[0].titre, "Un autre");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn plan_illisible() {
        let dir = dossier("illisible");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.toml"), "titre = ").unwrap();
        assert!(Project::open_or_init(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lecture_et_ecriture_atomique() {
        let dir = dossier("atomique");
        let f = dir.join("a/b/texte.md");
        assert_eq!(read(&f).unwrap(), "");
        // Les dossiers manquants sont créés ; aucun fichier temporaire ne reste.
        write_atomic(&f, "Premier.").unwrap();
        write_atomic(&f, "Second.").unwrap();
        assert_eq!(read(&f).unwrap(), "Second.");
        let restes: Vec<_> = fs::read_dir(dir.join("a/b")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(restes, ["texte.md"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
