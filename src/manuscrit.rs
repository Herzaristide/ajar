//! Le livre à l'écran : un seul texte continu, dont le plan se lit dans les
//! titres (`#`, `##`, `###`). Le niveau le plus profond employé donne les
//! chapitres, celui du dessus les parties, le suivant les livres : `#` seul,
//! ce sont des chapitres ; `#` et `##`, des parties et des chapitres ; les
//! trois, des livres, des parties et des chapitres.
//!
//! Sur le disque, chaque section (le texte d'un titre au suivant) garde son
//! fichier. Une marque posée au début de chaque titre retient de quelle section
//! il s'agit : renommer, déplacer ou effacer puis rétablir un titre ne fait
//! perdre ni ses notes ni son fichier.

use std::collections::HashMap;

use anyhow::Result;
use gtk::prelude::*;

use crate::project::{self, roman, Project};

pub const SANS_TITRE: &str = "Sans titre";

/// Le niveau d'une ligne de titre (son nombre de `#`, de 1 à 3), et son intitulé.
pub fn titre(ligne: &str) -> Option<(u8, &str)> {
    let dieses = ligne.chars().take_while(|&c| c == '#').count();
    let reste = ligne[dieses..].strip_prefix(' ')?;
    (1..=3).contains(&dieses).then(|| (dieses as u8, reste.trim()))
}

/// Le rôle d'un titre dans le plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rang {
    Livre,
    Partie,
    Chapitre,
}

impl Rang {
    pub fn nom(self) -> &'static str {
        match self {
            Rang::Livre => "Livre",
            Rang::Partie => "Partie",
            Rang::Chapitre => "Chapitre",
        }
    }

    /// La taille du titre, relative au texte.
    pub fn echelle(self) -> f64 {
        match self {
            Rang::Livre => 2.1,
            Rang::Partie => 1.75,
            Rang::Chapitre => 1.45,
        }
    }
}

/// Le rôle d'un titre de niveau `niveau`, selon les niveaux employés dans le
/// livre (`employes[0]` pour `#`) : il dépend du nombre de niveaux employés
/// plus profonds que lui.
pub fn rang(niveau: u8, employes: &[bool; 3]) -> Rang {
    match employes[niveau as usize..].iter().filter(|&&e| e).count() {
        0 => Rang::Chapitre,
        1 => Rang::Partie,
        _ => Rang::Livre,
    }
}

/// Met les chapitres bout à bout ; renvoie le texte et le début de chacun (en
/// caractères). Chaque chapitre finit sur un saut de ligne, pour que le titre
/// du suivant reste en début de ligne.
pub fn assembler<'a>(textes: impl IntoIterator<Item = &'a str>) -> (String, Vec<i32>) {
    let mut tout = String::new();
    let mut debuts = Vec::new();
    let mut n = 0;
    for texte in textes {
        if !tout.is_empty() && !tout.ends_with('\n') {
            tout.push('\n');
            n += 1;
        }
        debuts.push(n);
        tout.push_str(texte);
        n += texte.chars().count() as i32;
    }
    (tout, debuts)
}

