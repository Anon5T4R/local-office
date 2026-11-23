//! Tests of the suite's shell guards (docs/planos/padrao-craft.md, "Identidade única"): the kittest
//! guards stay on (glyphs and accessibility), every dialog fits the narrowest window the app allows
//! in both axes, and the accent reads as text and on a button on both themes.

use egui_kittest::Harness;
use egui_kittest::kittest::NodeT;

use localoffice_engine::Session;

use crate::WordApp;

/// The desktop shell's minimum inner size (apps/localoffice/src/main.rs).
const NARROW: f32 = 760.0;
const NARROW_HEIGHT: f32 = 480.0;

/// A 1×1 PNG: a real picture for the dialogs that ask for one.
const PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

fn app(text: &str) -> WordApp {
    let mut app = WordApp::new(Session::new(localoffice_doc::Document::from_text(text)), crate::Services::default());
    let _ = app.run("document.setText", serde_json::json!({ "text": text }));
    app
}

/// A document a reader would bring: a table, an inline picture and a selected word, so the dialogs
/// that ask for content open on it.
fn app_with_content() -> WordApp {
    let mut app = app("Alpha beta gamma delta epsilon");
    let _ = app.run("insert.table", serde_json::json!({ "rows": 2, "cols": 2 }));
    let _ = app.run("insert.picture", serde_json::json!({ "data": PIXEL_PNG }));
    let _ = app.run("select.text", serde_json::json!({ "text": "beta" }));
    app
}

/// The caret inside a table's first cell.
fn app_in_table() -> WordApp {
    let mut app = app("Alpha beta gamma delta epsilon");
    let _ = app.run("insert.table", serde_json::json!({ "rows": 2, "cols": 2 }));
    app
}

/// The picture selected.
fn app_picture() -> WordApp {
    let mut app = app("Alpha beta gamma");
    let _ = app.run("insert.picture", serde_json::json!({ "data": PIXEL_PNG }));
    let _ = app.run("select.objects", serde_json::json!({ "index": 0 }));
    app
}

/// A chart selected.
fn app_chart() -> WordApp {
    let mut app = app("Alpha beta gamma");
    let _ = app.run(
        "insert.chart",
        serde_json::json!({ "type": "column", "data": { "categories": ["A", "B"], "series": [{ "name": "S", "values": [1, 2] }] } }),
    );
    let _ = app.run("select.objects", serde_json::json!({ "index": 0 }));
    app
}

/// A word selected outside the table.
fn app_selected_text() -> WordApp {
    let mut app = app("Alpha beta gamma delta epsilon");
    let _ = app.run("select.text", serde_json::json!({ "text": "delta" }));
    app
}

/// The caret in a field, for Edit Field Code.
fn app_in_field() -> WordApp {
    let mut app = app("");
    let _ = app.run("insert.field", serde_json::json!({ "instr": "PAGE" }));
    let _ = app.run("caret.docStart", serde_json::json!({}));
    app
}

