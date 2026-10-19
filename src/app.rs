//! La fenêtre : le livre au centre, d'un seul tenant, le plan à gauche
//! (`Alt+A`), les notes à droite (`Alt+E`). Les panneaux flottent sur les
//! marges et ne déplacent jamais le texte. `Alt+↑` et `Alt+↓` vont d'un titre
//! à l'autre. `Ctrl+Maj+S` pose un jalon dans l'historique du livre.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::prelude::*;

use crate::config::{Config, State};
use crate::editor;
use crate::historique::Historique;
use crate::manuscrit::{Manuscrit, Rang};
use crate::project::{self, roman, Project};
use crate::typography;

/// Délai entre la dernière frappe et l'enregistrement.
const SAVE_DELAY_MS: u64 = 700;
/// Un instantané est pris après cette pause dans l'écriture…
const PAUSE_PHOTO: Duration = Duration::from_secs(2 * 60);
/// … ou, si l'on écrit sans s'arrêter, au plus tard après ce temps.
const ECART_PHOTO: Duration = Duration::from_secs(15 * 60);

pub struct Init {
    pub project: Project,
    /// Le texte de chaque chapitre, dans l'ordre du plan.
    pub chapitres: Vec<(String, String)>,
    pub config: Config,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    Texte,
    ChapterNotes,
    GeneralNotes,
}

#[derive(Debug)]
pub enum Msg {
    ToggleChapters,
    ToggleNotes,
    CloseAll,
    /// Estompe (`true`) ou ravive les panneaux ouverts.
    Fade(bool),
    /// Va au titre d'une partie ou d'un chapitre.
    Aller(usize),
    /// Va au titre précédent (`-1`) ou suivant (`1`).
    Voisin(i32),
    /// Relit les titres du texte.
    Analyse,
    /// Le curseur a bougé : il est peut-être passé dans un autre chapitre.
    Curseur,
    Edited(Doc),
    Save(u64),
    /// La pause qui suivait cet enregistrement a duré : on prend un instantané.
    Photo(u64),
    /// Demande le nom d'un jalon.
    DemanderJalon,
    Jalon(String),
    SelectionToNotes,
    ToggleTheme,
    /// Ouvre les réglages.
    Options,
    /// Thème sombre (encre) ou clair (papier).
    Sombre(bool),
    /// Texte justifié, ou aligné à gauche.
    Justifier(bool),
    ToggleFullscreen,
    Quit,
}

pub struct App {
    project: Project,
    manuscrit: Manuscrit,
    state: State,
    /// Chapitre dont les notes sont dans le panneau.
    notes_chapter: Option<String>,
    dirty: [bool; 3],
    save_generation: u64,
    /// Absent si le dossier de l'historique n'a pas pu être ouvert.
    historique: Option<Historique>,
    derniere_photo: Instant,
    /// Une analyse est déjà prévue : les frappes rapprochées n'en lancent qu'une.
    analyse_prevue: bool,
    /// Le titre proposé après « # » (en gris derrière le curseur) ; `Tab` le prend.
    suggestion: Rc<RefCell<Option<String>>>,
    jeton_fantome: Rc<Cell<u32>>,
    /// Vrai pendant qu'on charge un texte : ce n'est pas une modification.
    loading: Rc<Cell<bool>>,
    dark: bool,
}

pub struct Widgets {
    toasts: adw::ToastOverlay,
    editor: gtk::TextView,
    notes_chapter: gtk::TextView,
    notes_general: gtk::TextView,
    notes_stack: adw::ViewStack,
    notes_title: gtk::Label,
    left: gtk::Revealer,
    right: gtk::Revealer,
    /// Les réglages, au centre (`Alt+S`).
    reglages: gtk::Revealer,
    sombre: gtk::Switch,
    chapters: gtk::ListBox,
    fantome: gtk::Label,
}

