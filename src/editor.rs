//! La page centrale : une colonne de texte de largeur fixe, et le défilement
//! « machine à écrire » qui garde la ligne en cours au milieu de l'écran.

use adw::prelude::*;
use gtk::{glib, graphene};

use crate::config::Config;

pub struct Page {
    pub scrolled: gtk::ScrolledWindow,
    pub view: gtk::TextView,
}

pub fn build(config: &Config) -> Page {
    let view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .pixels_below_lines((config.taille * 0.7) as i32)
        .pixels_inside_wrap((config.taille * 0.45) as i32)
        .css_classes(["manuscrit"])
        .build();
    view.buffer().set_enable_undo(true);

    // Une colonne d'environ `largeur` caractères, centrée.
    let clamp = adw::Clamp::builder()
        .maximum_size((config.largeur as f64 * config.taille * 0.55) as i32)
        .tightening_threshold(0)
        .child(&view)
        .build();

    // Viewport explicite : sans `scroll_to_focus`, la page ne saute pas en haut
    // du texte quand il reprend le focus.
    let viewport = gtk::Viewport::builder().scroll_to_focus(false).child(&clamp).build();

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&viewport)
        .build();

    typewriter(&view, &scrolled);
    Page { scrolled, view }
}

fn typewriter(view: &gtk::TextView, scrolled: &gtk::ScrolledWindow) {
    let adj = scrolled.vadjustment();

    // Une demi-page de marge en haut et en bas : la première comme la dernière
    // ligne peuvent venir au centre.
    adj.connect_page_size_notify(glib::clone!(
        #[weak] view,
        move |adj| {
            let half = (adj.page_size() / 2.0) as i32;
            if view.top_margin() != half {
                view.set_top_margin(half);
                view.set_bottom_margin(half);
            }
        }
    ));

    // Une seule animation, relancée à chaque déplacement du curseur.
    let animation = adw::TimedAnimation::new(
        view,
        0.0,
        0.0,
        140,
        adw::PropertyAnimationTarget::new(&adj, "value"),
    );
    animation.set_easing(adw::Easing::EaseOutCubic);

    let center = glib::clone!(
        #[weak] view,
        #[weak] scrolled,
        #[strong] animation,
        move || {
            // Après le prochain calcul de mise en page.
            glib::idle_add_local_once(glib::clone!(
                #[strong] animation,
                move || center_cursor(&view, &scrolled, &animation)
            ));
        }
    );

    let buffer = view.buffer();
    buffer.connect_changed({
        let center = center.clone();
        move |_| center()
    });
    buffer.connect_mark_set(move |buffer, _, mark| {
        // Pas pendant une sélection à la souris : la page bougerait sous le pointeur.
        if mark.name().as_deref() == Some("insert") && !buffer.has_selection() {
            center();
        }
    });
}

fn center_cursor(view: &gtk::TextView, scrolled: &gtk::ScrolledWindow, animation: &adw::TimedAnimation) {
    let Some(content) = scrolled.child() else { return };
    let buffer = view.buffer();
    let rect = view.iter_location(&buffer.iter_at_mark(&buffer.get_insert()));
    let (_, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, 0, rect.y());
    let Some(point) = view.compute_point(&content, &graphene::Point::new(0.0, y as f32)) else {
        return;
    };

    let adj = scrolled.vadjustment();
    let target = (point.y() as f64 + rect.height() as f64 / 2.0 - adj.page_size() / 2.0)
        .clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower()));
    if (target - adj.value()).abs() < 1.0 {
        return;
    }

    animation.reset();
    animation.set_value_from(adj.value());
    animation.set_value_to(target);
    animation.play();
}
