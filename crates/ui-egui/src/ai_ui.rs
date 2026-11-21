//! The AI panel: the suite's chat panel and connections form, over this app's BYOK client.
//!
//! The experience - the roll, the composer, the strip, the connections form and what actually goes
//! to the model - is `craft-ai-ui`, shared with the other apps of the suite. What lives here is the
//! app's side of the contract: the system prompt, the document as context, writing an answer into
//! the document, and driving the shared `local-ai` client with what the panel hands over.

use craft_ai_ui as ui;
use localoffice_ai as ai;
use localoffice_engine::Session;

use crate::WordApp;

/// How much of the document goes as context (the head: a document is read from the top).
const CONTEXT_LIMIT: usize = 60_000;

/// The whole request budget: the turns plus the document.
const BUDGET_CHARS: usize = 120_000;

/// The chat beside the document.
pub struct AiState {
    pub open: bool,
    pub settings_open: bool,
    panel: ui::ChatPanel,
    strip: ui::StatusStrip,
    connections: ui::ConnectionsPanel,
    /// The connection in use, loaded from the suite's store.
    connection: Option<ai::Connection>,
    /// Whether the store was read already (once per state: the panel is drawn every frame).
    loaded: bool,
    stream: Option<ai::Stream>,
    /// A model list fetched from the provider, waiting for the form.
    models_job: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// The artifact actions: a new document, or the open one rewritten into a copy beside it.
    pub artifact: crate::ai_artifact::Artifact,
    /// The last error outside the panel (the store, the keychain).
    pub error: Option<String>,
}

impl Default for AiState {
    fn default() -> Self {
        let mut panel = ui::ChatPanel::default();
        panel.set_policy(ui::WindowPolicy { turns: ui::Turns::LastN(8), total_budget_chars: BUDGET_CHARS });
        Self {
            open: false,
            settings_open: false,
            panel,
            strip: ui::StatusStrip::default(),
            connections: ui::ConnectionsPanel::default(),
            connection: None,
            loaded: false,
            stream: None,
            models_job: None,
            artifact: crate::ai_artifact::Artifact::default(),
            error: None,
        }
    }
}

