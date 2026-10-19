//! L'historique du livre : des instantanés pris tout seuls pendant qu'on écrit,
//! et des jalons qu'on nomme (« Premier jet », « Envoi à l'éditrice »).
//!
//! Dessous, un dépôt git nu dans `.ajar/historique`, à part : un livre déjà
//! versionné par son auteur garde son propre `.git`. On n'y passe ni par un
//! index ni par une copie de travail : chaque instantané est un arbre bâti
//! directement depuis les fichiers du livre (`plan.toml`, `chapitres/`,
//! `notes/`). Un instantané identique au précédent n'est pas pris.
//!
//! Les jalons sont des tags annotés `jalon-…` posés sur un instantané ; leur nom,
//! libre, est dans le message du tag.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use gix::refs::transaction::PreviousValue;

/// Le dossier de l'application dans le livre, et le dépôt qu'il contient.
const DOSSIER: &str = ".ajar";
const DEPOT: &str = "historique";
/// Ce qui, dans le livre, entre dans un instantané : le plan, et les textes
/// Markdown de ces dossiers.
const PLAN: &str = "plan.toml";
const DOSSIERS: [&str; 2] = ["chapitres", "notes"];

pub struct Historique {
    repo: gix::Repository,
    racine: PathBuf,
}

#[derive(Debug, Clone)]
// `id` et `quand` serviront au panneau « Historique », à venir.
#[allow(dead_code)]
pub struct Instantane {
    pub id: ObjectId,
    /// Secondes depuis l'époque Unix.
    pub quand: i64,
    /// Mots des chapitres, à ce moment-là.
    pub mots: usize,
    /// Les noms des jalons posés sur cet instantané.
    pub jalons: Vec<String>,
}

impl Historique {
    /// Ouvre l'historique du livre, ou le crée.
    pub fn ouvrir(racine: &Path) -> Result<Self> {
        let dossier = racine.join(DOSSIER);
        let chemin = dossier.join(DEPOT);
        let repo = if chemin.join("HEAD").exists() {
            // Sans la configuration git de l'utilisateur : elle n'a rien à faire ici.
            gix::open_opts(&chemin, gix::open::Options::isolated())
        } else {
            fs::create_dir_all(&dossier)?;
            // Un livre déjà suivi par git ignore ce dossier.
            fs::write(dossier.join(".gitignore"), "*\n")?;
            gix::init_bare(&chemin)
        }
        .with_context(|| format!("ouverture de l’historique {}", chemin.display()))?;
        Ok(Self { repo, racine: racine.to_path_buf() })
    }

    /// Prend un instantané du livre tel qu'il est sur le disque. Renvoie `None`
    /// s'il n'a pas changé depuis le dernier.
    pub fn photographier(&self) -> Result<Option<Instantane>> {
        let (arbre, mots) = self.arbre()?;
        let parent = self.dernier()?;
        if let Some(p) = &parent {
            if self.repo.find_commit(*p)?.tree_id()? == arbre {
                return Ok(None);
            }
        }
        let quand = maintenant();
        let temps = format!("{quand} +0000");
        let signature = gix::actor::SignatureRef { name: "Ajar".into(), email: "ajar@localhost".into(), time: &temps };
        let message = format!("Instantané\n\nMots: {mots}\n");
        let id = self.repo.commit_as(signature, signature, "HEAD", message, arbre, parent)?.detach();
        Ok(Some(Instantane { id, quand, mots, jalons: Vec::new() }))
    }