fn harness(app: WordApp, width: f32) -> Harness<'static, WordApp> {
    Harness::builder().with_size(egui::vec2(width, NARROW_HEIGHT)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

/// Every control the screen announces stays inside the window, in both axes. The page panel is left
/// out: it is scrollable content, wider than the window at 100% zoom, and the view clips it — the
/// rulers' margin handles sit at page coordinates, far down the page.
///
/// A dialog body taller than the window scrolls, with its title and its row of buttons outside the
/// `ScrollArea`. AccessKit reports the scrolled content unclipped, so what hangs below the window is
/// accepted only when it comes before a vertical scroll bar inside the window and in its column (the
/// bar is drawn after the content, and the buttons after the bar). Everything else is shell: it fits
/// whole. Nothing may go above the window or past its sides.
fn inside_problems<S>(harness: &Harness<'_, S>, width: f32) -> Vec<String> {
    const TOL: f64 = 0.5;
    let (w, h) = (f64::from(width), f64::from(NARROW_HEIGHT));
    let mut nodes = Vec::new();
    inside_nodes(&harness.root(), &mut nodes);
    // Every vertical scroll bar in the window, by the index it was drawn at.
    let bars: Vec<(usize, egui::accesskit::Rect)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| {
            let rect = node.accesskit_node().bounding_box()?;
            let vertical = rect.y1 - rect.y0 > rect.x1 - rect.x0;
            let inside = rect.x0 >= -TOL && rect.x1 <= w + TOL && rect.y0 >= -TOL && rect.y1 <= h + TOL;
            (node.accesskit_node().role() == egui::accesskit::Role::ScrollBar && vertical && inside).then_some((i, rect))
        })
        .collect();
    let mut outside: Vec<String> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let role = node.accesskit_node().role();
        let Some(rect) = node.accesskit_node().bounding_box() else { continue };
        if rect.width() <= 0.0 {
            continue;
        }
        let label = node.accesskit_node().label().unwrap_or_default();
        if rect.x0 < -TOL || rect.x1 > w + TOL {
            outside.push(format!("{role:?} {label:?} outside the {w} px sides: {rect:?}"));
            continue;
        }
        if rect.y0 < -TOL {
            outside.push(format!("{role:?} {label:?} above the {h} px window: {rect:?}"));
            continue;
        }
        if rect.y1 <= h + TOL {
            continue;
        }
        if role == egui::accesskit::Role::ScrollBar {
            outside.push(format!("{role:?} {label:?} below the {h} px window: {rect:?}"));
            continue;
        }
        let scrolled = bars.iter().any(|&(bar_at, bar)| bar_at > i && rect.x1 <= bar.x1 + TOL && bar.y0 <= rect.y0 + TOL);
        if !scrolled {
            outside.push(format!("{role:?} {label:?} below the {h} px window, outside a body that scrolls: {rect:?}"));
        }
    }
    outside
}

/// The nodes [`inside_problems`] measures, in drawing order, without the page panel: the CentralPanel
/// that holds the document node draws the pages and their rulers, and its viewport clips them.
fn inside_nodes<'a>(root: &egui_kittest::Node<'a>, out: &mut Vec<egui_kittest::Node<'a>>) {
    for child in root.children() {
        let page = child.accesskit_node().role() == egui::accesskit::Role::Pane
            && child.children_recursive().any(|n| n.accesskit_node().role() == egui::accesskit::Role::Document);
        if page {
            continue;
        }
        out.push(child);
        inside_nodes(&child, out);
    }
}

/// [`inside_problems`] as an assertion for the shell, the panes and the update card.
fn assert_inside<S>(harness: &Harness<'_, S>, what: &str, width: f32) {
    let problems = inside_problems(harness, width);
    assert!(problems.is_empty(), "{what}: outside the {width}×{NARROW_HEIGHT} px window: {problems:#?}");
}

/// The dialogs that open on a document with content.
const DIALOGS: &[&str] = &[
    "font",
    "paragraph",
    "tabs",
    "borders",
    "pageBorders",
    "shading",
    "find",
    "replace",
    "goto",
    "insertTable",
    "pageSetup",
    "bookmark",
    "wordCount",
    "zoom",
    "watermark",
    "phonetic",
    "newStyle",
    "manageStyles",
    "newTableStyle",
    "commands",
    "icons",
    "enclose",
    "pasteSpecial",
    "renameDocument",
    "about",
    "contributors",
    "models",
    "customizeKeyboard",
    "customizeRibbon",
    "customizeQuickAccess",
    "noteOptions",
    "templates",
    "organizer",
    "customProperties",
    "defineList",
    "trackChangesOptions",
    "language",
    "lineNumbers",
    "hyphenation",
    "manualHyphenation",
    "envelopes",
    "labels",
    "pageNumbers",
    "columns",
    "symbol",
    "specialCharacters",
    "field",
    "insertSmartArt",
    "insertChart",
    "crossReference",
    "index",
    "markEntry",
    "toc",
    "tableOfFigures",
    "sourceManager",
    "createSource",
    "caption",
    "compare",
    "combine",
    "autoCorrect",
    "ruleIf",
    "matchFields",
    "mergeRecords",
    "recipientList",
    "insertMergeField",
    "findRecipient",
    "fieldCode",
    "editFieldCode",
    "encryptPassword",
    "tableProperties",
    "tableCellMargins",
    "tableFormula",
    "tableSort",
    "splitCells",
    "tableToText",
    "link",
    "textToTable",
    "pictureSize",
    "changeChartType",
    "chartData",
];