impl AiState {
    /// Loads the saved connection, if there is one (once: the panel is drawn every frame).
    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        match ai::load_all() {
            Ok(all) => self.connection = all.into_iter().next(),
            Err(error) => self.error = Some(error),
        }
    }

    /// The connection in use, or why there is none.
    pub(crate) fn ready_connection(&self) -> Result<ai::Connection, String> {
        let connection = self.connection.clone().ok_or_else(|| tl!("Configure a connection first.").to_string())?;
        if !ai::has_key(&connection) {
            return Err(tl!("The connection has no API key").to_string());
        }
        if connection.model.trim().is_empty() {
            return Err(tl!("Choose a model first").to_string());
        }
        Ok(connection)
    }

    /// Whether the panel has somewhere to send a question.
    pub fn ready(&self) -> bool {
        self.connection.as_ref().is_some_and(|connection| ai::has_key(connection) && !connection.model.trim().is_empty())
    }

    /// The connection in use (for the audit's list of names that are the user's own).
    #[cfg(test)]
    pub(crate) fn connection(&self) -> Option<&ai::Connection> {
        self.connection.as_ref()
    }

    pub fn model(&self) -> &str {
        self.connection.as_ref().map(|connection| connection.model.as_str()).unwrap_or_default()
    }

    /// Whether anything is in flight (the command that toggles the panel keeps it open then).
    pub fn busy(&self) -> bool {
        self.stream.is_some() || self.panel.busy()
    }

    /// What arrived since the last frame. Called once per frame from the app's own loop.
    pub fn poll(&mut self) {
        if let Some(job) = &self.models_job {
            match job.try_recv() {
                Ok(Ok(models)) => {
                    self.connections.models = models;
                    self.models_job = None;
                }
                Ok(Err(error)) => {
                    self.connections.failed(error);
                    self.models_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.models_job = None,
            }
        }
        let Some(stream) = &self.stream else { return };
        let (pieces, done) = stream.poll();
        for piece in pieces {
            self.strip.push(&piece);
            self.panel.push(&piece);
        }
        if let Some(result) = done {
            self.strip.finish();
            self.panel.finish(result);
            self.stream = None;
        }
    }

    /// Closes the panel, stopping whatever was running.
    pub fn close(&mut self) {
        self.stop();
        self.open = false;
    }

    fn stop(&mut self) {
        if let Some(stream) = &self.stream {
            stream.cancel();
        }
        self.stream = None;
        self.strip.finish();
        self.panel.stopped();
    }

    /// Hands the panel's request to the shared client.
    fn start(&mut self) {
        let Some(lines) = self.panel.take_request() else { return };
        let connection = match self.ready_connection() {
            Ok(connection) => connection,
            Err(error) => {
                self.panel.finish(Err(error));
                return;
            }
        };
        let messages: Vec<ai::Msg> = lines
            .into_iter()
            .map(|line| match line.role {
                ui::Role::System => ai::Msg::System(line.text),
                ui::Role::User => ai::Msg::User(line.text),
                ui::Role::Assistant => ai::Msg::Assistant(line.text),
            })
            .collect();
        self.strip.begin();
        self.stream = Some(ai::Stream::start(connection, messages));
    }

    /// Writes the last answer into the document at the caret (typed text, so it is one edit).
    fn insert_answer(&mut self, session: &mut Session) -> bool {
        let Some(answer) = self.panel.last_answer().map(str::to_string) else { return false };
        if answer.trim().is_empty() {
            return false;
        }
        session.run("edit.pasteText", &serde_json::json!({ "text": answer.trim() })).is_ok()
    }

    /// The panel itself, drawn inside the app's right side panel.
    fn draw(&mut self, ui: &mut egui::Ui, session: &mut Session, context: Option<ui::ContextSource>, model: &str, ready: bool) {
        self.panel.set_system(SYSTEM_PROMPT);
        self.panel.set_context(context);
        let labels = chat_labels(model, ready);
        if self.strip.show(ui, ui::StripLabels { stop: tl!("Stop"), dismiss: tl!("Dismiss") }) == ui::StripAction::Stop {
            self.stop();
        }
        ui.separator();
        match self.panel.show(ui, &labels) {
            ui::ChatAction::None => {}
            ui::ChatAction::Send | ui::ChatAction::Retry => self.start(),
            ui::ChatAction::Stop => self.stop(),
            ui::ChatAction::Clear => self.panel.clear(),
            ui::ChatAction::Insert => {
                self.insert_answer(session);
            }
            ui::ChatAction::Settings => self.settings_open = true,
            ui::ChatAction::Close => self.close(),
        }
    }

    /// The connections form: the suite's dialog, with this app's store and keychain behind it.
    pub fn settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            self.connections.open = false;
            return;
        }
        if !self.connections.open {
            self.connections.set_presets(presets());
            self.connections.open_with(self.connection.as_ref().map(draft_of));
        }
        let labels = form_labels();
        match self.connections.show(ctx, &labels) {
            ui::ConnectionsIntent::None => {}
            ui::ConnectionsIntent::Cancel => {
                self.connections.open = false;
                self.settings_open = false;
            }
            ui::ConnectionsIntent::Save(draft, key) => self.save(draft, key),
            ui::ConnectionsIntent::LoadModels(draft, key) => self.fetch_models(draft, key),
        }
    }

    /// Stores the connection (key in the keychain) and keeps it in use.
    fn save(&mut self, draft: ui::ConnectionDraft, key: String) {
        let connection = connection_of(&draft);
        if let Err(error) = ai::set_key(&connection, &key) {
            self.connections.failed(error);
            return;
        }
        match ai::save_all(std::slice::from_ref(&connection)) {
            Ok(()) => {
                self.connection = Some(connection);
                self.panel.clear();
                self.connections.saved();
                self.settings_open = false;
            }
            Err(error) => self.connections.failed(error),
        }
    }

    /// Asks the provider for its model list, on a worker thread.
    fn fetch_models(&mut self, draft: ui::ConnectionDraft, key: String) {
        let connection = connection_of(&draft);
        if let Err(error) = ai::set_key(&connection, &key) {
            self.connections.failed(error);
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(ai::list_models(&connection));
        });
        self.models_job = Some(rx);
    }
}

