//! La fenêtre : le texte au centre, les chapitres à gauche (`Ctrl+1`), les
//! notes à droite (`Ctrl+2`). Les panneaux flottent sur les marges et ne
//! déplacent jamais le texte.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::prelude::*;

use crate::config::{Config, State};
use crate::editor;
use crate::project::{self, roman, Project};
use crate::typography;

/// Délai entre la dernière frappe et l'enregistrement.
const SAVE_DELAY_MS: u64 = 700;

pub struct Init {
    pub project: Project,
    pub config: Config,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    Scene,
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
    Open(usize, usize),
    AddChapter,
    AddScene,
    RenameAsk(usize, Option<usize>),
    Rename(usize, Option<usize>, String),
    Edited(Doc),
    Save(u64),
    SelectionToNotes,
    ToggleTheme,
    ToggleFullscreen,
    Quit,
}

pub struct App {
    project: Project,
    state: State,
    /// Chapitre et scène affichés.
    current: Option<(usize, usize)>,
    /// Chapitre dont les notes sont dans le panneau.
    notes_chapter: Option<String>,
    dirty: [bool; 3],
    save_generation: u64,
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
    chapters: gtk::ListBox,
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
            .title("Scriptorium")
            .default_width(1280)
            .default_height(860)
            .css_classes(["scriptorium"])
            .build()
    }

    fn init(init: Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let Init { project, config, state } = init;
        load_css(&config);

        let loading = Rc::new(Cell::new(false));

        // Le texte.
        let page = editor::build(&config);
        let editor = page.view.clone();

        // Les chapitres, à gauche.
        let chapters = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["chapitres"])
            .build();
        chapters.connect_row_activated(glib::clone!(
            #[strong] sender,
            move |_, row| {
                if let Some((ci, si)) = parse_row(row) {
                    sender.input(Msg::Open(ci, si.unwrap_or(0)));
                }
            }
        ));

        let add_chapter = gtk::Button::builder().label("+ Chapitre").css_classes(["flat"]).build();
        add_chapter.connect_clicked(glib::clone!(#[strong] sender, move |_| sender.input(Msg::AddChapter)));
        let add_scene = gtk::Button::builder().label("+ Scène").css_classes(["flat"]).build();
        add_scene.connect_clicked(glib::clone!(#[strong] sender, move |_| sender.input(Msg::AddScene)));
        let footer = gtk::Box::builder().spacing(4).build();
        footer.append(&add_chapter);
        footer.append(&add_scene);

        let left_box = panel_box("Chapitres", &project.plan.titre);
        left_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&chapters)
                .build(),
        );
        left_box.append(&footer);
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

        if config.typographie {
            typography::attach(&editor);
            typography::attach(&notes_chapter);
            typography::attach(&notes_general);
        }

        for (view, doc) in [
            (&editor, Doc::Scene),
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

        // Le pointeur sur un panneau le ravive.
        for panel in [&left_box, &right_box] {
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(#[strong] sender, move |_, _, _| sender.input(Msg::Fade(false))));
            panel.add_controller(motion);
        }

        let overlay = gtk::Overlay::builder().child(&page.scrolled).build();
        overlay.add_overlay(&left);
        overlay.add_overlay(&right);
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
        let start = state
            .derniere_scene
            .as_deref()
            .filter(|_| state.dernier_livre.as_deref() == Some(project.root.as_path()))
            .and_then(|id| project.find_scene(id))
            .unwrap_or((0, 0));

        let model = App {
            project,
            state,
            current: None,
            notes_chapter: None,
            dirty: [false; 3],
            save_generation: 0,
            loading,
            dark,
        };
        apply_theme(&root, dark);

        let general = project::read(&model.project.general_notes_path()).unwrap_or_default();
        model.loading.set(true);
        notes_general.buffer().set_text(&general);
        model.loading.set(false);

        if config.plein_ecran {
            root.fullscreen();
        }
        root.present();
        sender.input(Msg::Open(start.0, start.1));

        let widgets = Widgets {
            toasts,
            editor,
            notes_chapter,
            notes_general,
            notes_stack,
            notes_title,
            left,
            right,
            chapters,
        };
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
                w.left.set_reveal_child(false);
                w.right.set_reveal_child(false);
                w.editor.grab_focus();
            }
            Msg::Fade(on) => fade(w, on),

            Msg::Open(ci, si) => {
                if !self.save_all(w) {
                    return;
                }
                let Some(scene) = self.project.plan.chapitres.get(ci).and_then(|c| c.scenes.get(si)) else {
                    return;
                };
                let text = match project::read(&self.project.scene_path(&scene.id)) {
                    Ok(t) => t,
                    Err(e) => return toast(w, &format!("Lecture impossible : {e}")),
                };
                self.with_loading(|| w.editor.buffer().set_text(&text));
                let buffer = w.editor.buffer();
                buffer.place_cursor(&buffer.end_iter());
                self.state.derniere_scene = Some(scene.id.clone());
                self.current = Some((ci, si));
                self.load_chapter_notes(w, ci);
                self.rebuild_chapters(w, &sender);
                // Ouvrir depuis le panneau : on retourne au texte.
                if w.left.reveals_child() {
                    w.left.set_reveal_child(false);
                }
                w.editor.grab_focus();
            }

            Msg::AddChapter => {
                let ci = self.project.add_chapter();
                if self.save_plan(w) {
                    sender.input(Msg::Open(ci, 0));
                }
            }
            Msg::AddScene => {
                let ci = self.current.map_or(0, |c| c.0);
                let si = self.project.add_scene(ci);
                if self.save_plan(w) {
                    sender.input(Msg::Open(ci, si));
                }
            }
            Msg::RenameAsk(ci, si) => self.ask_rename(root, &sender, ci, si),
            Msg::Rename(ci, si, title) => {
                let title = title.trim();
                if title.is_empty() {
                    return;
                }
                let ch = &mut self.project.plan.chapitres[ci];
                match si {
                    Some(si) => ch.scenes[si].titre = title.into(),
                    None => ch.titre = title.into(),
                }
                self.save_plan(w);
                self.rebuild_chapters(w, &sender);
                self.update_notes_title(w);
            }

            Msg::Edited(doc) => {
                self.dirty[doc as usize] = true;
                if doc == Doc::Scene {
                    fade(w, true);
                }
                self.save_generation += 1;
                let generation = self.save_generation;
                let sender = sender.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(SAVE_DELAY_MS), move || {
                    sender.input(Msg::Save(generation))
                });
            }
            Msg::Save(generation) => {
                if generation == self.save_generation {
                    self.save_all(w);
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

            Msg::ToggleTheme => {
                self.dark = !self.dark;
                apply_theme(root, self.dark);
                self.state.theme = Some(if self.dark { "encre" } else { "papier" }.into());
                self.state.save();
            }
            Msg::ToggleFullscreen => root.set_fullscreened(!root.is_fullscreen()),

            Msg::Quit => {
                if self.save_all(w) {
                    self.state.dernier_livre = Some(self.project.root.clone());
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

        let [scene, chapter, general] = &mut self.dirty;
        if let Some((ci, si)) = self.current {
            let id = &self.project.plan.chapitres[ci].scenes[si].id;
            write(scene, self.project.scene_path(id), &w.editor);
        }
        if let Some(id) = &self.notes_chapter {
            write(chapter, self.project.chapter_notes_path(id), &w.notes_chapter);
        }
        write(general, self.project.general_notes_path(), &w.notes_general);
        ok
    }

    fn save_plan(&self, w: &Widgets) -> bool {
        match self.project.save_plan() {
            Ok(()) => true,
            Err(e) => {
                toast(w, &format!("Plan non enregistré : {e:#}"));
                false
            }
        }
    }

    fn load_chapter_notes(&mut self, w: &Widgets, ci: usize) {
        let id = self.project.plan.chapitres[ci].id.clone();
        if self.notes_chapter.as_deref() != Some(id.as_str()) {
            let text = project::read(&self.project.chapter_notes_path(&id)).unwrap_or_default();
            self.with_loading(|| w.notes_chapter.buffer().set_text(&text));
            self.notes_chapter = Some(id);
        }
        self.update_notes_title(w);
    }

    fn update_notes_title(&self, w: &Widgets) {
        if let Some((ci, _)) = self.current {
            let ch = &self.project.plan.chapitres[ci];
            w.notes_title.set_label(&format!("{}. {}", roman(ci + 1), ch.titre));
        }
    }

    fn rebuild_chapters(&self, w: &Widgets, sender: &ComponentSender<Self>) {
        while let Some(row) = w.chapters.first_child() {
            w.chapters.remove(&row);
        }
        for (ci, ch) in self.project.plan.chapitres.iter().enumerate() {
            let current_chapter = self.current.map(|c| c.0) == Some(ci);
            w.chapters.append(&chapter_row(
                ci,
                None,
                &format!("{}. {}", roman(ci + 1), ch.titre),
                current_chapter,
                sender,
            ));
            for (si, scene) in ch.scenes.iter().enumerate() {
                w.chapters.append(&chapter_row(
                    ci,
                    Some(si),
                    &format!("{}. {}", si + 1, scene.titre),
                    self.current == Some((ci, si)),
                    sender,
                ));
            }
        }
    }

    fn ask_rename(&self, root: &adw::ApplicationWindow, sender: &ComponentSender<Self>, ci: usize, si: Option<usize>) {
        let ch = &self.project.plan.chapitres[ci];
        let current = si.map_or(&ch.titre, |si| &ch.scenes[si].titre);
        let entry = gtk::Entry::builder().text(current.as_str()).activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading(if si.is_some() { "Renommer la scène" } else { "Renommer le chapitre" })
            .extra_child(&entry)
            .default_response("ok")
            .close_response("annuler")
            .build();
        dialog.add_responses(&[("annuler", "Annuler"), ("ok", "Renommer")]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.connect_response(
            Some("ok"),
            glib::clone!(
                #[strong] sender,
                #[strong] entry,
                move |_, _| sender.input(Msg::Rename(ci, si, entry.text().into()))
            ),
        );
        dialog.present(Some(root));
        entry.grab_focus();
    }
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

fn chapter_row(
    ci: usize,
    si: Option<usize>,
    text: &str,
    current: bool,
    sender: &ComponentSender<App>,
) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let row = gtk::ListBoxRow::builder()
        .child(&label)
        .name(format!("{ci}:{}", si.map_or("-".into(), |s| s.to_string())))
        .css_classes([if si.is_some() { "scene" } else { "chapitre" }])
        .build();
    if current {
        row.add_css_class("courant");
    }

    let double = gtk::GestureClick::new();
    double.connect_pressed(glib::clone!(
        #[strong] sender,
        move |gesture, n, _, _| {
            if n == 2 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                sender.input(Msg::RenameAsk(ci, si));
            }
        }
    ));
    row.add_controller(double);
    row
}

fn parse_row(row: &gtk::ListBoxRow) -> Option<(usize, Option<usize>)> {
    let name = row.widget_name();
    let (ci, si) = name.split_once(':')?;
    Some((ci.parse().ok()?, si.parse().ok()))
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
        // Ctrl+1 / Ctrl+2 en QWERTY comme en AZERTY (& et é).
        let msg = match key {
            Key::_1 | Key::ampersand if ctrl => Msg::ToggleChapters,
            Key::_2 | Key::eacute if ctrl => Msg::ToggleNotes,
            Key::N | Key::n if ctrl && shift => Msg::SelectionToNotes,
            Key::t | Key::T if ctrl => Msg::ToggleTheme,
            Key::q | Key::Q if ctrl => Msg::Quit,
            Key::F11 => Msg::ToggleFullscreen,
            Key::Escape => Msg::CloseAll,
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
window.scriptorium.papier, window.papier .fond {{ background-color: #f5f0e6; color: #2a2521; }}
window.scriptorium.encre,  window.encre .fond  {{ background-color: #1b1a18; color: #d9d2c5; }}

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
list.chapitres row.chapitre {{ font-weight: bold; margin-top: 8px; }}
list.chapitres row.scene {{ padding-left: 22px; opacity: 0.8; }}
list.chapitres row.courant {{ opacity: 1; }}
window.papier list.chapitres row.courant {{ background-color: alpha(#8a5a2b, 0.16); }}
window.encre  list.chapitres row.courant {{ background-color: alpha(#d4a35f, 0.16); }}
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