impl Component for App {
    type CommandOutput = ();
    type Input = Msg;
    type Output = ();
    type Init = Init;
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title("Ajar")
            .default_width(1280)
            .default_height(860)
            .css_classes(["ajar"])
            .build()
    }

    fn init(init: Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let Init { project, chapitres, config, state } = init;
        load_css(&config);

        let loading = Rc::new(Cell::new(false));

        // Le texte.
        let page = editor::build(&config);
        let editor = page.view.clone();
        justifier(&editor, state.justifie);

        // La proposition de titre : un texte posé sur la page, hors du tampon.
        let suggestion: Rc<RefCell<Option<String>>> = Rc::default();
        let fantome = gtk::Label::builder().css_classes(["fantome"]).can_target(false).visible(false).build();
        editor.add_overlay(&fantome, 0, 0);
        let tab = gtk::EventControllerKey::new();
        tab.set_propagation_phase(gtk::PropagationPhase::Capture);
        tab.connect_key_pressed(glib::clone!(
            #[strong] suggestion,
            #[weak] editor,
            #[upgrade_or] glib::Propagation::Proceed,
            move |_, key, _, state| {
                if key != gdk::Key::Tab || !state.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let Some(titre) = suggestion.borrow_mut().take() else { return glib::Propagation::Proceed };
                editor.buffer().insert_at_cursor(&titre);
                glib::Propagation::Stop
            }
        ));
        editor.add_controller(tab);

        // Le plan, à gauche.
        let chapters = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["chapitres"])
            .build();
        chapters.connect_row_activated(glib::clone!(
            #[strong] sender,
            move |_, row| {
                if let Some(ci) = parse_row(row) {
                    sender.input(Msg::Aller(ci));
                }
            }
        ));

        let left_box = panel_box("Chapitres", &project.plan.titre);
        left_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&chapters)
                .build(),
        );
        let left = side_revealer(&left_box, gtk::Align::Start, gtk::RevealerTransitionType::SlideRight);

        // Les notes, à droite.
        let notes_chapter = notes_view();
        let notes_general = notes_view();
        let notes_title = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();
        let chapter_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        chapter_page.append(&notes_title);
        chapter_page.append(&scrolled(&notes_chapter));

        let notes_stack = adw::ViewStack::new();
        notes_stack.add_titled(&chapter_page, Some("chapitre"), "Chapitre");
        notes_stack.add_titled(&scrolled(&notes_general), Some("general"), "Général");
        let switcher = adw::InlineViewSwitcher::builder().stack(&notes_stack).build();

        let right_box = panel_box("Notes", "");
        right_box.append(&switcher);
        right_box.append(&notes_stack);
        let right = side_revealer(&right_box, gtk::Align::End, gtk::RevealerTransitionType::SlideLeft);

        // Les réglages, au centre, du même papier que les panneaux.
        let sombre = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let justifie = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let reglages_box = panel_box("Réglages", "S’appliquent aussitôt, et restent d’une séance à l’autre.");
        reglages_box.set_width_request(380);
        reglages_box.append(&reglage("Thème sombre", "L’encre plutôt que le papier · Ctrl+T", &sombre));
        reglages_box.append(&reglage("Texte justifié", "Aligné sur les deux bords, comme un livre imprimé", &justifie));
        let reglages = gtk::Revealer::builder()
            .child(&reglages_box)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(220)
            .build();

        if config.typographie {
            typography::attach(&editor);
            typography::attach(&notes_chapter);
            typography::attach(&notes_general);
        }

        for (view, doc) in [
            (&editor, Doc::Texte),
            (&notes_chapter, Doc::ChapterNotes),
            (&notes_general, Doc::GeneralNotes),
        ] {
            view.buffer().connect_changed(glib::clone!(
                #[strong] sender,
                #[strong] loading,
                move |_| {
                    if !loading.get() {
                        sender.input(Msg::Edited(doc));
                    }
                }
            ));
        }
        editor.buffer().connect_mark_set(glib::clone!(
            #[strong] sender,
            move |_, _, mark| {
                if mark.name().as_deref() == Some("insert") {
                    sender.input(Msg::Curseur);
                }
            }
        ));

        // Le pointeur sur un panneau le ravive.
        for panel in [&left_box, &right_box] {
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(#[strong] sender, move |_, _, _| sender.input(Msg::Fade(false))));
            panel.add_controller(motion);
        }

        let overlay = gtk::Overlay::builder().child(&page.scrolled).build();
        overlay.add_overlay(&left);
        overlay.add_overlay(&right);
        overlay.add_overlay(&reglages);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&overlay));
        toasts.add_css_class("fond");
        root.set_content(Some(&toasts));

        install_shortcuts(&root, &sender);
        root.connect_close_request(glib::clone!(
            #[strong] sender,
            move |_| {
                sender.input(Msg::Quit);
                glib::Propagation::Stop
            }
        ));

        let theme = state.theme.clone().unwrap_or_else(|| config.theme.clone());
        let dark = match theme.as_str() {
            "encre" => true,
            "papier" => false,
            _ => adw::StyleManager::default().is_dark(),
        };
        let curseur = state
            .curseur
            .filter(|_| state.dernier_livre.as_deref() == Some(project.root.as_path()));

        let buffer = editor.buffer();
        let mut manuscrit = Manuscrit::new(&buffer, config.taille);
        loading.set(true);
        manuscrit.charger(chapitres, &project);
        loading.set(false);
        // Sans position retenue, on reprend à la fin du livre.
        buffer.place_cursor(&curseur.map_or(buffer.end_iter(), |c| buffer.iter_at_offset(c)));

        let (historique, echec_historique) = match Historique::ouvrir(&project.root) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(format!("Historique indisponible : {e:#}"))),
        };
        let mut model = App {
            project,
            manuscrit,
            state,
            notes_chapter: None,
            dirty: [false; 3],
            save_generation: 0,
            historique,
            derniere_photo: Instant::now(),
            analyse_prevue: false,
            suggestion,
            jeton_fantome: Rc::default(),
            loading,
            dark,
        };
        apply_theme(&root, dark);
        // Les interrupteurs partent de l'état actuel, puis seulement on les écoute.
        sombre.set_active(dark);
        justifie.set_active(model.state.justifie);
        sombre.connect_active_notify(glib::clone!(#[strong] sender, move |s| sender.input(Msg::Sombre(s.is_active()))));
        justifie.connect_active_notify(glib::clone!(#[strong] sender, move |s| sender.input(Msg::Justifier(s.is_active()))));

        let general = project::read(&model.project.general_notes_path()).unwrap_or_default();
        model.loading.set(true);
        notes_general.buffer().set_text(&general);
        model.loading.set(false);

        if config.plein_ecran {
            root.fullscreen();
        }
        root.present();
        editor.grab_focus();
        sender.input(Msg::Analyse);

        let widgets = Widgets {
            toasts,
            editor,
            notes_chapter,
            notes_general,
            notes_stack,
            notes_title,
            left,
            right,
            reglages,
            sombre,
            chapters,
            fantome,
        };
        if let Some(e) = echec_historique {
            toast(&widgets, &e);
        }
        // Ce que le livre est à l'ouverture, modifié ou non hors de l'application.
        model.photographier(&widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        w: &mut Self::Widgets,
        msg: Msg,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            Msg::ToggleChapters => {
                let open = !w.left.reveals_child();
                w.left.set_reveal_child(open);
                fade(w, false);
                if open {
                    w.chapters.grab_focus();
                } else {
                    w.editor.grab_focus();
                }
            }
            Msg::ToggleNotes => {
                let open = !w.right.reveals_child();
                w.right.set_reveal_child(open);
                fade(w, false);
                if open {
                    let view = if w.notes_stack.visible_child_name().as_deref() == Some("general") {
                        &w.notes_general
                    } else {
                        &w.notes_chapter
                    };
                    view.grab_focus();
                } else {
                    w.editor.grab_focus();
                }
            }
            Msg::CloseAll => {
                w.reglages.set_reveal_child(false);
                w.left.set_reveal_child(false);
                w.right.set_reveal_child(false);
                w.editor.grab_focus();
            }
            Msg::Fade(on) => fade(w, on),

            Msg::Aller(ci) => {
                self.analyser(w);
                let Some(it) = self.manuscrit.position_titre(ci) else { return };
                w.editor.buffer().place_cursor(&it);
                // Depuis le panneau : on retourne au texte.
                w.left.set_reveal_child(false);
                w.editor.grab_focus();
            }
            Msg::Voisin(sens) => {
                self.analyser(w);
                let buffer = w.editor.buffer();
                let pos = buffer.cursor_position();
                let ci = self.manuscrit.chapitre_a(pos);
                // Vers le haut, depuis le texte d'une section : d'abord son propre titre.
                let dans_le_texte = self.manuscrit.position_titre(ci).is_some_and(|t| pos > t.offset());
                let cible = match sens {
                    -1 if dans_le_texte => Some(ci),
                    -1 => ci.checked_sub(1),
                    _ => Some(ci + 1),
                };
                if let Some(it) = cible.and_then(|c| self.manuscrit.position_titre(c)) {
                    buffer.place_cursor(&it);
                    w.editor.grab_focus();
                }
            }
            Msg::Analyse => {
                self.analyse_prevue = false;
                self.analyser(w);
            }
            Msg::Curseur => {
                self.suivre_curseur(w, false);
                self.proposer(w);
            }

            Msg::Edited(doc) => {
                self.dirty[doc as usize] = true;
                if doc == Doc::Texte {
                    fade(w, true);
                    // Juste après la frappe : un titre tapé apparaît aussitôt dans le plan.
                    if !self.analyse_prevue {
                        self.analyse_prevue = true;
                        let sender = sender.clone();
                        glib::idle_add_local_once(move || sender.input(Msg::Analyse));
                    }
                }
                self.save_generation += 1;
                let generation = self.save_generation;
                let sender = sender.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(SAVE_DELAY_MS), move || {
                    sender.input(Msg::Save(generation))
                });
            }
            Msg::Save(generation) => {
                if generation != self.save_generation || !self.save_all(w) {
                    return;
                }
                if self.derniere_photo.elapsed() >= ECART_PHOTO {
                    self.photographier(w);
                } else {
                    let sender = sender.clone();
                    glib::timeout_add_local_once(PAUSE_PHOTO, move || sender.input(Msg::Photo(generation)));
                }
            }
            Msg::Photo(generation) => {
                // Une frappe depuis : la pause n'a pas duré.
                if generation == self.save_generation {
                    self.photographier(w);
                }
            }
            Msg::DemanderJalon => demander_jalon(root, &sender),
            Msg::Jalon(nom) => {
                if !self.save_all(w) {
                    return;
                }
                let Some(h) = &self.historique else {
                    toast(w, "Historique indisponible : le jalon n’a pas été posé");
                    return;
                };
                match h.jalon(&nom) {
                    Ok(i) => {
                        self.derniere_photo = Instant::now();
                        toast(w, &format!("Jalon « {} » posé · {} mots", nom.trim(), i.mots));
                    }
                    Err(e) => toast(w, &format!("Échec du jalon : {e:#}")),
                }
            }

            Msg::SelectionToNotes => {
                let buffer = w.editor.buffer();
                let Some((start, end)) = buffer.selection_bounds() else { return };
                let extrait = buffer.text(&start, &end, false);
                let notes = w.notes_chapter.buffer();
                let mut fin = notes.end_iter();
                let sep = if notes.char_count() == 0 { "" } else { "\n\n" };
                notes.insert(&mut fin, &format!("{sep}> {}\n", extrait.replace('\n', "\n> ")));
                w.notes_stack.set_visible_child_name("chapitre");
                w.right.set_reveal_child(true);
                fade(w, false);
                toast(w, "Passage mis dans les notes du chapitre");
            }

            Msg::ToggleTheme => sender.input(Msg::Sombre(!self.dark)),
            Msg::Sombre(dark) => {
                // Ctrl+T aussi : l'interrupteur suit (sans boucle, la valeur est déjà la bonne).
                w.sombre.set_active(dark);
                if dark != self.dark {
                    self.dark = dark;
                    apply_theme(root, dark);
                    self.state.theme = Some(if dark { "encre" } else { "papier" }.into());
                    self.state.save();
                }
            }
            Msg::Justifier(on) => {
                justifier(&w.editor, on);
                self.state.justifie = on;
                self.state.save();
            }
            Msg::Options => {
                let open = !w.reglages.reveals_child();
                w.reglages.set_reveal_child(open);
                if open {
                    w.sombre.grab_focus();
                } else {
                    w.editor.grab_focus();
                }
            }
            Msg::ToggleFullscreen => root.set_fullscreened(!root.is_fullscreen()),

            Msg::Quit => {
                if self.save_all(w) {
                    self.photographier(w);
                    self.state.dernier_livre = Some(self.project.root.clone());
                    self.state.curseur = Some(w.editor.buffer().cursor_position());
                    self.state.save();
                    relm4::main_application().quit();
                } else {
                    toast(w, "Fermeture annulée : le texte n’a pas pu être enregistré");
                }
            }
        }
    }
}

impl App {
    fn with_loading(&self, f: impl FnOnce()) {
        self.loading.set(true);
        f();
        self.loading.set(false);
    }

    /// Enregistre ce qui a changé. Renvoie `false` si un enregistrement a échoué.
    fn save_all(&mut self, w: &Widgets) -> bool {
        let mut ok = true;
        let [texte, chapter, general] = &mut self.dirty;
        if *texte {
            match self.manuscrit.enregistrer(&mut self.project) {
                Ok(()) => *texte = false,
                Err(e) => {
                    ok = false;
                    toast(w, &format!("Échec de l’enregistrement : {e:#}"));
                }
            }
        }

        let mut write = |dirty: &mut bool, path: std::path::PathBuf, view: &gtk::TextView| {
            if !*dirty {
                return;
            }
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            match project::write_atomic(&path, &text) {
                Ok(()) => *dirty = false,
                Err(e) => {
                    ok = false;
                    toast(w, &format!("Échec de l’enregistrement : {e:#}"));
                }
            }
        };
        if let Some(id) = &self.notes_chapter {
            write(chapter, self.project.chapter_notes_path(id), &w.notes_chapter);
        }
        write(general, self.project.general_notes_path(), &w.notes_general);
        ok
    }

    /// Prend un instantané du livre enregistré, s'il a changé. Un échec se
    /// signale sans rien arrêter : le texte, lui, est sur le disque.
    fn photographier(&mut self, w: &Widgets) {
        let Some(h) = &self.historique else { return };
        match h.photographier() {
            Ok(_) => self.derniere_photo = Instant::now(),
            Err(e) => toast(w, &format!("Échec de l’instantané : {e:#}")),
        }
    }

    /// Relit les titres du texte et met le plan à jour s'il a changé.
    fn analyser(&mut self, w: &Widgets) {
        let change = self.manuscrit.analyser(&self.project);
        self.suivre_curseur(w, change || w.chapters.first_child().is_none());
        self.proposer(w);
    }

    /// Montre, ou retire, le titre proposé derrière le curseur.
    fn proposer(&self, w: &Widgets) {
        let buffer = w.editor.buffer();
        let curseur = buffer.iter_at_mark(&buffer.get_insert());
        let proposition = self.manuscrit.proposition(&curseur);
        let Some((titre, rang)) = proposition else {
            self.suggestion.replace(None);
            w.fantome.set_visible(false);
            return;
        };
        // Déjà proposé : le suivi en cours le garde en place.
        if self.suggestion.borrow().as_deref() == Some(titre.as_str()) {
            return;
        }
        self.suggestion.replace(Some(titre.clone()));
        placer_fantome(&w.editor, &w.fantome, &titre, rang, &self.suggestion, &self.jeton_fantome);
    }

    /// Met dans le panneau les notes du chapitre où est le curseur, et le
    /// signale dans le plan.
    fn suivre_curseur(&mut self, w: &Widgets, plan_change: bool) {
        let buffer = w.editor.buffer();
        let ci = self.manuscrit.chapitre_a(buffer.cursor_position());
        let Some(id) = self.manuscrit.chapitres.get(ci).map(|c| c.id.clone()) else { return };
        let nouveau = self.notes_chapter.as_deref() != Some(id.as_str());
        if nouveau {
            // Les notes du chapitre qu'on quitte partent d'abord sur le disque.
            if self.dirty[Doc::ChapterNotes as usize] && !self.save_all(w) {
                return;
            }
            let text = project::read(&self.project.chapter_notes_path(&id)).unwrap_or_default();
            self.with_loading(|| w.notes_chapter.buffer().set_text(&text));
            self.notes_chapter = Some(id);
        }
        if nouveau || plan_change {
            w.notes_title.set_label(&self.libelle(ci));
            self.rebuild_chapters(w, ci);
        }
    }

    /// « III. Titre » pour un chapitre (numérotés d'un bout à l'autre, et de
    /// nouveau depuis I à chaque livre), le seul intitulé pour un livre ou une
    /// partie.
    fn libelle(&self, ci: usize) -> String {
        let chapitres = &self.manuscrit.chapitres;
        let ch = &chapitres[ci];
        if ch.rang != Rang::Chapitre {
            return ch.intitule().to_string();
        }
        format!("{}. {}", roman(self.manuscrit.numero(ci)), ch.intitule())
    }

    fn rebuild_chapters(&self, w: &Widgets, courant: usize) {
        while let Some(row) = w.chapters.first_child() {
            w.chapters.remove(&row);
        }
        let chapitres = &self.manuscrit.chapitres;
        let existe = |r: Rang| chapitres.iter().any(|c| c.rang == r);
        let (livres, parties) = (existe(Rang::Livre), existe(Rang::Partie));
        for (ci, ch) in chapitres.iter().enumerate() {
            // Un cran de retrait par rang présent au-dessus.
            let (classe, retrait) = match ch.rang {
                Rang::Livre => ("livre", 0),
                Rang::Partie => ("partie", livres as u8),
                Rang::Chapitre => ("chapitre", livres as u8 + parties as u8),
            };
            w.chapters.append(&chapter_row(ci, &self.libelle(ci), classe, retrait, ci == courant));
        }
    }
}

/// Pose le titre proposé juste derrière le curseur, à la taille et à la graisse
/// du titre qu'il complète, et l'y garde tant qu'il est proposé : la ligne est
/// centrée, et toute remise en page (frappe, fenêtre redimensionnée) la déplace.
///
/// `jeton` distingue les appels : un placement plus récent remplace celui-ci.
fn placer_fantome(
    view: &gtk::TextView,
    fantome: &gtk::Label,
    titre: &str,
    rang: Rang,
    suggestion: &Rc<RefCell<Option<String>>>,
    jeton: &Rc<Cell<u32>>,
) {
    fantome.set_visible(false);
    let attrs = gtk::pango::AttrList::new();
    attrs.insert(gtk::pango::AttrFloat::new_scale(rang.echelle()));
    attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Semibold));
    fantome.set_attributes(Some(&attrs));
    fantome.set_label(titre);

    jeton.set(jeton.get().wrapping_add(1));
    let mien = jeton.get();
    let (titre, fantome, suggestion, jeton) = (titre.to_string(), fantome.clone(), suggestion.clone(), jeton.clone());
    let tops = Cell::new(0);
    let place = Cell::new(None);
    view.add_tick_callback(move |view, _| {
        // Remplacé par un placement plus récent, ou plus rien à proposer.
        if jeton.get() != mien || suggestion.borrow().as_deref() != Some(titre.as_str()) {
            return glib::ControlFlow::Break;
        }
        // Au premier top, la ligne n'a pas encore été remise en page depuis la
        // frappe : sa position serait fausse.
        tops.set(tops.get() + 1);
        if tops.get() < 2 {
            return glib::ControlFlow::Continue;
        }
        let buffer = view.buffer();
        let rect = view.iter_location(&buffer.iter_at_mark(&buffer.get_insert()));
        // Le haut du rectangle est le haut des lettres (sa hauteur compte l'espace
        // sous la ligne) ; le texte gris s'y aligne, sans interligne.
        // Ne bouger que s'il le faut : chaque déplacement relance une mise en page.
        let ici = Some((rect.x(), rect.y()));
        if place.replace(ici) != ici {
            view.move_overlay(&fantome, rect.x(), rect.y());
            fantome.set_visible(true);
        }
        glib::ControlFlow::Continue
    });
}