    /// Pose un jalon sur l'état présent du livre, en prenant d'abord un
    /// instantané s'il le faut.
    pub fn jalon(&self, nom: &str) -> Result<Instantane> {
        let nom = nom.trim();
        anyhow::ensure!(!nom.is_empty(), "un jalon a besoin d’un nom");
        self.photographier()?;
        let id = self.dernier()?.context("le livre n’a encore aucun instantané")?;
        let quand = maintenant();
        let temps = format!("{quand} +0000");
        let signature = gix::actor::SignatureRef { name: "Ajar".into(), email: "ajar@localhost".into(), time: &temps };
        // Le nom d'un tag git ne prend ni espaces ni accents : il est fait de la
        // date, le nom choisi va dans le message.
        let mut n = 0;
        loop {
            let etiquette = if n == 0 { format!("jalon-{quand}") } else { format!("jalon-{quand}-{n}") };
            if self.repo.try_find_reference(format!("refs/tags/{etiquette}").as_str())?.is_none() {
                self.repo.tag(
                    &etiquette,
                    id,
                    gix::objs::Kind::Commit,
                    Some(signature),
                    nom,
                    PreviousValue::MustNotExist,
                )?;
                break;
            }
            n += 1;
        }
        let mut instantane = self.instantane(id, &self.jalons()?)?;
        instantane.jalons.retain(|j| j == nom);
        Ok(instantane)
    }

    /// Tous les instantanés, du plus récent au plus ancien.
    #[cfg_attr(not(test), allow(dead_code))] // Pour le panneau « Historique », à venir.
    pub fn lister(&self) -> Result<Vec<Instantane>> {
        let jalons = self.jalons()?;
        let mut tous = Vec::new();
        let mut suivant = self.dernier()?;
        while let Some(id) = suivant {
            tous.push(self.instantane(id, &jalons)?);
            suivant = self.repo.find_commit(id)?.parent_ids().next().map(|p| p.detach());
        }
        Ok(tous)
    }

    /// Le texte d'un fichier du livre (`chapitres/01.md`…) dans un instantané ;
    /// `None` s'il n'existait pas encore.
    #[cfg_attr(not(test), allow(dead_code))] // Pour la comparaison et la restauration, à venir.
    pub fn lire(&self, instantane: ObjectId, chemin: &str) -> Result<Option<String>> {
        let arbre = self.repo.find_commit(instantane)?.tree()?;
        let Some(entree) = arbre.lookup_entry_by_path(chemin)? else { return Ok(None) };
        let blob = entree.object()?;
        Ok(Some(String::from_utf8(blob.data.clone()).context("texte illisible dans l’historique")?))
    }

