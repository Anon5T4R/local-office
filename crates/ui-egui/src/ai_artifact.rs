//! The artifact actions: one request, answered as a whole document, applied by the engine.
//!
//! Two actions share the contract in [`localoffice_ai::actions`]: a **new document** from a
//! description, and the open document **rewritten into a copy** beside itself. The original file is
//! never written - `EditDoc` saves the answer as `<name> AI.docx` next to it - and an answer the
//! engine refuses reopens the dialog with the reason instead of leaving half a document behind.

use std::path::{Path, PathBuf};

use craft_ai_ui as ui;
use localoffice_ai::{Stream, actions};
use localoffice_engine::Session;
use serde_json::json;

use crate::i18n;

/// How much of a document one rewrite may send (a longer one is refused, never silently cut).
const EDIT_LIMIT: usize = 120_000;

/// What the dialog asks the model for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// A whole new document, in its own tab.
    #[default]
    NewDoc,
    /// The open document rewritten into a copy beside it; the original stays as it is.
    CopyEdit,
}

/// The document's shape (the dialog's preset). `None` adds nothing to the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    None,
    Report,
    Letter,
    Resume,
    Article,
}

impl Shape {
    const ALL: [Shape; 5] = [Shape::None, Shape::Report, Shape::Letter, Shape::Resume, Shape::Article];

    fn label(self) -> &'static str {
        match self {
            Shape::None => tl!("None"),
            Shape::Report => tl!("Report"),
            Shape::Letter => tl!("Letter"),
            Shape::Resume => tl!("Resume"),
            Shape::Article => tl!("Article"),
        }
    }

    /// What the request adds for this shape. Model-facing, so English (the answer's language is
    /// the prompt's business, not this line's).
    fn request_text(self) -> Option<&'static str> {
        match self {
            Shape::None => None,
            Shape::Report => Some("a report"),
            Shape::Letter => Some("a letter"),
            Shape::Resume => Some("a resume"),
            Shape::Article => Some("an article"),
        }
    }
}

/// The last request, kept for the dialog's Repeat last.
#[derive(Clone, Debug, PartialEq)]
struct Last {
    kind: Kind,
    topic: String,
    shape: Shape,
}

/// The artifact actions' state: the dialog, the strip and the request in flight.
#[derive(Default)]
pub struct Artifact {
    /// Whether the dialog is open.
    pub dialog: bool,
    kind: Kind,
    /// The description (new document) or the instruction (rewrite).
    topic: String,
    shape: Shape,
    last: Option<Last>,
    strip: ui::StatusStrip,
    stream: Option<Stream>,
    /// What the answer will be interpreted as.
    request: Option<actions::Request>,
    /// Why the last attempt did not become a document: the dialog reopens showing it.
    pub reason: Option<String>,
}

impl Artifact {
    /// Whether a request is in flight.
    pub fn running(&self) -> bool {
        self.stream.is_some()
    }

