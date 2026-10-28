//! The AI assistant: a floating window with the user's own connections (BYOK), the model picker
//! that reads each provider's `/models`, a chat about the open document and a Summarize button.
//!
//! Nothing here talks to a LocalOffice server: there is none. Every call goes straight to the
//! provider the user configured, with the key the OS keychain holds (the shared `local-ai` crate
//! does the wire work). The panel is opened by `ui.ai` — Home ▸ AI — and its state lives in the
//! UI, not in the document session, so it never reaches a saved file.

use crate::WordApp;
use crate::theme::{self, Tokens};
use localoffice_ai::{Connection, Job, Msg, PROVIDERS, provider};

/// How much document text one chat request may carry (about 30k tokens of English).
const MAX_CONTEXT: usize = 120_000;

/// Saying, inside the prompt, that the document was longer than one request.
const CUT_NOTICE: &str = "\n[the document continues; the text was cut to fit one request]";

/// A chat turn as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Turn {
    User(String),
    Assistant(String),
    /// The request failed; shown apart, not part of the next request.
    Failed(String),
}

/// What the panel holds between frames.
#[derive(Default)]
pub struct AiState {
    /// The panel is showing (`ui.ai`).
    pub open: bool,
    /// The saved connections, read once when the panel first opens.
    pub connections: Vec<Connection>,
    pub loaded: bool,
    pub selected: usize,
    /// The key being typed for the selected connection (cleared once saved).
    pub key_draft: String,
    pub editing_key: bool,
    /// The models the provider answered with, and the fetch that is filling them.
    pub models: Vec<String>,
    pub models_job: Option<Job<Vec<String>>>,
    pub chat: Vec<Turn>,
    pub input: String,
    pub job: Option<Job<String>>,
    /// Chat about the selection instead of the whole document.
    pub selection_only: bool,
    /// The last error outside a turn (connections file, keychain, model list).
    pub error: Option<String>,
}

impl AiState {
    /// Read the saved connections once, on the first frame the panel is open.
    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        match localoffice_ai::load_all() {
            Ok(all) => self.connections = all,
            Err(e) => self.error = Some(e),
        }
    }

    fn selected_conn(&self) -> Option<&Connection> {
        self.connections.get(self.selected)
    }

    /// The connection the calls use, or why there is none.
    fn ready(&self) -> Result<Connection, String> {
        let conn = self.selected_conn().ok_or_else(|| tl!("Add a connection first").to_string())?;
        if !localoffice_ai::has_key(conn) {
            return Err(tl!("The connection has no API key").to_string());
        }
        if conn.model.is_empty() {
            return Err(tl!("Choose a model first").to_string());
        }
        Ok(conn.clone())
    }

    /// Write the connections file; a failure keeps showing, as the file is the setup.
    fn save(&mut self) {
        if let Err(e) = localoffice_ai::save_all(&self.connections) {
            self.error = Some(e);
        }
    }
}

/// Poll the background jobs; called every frame before the window draws, open or not, so an
/// answer that arrives while the panel is closed isn't lost.
fn poll(app: &mut WordApp) {
    if app.ai.models_job.as_ref().is_some_and(|j| j.poll().is_some()) {
        let job = app.ai.models_job.take();
        match job.and_then(|j| j.poll()) {
            Some(Ok(models)) => app.ai.models = models,
            Some(Err(e)) => app.ai.error = Some(e),
            None => {}
        }
    }
    if app.ai.job.as_ref().is_some_and(|j| j.poll().is_some()) {
        let job = app.ai.job.take();
        match job.and_then(|j| j.poll()) {
            Some(Ok(answer)) => app.ai.chat.push(Turn::Assistant(answer)),
            Some(Err(e)) => app.ai.chat.push(Turn::Failed(e)),
            None => {}
        }
    }
}

/// Whether the chat reads the whole document (the default) or just the selection.
fn reads_whole_document(app: &WordApp) -> bool {
    // With a collapsed caret there is no selection to read: the whole document is the only choice.
    !app.ai.selection_only || app.session.sel.is_collapsed()
}

/// The document text the chat sees: the whole body, or the selection, capped at [`MAX_CONTEXT`].
fn context_text(app: &WordApp) -> String {
    let text = if reads_whole_document(app) { app.session.doc.plain_text(localoffice_doc::StoryRef::Body) } else { app.session.selected_text() };
    capped(text)
}

/// Cut the context to one request's worth, on a character boundary, saying where it was cut.
fn capped(mut text: String) -> String {
    if text.len() <= MAX_CONTEXT {
        return text;
    }
    let mut cut = MAX_CONTEXT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str(CUT_NOTICE);
    text
}