/// Every dialog of the app fits the narrowest window in both axes: with a document open no control
/// passes the window's edges, the title stays below the top edge and the row of buttons above the
/// bottom one (a body too tall for the window scrolls), so nothing is cut off or covered.
#[test]
fn every_dialog_fits_the_narrow_window() {
    let builders: [(&str, fn() -> WordApp); 6] = [
        ("a document", app_with_content),
        ("a table", app_in_table),
        ("a picture", app_picture),
        ("a chart", app_chart),
        ("selected text", app_selected_text),
        ("a field", app_in_field),
    ];
    let mut failed: Vec<&str> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    for &name in DIALOGS {
        let mut opened = None;
        for (_, build) in builders {
            let mut app = build();
            if let Some(dialog) = crate::dialogs::Dialog::open(name, &mut app) {
                opened = Some((app, dialog));
                break;
            }
        }
        let Some((mut app, dialog)) = opened else {
            failed.push(name);
            continue;
        };
        app.dialog = Some(dialog);
        let mut h = harness(app, NARROW);
        h.run_steps(4);
        problems.extend(inside_problems(&h, NARROW).into_iter().map(|p| format!("{name}: {p}")));
    }
    assert!(problems.is_empty(), "{} dialog(s) outside the {NARROW}×{NARROW_HEIGHT} px window:\n{}", problems.len(), problems.join("\n"));
    assert!(failed.is_empty(), "these dialogs did not open on any document the test builds: {failed:?}");
}

/// The top bar, the ribbon, the status bar and the Restrict Editing pane keep every control inside
/// the narrowest window, in both axes.
#[test]
fn the_shell_fits_the_narrow_window() {
    let mut h = harness(app_with_content(), NARROW);
    h.run_steps(3);
    assert_inside(&h, "shell", NARROW);

    // Restrict Editing is a pane, not a dialog: it opens beside the page.
    h.state_mut().restrict.open = true;
    h.run_steps(3);
    assert_inside(&h, "restrict editing", NARROW);
}

/// The update card (craft-update's `updater.offer`) fits the narrowest window, in both axes.
#[test]
fn the_update_card_fits_the_narrow_window() {
    let release = craft_update::Release { version: craft_update::version::parse("0.99.0"), tag: "v0.99.0".into(), ..Default::default() };
    let mut prompt = craft_update::StartupPrompt::new(&craft_update::suite::LOCALOFFICE, release, env!("CARGO_PKG_VERSION"));
    let mut h = Harness::builder().with_size(egui::vec2(NARROW, NARROW_HEIGHT)).build_ui(|ui| {
        prompt.show(ui.ctx());
    });
    h.run_steps(3);
    assert_inside(&h, "update card", NARROW);
}

/// Relative luminance (WCAG 2.x).
fn luminance(color: egui::Color32) -> f32 {
    let channel = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The accent as text (on the panel and on the ribbon) and as a filled button (its label on top)
/// reads at 4.5:1 or better on both themes.
#[test]
fn the_accent_reads_on_both_themes() {
    for theme in [crate::theme::Tokens::light(), crate::theme::Tokens::dark()] {
        let name = if theme.dark { "dark" } else { "light" };
        for (what, fg, bg) in [
            ("accent text on the panel", theme.accent_text, theme.panel),
            ("accent text on the ribbon", theme.accent_text, theme.ribbon),
            ("accent on the panel", theme.accent, theme.panel),
            ("label on an accent button", theme.on_accent, theme.accent),
        ] {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 4.5, "{name}: {what} is {ratio:.2}:1 ({fg:?} on {bg:?})");
        }
    }
}
