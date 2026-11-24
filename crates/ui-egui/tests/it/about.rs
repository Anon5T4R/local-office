//! The About window: it only closes, so it carries a single Close button — no Cancel.

use egui_kittest::kittest::{By, NodeT, Queryable};
use localwriter_engine::Session;
use localwriter_ui_egui::{Services, WordApp};
use serde_json::json;

/// A LocalWriter window, with the About dialog still closed. `open_about` opens it.
fn window() -> egui_kittest::Harness<'static, WordApp> {
    let mut app = WordApp::new(Session::new(localwriter_doc::Document::from_text("one")), Services::default());
    // The labels below are English; an integration test otherwise follows the machine's language.
    app.ui.language = "en".into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.step();
    }
    h
}

fn open_about(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.state_mut().run("ui.dialog", json!({"name": "about"})).unwrap();
    for _ in 0..3 {
        h.step();
    }
    assert!(h.state().dialog.is_some(), "the About dialog is open");
}

fn buttons(h: &egui_kittest::Harness<'static, WordApp>) -> Vec<String> {
    h.query_all(By::new().role(egui::accesskit::Role::Button)).filter_map(|n| n.accesskit_node().label()).map(|l| l.trim().to_string()).collect()
}

/// The buttons the About window itself brings: the window's labels minus the ones already there.
fn about_buttons(h: &egui_kittest::Harness<'static, WordApp>, before: &[String]) -> Vec<String> {
    let mut added: Vec<String> = buttons(h).into_iter().filter(|l| !before.contains(l)).collect();
    added.sort();
    added
}

/// No Cancel — the About window only closes. Its own buttons are the three tabs, the body's Close
/// and the window chrome's Close (`window_close`), nothing else.
#[test]
fn about_has_a_single_close_button_and_no_cancel() {
    let mut h = window();
    let before = buttons(&h);
    open_about(&mut h);
    let added = about_buttons(&h, &before);
    assert_eq!(
        added,
        ["About", "Close", "Close", "Contributors", "Models"].map(String::from).to_vec(),
        "tabs, one body Close and the window's Close — no Cancel"
    );

    // Either Close — the body's or the window chrome's — closes the About window.
    h.query_all(By::new().role(egui::accesskit::Role::Button))
        .find(|n| n.accesskit_node().label().as_deref().map(str::trim) == Some("Close"))
        .expect("a Close button")
        .click();
    for _ in 0..3 {
        h.step();
    }
    assert!(h.state().dialog.is_none(), "Close closes the About window");
}