/// Demande le nom d'un jalon ; `Entrée` le pose.
fn demander_jalon(root: &adw::ApplicationWindow, sender: &ComponentSender<App>) {
    let dialog = adw::AlertDialog::new(
        Some("Marquer un jalon"),
        Some("Un nom pour retrouver cet état du livre"),
    );
    let nom = gtk::Entry::builder().placeholder_text("Premier jet").activates_default(true).build();
    dialog.set_extra_child(Some(&nom));
    dialog.add_responses(&[("annuler", "Annuler"), ("poser", "Poser le jalon")]);
    dialog.set_response_appearance("poser", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("poser"));
    dialog.set_close_response("annuler");
    dialog.set_response_enabled("poser", false);
    nom.connect_changed(glib::clone!(
        #[weak] dialog,
        move |nom| dialog.set_response_enabled("poser", !nom.text().trim().is_empty())
    ));
    dialog.connect_response(None, glib::clone!(
        #[strong] sender,
        #[weak] nom,
        move |_, reponse| {
            if reponse == "poser" {
                sender.input(Msg::Jalon(nom.text().to_string()));
            }
        }
    ));
    dialog.present(Some(root));
    nom.grab_focus();
}

/// Le texte justifié sur les deux bords, comme un livre imprimé, ou aligné à
/// gauche. Les titres gardent leur centrage, porté par leur style.
fn justifier(view: &gtk::TextView, on: bool) {
    view.set_justification(if on { gtk::Justification::Fill } else { gtk::Justification::Left });
}