/// Retrouve, pour chaque chapitre du texte, l'ancre qui le désignait.
///
/// `chapitres` : début et intitulé de chaque chapitre, dans l'ordre (`None`
/// pour le texte d'avant le premier titre). `ancres` : position et dernier
/// intitulé connu de chaque ancre, les chapitres vivants d'abord. `blanc(a, b)` :
/// rien que des blancs entre les positions `a` et `b`.
///
/// Une ancre « touche » le titre qui la suit s'il n'y a que du blanc entre eux
/// (un saut de ligne ajouté avant le titre, un titre effacé puis rétabli). Par
/// ordre de préférence, un chapitre reprend :
/// 1. l'ancre qui le touche et porte son intitulé ;
/// 2. l'ancre qui porte son intitulé, où qu'elle soit (un chapitre coupé puis
///    collé ailleurs, ou un titre tapé juste devant le sien) ;
/// 3. l'ancre qui le touche (un titre qu'on renomme).
pub fn associer(
    chapitres: &[(i32, Option<&str>)],
    ancres: &[(i32, Option<&str>)],
    blanc: impl Fn(i32, i32) -> bool,
) -> Vec<Option<usize>> {
    let mut choix: Vec<Option<usize>> = vec![None; chapitres.len()];
    let mut pris = vec![false; ancres.len()];
    let touche: Vec<Option<usize>> = ancres
        .iter()
        .map(|&(p, _)| {
            let k = chapitres.partition_point(|&(h, _)| h < p);
            chapitres.get(k).filter(|&&(h, _)| blanc(p, h)).map(|_| k)
        })
        .collect();

    // La plus proche l'emporte ; à égalité, la première (un chapitre vivant).
    let contact = |choix: &mut Vec<Option<usize>>, pris: &mut Vec<bool>, accepte: &dyn Fn(usize, usize) -> bool| {
        let mut meilleur: Vec<Option<usize>> = vec![None; chapitres.len()];
        for (j, k) in touche.iter().enumerate() {
            let Some(k) = *k else { continue };
            if pris[j] || choix[k].is_some() || !accepte(j, k) {
                continue;
            }
            if meilleur[k].is_some_and(|i| ancres[i].0 >= ancres[j].0) {
                continue;
            }
            meilleur[k] = Some(j);
        }
        for (k, j) in meilleur.into_iter().enumerate() {
            if let Some(j) = j {
                choix[k] = Some(j);
                pris[j] = true;
            }
        }
    };

    contact(&mut choix, &mut pris, &|j, k| ancres[j].1 == chapitres[k].1);

    for (k, &(h, intitule)) in chapitres.iter().enumerate() {
        let Some(intitule) = intitule.filter(|t| !t.is_empty()) else { continue };
        if choix[k].is_some() {
            continue;
        }
        let j = (0..ancres.len())
            .filter(|&j| !pris[j] && ancres[j].1 == Some(intitule))
            .min_by_key(|&j| (ancres[j].0 - h).abs());
        if let Some(j) = j {
            pris[j] = true;
            choix[k] = Some(j);
        }
    }

    // Un chapitre titré ne devient pas le texte d'avant le premier titre :
    // c'est qu'on a écrit au-dessus de lui.
    contact(&mut choix, &mut pris, &|j, k| chapitres[k].1.is_some() || ancres[j].1.is_none());
    choix
}


pub struct Chapitre {
    pub id: String,
    /// `None` : le texte d'avant le premier titre.
    pub titre: Option<String>,
    pub rang: Rang,
    ancre: gtk::TextMark,
}

impl Chapitre {
    pub fn intitule(&self) -> &str {
        match self.titre.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ => SANS_TITRE,
        }
    }
}

pub struct Manuscrit {
    buffer: gtk::TextBuffer,
    /// Livres, parties et chapitres, dans l'ordre du texte.
    pub chapitres: Vec<Chapitre>,
    /// Sections dont le titre a disparu du texte. Gardées pour la session : si le
    /// titre revient (annulation), la section retrouve son identité.
    dormants: Vec<Chapitre>,
    /// Le dernier contenu écrit de chaque fichier.
    ecrit: HashMap<String, String>,
    tags: [gtk::TextTag; 4],
}

impl Manuscrit {
    pub fn new(buffer: &gtk::TextBuffer, taille: f64) -> Self {
        let titre = |scale: f64, avant: f64, apres: f64| {
            gtk::TextTag::builder()
                .scale(scale)
                .weight(600)
                .justification(gtk::Justification::Center)
                .pixels_above_lines((taille * avant) as i32)
                .pixels_below_lines((taille * apres) as i32)
                .build()
        };
        let livre = titre(Rang::Livre.echelle(), 5.0, 2.5);
        let partie = titre(Rang::Partie.echelle(), 4.0, 2.0);
        let chapitre = titre(Rang::Chapitre.echelle(), 2.5, 1.2);
        // Les dièses restent visibles, mais en retrait.
        let marque = gtk::TextTag::builder()
            .foreground_rgba(&gtk::gdk::RGBA::new(0.54, 0.50, 0.44, 0.55))
            .weight(400)
            .build();
        for tag in [&livre, &partie, &chapitre, &marque] {
            buffer.tag_table().add(tag);
        }
        Self {
            buffer: buffer.clone(),
            chapitres: Vec::new(),
            dormants: Vec::new(),
            ecrit: HashMap::new(),
            tags: [livre, partie, chapitre, marque],
        }
    }