/// The system prompt: the same for every turn of the conversation (model-facing text, not UI).
const SYSTEM_PROMPT: &str = "You are a writing assistant inside a word processor. Answer in the \
     language of the user's message, briefly, in whole sentences. When the user asks for the \
     document's text, write it as text they can insert; never claim to have changed the document \
     yourself.";

/// The document as the panel's context: the head of the body, within the budget.
fn context_source(session: &Session) -> Option<ui::ContextSource> {
    let text = session.doc.plain_text(localoffice_doc::StoryRef::Body);
    if text.trim().is_empty() {
        return None;
    }
    Some(ui::ContextSource::new(ui::ContextKind::Document, tl!("the document"), text, CONTEXT_LIMIT, ui::Side::Head))
}

/// The chat labels, in the app's language.
fn chat_labels<'a>(model: &'a str, ready: bool) -> ui::ChatLabels<'a> {
    ui::ChatLabels {
        title: tl!("AI assistant"),
        not_configured: if ready { "" } else { tl!("Configure a connection first.") },
        configure: tl!("Settings"),
        settings: tl!("Settings"),
        clear: tl!("Clear"),
        close: tl!("Close"),
        send: tl!("Send"),
        stop: tl!("Stop"),
        insert: tl!("Insert"),
        retry: tl!("Retry"),
        dismiss: tl!("Dismiss"),
        empty: tl!("Ask about the document."),
        input_hint: tl!("Write to the assistant."),
        chip: &chip,
        model,
        suggestions: &[],
        to_end: tl!("To the end"),
    }
}

/// The context chip: what actually goes to the model.
fn chip(info: &ui::ContextInfo) -> String {
    crate::i18n::fmt(
        tl!("reading: {label} · {sent} of {total} chars · {kept} of {turns} turns"),
        &[
            ("label", &info.label),
            ("sent", &info.sent.to_string()),
            ("total", &info.total.to_string()),
            ("kept", &info.kept.to_string()),
            ("turns", &info.turns.to_string()),
        ],
    )
}

fn form_labels<'a>() -> ui::ConnectionsLabels<'a> {
    ui::ConnectionsLabels {
        title: tl!("AI connections"),
        provider: tl!("Provider"),
        base_url: tl!("Base URL"),
        model: tl!("Model"),
        api_key: tl!("API key"),
        list_models: tl!("Fetch models"),
        save: tl!("Save"),
        cancel: tl!("Cancel"),
        hint_url: tl!("Empty uses the provider's own address."),
        hint_key: tl!("The key stays in the operating system's keychain."),
        close: tl!("Close"),
        needs_key: tl!("Paste the API key first: this provider asks for one."),
        needs_url: tl!("Choose a provider or type its address first."),
        needs_model: tl!("Choose a model first (Fetch models lists them)."),
    }
}

fn presets() -> Vec<ui::Preset> {
    ai::PROVIDERS
        .iter()
        .map(|provider| ui::Preset { id: provider.id.to_owned(), label: provider.label.to_owned(), base_url: provider.base_url.to_owned() })
        .collect()
}

fn draft_of(connection: &ai::Connection) -> ui::ConnectionDraft {
    ui::ConnectionDraft { provider: connection.provider.clone(), base_url: connection.base_url.clone(), model: connection.model.clone() }
}

fn connection_of(draft: &ui::ConnectionDraft) -> ai::Connection {
    let provider = ai::provider(&draft.provider);
    let label = provider.map(|preset| preset.label).unwrap_or(draft.provider.as_str());
    ai::Connection {
        id: ai::APP.id.to_lowercase(),
        provider: draft.provider.clone(),
        label: label.to_string(),
        base_url: draft.base_url.clone(),
        model: draft.model.clone(),
    }
}

/// The AI panel beside the document. Drawn from the app's layout, before the page.
pub fn panel(app: &mut WordApp, ui: &mut egui::Ui) {
    if !app.ai.open {
        return;
    }
    app.ai.load();
    let context = context_source(&app.session);
    let WordApp { ai, session, .. } = app;
    let model = ai.model().to_string();
    let ready = ai.ready();
    egui::Panel::right("ai-chat").resizable(true).default_size(360.0).max_size(560.0).show(ui, |ui| {
        ai.artifact.controls(ui);
        ai.draw(ui, session, context, &model, ready);
    });
}