/// The system turn: who the assistant is helping, and the document text it reads.
fn system_turn(app: &WordApp, framing: &str) -> Msg {
    let name = app.prompt_name();
    let context = context_text(app);
    Msg::System(format!(
        "{framing} The document is {name:?}. Its text follows after the marker. Answer in the user's language; be concise; quote the document when it helps.\n\n--- document text ---\n{context}"
    ))
}

/// Send the chat input: the doc context as the system turn, then the conversation so far.
fn send(app: &mut WordApp) {
    let input = std::mem::take(&mut app.ai.input);
    if input.trim().is_empty() {
        return;
    }
    let conn = match app.ai.ready() {
        Ok(c) => c,
        Err(e) => {
            app.ai.chat.push(Turn::Failed(e));
            return;
        }
    };
    let mut history = vec![system_turn(app, "You help with a document the user has open in LocalOffice.")];
    for turn in &app.ai.chat {
        match turn {
            Turn::User(t) => history.push(Msg::User(t.clone())),
            Turn::Assistant(t) => history.push(Msg::Assistant(t.clone())),
            Turn::Failed(_) => {}
        }
    }
    history.push(Msg::User(input.clone()));
    app.ai.chat.push(Turn::User(input));
    app.ai.job = Some(Job::spawn(move || localoffice_ai::chat(&conn, &history)));
}

/// Summarize: one user turn asking for a summary of the context.
fn summarize(app: &mut WordApp) {
    let conn = match app.ai.ready() {
        Ok(c) => c,
        Err(e) => {
            app.ai.chat.push(Turn::Failed(e));
            return;
        }
    };
    let framing = "You help with a document the user has open in LocalOffice. Answer in the document's language; be concise; use short paragraphs and, when it helps, a short list.";
    let mut history = vec![system_turn(app, framing)];
    history.push(Msg::User("Summarize this document: what it is, its main points and any actions it asks of the reader.".into()));
    app.ai.chat.push(Turn::User(tl!("Summarize this document.").to_string()));
    app.ai.job = Some(Job::spawn(move || localoffice_ai::chat(&conn, &history)));
}

/// The window. Called every frame from [`WordApp::show_overlays`](crate::WordApp).
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    poll(app);
    if !app.ai.open {
        return;
    }
    app.ai.load();
    let t = Tokens::get(ctx);
    let window = egui::Window::new(tl!("AI assistant"))
        .id(egui::Id::unique("ai-assistant"))
        .collapsible(false)
        .resizable(true)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(560.0)
        .default_height(620.0);
    let shown = crate::window_close::window(ctx, window).show(ctx, |ui| {
        ui.set_min_width(440.0);
        connections_row(app, ui);
        ui.add_space(6.0);
        model_row(app, ui);
        ui.add_space(6.0);
        if let Some(e) = app.ai.error.take() {
            ui.label(egui::RichText::new(e).color(t.red).small());
        }
        chat_area(app, ui, &t);
        input_row(app, ui);
    });
    if shown.is_some_and(|s| crate::window_close::show(ctx, &s.response, &t)) {
        app.ai.open = false;
    }
}