    /// Affiche les sections (identifiant, texte) bout à bout.
    pub fn charger(&mut self, chapitres: Vec<(String, String)>, project: &Project) {
        let (texte, debuts) = assembler(chapitres.iter().map(|(_, t)| t.as_str()));
        self.buffer.set_text(&texte);
        for ch in self.chapitres.drain(..).chain(self.dormants.drain(..)) {
            self.buffer.delete_mark(&ch.ancre);
        }
        for ((id, texte), debut) in chapitres.into_iter().zip(debuts) {
            let titre = texte.lines().next().and_then(titre).map(|(_, t)| t.to_string());
            let ancre = self.buffer.create_mark(None, &self.buffer.iter_at_offset(debut), true);
            self.chapitres.push(Chapitre { id: id.clone(), titre, rang: Rang::Chapitre, ancre });
            self.ecrit.insert(id, texte);
        }
        self.analyser(project);
    }

    /// Relit les titres du texte. Renvoie `true` si le plan a changé.
    pub fn analyser(&mut self, project: &Project) -> bool {
        let avant = self.signature();
        let buffer = &self.buffer;

        let mut lignes = Vec::new();
        for l in 0..buffer.line_count() {
            let Some(debut) = buffer.iter_at_line(l) else { continue };
            if debut.char() != '#' {
                continue;
            }
            let mut fin = debut;
            fin.forward_to_line_end();
            if let Some((niveau, intitule)) = titre(&buffer.text(&debut, &fin, false)) {
                lignes.push((debut.offset(), niveau, intitule.to_string()));
            }
        }
        let mut employes = [false; 3];
        for l in &lignes {
            employes[l.1 as usize - 1] = true;
        }
        self.styler(&lignes, &employes);

        // Le texte d'avant le premier titre forme une section à part, s'il n'est
        // pas blanc (ou s'il n'y a aucun titre).
        let blanc = |a: i32, b: i32| {
            let mut it = buffer.iter_at_offset(a);
            while it.offset() < b {
                if !it.char().is_whitespace() {
                    return false;
                }
                it.forward_char();
            }
            true
        };
        let mut cibles: Vec<(i32, Option<String>, Rang)> = Vec::new();
        if lignes.first().is_none_or(|l| !blanc(0, l.0)) {
            cibles.push((0, None, Rang::Chapitre));
        }
        cibles.extend(lignes.into_iter().map(|(pos, niveau, t)| (pos, Some(t), rang(niveau, &employes))));

        let mut anciens: Vec<Option<Chapitre>> =
            self.chapitres.drain(..).chain(self.dormants.drain(..)).map(Some).collect();
        let positions: Vec<(i32, Option<&str>)> = anciens
            .iter()
            .flatten()
            .map(|c| (buffer.iter_at_mark(&c.ancre).offset(), c.titre.as_deref()))
            .collect();
        let choix = associer(
            &cibles.iter().map(|c| (c.0, c.1.as_deref())).collect::<Vec<_>>(),
            &positions,
            blanc,
        );

        let mut ids: Vec<String> = anciens.iter().flatten().map(|c| c.id.clone()).collect();
        for ((pos, titre, rang), choix) in cibles.into_iter().zip(choix) {
            let debut = buffer.iter_at_offset(pos);
            let mut ch = match choix.and_then(|j| anciens[j].take()) {
                Some(ch) => ch,
                None => {
                    let id = project.nouvel_id(ids.iter().map(String::as_str));
                    ids.push(id.clone());
                    let ancre = buffer.create_mark(None, &debut, true);
                    Chapitre { id, titre: None, rang, ancre }
                }
            };
            buffer.move_mark(&ch.ancre, &debut);
            ch.titre = titre;
            ch.rang = rang;
            self.chapitres.push(ch);
        }
        self.dormants = anciens.into_iter().flatten().collect();

        self.signature() != avant
    }

    fn signature(&self) -> Vec<(String, String, Rang)> {
        self.chapitres.iter().map(|c| (c.id.clone(), c.intitule().to_string(), c.rang)).collect()
    }