fn fade(w: &Widgets, on: bool) {
    for panel in [&w.left, &w.right] {
        if on && panel.reveals_child() {
            panel.add_css_class("estompe");
        } else {
            panel.remove_css_class("estompe");
        }
    }
}

fn toast(w: &Widgets, text: &str) {
    w.toasts.add_toast(adw::Toast::new(text));
}

fn apply_theme(root: &adw::ApplicationWindow, dark: bool) {
    adw::StyleManager::default().set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    root.remove_css_class(if dark { "papier" } else { "encre" });
    root.add_css_class(if dark { "encre" } else { "papier" });
}

fn chapter_row(ci: usize, text: &str, classe: &str, retrait: u8, current: bool) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let row = gtk::ListBoxRow::builder()
        .child(&label)
        .name(ci.to_string())
        .css_classes([classe])
        .build();
    if retrait > 0 {
        row.add_css_class(&format!("retrait-{retrait}"));
    }
    if current {
        row.add_css_class("courant");
    }
    row
}

fn parse_row(row: &gtk::ListBoxRow) -> Option<usize> {
    row.widget_name().parse().ok()
}

/// Une ligne de réglage : intitulé et explication à gauche, interrupteur à droite.
fn reglage(titre: &str, detail: &str, interrupteur: &gtk::Switch) -> gtk::Box {
    let textes = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    textes.append(&gtk::Label::builder().label(titre).xalign(0.0).build());
    textes.append(
        &gtk::Label::builder()
            .label(detail)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    let ligne = gtk::Box::builder().spacing(18).css_classes(["reglage"]).build();
    ligne.append(&textes);
    ligne.append(interrupteur);
    ligne
}

fn panel_box(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .width_request(300)
        .css_classes(["panneau"])
        .build();
    b.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
    if !subtitle.is_empty() {
        b.append(
            &gtk::Label::builder()
                .label(subtitle)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
    }
    b
}

fn side_revealer(child: &gtk::Box, halign: gtk::Align, transition: gtk::RevealerTransitionType) -> gtk::Revealer {
    gtk::Revealer::builder()
        .child(child)
        .halign(halign)
        .valign(gtk::Align::Fill)
        .transition_type(transition)
        .transition_duration(220)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build()
}

fn notes_view() -> gtk::TextView {
    let view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .pixels_below_lines(6)
        .css_classes(["notes"])
        .build();
    view.buffer().set_enable_undo(true);
    view
}

fn scrolled(view: &gtk::TextView) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(view)
        .build()
}

fn install_shortcuts(root: &adw::ApplicationWindow, sender: &ComponentSender<App>) {
    use gdk::{Key, ModifierType as M};
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sender = sender.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(M::CONTROL_MASK);
        let shift = state.contains(M::SHIFT_MASK);
        let alt = state.contains(M::ALT_MASK);
        let msg = match key {
            Key::a | Key::A if alt => Msg::ToggleChapters,
            Key::e | Key::E if alt => Msg::ToggleNotes,
            Key::Up if alt => Msg::Voisin(-1),
            Key::Down if alt => Msg::Voisin(1),
            Key::N | Key::n if ctrl && shift => Msg::SelectionToNotes,
            Key::S | Key::s if ctrl && shift => Msg::DemanderJalon,
            Key::t | Key::T if ctrl => Msg::ToggleTheme,
            Key::q | Key::Q if ctrl => Msg::Quit,
            Key::F11 => Msg::ToggleFullscreen,
            Key::s | Key::S if alt => Msg::Options,
            // Échap ferme les panneaux, mais continue sa route : une boîte de
            // dialogue ouverte doit pouvoir se fermer aussi.
            Key::Escape => {
                sender.input(Msg::CloseAll);
                return glib::Propagation::Proceed;
            }
            _ => return glib::Propagation::Proceed,
        };
        sender.input(msg);
        glib::Propagation::Stop
    });
    root.add_controller(keys);
}