    /// The strip and the button that opens the dialog, drawn above the chat.
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        if self.strip.show(ui, ui::StripLabels { stop: tl!("Stop"), dismiss: tl!("Dismiss") }) == ui::StripAction::Stop {
            self.stop();
        }
        if self.running() {
            return;
        }
        if ui.button(tl!("Write with AI")).clicked() {
            self.dialog = true;
        }
    }

    /// Stops the request in flight; nothing is applied from a stopped answer.
    fn stop(&mut self) {
        if let Some(stream) = &self.stream {
            stream.cancel();
        }
        self.stream = None;
        self.request = None;
        self.strip.finish();
    }

    /// What arrived since the last frame, applied once the answer is whole. Returns the message the
    /// status bar shows when a document was written.
    pub fn poll(&mut self, session: &mut Session) -> Option<String> {
        let stream = self.stream.as_ref()?;
        let (pieces, done) = stream.poll();
        for piece in pieces {
            self.strip.push(&piece);
        }
        let result = done?;
        self.stream = None;
        self.strip.finish();
        let request = self.request.take()?;
        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                self.strip.fail(error.clone());
                self.reason = Some(error);
                self.dialog = true;
                return None;
            }
        };
        let outcome = match actions::interpret(&request, &answer) {
            Ok(outcome) => outcome,
            Err(reason) => {
                self.reason = Some(reason);
                self.dialog = true;
                return None;
            }
        };
        let kind = kind_of(&request);
        match apply(session, kind, &outcome) {
            Ok(copy) => Some(match copy {
                Some(path) => i18n::fmt(tl!("The AI wrote the copy beside the original: {name}"), &[("name", &name_of(&path))]),
                None => tl!("The AI wrote a new document.").to_string(),
            }),
            Err(reason) => {
                self.reason = Some(reason);
                self.dialog = true;
                None
            }
        }
    }

    /// The dialog: what to write, how, and the reason the last answer was refused.
    pub fn dialog(&mut self, session: &mut Session, connection: Result<localoffice_ai::Connection, String>, ctx: &egui::Context) {
        if !self.dialog {
            return;
        }
        let copy_name = session.path.as_deref().map(copy_path).map(|path| name_of(&path));
        let mut action = DialogAction::None;
        egui::Window::new(tl!("Write with AI")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
            ctx,
            |ui| {
                ui.set_width(420.0);
                action = self.body(ui, copy_name.as_deref());
            },
        );
        match action {
            DialogAction::None => {}
            DialogAction::Cancel => {
                self.dialog = false;
                self.reason = None;
            }
            DialogAction::Repeat => {
                if let Some(last) = self.last.clone() {
                    self.kind = last.kind;
                    self.topic = last.topic;
                    self.shape = last.shape;
                    self.start(session, connection);
                }
            }
            DialogAction::Write => self.start(session, connection),
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, copy_name: Option<&str>) -> DialogAction {
        let mut action = DialogAction::None;
        ui.radio_value(&mut self.kind, Kind::NewDoc, tl!("New document"));
        ui.radio_value(&mut self.kind, Kind::CopyEdit, tl!("Rewrite a copy"));
        ui.add_space(6.0);
        match self.kind {
            Kind::NewDoc => {
                let question = ui.label(tl!("What should the new document be about?"));
                ui.add(egui::TextEdit::multiline(&mut self.topic).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(question.id);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = ui.label(tl!("Shape"));
                    egui::ComboBox::from_id_salt("ai-shape")
                        .selected_text(self.shape.label())
                        .show_ui(ui, |ui| {
                            for shape in Shape::ALL {
                                ui.selectable_value(&mut self.shape, shape, shape.label());
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });
            }
            Kind::CopyEdit => {
                let question = ui.label(tl!("What should change?"));
                ui.add(egui::TextEdit::multiline(&mut self.topic).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(question.id);
                ui.add_space(4.0);
                let hint = match copy_name {
                    Some(name) => i18n::fmt(tl!("The original stays as it is: the copy is saved beside it as {name}."), &[("name", name)]),
                    None => tl!("Save the document first: the copy is written beside it.").to_string(),
                };
                let color = if copy_name.is_some() { ui.visuals().weak_text_color() } else { ui.visuals().warn_fg_color };
                ui.label(egui::RichText::new(hint).small().color(color));
            }
        }
        if let Some(reason) = &self.reason {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(reason).small().color(ui.visuals().warn_fg_color));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Cancel")).clicked() {
                action = DialogAction::Cancel;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Write")).clicked() {
                    action = DialogAction::Write;
                }
                if self.last.is_some() && ui.button(tl!("Repeat last")).clicked() {
                    action = DialogAction::Repeat;
                }
            });
        });
        action
    }

    /// Turns the dialog into a request and starts it on a worker thread.
    fn start(&mut self, session: &Session, connection: Result<localoffice_ai::Connection, String>) {
        self.reason = None;
        let request = match self.request_for(session) {
            Ok(request) => request,
            Err(reason) => {
                self.reason = Some(reason);
                return;
            }
        };
        let connection = match connection {
            Ok(connection) => connection,
            Err(reason) => {
                self.reason = Some(reason);
                return;
            }
        };
        let messages = match actions::messages(&request, i18n::current().code()) {
            Ok(messages) => messages,
            Err(error) => {
                self.reason = Some(error);
                return;
            }
        };
        self.last = Some(Last { kind: self.kind, topic: self.topic.clone(), shape: self.shape });
        self.request = Some(request);
        self.strip.begin();
        self.stream = Some(Stream::start(connection, messages));
        self.dialog = false;
    }

    fn request_for(&self, session: &Session) -> Result<actions::Request, String> {
        let text = self.topic.trim().to_string();
        match self.kind {
            Kind::NewDoc => {
                if text.is_empty() {
                    return Err(tl!("Write what the new document should be about.").into());
                }
                Ok(actions::Request::GenerateDoc {
                    topic: text,
                    language: i18n::current().name().to_string(),
                    shape: self.shape.request_text().map(str::to_string),
                })
            }
            Kind::CopyEdit => {
                if text.is_empty() {
                    return Err(tl!("Write what should change.").into());
                }
                if session.path.is_none() {
                    return Err(tl!("Save the document first: the copy is written beside it.").into());
                }
                let document = session.doc.plain_text(localoffice_doc::StoryRef::Body);
                if document.trim().is_empty() {
                    return Err(tl!("The document has no text to rewrite.").into());
                }
                if document.chars().count() > EDIT_LIMIT {
                    return Err(tl!("The document is too long for a rewrite.").into());
                }
                Ok(actions::Request::EditDoc { instruction: text, document })
            }
        }
    }
}