    /// Met les titres en forme. Le texte tapé n'hérite d'aucun style : on
    /// reprend tout à chaque analyse.
    fn styler(&self, lignes: &[(i32, u8, String)], employes: &[bool; 3]) {
        // Les plages voulues pour chaque style (livre, partie, chapitre, dièses).
        let mut voulu: [Vec<(i32, i32)>; 4] = Default::default();
        for &(pos, niveau, _) in lignes {
            let mut fin = self.buffer.iter_at_offset(pos);
            fin.forward_to_line_end();
            let i = match rang(niveau, employes) {
                Rang::Livre => 0,
                Rang::Partie => 1,
                Rang::Chapitre => 2,
            };
            voulu[i].push((pos, fin.offset()));
            voulu[3].push((pos, pos + niveau as i32));
        }
        // On ne touche qu'aux plages qui changent : tout retirer puis tout
        // remettre obligerait GTK à remettre en page le livre entier, et la
        // position du curseur resterait fausse le temps qu'il le fasse.
        for (tag, voulu) in self.tags.iter().zip(&voulu) {
            let actuel = self.plages(tag);
            for &(a, b) in actuel.iter().filter(|p| !voulu.contains(p)) {
                self.buffer.remove_tag(tag, &self.buffer.iter_at_offset(a), &self.buffer.iter_at_offset(b));
            }
            for &(a, b) in voulu.iter().filter(|p| !actuel.contains(p)) {
                self.buffer.apply_tag(tag, &self.buffer.iter_at_offset(a), &self.buffer.iter_at_offset(b));
            }
        }
    }

    /// Les plages du texte qui portent un style.
    fn plages(&self, tag: &gtk::TextTag) -> Vec<(i32, i32)> {
        let mut plages = Vec::new();
        let mut it = self.buffer.start_iter();
        if !it.starts_tag(Some(tag)) && !it.forward_to_tag_toggle(Some(tag)) {
            return plages;
        }
        loop {
            let debut = it.offset();
            it.forward_to_tag_toggle(Some(tag));
            plages.push((debut, it.offset()));
            if !it.forward_to_tag_toggle(Some(tag)) {
                return plages;
            }
        }
    }

    /// Le numéro d'une section parmi celles de son rang : les livres d'un bout
    /// à l'autre, les parties et les chapitres depuis le début de leur livre.
    pub fn numero(&self, ci: usize) -> usize {
        let rang = self.chapitres[ci].rang;
        self.chapitres[..=ci]
            .iter()
            .rev()
            .take_while(|c| rang == Rang::Livre || c.rang != Rang::Livre)
            .filter(|c| c.rang == rang)
            .count()
    }

    /// Le titre à proposer quand le curseur suit un titre encore vide
    /// (« ## » tapé en début de ligne) : « Partie II », « Chapitre IV »…
    pub fn proposition(&self, curseur: &gtk::TextIter) -> Option<(String, Rang)> {
        if !curseur.ends_line() || self.buffer.has_selection() {
            return None;
        }
        let mut debut = *curseur;
        debut.set_line_offset(0);
        let ligne = self.buffer.text(&debut, curseur, false);
        let (_, intitule) = titre(&ligne)?;
        let ci = self.chapitre_a(curseur.offset());
        let ch = &self.chapitres[ci];
        // Le plan doit déjà connaître ce titre : sinon, l'analyse n'est pas passée.
        if !intitule.is_empty() || ch.titre.is_none() || self.debut(ch) != debut.offset() {
            return None;
        }
        Some((format!("{} {}", ch.rang.nom(), roman(self.numero(ci))), ch.rang))
    }

    /// L'index de la section où se trouve une position.
    pub fn chapitre_a(&self, pos: i32) -> usize {
        let debuts: Vec<i32> = self.chapitres.iter().map(|c| self.debut(c)).collect();
        debuts.partition_point(|&d| d <= pos).saturating_sub(1)
    }

    fn debut(&self, ch: &Chapitre) -> i32 {
        self.buffer.iter_at_mark(&ch.ancre).offset()
    }

    /// La fin de la ligne de titre d'une section.
    pub fn position_titre(&self, ci: usize) -> Option<gtk::TextIter> {
        let mut it = self.buffer.iter_at_offset(self.debut(self.chapitres.get(ci)?));
        if !it.ends_line() {
            it.forward_to_line_end();
        }
        Some(it)
    }

