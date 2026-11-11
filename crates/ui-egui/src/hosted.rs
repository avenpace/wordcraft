//! Hosted in Clawku ([`Services::hosted`](crate::Services::hosted)): the host page owns the file
//! list, New, Open, Close, Rename, Share, versions, sign-in, the user's name and help, and the
//! editor shows one document, the one the host sent. What a hosted editor leaves out is listed
//! here, so the ribbon, File, search and the shortcuts agree.

/// Commands a hosted editor neither shows nor runs: they make or show another document (New,
/// templates, Close, Recover, New Window, Switch Windows), rename or AutoSave the host's file,
/// save a template into a local folder, set the user's name (the host's `?author=` names them),
/// or reach the standalone app's community and updates.
pub(crate) const HIDDEN_COMMANDS: &[&str] = &[
    "file.new",
    "file.newFromTemplate",
    "file.listTemplates",
    "file.saveTemplate",
    "file.close",
    "file.closeDocument",
    "file.recover",
    "file.rename",
    "file.autosave",
    "file.setAuthor",
    "view.newWindow",
    "view.switchWindows",
    "ui.saveAsTemplate",
    "ui.recoveryDiscard",
    "ui.fileBrowser",
    "ui.discord",
    "ui.checkForUpdates",
    "help.checkForUpdates",
];

/// File's pages a hosted editor leaves out: Home and New (another document), Recover (local
/// recovery copies) and Close.
pub(crate) const HIDDEN_PAGES: &[&str] = &["home", "new", "recover", "close"];

/// The File page a hosted editor opens on instead of Home.
pub(crate) const FIRST_PAGE: &str = "info";

/// Whether a hosted editor leaves out command `id`.
pub(crate) fn hides_command(id: &str) -> bool {
    HIDDEN_COMMANDS.contains(&id)
}

/// Whether a hosted editor leaves out File page `page`.
pub(crate) fn hides_page(page: &str) -> bool {
    HIDDEN_PAGES.contains(&page)
}