fn load_css(config: &Config) {
    let css = format!(
        r#"
window.ajar.papier, window.papier .fond {{ background-color: #f5f0e6; color: #2a2521; }}
window.ajar.encre,  window.encre .fond  {{ background-color: #1b1a18; color: #d9d2c5; }}

textview.manuscrit, textview.manuscrit text,
textview.notes, textview.notes text {{ background-color: transparent; }}
textview.manuscrit {{
    font-family: "{police}", "EB Garamond", serif;
    font-size: {taille}px;
    line-height: 1.55;
}}
window.papier textview.manuscrit text {{ color: #2a2521; caret-color: #8a5a2b; }}
window.encre  textview.manuscrit text {{ color: #d9d2c5; caret-color: #d4a35f; }}
window.papier textview text selection {{ background-color: alpha(#8a5a2b, 0.22); }}
window.encre  textview text selection {{ background-color: alpha(#d4a35f, 0.25); }}

textview.notes {{ font-family: "{police}", serif; font-size: {notes}px; }}

.panneau {{
    padding: 18px;
    border-radius: 14px;
    transition: opacity 400ms ease;
}}
window.papier .panneau {{ background-color: alpha(#ebe4d6, 0.97); box-shadow: 0 2px 18px alpha(black, 0.10); }}
window.encre  .panneau {{ background-color: alpha(#262421, 0.97); box-shadow: 0 2px 18px alpha(black, 0.45); }}
revealer.estompe {{ opacity: 0.25; transition: opacity 900ms ease; }}

list.chapitres, list.chapitres row {{ background: transparent; }}
list.chapitres row {{ border-radius: 8px; padding: 4px 8px; }}
list.chapitres row.livre {{ font-weight: bold; font-size: 1.1em; margin-top: 18px; }}
list.chapitres row.partie {{ font-weight: bold; font-variant: small-caps; letter-spacing: 0.04em; margin-top: 12px; }}
list.chapitres row.chapitre {{ margin-top: 2px; }}
list.chapitres row.retrait-1 {{ padding-left: 22px; }}
list.chapitres row.retrait-2 {{ padding-left: 40px; }}
list.chapitres row.courant {{ opacity: 1; }}
window.papier list.chapitres row.courant {{ background-color: alpha(#8a5a2b, 0.16); }}
window.encre  list.chapitres row.courant {{ background-color: alpha(#d4a35f, 0.16); }}
textview.manuscrit label.fantome {{ line-height: 1; }}
window.papier label.fantome {{ color: alpha(#2a2521, 0.32); }}
window.encre  label.fantome {{ color: alpha(#d9d2c5, 0.32); }}

.panneau .reglage {{ padding: 10px 4px; }}
window.papier .panneau .reglage + .reglage {{ border-top: 1px solid alpha(#2a2521, 0.08); }}
window.encre  .panneau .reglage + .reglage {{ border-top: 1px solid alpha(#d9d2c5, 0.08); }}
window.papier .panneau switch:checked {{ background-color: #8a5a2b; }}
window.encre  .panneau switch:checked {{ background-color: #d4a35f; }}
window.encre  .panneau switch:checked slider {{ background-color: #1b1a18; }}
"#,
        police = config.police,
        taille = config.taille,
        notes = (config.taille * 0.8).round(),
    );
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("aucun écran"),
        &provider,
        // Au-dessus du thème de l'utilisateur : l'app impose son papier et son encre.
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Laisse GTK travailler (mise en page, dessin) pendant `ms` millisecondes.
    fn attendre(ms: u64) {
        let fin = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < fin {
            glib::MainContext::default().iteration(false);
        }
    }

    /// Capture de la proposition de titre, à vérifier à l'œil. Ouvre une
    /// fenêtre une seconde : `cargo test capture -- --ignored`, puis regarder
    /// `target/capture-proposition.png`.
    #[test]
    #[ignore]
    fn capture_proposition() {
        adw::init().unwrap();
        let config = Config { plein_ecran: false, ..Config::default() };
        load_css(&config);
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(700)
            .css_classes(["ajar", "papier"])
            .build();
        let page = editor::build(&config);
        window.set_child(Some(&page.scrolled));
        // La fenêtre prend le focus : ce qu'on tape ailleurs ne doit pas y entrer.
        page.view.set_editable(false);
        page.view.set_can_focus(false);
        let dir = std::env::temp_dir().join(format!("ajar-capture-{}", std::process::id()));
        let project = Project::open_or_init(&dir).unwrap();
        let buffer = page.view.buffer();
        let mut m = Manuscrit::new(&buffer, config.taille);
        m.charger(
            vec![("01".into(), "# Le départ\n\nIl partit avant le jour, sans un regard pour la maison.\n\n".into())],
            &project,
        );
        let fantome = gtk::Label::builder().css_classes(["fantome"]).can_target(false).visible(false).build();
        page.view.add_overlay(&fantome, 0, 0);
        window.present();
        attendre(400);

        // Comme dans l'application : « # » tapé, analyse, proposition aussitôt.
        buffer.insert(&mut buffer.end_iter(), "# ");
        m.analyser(&project);
        let curseur = buffer.end_iter();
        buffer.place_cursor(&curseur);
        let (titre, rang) = m.proposition(&curseur).expect("aucune proposition");
        let suggestion = Rc::new(RefCell::new(Some(titre.clone())));
        placer_fantome(&page.view, &fantome, &titre, rang, &suggestion, &Rc::default());
        attendre(600);

        let (l, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, l as f64, h as f64);
        let node = snapshot.to_node().unwrap();
        let texture = window.native().unwrap().renderer().unwrap().render_texture(node, None);
        let sortie = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/capture-proposition.png");
        texture.save_to_png(&sortie).unwrap();
        eprintln!("capture : {}", sortie.display());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
