//! Tracked changes under the pointer: who made the change, what it was and when, in a tip over
//! the text (Word's ScreenTip on a revision). People reviewing a document together see whose
//! edit each coloured passage is without opening the Reviewing pane.

use super::{DocLayout, Pos, WordApp};
use wordcraft_doc::RevisionKind;

/// The longest stretch of changed text a tip quotes.
const QUOTE_CHARS: usize = 80;

/// The tip for the tracked change under the point (x, y) on `page`, where `pos` is the caret
/// position the point hits; None over unchanged text, or when markup is hidden (No Markup shows
/// the final text, with nothing to explain). As for links, the point must be on the changed
/// character itself, not beside or below it.
pub(super) fn revision_under(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, pos: &Pos) -> Option<String> {
    if !app.session.view.show_markup {
        return None;
    }
    let doc = &app.session.doc;
    let para = doc.para_at(pos)?;
    let at = wordcraft_geom::Point::new(x, y);
    // The caret falls between characters: the one under the point is on either side of it.
    [(pos.off, para.next_boundary(pos.off)), (para.prev_boundary(pos.off), pos.off)].into_iter().filter(|(a, b)| a < b).find_map(|(a, b)| {
        let (idx, field) = revision_of(para.props_of_char(a))?;
        let rev = doc.revisions.get(idx as usize)?;
        let (from, to) = (Pos { off: a, ..pos.clone() }, Pos { off: b, ..pos.clone() });
        if !layout.selection_rects(doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(at)) {
            return None;
        }
        // The whole stretch of this change around the character, in this paragraph
        let same = |off: usize| revision_of(para.props_of_char(off)).is_some_and(|(i, f)| i == idx && f == field);
        let mut start = a;
        while start > 0 {
            let prev = para.prev_boundary(start);
            if prev >= start || !same(prev) {
                break;
            }
            start = prev;
        }
        let mut end = b;
        while end < para.len() && same(end) {
            let next = para.next_boundary(end);
            if next <= end {
                break;
            }
            end = next;
        }
        let text = para.text.get(start..end).unwrap_or_default();
        Some(tip_text(&rev.author, kind_of(rev, field), &crate::local_time::revision_time(&rev.date), text))
    })
}

/// Which change a character belongs to: its insertion, else its deletion, else a formatting
/// change (the revision index, and which of the three it is).
fn revision_of(props: &wordcraft_doc::CharProps) -> Option<(u32, Field)> {
    props
        .ins
        .map(|i| (i, Field::Ins))
        .or_else(|| props.del.map(|i| (i, Field::Del)))
        .or_else(|| props.fmt_change.as_ref().map(|c| (c.rev, Field::Fmt)))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Ins,
    Del,
    Fmt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Inserted,
    Deleted,
    Formatted,
    Moved,
}

fn kind_of(rev: &wordcraft_doc::Revision, field: Field) -> Kind {
    if rev.move_name.is_some() {
        return Kind::Moved;
    }
    match (field, rev.kind) {
        (Field::Fmt, _) | (_, RevisionKind::Format) => Kind::Formatted,
        (Field::Del, _) | (_, RevisionKind::Delete) => Kind::Deleted,
        _ => Kind::Inserted,
    }
}

/// "Andi inserted" / the time / the changed text in quotes (shortened), one per line.
fn tip_text(author: &str, kind: Kind, time: &str, text: &str) -> String {
    let who = if author.trim().is_empty() { tl!("Unknown author").to_string() } else { author.trim().to_string() };
    let what = match kind {
        Kind::Inserted => tl!("{author} inserted"),
        Kind::Deleted => tl!("{author} deleted"),
        Kind::Formatted => tl!("{author} changed formatting"),
        Kind::Moved => tl!("{author} moved"),
    };
    let mut lines = vec![crate::i18n::fmt(what, &[("author", &who)])];
    if !time.is_empty() {
        lines.push(time.to_string());
    }
    // Objects (pictures, fields) sit in the text as one placeholder character: leave them out
    let quoted: String = text.chars().filter(|c| *c != wordcraft_doc::para::OBJ && !c.is_control()).collect();
    let quoted = quoted.trim();
    if kind != Kind::Formatted && !quoted.is_empty() {
        let short: String = quoted.chars().take(QUOTE_CHARS).collect();
        let more = if quoted.chars().count() > QUOTE_CHARS { "…" } else { "" };
        lines.push(format!("“{short}{more}”"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tip_tests {
    use super::{Kind, tip_text};

    #[test]
    fn says_who_did_what_and_when_with_the_text() {
        assert_eq!(
            tip_text("Andi (Oris)", Kind::Inserted, "2026-11-09 14:18", "Payment within 30 days."),
            "Andi (Oris) inserted\n2026-11-09 14:18\n“Payment within 30 days.”"
        );
        assert_eq!(tip_text("", Kind::Deleted, "", "old clause"), "Unknown author deleted\n“old clause”");
        assert_eq!(tip_text("Uung", Kind::Formatted, "2026-11-09 09:00", "bold text"), "Uung changed formatting\n2026-11-09 09:00");
    }

    #[test]
    fn long_changes_are_shortened() {
        let long = "x".repeat(200);
        let tip = tip_text("A", Kind::Inserted, "t", &long);
        assert!(tip.ends_with("…”") && tip.lines().last().unwrap().chars().count() == 80 + 3);
    }
}