    /// Écrit les sections qui ont changé, puis le plan.
    pub fn enregistrer(&mut self, project: &mut Project) -> Result<()> {
        self.analyser(project);
        let fin = self.buffer.end_iter();
        for (i, ch) in self.chapitres.iter().enumerate() {
            // La première part du tout début : les blancs d'avant son titre comptent.
            let debut = if i == 0 { self.buffer.start_iter() } else { self.buffer.iter_at_mark(&ch.ancre) };
            let suite = self.chapitres.get(i + 1).map_or(fin, |s| self.buffer.iter_at_mark(&s.ancre));
            let texte = self.buffer.text(&debut, &suite, false).to_string();
            if self.ecrit.get(&ch.id) != Some(&texte) {
                project::write_atomic(&project.chapter_path(&ch.id), &texte)?;
                self.ecrit.insert(ch.id.clone(), texte);
            }
        }
        project.set_chapitres(
            self.chapitres.iter().map(|c| (c.id.clone(), c.titre.clone().unwrap_or_default())),
        );
        project.save_plan()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// `blanc` sur un texte en clair.
    fn blanc(texte: &str) -> impl Fn(i32, i32) -> bool + '_ {
        move |a, b| texte.chars().skip(a as usize).take((b - a) as usize).all(char::is_whitespace)
    }

    #[test]
    fn lignes_de_titre() {
        assert_eq!(titre("# Le départ "), Some((1, "Le départ")));
        assert_eq!(titre("## Aube"), Some((2, "Aube")));
        assert_eq!(titre("# "), Some((1, "")));
        assert_eq!(titre("#hashtag"), None);
        assert_eq!(titre("### Tome"), Some((3, "Tome")));
        assert_eq!(titre("#### Trop bas"), None);
    }

    #[test]
    fn rangs_selon_les_niveaux_employes() {
        use Rang::*;
        assert_eq!(rang(1, &[true, false, false]), Chapitre);
        assert_eq!(rang(1, &[true, true, false]), Partie);
        assert_eq!(rang(2, &[true, true, false]), Chapitre);
        assert_eq!(rang(1, &[true, true, true]), Livre);
        assert_eq!(rang(2, &[true, true, true]), Partie);
        assert_eq!(rang(3, &[true, true, true]), Chapitre);
        // Un niveau sauté ne compte pas : `#` et `###` font parties et chapitres.
        assert_eq!(rang(1, &[true, false, true]), Partie);
        assert_eq!(rang(3, &[false, false, true]), Chapitre);
    }

    #[test]
    fn assemblage() {
        let (texte, debuts) = assembler(["# Un\nIl partit.", "# Deux\n", "# Trois"]);
        assert_eq!(texte, "# Un\nIl partit.\n# Deux\n# Trois");
        assert_eq!(debuts, vec![0, 16, 23]);
    }

    #[test]
    fn renommer_garde_l_ancre() {
        let texte = "# Un\nA\n# Deux bis\nB";
        let r = associer(&[(0, Some("Un")), (7, Some("Deux bis"))], &[(0, Some("Un")), (7, Some("Deux"))], blanc(texte));
        assert_eq!(r, vec![Some(0), Some(1)]);
    }

    #[test]
    fn saut_de_ligne_avant_un_titre() {
        // Entrée tapée au début de « # Deux » : l'ancre reste avant le saut.
        let texte = "# Un\nA\n\n# Deux\nB";
        let r = associer(&[(0, Some("Un")), (8, Some("Deux"))], &[(0, Some("Un")), (7, Some("Deux"))], blanc(texte));
        assert_eq!(r, vec![Some(0), Some(1)]);
    }