/// What the dialog's buttons ask for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogAction {
    None,
    Cancel,
    Repeat,
    Write,
}

/// The action a request was built for.
fn kind_of(request: &actions::Request) -> Kind {
    match request {
        actions::Request::GenerateDoc { .. } => Kind::NewDoc,
        actions::Request::EditDoc { .. } => Kind::CopyEdit,
    }
}

/// A block's paragraph, as the engine is told to style it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ParaStyle {
    Normal,
    Heading(u8),
    Bullet,
}

/// One paragraph of the answer: heading, paragraph, or one line per list item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Placed {
    lines: usize,
    style: ParaStyle,
}

/// Writes the answer into the session: a new document, or the open one replaced (the copy is saved
/// after this, so a failure here never writes a file).
fn apply(session: &mut Session, kind: Kind, outcome: &actions::Outcome) -> Result<Option<PathBuf>, String> {
    let actions::Outcome::Doc { blocks, .. } = outcome;
    match kind {
        Kind::NewDoc => {
            let before = session.document_id();
            session.run("file.new", &json!({})).map_err(|error| error.to_string())?;
            let created = session.document_id();
            if let Err(reason) = write_blocks(session, blocks, false) {
                let _ = session.close_document(created);
                let _ = session.switch_document(before);
                return Err(reason);
            }
            Ok(None)
        }
        Kind::CopyEdit => {
            let source = session.path.clone().ok_or_else(|| tl!("Save the document first: the copy is written beside it.").to_string())?;
            let copy = copy_path(&source);
            let snapshot = session.edit_snapshot();
            let path_before = session.path.clone();
            let written = write_blocks(session, blocks, true)
                .and_then(|()| session.run("file.saveAs", &json!({ "path": copy.to_string_lossy() })).map_err(|error| error.to_string()));
            if written.is_err() {
                session.restore(snapshot);
                session.path = path_before;
                return written.map(|_| None);
            }
            Ok(Some(copy))
        }
    }
}

/// Pastes the blocks as paragraphs and styles them, all through the engine's own commands.
fn write_blocks(session: &mut Session, blocks: &[actions::Block], replace: bool) -> Result<(), String> {
    let (text, placed) = layout(blocks);
    if replace {
        session.run("select.all", &json!({})).map_err(|error| error.to_string())?;
    }
    session.run("edit.pasteText", &json!({ "text": text })).map_err(|error| error.to_string())?;
    let mut line = 0;
    for item in placed {
        if let Some((command, params)) = command_of(item.style) {
            session.run("caret.set", &json!({ "pos": { "block": line, "off": 0 } })).map_err(|error| error.to_string())?;
            session.run("select.paragraph", &json!({})).map_err(|error| error.to_string())?;
            session.run(command, &params).map_err(|error| error.to_string())?;
        }
        line += item.lines;
    }
    Ok(())
}

/// The answer as one paragraph per line and the style each paragraph takes.
fn layout(blocks: &[actions::Block]) -> (String, Vec<Placed>) {
    let mut lines: Vec<String> = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    for block in blocks {
        match block {
            actions::Block::Heading { level, text } => {
                lines.push(one_line(text));
                placed.push(Placed { lines: 1, style: ParaStyle::Heading(*level) });
            }
            actions::Block::Paragraph { text } => {
                lines.push(one_line(text));
                placed.push(Placed { lines: 1, style: ParaStyle::Normal });
            }
            actions::Block::List { items } => {
                for item in items {
                    lines.push(one_line(item));
                    placed.push(Placed { lines: 1, style: ParaStyle::Bullet });
                }
            }
        }
    }
    (lines.join("\n"), placed)
}

/// A block's text as one paragraph: its own line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split(['\n', '\r']).map(str::trim).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The command that gives one paragraph its style, and its parameters.
fn command_of(style: ParaStyle) -> Option<(&'static str, serde_json::Value)> {
    match style {
        ParaStyle::Normal => None,
        ParaStyle::Heading(level) => Some(("para.style", json!({ "style": format!("Heading{}", level.clamp(1, 3)) }))),
        ParaStyle::Bullet => Some(("para.bullets", json!({}))),
    }
}