/// The connection combo and its buttons: key, new, delete.
fn connections_row(app: &mut WordApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(tl!("Connection"));
        let labels: Vec<String> = app.ai.connections.iter().map(|c| format!("{} — {}", c.label, provider_label(&c.provider))).collect();
        let current = labels.get(app.ai.selected).map(String::as_str).unwrap_or(tl!("add one…"));
        egui::ComboBox::from_id_salt("ai-connection").selected_text(current).show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                ui.selectable_value(&mut app.ai.selected, i, label);
            }
        });
        let has = app.ai.selected_conn().is_some();
        let keyed = app.ai.selected_conn().is_some_and(localoffice_ai::has_key);
        let key = ui.add_enabled(has, egui::Button::new(if keyed { "🔑 ✓" } else { "🔑" }));
        if key.on_hover_text(tl!("Set the API key (kept in the OS keychain)")).clicked() {
            app.ai.editing_key = !app.ai.editing_key;
            app.ai.key_draft.clear();
        }
        let add = ui.add(egui::Button::new("＋"));
        if add.on_hover_text(tl!("Add a connection")).clicked() {
            let mut n = app.ai.connections.len() + 1;
            while app.ai.connections.iter().any(|c| c.id == format!("conn-{n}")) {
                n += 1;
            }
            app.ai.connections.push(Connection {
                id: format!("conn-{n}"),
                provider: "gemini".into(),
                label: "Gemini".into(),
                base_url: String::new(),
                model: String::new(),
            });
            app.ai.selected = app.ai.connections.len() - 1;
            app.ai.editing_key = true;
            app.ai.key_draft.clear();
            app.ai.models.clear();
            app.ai.save();
        }
        let delete = ui.add_enabled(has, egui::Button::new("🗑"));
        if delete.on_hover_text(tl!("Delete this connection")).clicked() {
            if let Some(c) = app.ai.selected_conn() {
                let _ = localoffice_ai::clear_key(c);
            }
            app.ai.connections.remove(app.ai.selected);
            app.ai.selected = app.ai.selected.saturating_sub(1);
            app.ai.models.clear();
            app.ai.save();
        }
    });
    // The key editor and the connection's fields, under the row.
    if app.ai.selected_conn().is_some() && app.ai.editing_key {
        ui.horizontal(|ui| {
            let field = egui::TextEdit::singleline(&mut app.ai.key_draft).password(true).hint_text(tl!("API key (stays in the OS keychain)"));
            let response = ui.add(field);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tl!("API key")));
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(tl!("Save key")).clicked() || enter {
                let draft = std::mem::take(&mut app.ai.key_draft);
                if let Some(conn) = app.ai.selected_conn()
                    && let Err(e) = localoffice_ai::set_key(conn, draft.trim())
                {
                    app.ai.error = Some(e);
                }
                app.ai.editing_key = false;
            }
        });
    }
    if app.ai.selected_conn().is_some() {
        let name = app.ai.selected_conn().map(|c| c.label.clone()).unwrap_or_default();
        ui.collapsing(crate::i18n::fmt(tl!("{name} details"), &[("name", &name)]), |ui| {
            egui::Grid::new("ai-conn-details").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label(tl!("Provider"));
                egui::ComboBox::from_id_salt("ai-provider").selected_text(provider_label(&app.ai.connections[app.ai.selected].provider)).show_ui(
                    ui,
                    |ui| {
                        for p in PROVIDERS {
                            ui.selectable_value(&mut app.ai.connections[app.ai.selected].provider, p.id.to_owned(), p.label);
                        }
                    },
                );
                ui.end_row();
                ui.label(tl!("Name"));
                ui.add(egui::TextEdit::singleline(&mut app.ai.connections[app.ai.selected].label));
                ui.end_row();
                ui.label(tl!("Base URL"));
                let hint = provider(&app.ai.connections[app.ai.selected].provider).map(|p| p.base_url).unwrap_or("");
                ui.add(egui::TextEdit::singleline(&mut app.ai.connections[app.ai.selected].base_url).hint_text(hint));
                ui.end_row();
            });
            if ui.button(tl!("Save connection")).clicked() {
                app.ai.save();
            }
        });
    }
}

/// A provider preset's name as the shared crate lists it (a brand name, not translated).
fn provider_label(id: &str) -> String {
    provider(id).map(|p| p.label.to_owned()).unwrap_or_else(|| id.to_owned())
}

/// The model combo (the provider's own list) and its refresh button.
fn model_row(app: &mut WordApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(tl!("Model"));
        let fetching = app.ai.models_job.is_some();
        if fetching {
            ui.add(egui::Spinner::new().size(16.0));
        }
        let has = app.ai.selected_conn().is_some();
        let mut picked = false;
        if app.ai.models.is_empty() && has {
            // No list yet: a free-text model name still works.
            let field = egui::TextEdit::singleline(&mut app.ai.connections[app.ai.selected].model).hint_text(tl!("model id (or fetch the list)"));
            let r = ui.add(field);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tl!("Model")));
            if r.lost_focus() {
                picked = true;
            }
        } else if !app.ai.models.is_empty() {
            let current = app.ai.selected_conn().map(|c| c.model.clone()).unwrap_or_default();
            let shown = if current.is_empty() { tl!("choose…").to_string() } else { current };
            egui::ComboBox::from_id_salt("ai-model").selected_text(shown).show_ui(ui, |ui| {
                for m in &app.ai.models {
                    if ui.selectable_value(&mut app.ai.connections[app.ai.selected].model, m.clone(), m).clicked() {
                        picked = true;
                    }
                }
            });
        }
        let refresh = ui.add_enabled(!fetching && has, egui::Button::new("⟳"));
        if refresh.on_hover_text(tl!("Fetch the provider's model list")).clicked()
            && let Some(conn) = app.ai.selected_conn().cloned()
        {
            app.ai.models_job = Some(Job::spawn(move || localoffice_ai::list_models(&conn)));
        }
        if picked {
            app.ai.save();
        }
    });
}

