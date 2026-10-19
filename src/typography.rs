//! Typographie française appliquée pendant la frappe : guillemets « »,
//! espaces insécables, tiret de dialogue, points de suspension, apostrophe.

use gtk::prelude::*;
use gtk::{gdk, glib};

/// Espace insécable (devant « : », à l'intérieur des guillemets, après le tiret).
pub const NBSP: char = '\u{00A0}';
/// Espace fine insécable (devant « ; ! ? »).
pub const NNBSP: char = '\u{202F}';

/// Ce qu'il faut faire à la place de la frappe brute.
#[derive(Debug, PartialEq, Eq)]
pub struct Edit {
    /// Nombre de caractères à effacer avant le curseur.
    pub delete: usize,
    /// Texte à insérer ensuite (vide : la frappe est simplement absorbée).
    pub insert: String,
}

impl Edit {
    fn new(delete: usize, insert: impl Into<String>) -> Self {
        Self { delete, insert: insert.into() }
    }
}

fn is_space(c: char) -> bool {
    c == NBSP || c == NNBSP
}

/// Un mot (ou une fermeture) se termine juste avant : une ponctuation haute
/// peut suivre, précédée de son espace.
fn ends_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '»' | ')' | ']' | '…' | '’' | '.')
}

/// `before` : le paragraphe en cours, du début jusqu'au curseur.
/// Renvoie `None` quand la touche doit s'insérer telle quelle.
pub fn transform(before: &str, typed: char) -> Option<Edit> {
    let mut rev = before.chars().rev();
    let prev = rev.next();
    let prev2 = rev.next();

    match typed {
        '\'' => Some(Edit::new(0, "’")),

        '"' => {
            let ouverts = before.matches('«').count() as isize - before.matches('»').count() as isize;
            if ouverts > 0 {
                match prev {
                    Some(' ') => Some(Edit::new(1, format!("{NBSP}»"))),
                    Some(c) if is_space(c) => Some(Edit::new(0, "»")),
                    _ => Some(Edit::new(0, format!("{NBSP}»"))),
                }
            } else {
                Some(Edit::new(0, format!("«{NBSP}")))
            }
        }

        ';' | '!' | '?' | ':' => {
            let espace = if typed == ':' { NBSP } else { NNBSP };
            match prev {
                Some(' ') => Some(Edit::new(1, format!("{espace}{typed}"))),
                // 10:30, et pas d'espace dans « ?! » ni après une espace déjà posée.
                Some(c) if typed == ':' && c.is_ascii_digit() => None,
                Some(c) if ends_word(c) => Some(Edit::new(0, format!("{espace}{typed}"))),
                _ => None,
            }
        }

        '.' if prev == Some('.') && prev2 == Some('.') => Some(Edit::new(2, "…")),

        '-' if prev == Some('-') => Some(Edit::new(1, "—")),

        ' ' => match prev {
            // Après une espace insécable, l'espace tapée par habitude est de trop.
            Some(c) if is_space(c) => Some(Edit::new(0, "")),
            // « - » en début de paragraphe : réplique de dialogue.
            Some('-') if before.chars().count() == 1 => Some(Edit::new(1, format!("—{NBSP}"))),
            _ => None,
        },

        _ => None,
    }
}

/// Branche la typographie sur une zone de texte.
pub fn attach(view: &gtk::TextView) {
    let keys = gtk::EventControllerKey::new();
    // Phase de capture : on passe avant la méthode de saisie de la vue.
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = view.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(view) = weak.upgrade() else { return glib::Propagation::Proceed };
        if state.intersects(
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK,
        ) || !view.is_editable()
        {
            return glib::Propagation::Proceed;
        }
        let Some(typed) = key.to_unicode() else { return glib::Propagation::Proceed };
        if apply(&view.buffer(), typed) {
            view.scroll_mark_onscreen(&view.buffer().get_insert());
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    view.add_controller(keys);
}

fn apply(buffer: &gtk::TextBuffer, typed: char) -> bool {
    let has_selection = buffer.has_selection();
    // Une sélection est remplacée par la frappe : on raisonne après l'avoir effacée.
    let paragraph_before = |buffer: &gtk::TextBuffer| {
        let end = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = end;
        start.set_line_offset(0);
        buffer.text(&start, &end, false).to_string()
    };

    let before = if has_selection {
        let (start, _) = buffer.selection_bounds().unwrap();
        let mut line = start;
        line.set_line_offset(0);
        buffer.text(&line, &start, false).to_string()
    } else {
        paragraph_before(buffer)
    };

    let Some(edit) = transform(&before, typed) else { return false };

    buffer.begin_user_action();
    if has_selection {
        buffer.delete_selection(true, true);
    }
    if edit.delete > 0 {
        let mut end = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = end;
        start.backward_chars(edit.delete as i32);
        buffer.delete(&mut start, &mut end);
    }
    if !edit.insert.is_empty() {
        buffer.insert_at_cursor(&edit.insert);
    }
    buffer.end_user_action();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(texte: &str) -> String {
        let mut s = String::new();
        for c in texte.chars() {
            match transform(&s, c) {
                Some(e) => {
                    for _ in 0..e.delete {
                        s.pop();
                    }
                    s.push_str(&e.insert);
                }
                None => s.push(c),
            }
        }
        s
    }

    #[test]
    fn guillemets() {
        assert_eq!(tape("Il dit \"bonjour\"."), "Il dit «\u{a0}bonjour\u{a0}».");
        assert_eq!(tape("\" oui \""), "«\u{a0}oui\u{a0}»");
    }

    #[test]
    fn ponctuation_haute() {
        assert_eq!(tape("Vraiment?"), "Vraiment\u{202f}?");
        assert_eq!(tape("Vraiment ?!"), "Vraiment\u{202f}?!");
        assert_eq!(tape("Ainsi : rien"), "Ainsi\u{a0}: rien");
        assert_eq!(tape("à 10:30"), "à 10:30");
        assert_eq!(tape("oui; non"), "oui\u{202f}; non");
    }

    #[test]
    fn dialogue_et_signes() {
        assert_eq!(tape("- Viens."), "—\u{a0}Viens.");
        assert_eq!(tape("Et puis..."), "Et puis…");
        assert_eq!(tape("l'aube -- enfin"), "l’aube — enfin");
        assert_eq!(tape("a - b"), "a - b");
    }
}