/// The copy beside the original: `<stem> AI.<ext>`, numbered when that name is taken.
fn copy_path(source: &Path) -> PathBuf {
    let dir = source.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let stem = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into());
    let extension = source.extension().map(|extension| extension.to_string_lossy().into_owned());
    let mut number = 1;
    loop {
        let suffix = if number == 1 { String::new() } else { format!(" {number}") };
        let name = format!("{stem} AI{suffix}");
        let candidate = match &extension {
            Some(extension) => dir.join(format!("{name}.{extension}")),
            None => dir.join(name),
        };
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

/// A path's file name, for the hint and the status bar.
fn name_of(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use localoffice_ai::actions::{Block, Outcome};
    use localoffice_engine::sample::sample_document;

    fn session() -> Session {
        Session::new(sample_document())
    }

    fn doc(title: &str) -> Outcome {
        Outcome::Doc {
            title: title.into(),
            blocks: vec![
                Block::Heading { level: 1, text: "Sepse".into() },
                Block::Paragraph { text: "Definição.".into() },
                Block::List { items: vec!["febre".into(), "leucocitose".into()] },
            ],
        }
    }

    fn style_at(session: &Session, block: usize) -> Option<String> {
        session.doc.para(localoffice_doc::StoryRef::Body, &localoffice_doc::Path::top(block)).and_then(|para| para.props.style.clone())
    }

    /// A new document is its own tab, with the answer's paragraphs and their styles.
    #[test]
    fn a_new_document_is_written_as_its_own_document() {
        let mut s = session();
        let before = s.document_id();
        let copy = apply(&mut s, Kind::NewDoc, &doc("Protocolo")).expect("applied");
        assert!(copy.is_none(), "a new document writes no copy");
        assert_ne!(s.document_id(), before, "the new document is the active one");
        assert!(s.is_open(before), "the document that was open stays open");
        assert_eq!(s.doc.plain_text(localoffice_doc::StoryRef::Body).trim_end(), "Sepse\nDefinição.\nfebre\nleucocitose");
        assert_eq!(style_at(&s, 0).as_deref(), Some("Heading1"));
        assert_eq!(style_at(&s, 1), None, "a paragraph keeps the Normal style");
        let listed =
            s.doc.para(localoffice_doc::StoryRef::Body, &localoffice_doc::Path::top(2)).and_then(|p| s.doc.styles.resolve_para(&p.props).numbering);
        assert!(listed.is_some_and(|numbering| numbering.num != 0), "list items carry bullets");
    }

    /// The rewrite is a copy beside the original, and the original file is byte-identical.
    #[test]
    fn the_rewrite_goes_to_a_copy_and_leaves_the_original_alone() {
        let dir = std::env::temp_dir().join(format!("localoffice-ai-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let original = dir.join("Protocolo.docx");
        std::fs::write(&original, b"conteudo original").expect("seed");
        let mut s = session();
        s.path = Some(original.clone());
        let copy = apply(&mut s, Kind::CopyEdit, &doc("Protocolo")).expect("applied").expect("a copy path");
        assert_eq!(copy.file_name().and_then(|n| n.to_str()), Some("Protocolo AI.docx"));
        assert_eq!(std::fs::read(&original).expect("original"), b"conteudo original", "the original is untouched");
        assert_eq!(s.path.as_deref(), Some(copy.as_path()), "the copy is the open document");
        assert!(copy.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A second rewrite of the same document takes the next name instead of overwriting the first.
    #[test]
    fn a_taken_copy_name_is_numbered() {
        let dir = std::env::temp_dir().join(format!("localoffice-ai-number-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let original = dir.join("Ata.docx");
        std::fs::write(&original, b"x").expect("seed");
        std::fs::write(dir.join("Ata AI.docx"), b"x").expect("seed");
        assert_eq!(copy_path(&original).file_name().and_then(|n| n.to_str()), Some("Ata AI 2.docx"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed copy is taken back whole: the open document and its path are as they were.
    #[test]
    fn a_failed_rewrite_is_taken_back_whole() {
        let dir = std::env::temp_dir().join(format!("localoffice-ai-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let original = dir.join("Ata.docx");
        std::fs::write(&original, b"x").expect("seed");
        let mut s = session();
        s.path = Some(original.clone());
        let before = s.doc.plain_text(localoffice_doc::StoryRef::Body);
        // The copy lands in a folder that does not exist: saving fails after the document was
        // rewritten, so the whole apply has to come back.
        let missing = dir.join("missing").join("Ata.docx");
        s.path = Some(missing);
        let reason = apply(&mut s, Kind::CopyEdit, &doc("Protocolo")).expect_err("the save fails");
        assert!(!reason.is_empty());
        assert_eq!(s.doc.plain_text(localoffice_doc::StoryRef::Body), before, "the document came back");
        assert_eq!(s.path.as_deref(), Some(dir.join("missing").join("Ata.docx").as_path()), "so did its path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dialog refuses a rewrite of a document that was never saved, with the reason.
    #[test]
    fn an_unsaved_document_cannot_be_rewritten() {
        let artifact = Artifact { kind: Kind::CopyEdit, topic: "resuma".into(), ..Artifact::default() };
        let s = session();
        let reason = artifact.request_for(&s).expect_err("no path, no copy");
        assert!(reason.contains("Save the document first"), "{reason}");
    }

    /// The shape travels to the request only when the dialog picked one.
    #[test]
    fn the_shape_is_optional() {
        let mut artifact = Artifact { topic: "sepse".into(), ..Artifact::default() };
        let request = artifact.request_for(&session()).expect("a request");
        assert!(matches!(&request, actions::Request::GenerateDoc { shape: None, .. }));
        artifact.shape = Shape::Report;
        let request = artifact.request_for(&session()).expect("a request");
        assert!(matches!(&request, actions::Request::GenerateDoc { shape: Some(shape), .. } if shape == "a report"));
    }

    /// The layout of an answer: one line per paragraph, list items each on their own line.
    #[test]
    fn the_layout_keeps_one_paragraph_per_line() {
        let blocks = match doc("x") {
            Outcome::Doc { blocks, .. } => blocks,
        };
        let (text, placed) = layout(&blocks);
        assert_eq!(text, "Sepse\nDefinição.\nfebre\nleucocitose");
        assert_eq!(
            placed,
            vec![
                Placed { lines: 1, style: ParaStyle::Heading(1) },
                Placed { lines: 1, style: ParaStyle::Normal },
                Placed { lines: 1, style: ParaStyle::Bullet },
                Placed { lines: 1, style: ParaStyle::Bullet },
            ]
        );
    }

    /// The key the ignored real-model test uses: the environment first, then the suite's
    /// `ai-test.env` on the Desktop (never printed).
    fn real_key() -> String {
        if let Some(key) = std::env::var("OPENROUTER_API_KEY").ok().filter(|key| !key.trim().is_empty()) {
            return key.trim().to_string();
        }
        let path = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Desktop").join("ai-test.env");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("OPENROUTER_API_KEY or {}: {error}", path.display()));
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("OPENROUTER_API_KEY=") {
                return value.trim().trim_matches('"').to_string();
            }
        }
        panic!("OPENROUTER_API_KEY is in neither the environment nor {}", path.display());
    }

    /// The real-model end-to-end: network and a key, so `#[ignore]`. A free model works with the
    /// suite's zero-credit key. Run with
    /// `AI_TEST_MODEL=nvidia/nemotron-3-ultra-550b-a55b:free cargo test -p localoffice-ui-egui --lib ai_artifact::tests::a_real_model_writes_a_document -- --ignored --nocapture`.
    #[test]
    #[ignore = "network + a real provider key"]
    fn a_real_model_writes_a_document() {
        let key = real_key();
        let connection = localoffice_ai::Connection {
            id: "real".into(),
            provider: "openrouter".into(),
            label: "OpenRouter".into(),
            base_url: String::new(),
            model: std::env::var("AI_TEST_MODEL").unwrap_or_else(|_| "nvidia/nemotron-3-ultra-550b-a55b:free".into()),
        };
        let request = actions::Request::GenerateDoc {
            topic: "protocolo rápido de sepse para a equipe do pronto-socorro".into(),
            language: "Português (Brasil)".into(),
            shape: Some("a report".into()),
        };
        let messages = actions::messages(&request, "pt-br").expect("messages");
        let mut pieces = 0;
        let answer = localoffice_ai::chat_stream_with_key(&connection, &key, &messages, &mut |_: &str| pieces += 1).expect("a real answer");
        println!("--- answer ({pieces} pieces) ---\n{answer}\n--------------");
        let outcome = actions::interpret(&request, &answer).expect("the answer fits the contract");
        let mut s = session();
        let before = s.document_id();
        let copy = apply(&mut s, Kind::NewDoc, &outcome).expect("applied");
        assert!(copy.is_none(), "a new document writes no copy");
        assert_ne!(s.document_id(), before, "the document is the new one");
        let text = s.doc.plain_text(localoffice_doc::StoryRef::Body);
        println!("--- document ---\n{text}\n--------------");
        assert!(text.split_whitespace().count() > 40, "real content, not a stub: {text}");
        assert!(pieces > 1, "the answer streamed in pieces");
    }
}