    /// Le dernier instantané pris, s'il y en a un.
    fn dernier(&self) -> Result<Option<ObjectId>> {
        let Some(mut head) = self.repo.try_find_reference("HEAD")? else { return Ok(None) };
        // Avant le premier instantané, HEAD désigne une branche qui n'existe pas.
        match head.peel_to_id() {
            Ok(id) => Ok(Some(id.detach())),
            Err(_) if self.repo.head()?.is_unborn() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn instantane(&self, id: ObjectId, jalons: &[(ObjectId, String)]) -> Result<Instantane> {
        let commit = self.repo.find_commit(id)?;
        let mots = commit
            .message_raw()?
            .to_string()
            .lines()
            .find_map(|l| l.strip_prefix("Mots: ")?.trim().parse().ok())
            .unwrap_or(0);
        Ok(Instantane {
            id,
            quand: commit.time()?.seconds,
            mots,
            jalons: jalons.iter().filter(|(c, _)| *c == id).map(|(_, n)| n.clone()).collect(),
        })
    }

    /// Les jalons posés : (instantané, nom), par ordre de pose.
    fn jalons(&self) -> Result<Vec<(ObjectId, String)>> {
        let mut jalons = Vec::new();
        let refs = self.repo.references()?;
        for r in refs.prefixed("refs/tags/jalon-")? {
            let mut r = r.map_err(|e| anyhow::anyhow!("{e}"))?;
            let tag = self.repo.find_object(r.target().id())?.try_into_tag()?;
            let nom = tag.decode()?.message.to_string().trim().to_string();
            let quand = tag.tagger()?.and_then(|t| t.time().ok()).map_or(0, |t| t.seconds);
            jalons.push((quand, r.peel_to_id()?.detach(), nom));
        }
        jalons.sort_by_key(|(quand, _, _)| *quand);
        Ok(jalons.into_iter().map(|(_, id, nom)| (id, nom)).collect())
    }

    /// L'arbre du livre tel qu'il est sur le disque, et ses mots.
    fn arbre(&self) -> Result<(ObjectId, usize)> {
        let mut editeur = self.repo.edit_tree(ObjectId::empty_tree(self.repo.object_hash()))?;
        let mut mots = 0;
        let mut fichiers = Vec::new();
        if self.racine.join(PLAN).exists() {
            fichiers.push(PLAN.to_string());
        }
        for dossier in DOSSIERS {
            textes(&self.racine, Path::new(dossier), &mut fichiers)?;
        }
        for chemin in fichiers {
            let contenu = fs::read(self.racine.join(&chemin)).with_context(|| format!("lecture de {chemin}"))?;
            if chemin.starts_with("chapitres/") {
                mots += compter_mots(&String::from_utf8_lossy(&contenu));
            }
            let blob = self.repo.write_blob(&contenu)?;
            editeur.upsert(chemin.as_str(), EntryKind::Blob, blob)?;
        }
        Ok((editeur.write()?.detach(), mots))
    }
}

/// Les fichiers Markdown sous `dossier`, en chemins relatifs au livre séparés
/// par « / ». Les fichiers cachés (dont les `.tmp` d'une écriture en cours)
/// n'en sont pas.
fn textes(racine: &Path, dossier: &Path, sortie: &mut Vec<String>) -> Result<()> {
    let Ok(entrees) = fs::read_dir(racine.join(dossier)) else { return Ok(()) };
    for entree in entrees {
        let entree = entree?;
        let nom = entree.file_name().to_string_lossy().into_owned();
        if nom.starts_with('.') {
            continue;
        }
        let relatif = dossier.join(&nom);
        if entree.file_type()?.is_dir() {
            textes(racine, &relatif, sortie)?;
        } else if nom.ends_with(".md") {
            let chemin: Vec<_> = relatif.components().map(|c| c.as_os_str().to_string_lossy()).collect();
            sortie.push(chemin.join("/"));
        }
    }
    Ok(())
}

/// Les mots d'un texte : ni les « # » des titres ni les « * * * » des
/// séparations n'en sont.
pub fn compter_mots(texte: &str) -> usize {
    texte.split_whitespace().filter(|m| m.chars().any(char::is_alphanumeric)).count()
}

fn maintenant() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::write_atomic;

    fn livre(nom: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ajar-historique-{nom}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.toml"), "titre = \"Livre\"\n").unwrap();
        write_atomic(&dir.join("chapitres/01.md"), "# Un\n\nIl partit.\n").unwrap();
        dir
    }