    #[test]
    fn titre_efface_puis_nouveau() {
        // « # Deux » effacé : son ancre est au milieu du texte d'« Un ».
        let texte = "# Un\nA\nB\n# Trois\nC";
        let r = associer(
            &[(0, Some("Un")), (9, Some("Trois"))],
            &[(0, Some("Un")), (7, Some("Deux")), (9, Some("Trois"))],
            blanc(texte),
        );
        assert_eq!(r, vec![Some(0), Some(2)]);
        // Un titre tapé plus tard dans « Un » ne reprend pas l'ancre de « Deux ».
        let texte = "# Un\nA\nB\n# Quatre\n# Trois\nC";
        let r = associer(
            &[(0, Some("Un")), (9, Some("Quatre")), (18, Some("Trois"))],
            &[(0, Some("Un")), (18, Some("Trois")), (7, Some("Deux"))],
            blanc(texte),
        );
        assert_eq!(r, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn titre_tape_juste_devant_le_suivant() {
        // L'ancre de « Deux » est restée devant le texte inséré.
        let texte = "# Un\nA\n# Entre\nE\n# Deux\nB";
        let r = associer(
            &[(0, Some("Un")), (7, Some("Entre")), (17, Some("Deux"))],
            &[(0, Some("Un")), (7, Some("Deux"))],
            blanc(texte),
        );
        assert_eq!(r, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn chapitre_deplace_garde_son_intitule() {
        // « Deux » coupé puis collé avant « Un » : les ancres se sont tassées.
        let texte = "# Deux\nB\n# Un\nA";
        let r = associer(&[(0, Some("Deux")), (9, Some("Un"))], &[(9, Some("Un")), (9, Some("Deux"))], blanc(texte));
        assert_eq!(r, vec![Some(1), Some(0)]);
    }

    #[test]
    fn titre_tape_en_tete_d_un_texte_sans_titre() {
        let texte = "# Le départ\nIl partit.";
        let r = associer(&[(0, Some("Le départ"))], &[(0, None)], blanc(texte));
        assert_eq!(r, vec![Some(0)]);
    }

    /// Position (en caractères) d'un extrait dans le tampon.
    fn chercher(buffer: &gtk::TextBuffer, extrait: &str) -> gtk::TextIter {
        let tout = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let octets = tout.find(extrait).expect("extrait absent");
        buffer.iter_at_offset(tout[..octets].chars().count() as i32)
    }

    fn plan(m: &Manuscrit) -> Vec<(String, String)> {
        m.chapitres.iter().map(|c| (c.id.clone(), c.intitule().to_string())).collect()
    }

    /// De bout en bout, sur un vrai tampon GTK : charger, taper et effacer des
    /// titres, enregistrer, relire.
    #[test]
    fn document_continu() {
        if gtk::init().is_err() {
            eprintln!("pas d’écran : test sauté");
            return;
        }
        let dir = std::env::temp_dir().join(format!("ajar-manuscrit-{}", std::process::id()));
        let mut p = Project::open_or_init(&dir).unwrap();
        let buffer = gtk::TextBuffer::new(None);
        let mut m = Manuscrit::new(&buffer, 19.0);
        project::write_atomic(&p.chapter_path("01"), "# Un\n\nIl partit.\n").unwrap();
        project::write_atomic(&p.chapter_path("02"), "# Deux\n\nIl arriva.").unwrap();
        p.set_chapitres([("01".into(), "Un".into()), ("02".into(), "Deux".into())]);
        m.charger(p.lire_chapitres().unwrap(), &p);
        let s = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        assert_eq!(plan(&m), s(&[("01", "Un"), ("02", "Deux")]));

        // Un titre tapé entre les deux ouvre un chapitre ; renommer garde l'identité.
        buffer.insert(&mut chercher(&buffer, "# Deux"), "# Entre\n\nTexte.\n");
        assert!(m.analyser(&p));
        assert_eq!(plan(&m), s(&[("01", "Un"), ("03", "Entre"), ("02", "Deux")]));
        buffer.insert(&mut chercher(&buffer, "\n\nIl arriva"), " bis");
        assert!(m.analyser(&p));
        assert_eq!(plan(&m), s(&[("01", "Un"), ("03", "Entre"), ("02", "Deux bis")]));
        m.enregistrer(&mut p).unwrap();
        let chapitres = dir.join("chapitres");
        let lire = |id: &str| fs::read_to_string(chapitres.join(format!("{id}.md"))).unwrap();
        assert_eq!(lire("01"), "# Un\n\nIl partit.\n");
        assert_eq!(lire("03"), "# Entre\n\nTexte.\n");
        assert_eq!(lire("02"), "# Deux bis\n\nIl arriva.");

        // Une ligne vide ajoutée avant un titre ne change pas de chapitre.
        buffer.insert(&mut chercher(&buffer, "# Deux"), "\n");
        m.analyser(&p);
        assert_eq!(plan(&m)[2].0, "02");

        // Le titre « Entre » effacé : son texte rejoint « Un ».
        let mut debut = chercher(&buffer, "# Entre");
        let mut fin = chercher(&buffer, "\nTexte.");
        buffer.delete(&mut debut, &mut fin);
        m.enregistrer(&mut p).unwrap();
        assert_eq!(plan(&m), s(&[("01", "Un"), ("02", "Deux bis")]));
        assert_eq!(lire("01"), "# Un\n\nIl partit.\n\nTexte.\n\n");

        // Le titre rétabli au même endroit retrouve son identité.
        buffer.insert(&mut chercher(&buffer, "\nTexte."), "# Entre");
        m.analyser(&p);
        assert_eq!(plan(&m)[1].0, "03");

        // Relire le livre redonne exactement le même texte.
        m.enregistrer(&mut p).unwrap();
        let p2 = Project::open_or_init(&dir).unwrap();
        let (texte, _) = assembler(p2.lire_chapitres().unwrap().iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>());
        assert_eq!(texte, buffer.text(&buffer.start_iter(), &buffer.end_iter(), false));

        // Un titre `##` fait des `#` des parties, un `###` en plus des livres ;
        // les effacer rend les `#` à leur rôle de chapitres.
        use Rang::*;
        let rangs = |m: &Manuscrit| m.chapitres.iter().map(|c| c.rang).collect::<Vec<_>>();
        assert_eq!(rangs(&m), [Chapitre, Chapitre, Chapitre]);
        buffer.insert(&mut chercher(&buffer, "# Deux"), "## Halte\n");
        assert!(m.analyser(&p));
        assert_eq!(rangs(&m), [Partie, Partie, Chapitre, Partie]);
        buffer.insert(&mut chercher(&buffer, "## Halte"), "### Repos\n");
        assert!(m.analyser(&p));
        assert_eq!(rangs(&m), [Livre, Livre, Chapitre, Partie, Livre]);
        let numeros: Vec<usize> = (0..5).map(|ci| m.numero(ci)).collect();
        assert_eq!(numeros, [1, 2, 1, 1, 3]);
        let mut debut = chercher(&buffer, "## Halte");
        let mut fin = chercher(&buffer, "# Deux");
        buffer.delete(&mut debut, &mut fin);
        assert!(m.analyser(&p));
        assert_eq!(rangs(&m), [Partie, Partie, Chapitre, Partie]);
        let mut debut = chercher(&buffer, "### Repos");
        let mut fin = chercher(&buffer, "# Deux");
        buffer.delete(&mut debut, &mut fin);
        assert!(m.analyser(&p));
        assert_eq!(rangs(&m), [Chapitre, Chapitre, Chapitre]);

        // Un titre vide en fin de ligne : on propose le suivant de son rang.
        let proposer = |m: &mut Manuscrit, ajout: &str| {
            buffer.insert(&mut buffer.end_iter(), ajout);
            m.analyser(&p);
            buffer.place_cursor(&buffer.end_iter());
            m.proposition(&buffer.end_iter())
        };
        assert_eq!(proposer(&mut m, "\n# "), Some(("Chapitre IV".into(), Chapitre)));
        // Le premier `##` fait des `#` des parties : c'est le chapitre I.
        assert_eq!(proposer(&mut m, "\n## "), Some(("Chapitre I".into(), Chapitre)));
        assert_eq!(proposer(&mut m, "\n# "), Some(("Partie V".into(), Partie)));
        // Rien dès qu'un intitulé est commencé, ni sans espace après les dièses.
        assert_eq!(proposer(&mut m, "Ép"), None);
        assert_eq!(proposer(&mut m, "\n##"), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn texte_tape_avant_le_premier_titre() {
        // L'ancre d'« Un » est restée au tout début ; le préambule est neuf.
        let texte = "Avant\n# Un\nA";
        let r = associer(&[(0, None), (6, Some("Un"))], &[(0, Some("Un"))], blanc(texte));
        assert_eq!(r, vec![None, Some(0)]);
    }
}