/// A copy's name for the host (an export, Save these pages as PDF): never the document's own,
/// which the host would overwrite. `name` is kept unless it is the document's file name, then
/// `report.docx` becomes `report (copy).docx`.
pub(crate) fn copy_name(name: &str, document: Option<&str>) -> String {
    if document.is_none_or(|d| !d.eq_ignore_ascii_case(name)) {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (copy).{ext}"),
        _ => format!("{name} (copy)"),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use crate::{Services, WordApp};

    /// What the host page was handed: (name, `saveAs`, size).
    type Saves = Rc<RefCell<Vec<(String, bool, usize)>>>;

    /// The web app framed by Clawku: hosted, saves and Share going to the host page, which
    /// sent `name` to edit (with typing since). `share` counts the Share messages.
    fn hosted_app(name: &str) -> (WordApp, Saves, Rc<RefCell<u32>>) {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        let saves: Saves = Rc::default();
        let shares = Rc::new(RefCell::new(0));
        let (seen, shared) = (saves.clone(), shares.clone());
        a.services.hosted = true;
        a.services.download = Some(Box::new(|_, _| Err("downloads go to the host page".into())));
        a.services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], save_as: bool| {
            seen.borrow_mut().push((name.to_string(), save_as, bytes.len()));
            Ok(())
        }));
        a.services.host_share = Some(Box::new(move || {
            *shared.borrow_mut() += 1;
            true
        }));
        // As in the browser: the "dialog" keeps the suggested name.
        a.services.pick_save = Some(Box::new(|name: &str| Some(name.to_string())));
        let doc = wordcraft_doc::Document::from_text("Quarterly report");
        let bytes = wordcraft_engine::io::save_bytes_with(name, &doc, None, &Default::default()).unwrap();
        a.execute("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})).unwrap();
        a.run("text.insert", json!({"text": "Edited "})).unwrap();
        (a, saves, shares)
    }

    fn calls(saves: &Saves) -> Vec<(String, bool)> {
        saves.borrow().iter().map(|(n, s, _)| (n.clone(), *s)).collect()
    }

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    /// Save overwrites the host's file under the name it sent; Save As hands it over under that
    /// name for the host to ask the new one; exports and Save these pages as PDF are copies
    /// beside it, never under the document's own name, and leave the document as it was.
    #[test]
    fn hosted_saves_overwrite_save_as_asks_the_host_and_exports_are_copies() {
        let (mut a, saves, _) = hosted_app("report.docx");
        a.run("file.save", json!({})).unwrap();
        assert!(!a.session.dirty);
        a.save_as_dialog();
        assert_eq!(calls(&saves), [("report.docx".to_string(), false), ("report.docx".to_string(), true)]);
        saves.borrow_mut().clear();

        a.run("text.insert", json!({"text": "More "})).unwrap();
        for ext in ["docx", "odt", "pdf", "md", "png"] {
            a.export_dialog(ext);
        }
        a.print_pdf_dialog();
        assert_eq!(
            calls(&saves),
            ["report (copy).docx", "report.odt", "report.pdf", "report.md", "report.png", "report (pages).pdf"].map(|n| (n.to_string(), false))
        );
        assert!(saves.borrow().iter().all(|(_, _, len)| *len > 0), "every copy has its bytes: {:?}", saves.borrow());
        assert!(a.session.dirty, "a copy doesn't save the document");
        assert_eq!(a.session.path.as_deref(), Some(std::path::Path::new("report.docx")), "the document is still the host's file");
    }

    /// Save As of a file the host sent in a format that doesn't keep everything still asks the
    /// host (`saveAs`), under that file's own name.
    #[test]
    fn hosted_save_as_keeps_the_documents_own_name() {
        let (mut a, saves, _) = hosted_app("notes.odt");
        a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(calls(&saves), [("notes.odt".to_string(), true)]);
    }

    /// Share, from the search box's `file.share` too, is the host's Share dialog, not Save As.
    #[test]
    fn hosted_share_goes_to_the_host() {
        let (mut a, saves, shares) = hosted_app("report.docx");
        assert_eq!(a.run("file.share", json!({})).unwrap(), json!({"shared": "host"}));
        a.run("ui.backstage", json!({"value": false})).unwrap();
        assert_eq!(*shares.borrow(), 1);
        assert!(!a.ui.backstage && a.dialog.is_none() && saves.borrow().is_empty());
    }

    /// One document, the host's: New, Close, Rename, AutoSave and New Window do nothing, and
    /// the Start screen never shows.
    #[test]
    fn hosted_commands_the_host_owns_do_nothing() {
        let (mut a, _, _) = hosted_app("report.docx");
        assert!(!a.start_screen());
        assert!(!a.ui.backstage && a.has_document());
        let document = a.session.document_id();
        let autosave = a.autosave;
        for id in
            ["file.new", "file.newFromTemplate", "file.close", "file.closeDocument", "file.rename", "file.autosave", "view.newWindow", "ui.discord"]
        {
            assert_eq!(a.run(id, json!({})).unwrap(), json!({"hosted": true}), "{id}");
        }
        assert_eq!(a.session.document_id(), document);
        assert_eq!(a.session.documents().len(), 1);
        assert!(a.dialog.is_none(), "no Save Changes or Rename prompt");
        assert_eq!(a.autosave, autosave);
    }

    /// File opens on Info with no Home, New, Recover, Close, Recent or community; the title bar
    /// has no AutoSave, account or community button; there is no Help tab, and Options has no
    /// user name or Agents pane.
    #[test]
    fn hosted_chrome_leaves_out_what_the_host_owns() {
        let (mut a, _, _) = hosted_app("report.docx");
        a.ui.show_discord = true;
        let mut h = harness(a);
        h.run_steps(4);
        for gone in ["AutoSave", "Account", "Community", "Help"] {
            assert!(h.query_by_label(gone).is_none(), "{gone} is left out");
        }
        h.state_mut().run("ui.tab", json!({"tab": "File"})).unwrap();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "info", "File opens on Info, not Home");
        for gone in ["Home", "New", "Recover", "Close", "Clawku community"] {
            assert!(h.query_by_label(gone).is_none(), "File has no {gone}");
        }
        assert!(h.query_by_label("Save As").is_some() && h.query_by_label("About").is_some());
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("Recent").is_none(), "no Recent list");
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("Save as Word Template…").is_none());
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "options", "pane": "general"})).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("User name:").is_none() && h.query_by_label("Agents").is_none());
    }

    /// Not hosted, the same app keeps all of it.
    #[test]
    fn standalone_keeps_home_account_and_autosave() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.ui.show_discord = true;
        let mut h = harness(a);
        h.run_steps(4);
        for kept in ["AutoSave", "Account", "Community", "Help"] {
            assert!(h.query_by_label(kept).is_some(), "{kept} is there");
        }
        assert!(h.state_mut().start_screen());
        h.run_steps(4);
        for kept in ["Home", "New", "Recover", "Close", "Clawku community", "Recent"] {
            assert!(h.query_by_label(kept).is_some(), "{kept} is there");
        }
    }

    /// The browser on its own: Save these pages as PDF downloads the pages' PDF (it used to
    /// write nothing and say it had).
    #[test]
    fn web_save_these_pages_as_pdf_downloads_the_pdf() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Pages")), Services::default());
        let got: Rc<RefCell<Vec<(String, usize)>>> = Rc::default();
        let seen = got.clone();
        a.services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            seen.borrow_mut().push((name.to_string(), bytes.len()));
            Ok(())
        }));
        a.services.pick_save = Some(Box::new(|name: &str| Some(name.to_string())));
        a.print_pdf_dialog();
        let got = got.borrow();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].0.ends_with(".pdf") && got[0].1 > 0, "{got:?}");
    }

    #[test]
    fn a_copy_is_never_named_as_the_document() {
        assert_eq!(super::copy_name("report.docx", Some("report.docx")), "report (copy).docx");
        assert_eq!(super::copy_name("REPORT.docx", Some("report.docx")), "REPORT (copy).docx");
        assert_eq!(super::copy_name("report.odt", Some("report.docx")), "report.odt");
        assert_eq!(super::copy_name("report.pdf", None), "report.pdf");
    }
}