    #[test]
    fn instantanes_et_lecture() {
        let dir = livre("lecture");
        let h = Historique::ouvrir(&dir).unwrap();
        assert!(h.lister().unwrap().is_empty());

        let premier = h.photographier().unwrap().expect("premier instantané");
        assert_eq!(premier.mots, 3);
        // Rien n'a changé : pas de nouvel instantané.
        assert!(h.photographier().unwrap().is_none());

        write_atomic(&dir.join("chapitres/01.md"), "# Un\n\nIl partit avant le jour.\n").unwrap();
        write_atomic(&dir.join("notes/generales.md"), "Penser à la mère.").unwrap();
        // Un fichier en cours d'écriture n'entre pas dans l'instantané.
        fs::write(dir.join("chapitres/.02.md.tmp"), "brouillon").unwrap();
        let second = h.photographier().unwrap().expect("second instantané");
        assert_eq!(second.mots, 6);

        // Rouvert, l'historique retrouve tout.
        let h = Historique::ouvrir(&dir).unwrap();
        let tous = h.lister().unwrap();
        assert_eq!(tous.iter().map(|i| i.id).collect::<Vec<_>>(), vec![second.id, premier.id]);
        assert_eq!(h.lire(premier.id, "chapitres/01.md").unwrap().as_deref(), Some("# Un\n\nIl partit.\n"));
        assert_eq!(h.lire(premier.id, "notes/generales.md").unwrap(), None);
        assert_eq!(h.lire(second.id, "notes/generales.md").unwrap().as_deref(), Some("Penser à la mère."));
        assert_eq!(h.lire(second.id, "chapitres/.02.md.tmp").unwrap(), None);
        assert_eq!(fs::read_to_string(dir.join(".ajar/.gitignore")).unwrap(), "*\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jalons() {
        let dir = livre("jalons");
        let h = Historique::ouvrir(&dir).unwrap();
        assert!(h.jalon("  ").is_err());

        // Sans instantané préalable, le jalon en prend un.
        let premier = h.jalon("Premier jet").unwrap();
        assert_eq!(premier.jalons, vec!["Premier jet"]);
        // Deux jalons sur le même état, la même seconde : aucun n'écrase l'autre.
        let relu = h.jalon("Relu par Anne").unwrap();
        assert_eq!(relu.id, premier.id);

        write_atomic(&dir.join("chapitres/01.md"), "# Un\n\nElle partit.\n").unwrap();
        let ensuite = h.photographier().unwrap().unwrap();

        let tous = h.lister().unwrap();
        assert_eq!(tous.len(), 2);
        assert_eq!(tous[0].id, ensuite.id);
        assert!(tous[0].jalons.is_empty());
        assert_eq!(tous[1].jalons, vec!["Premier jet", "Relu par Anne"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn suppressions_et_sous_dossiers() {
        let dir = livre("sous-dossiers");
        write_atomic(&dir.join("notes/chapitres/01.md"), "Note du un.").unwrap();
        write_atomic(&dir.join("chapitres/02.md"), "# Deux\n").unwrap();
        // Hors des dossiers du livre, ou pas du Markdown : ignoré.
        fs::write(dir.join("chapitres/image.png"), [0u8, 1, 2]).unwrap();
        fs::write(dir.join("brouillon.md"), "à part").unwrap();
        let h = Historique::ouvrir(&dir).unwrap();
        let avant = h.photographier().unwrap().unwrap();
        assert_eq!(h.lire(avant.id, "notes/chapitres/01.md").unwrap().as_deref(), Some("Note du un."));
        assert_eq!(h.lire(avant.id, "chapitres/image.png").unwrap(), None);
        assert_eq!(h.lire(avant.id, "brouillon.md").unwrap(), None);

        fs::remove_file(dir.join("chapitres/02.md")).unwrap();
        let apres = h.photographier().unwrap().expect("la suppression est un changement");
        assert_eq!(h.lire(apres.id, "chapitres/02.md").unwrap(), None);
        assert_eq!(h.lire(avant.id, "chapitres/02.md").unwrap().as_deref(), Some("# Deux\n"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn livre_vide() {
        let dir = std::env::temp_dir().join(format!("ajar-historique-vide-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let h = Historique::ouvrir(&dir).unwrap();
        let i = h.photographier().unwrap().expect("même vide, un premier instantané");
        assert_eq!(i.mots, 0);
        assert!(h.photographier().unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ordre_chronologique() {
        let dir = livre("ordre");
        let h = Historique::ouvrir(&dir).unwrap();
        let mut ids = Vec::new();
        for n in 1..=4 {
            write_atomic(&dir.join("chapitres/01.md"), &"mot ".repeat(n)).unwrap();
            ids.push(h.photographier().unwrap().unwrap().id);
        }
        let tous = h.lister().unwrap();
        ids.reverse();
        assert_eq!(tous.iter().map(|i| i.id).collect::<Vec<_>>(), ids);
        assert_eq!(tous.iter().map(|i| i.mots).collect::<Vec<_>>(), [4, 3, 2, 1]);
        assert!(tous.windows(2).all(|w| w[0].quand >= w[1].quand));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mots() {
        assert_eq!(compter_mots("# Le départ\n\nIl partit — vite.\n\n* * *\n\nL’aube."), 6);
    }
}