/// The connections dialog (a modal over the window), the artifact dialog and the requests in
/// flight: drawn where the other dialogs are.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    app.ai.poll();
    let notice = {
        let WordApp { ai, session, .. } = app;
        ai.artifact.poll(session)
    };
    if let Some(notice) = notice {
        app.status(notice);
    }
    let WordApp { ai, session, .. } = app;
    let connection = ai.ready_connection();
    ai.artifact.dialog(session, connection, ctx);
    ai.settings(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> WordApp {
        WordApp::new(localoffice_engine::Session::new(localoffice_engine::sample::sample_document()), crate::Services::default())
    }

    /// Home ? AI is a UI command: no value toggles the panel, `value` opens or closes it, and a
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

    /// The document travels as the panel's context, with a budget and the head as the side.
    #[test]
    fn the_document_is_the_panels_context() {
        let a = app();
        let context = context_source(&a.session).expect("the sample document");
        assert!(!context.text.trim().is_empty());
        assert_eq!(context.budget_chars, CONTEXT_LIMIT);
        assert_eq!(context.side, ui::Side::Head);
    }

    /// The policy the panel asks for: the last 8 turns and one budget for everything that leaves.
    #[test]
    fn the_panel_keeps_the_last_turns_within_the_budget() {
        let a = app();
        let policy = a.ai.panel.policy();
        assert_eq!(policy.turns, ui::Turns::LastN(8));
        assert_eq!(policy.total_budget_chars, BUDGET_CHARS);
    }

    impl AiState {
        /// Visual QA: a saved connection and a transcript with markdown, without a request.
        #[cfg(test)]
        pub(crate) fn seed_for_visual_qa(&mut self) {
            self.connection = Some(ai::Connection {
                id: "qa".into(),
                provider: "custom".into(),
                label: "Local QA".into(),
                base_url: String::new(),
                model: "qa-model".into(),
            });
            self.panel.push_answer(
                "The **Open Studio Handbook** in *plain* words:\n\n- **Objective:** a shared atelier.\n- `Rule`: clean as you go.\n\n### Notes\nAnyone can join.",
            );
        }
    }

    /// Visual QA (ignored in CI): renders the panel and the artifact dialog in both themes to PNGs
    /// for the vision pass. Run with
    /// `cargo test -p localoffice-ui-egui --lib ai_ui::tests::renders_the_ai_surfaces_for_visual_qa -- --ignored --nocapture`.
    #[test]
    #[ignore = "visual QA: renders the AI surfaces to PNGs for review"]
    fn renders_the_ai_surfaces_for_visual_qa() {
        fn save(h: &mut egui_kittest::Harness<'_, WordApp>, name: &str) {
            h.run_steps(3);
            let image = h.render().expect("render");
            let path = std::env::temp_dir().join(format!("localoffice-{name}.png"));
            image.save(&path).expect("save");
            println!("{}", path.display());
        }
        for (name, theme) in [("light", egui::Theme::Light), ("dark", egui::Theme::Dark)] {
            let mut a = app();
            a.ai.open = true;
            a.ai.seed_for_visual_qa();
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1100.0, 760.0))
                .build_ui_state(|ui, app: &mut WordApp| crate::ai_ui::panel(app, ui), a);
            h.ctx.set_theme(theme);
            save(&mut h, &format!("ai-panel-{name}"));

            let mut b = app();
            b.ai.artifact.dialog = true;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    crate::ai_ui::show(app, &ctx);
                },
                b,
            );
            h.ctx.set_theme(theme);
            save(&mut h, &format!("ai-dialog-{name}"));

            // The whole shell, with the panel beside the page.
            let mut c = app();
            c.ai.open = true;
            c.ai.seed_for_visual_qa();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                c,
            );
            h.ctx.set_theme(theme);
            save(&mut h, &format!("ai-shell-{name}"));
        }
    }
}