/// The conversation, scrolled to the bottom.
fn chat_area(app: &mut WordApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let whole = reads_whole_document(app);
        let selection = ui.add_enabled(!app.session.sel.is_collapsed(), egui::Button::selectable(!whole, tl!("Selection")));
        if selection.on_hover_text(tl!("Chat about the selected text")).clicked() {
            app.ai.selection_only = true;
        }
        if ui.selectable_label(whole, tl!("Whole document")).clicked() {
            app.ai.selection_only = false;
        }
        let busy = app.ai.job.is_some();
        if ui.add_enabled(!busy, egui::Button::new(tl!("Summarize"))).clicked() {
            summarize(app);
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
    });
    ui.add_space(4.0);
    // A capped area: without it the chat asks for the whole screen while sizing and the window
    // grows past the user's resize.
    let chat_height = (ui.available_height() - 44.0).clamp(140.0, 460.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).max_height(chat_height).show(ui, |ui| {
        if app.ai.chat.is_empty() {
            ui.label(egui::RichText::new(tl!("Ask about the document, or press Summarize.")).color(t.text_dim));
        }
        for turn in &app.ai.chat {
            match turn {
                Turn::User(text) => {
                    ui.label(egui::RichText::new(tl!("You")).color(t.text_dim).small());
                    ui.label(egui::RichText::new(text).font(theme::regular(13.0)));
                }
                Turn::Assistant(text) => {
                    ui.separator();
                    ui.label(egui::RichText::new(tl!("Assistant")).color(t.text_dim).small());
                    ui.label(egui::RichText::new(text).font(theme::regular(13.0)));
                }
                Turn::Failed(text) => {
                    ui.label(egui::RichText::new(format!("✗ {text}")).color(t.red).small());
                }
            }
            ui.add_space(4.0);
        }
        if app.ai.job.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(egui::RichText::new(tl!("thinking…")).color(t.text_dim));
            });
        }
    });
}

/// The input box and Send.
fn input_row(app: &mut WordApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let mut send_now = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let busy = app.ai.job.is_some();
        if ui.add_enabled(!busy, egui::Button::new(tl!("Send"))).clicked() {
            send_now = true;
        }
        let field = egui::TextEdit::singleline(&mut app.ai.input)
            .hint_text(tl!("Ask about the document…"))
            .desired_width((ui.available_width() - 70.0).max(120.0))
            .lock_focus(true);
        let response = ui.add(field);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tl!("Ask about the document")));
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !busy {
            send_now = true;
        }
    });
    if send_now {
        send(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> WordApp {
        WordApp::new(localoffice_engine::Session::new(localoffice_engine::sample::sample_document()), crate::Services::default())
    }

    /// Home ▸ AI is a UI command: no value toggles the panel, `value` opens or closes it, and a
    /// value of another type is refused.
    #[test]
    fn the_panel_is_a_ui_command() {
        let mut a = app();
        assert!(!a.ai.open, "the panel starts closed");
        assert_eq!(a.run("ui.ai", json!({})).unwrap(), json!({"open": true}), "no value opens it");
        assert!(a.ai.open);
        assert_eq!(a.run("ui.ai", json!({})).unwrap(), json!({"open": false}), "and toggles it shut");
        assert_eq!(a.run("ui.ai", json!({"value": true})).unwrap(), json!({"open": true}));
        assert_eq!(a.run("ui.ai", json!({"value": false})).unwrap(), json!({"open": false}));
        for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
            assert!(a.run("ui.ai", junk).is_err());
        }
        assert!(!a.ai.open, "a rejected value leaves the panel as it was");
    }

    /// The context a request carries is cut to [`MAX_CONTEXT`] bytes on a character boundary, and
    /// says so where it was cut.
    #[test]
    fn the_context_is_cut_to_one_request() {
        assert_eq!(capped("short".into()), "short");
        let cut = capped("á".repeat(MAX_CONTEXT));
        assert_eq!(cut.len(), MAX_CONTEXT + CUT_NOTICE.len(), "one character over the cap is still cut whole");
        assert!(cut.ends_with(CUT_NOTICE));
        assert!(cut.is_char_boundary(MAX_CONTEXT), "the cut lands on a character boundary");
    }

    /// A fresh panel reads the whole document; choosing the selection with a collapsed caret
    /// still reads the document, since there is nothing selected.
    #[test]
    fn the_context_follows_the_selection() {
        let mut a = app();
        assert!(reads_whole_document(&a), "the whole document by default");
        a.ai.selection_only = true;
        assert!(reads_whole_document(&a), "a collapsed caret has no selection to read");
        a.run("select.all", json!({})).unwrap();
        assert!(!reads_whole_document(&a), "a selection is read instead");
        assert!(!context_text(&a).is_empty());
    }
}
