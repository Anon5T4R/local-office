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
/// answer that arrives while the panel is closed isn't lost. One `poll` per job: the first
/// `try_recv` takes the result out of the channel, so a second one would find nothing and
/// swallow it — the answer and the model list both went missing that way.
fn poll(app: &mut WordApp) {
    if let Some(result) = app.ai.models_job.as_ref().and_then(|j| j.poll()) {
        app.ai.models_job.take();
        match result {
            Ok(models) => app.ai.models = models,
            Err(e) => app.ai.error = Some(e),
        }
    }
    if let Some(result) = app.ai.job.as_ref().and_then(|j| j.poll()) {
        app.ai.job.take();
        match result {
            Ok(answer) => app.ai.chat.push(Turn::Assistant(answer)),
            Err(e) => app.ai.chat.push(Turn::Failed(e)),
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

/// How one run of an assistant answer is marked up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MdStyle {
    /// The text as it came.
    Plain,
    /// A `**…**` pair.
    Bold,
    /// A single `*` pair.
    Italic,
    /// Between backticks.
    Code,
}

/// A run of an answer's line, cut where its marking changes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MdSeg {
    text: String,
    style: MdStyle,
}

/// One line of an answer: ordinary text, a list item (`- `) or a heading (`# s`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum MdLine {
    Text(Vec<MdSeg>),
    Item(Vec<MdSeg>),
    Heading(Vec<MdSeg>),
}

/// The light markdown over assistant answers: `- ` opens a list item, a run of `#`s and a space
/// a heading, and the inline pairs (`**bold**`, `*italic*`, backticks) cut their runs apart
/// ([`md_segs`]). Just enough to read an answer at a glance: links, tables and fences are none
/// of its business, and the user's turns are always drawn as they typed them.
fn md(text: &str) -> Vec<MdLine> {
    text.split('\n')
        .map(|line| {
            let hashes = line.chars().take_while(|&c| c == '#').count();
            // `#` is ASCII, so the character count of the opening run is its width in bytes.
            if hashes > 0
                && let Some(rest) = line[hashes..].strip_prefix(' ')
            {
                return MdLine::Heading(md_segs(rest));
            }
            if let Some(rest) = line.strip_prefix("- ") {
                return MdLine::Item(md_segs(rest));
            }
            MdLine::Text(md_segs(line))
        })
        .collect()
}

/// The runs of one line: `**bold**`, `*italic*` and `` `code` ``, plain text apart from those.
/// A marker without its pair later on the line is literal text, so `2 * 3` draws as typed, and
/// runs don't nest: each opener closes at the first pair that answers it.
fn md_segs(line: &str) -> Vec<MdSeg> {
    let cs: Vec<char> = line.chars().collect();
    let mut out: Vec<MdSeg> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < cs.len() {
        // A styled run when the marker at `i` finds its pair: (style, run, first byte past it).
        let run = if cs[i] == '`'
            && let Some(end) = (i + 1..cs.len()).find(|&k| cs[k] == '`')
        {
            Some((MdStyle::Code, i + 1, end, end + 1))
        } else if cs[i] == '*'
            && let Some(run) = md_star_pair(&cs, i)
        {
            Some(run)
        } else {
            None
        };
        match run {
            Some((style, start, end, next)) => {
                if !plain.is_empty() {
                    out.push(MdSeg { text: std::mem::take(&mut plain), style: MdStyle::Plain });
                }
                if start < end {
                    out.push(MdSeg { text: cs[start..end].iter().collect(), style });
                }
                i = next;
            }
            None => {
                plain.push(cs[i]);
                i += 1;
            }
        }
    }
    if !plain.is_empty() {
        out.push(MdSeg { text: plain, style: MdStyle::Plain });
    }
    out
}

/// What the `*` at index `i` opens: a `**` pair (bold) with where it closes, or the next single
/// `*` (italic). `None` when the rest of the line has no closer, leaving the `*` literal text.
fn md_star_pair(cs: &[char], i: usize) -> Option<(MdStyle, usize, usize, usize)> {
    if cs.get(i + 1) == Some(&'*') {
        let end = (i + 2..cs.len()).find(|&k| cs[k] == '*' && cs.get(k + 1) == Some(&'*'))?;
        return Some((MdStyle::Bold, i + 2, end, end + 2));
    }
    let end = (i + 1..cs.len()).find(|&k| cs[k] == '*')?;
    Some((MdStyle::Italic, i + 1, end, end + 1))
}

/// The system turn: who the assistant is helping, and the document text it reads.
fn system_turn(app: &WordApp, framing: &str) -> Msg {
    let name = app.prompt_name();
    let context = context_text(app);
    Msg::System(format!(
        "{framing} The document is {name:?}. Its text follows after the marker. Answer in the user's language; be concise; quote the document when it helps.\n\n--- document text ---\n{context}"
    ))
}

/// Send the chat input: the doc context as the system turn, then the conversation so far. The
/// connection is checked before the input is taken: with none ready the question stays in the
/// box, ready to go once one is, instead of vanishing.
fn send(app: &mut WordApp) {
    let conn = match app.ai.ready() {
        Ok(c) => c,
        Err(e) => {
            app.ai.chat.push(Turn::Failed(e));
            return;
        }
    };
    let input = std::mem::take(&mut app.ai.input);
    if input.trim().is_empty() {
        return;
    }
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
        connections_row(app, ui, &t);
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

/// The connection combo and its buttons: key, new, delete. The buttons are the house-style
/// small icons (`chrome::small_icon`), with the key one showing a check once the key is set.
fn connections_row(app: &mut WordApp, ui: &mut egui::Ui, t: &Tokens) {
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
        if has {
            let keyed = app.ai.selected_conn().is_some_and(localoffice_ai::has_key);
            let key_tip =
                if keyed { tl!("The key is set (in the OS keychain); click to change it") } else { tl!("Set the API key (kept in the OS keychain)") };
            if crate::chrome::small_icon(ui, if keyed { "check" } else { "protect" }, key_tip) {
                app.ai.editing_key = !app.ai.editing_key;
                app.ai.key_draft.clear();
            }
        }
        if crate::chrome::small_icon(ui, "plus", tl!("Add a connection")) {
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
        if has {
            let delete_tip = tl!("Delete this connection");
            if crate::chrome::small_icon(ui, "close", delete_tip) {
                if let Some(c) = app.ai.selected_conn() {
                    let _ = localoffice_ai::clear_key(c);
                }
                app.ai.connections.remove(app.ai.selected);
                app.ai.selected = app.ai.selected.saturating_sub(1);
                app.ai.models.clear();
                app.ai.save();
            }
        }
    });
    // Nothing set up yet: say what a connection is, so the panel never looks like it refuses to
    // work.
    if app.ai.connections.is_empty() {
        ui.label(egui::RichText::new(tl!("No connection yet: press the add button beside Connection, choose a provider, paste its API key, then pick a model. The key stays in the OS keychain.")).color(t.text_dim).small());
    }
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
                let provider_before = app.ai.connections[app.ai.selected].provider.clone();
                egui::ComboBox::from_id_salt("ai-provider").selected_text(provider_label(&app.ai.connections[app.ai.selected].provider)).show_ui(
                    ui,
                    |ui| {
                        for p in PROVIDERS {
                            ui.selectable_value(&mut app.ai.connections[app.ai.selected].provider, p.id.to_owned(), p.label);
                        }
                    },
                );
                if app.ai.connections[app.ai.selected].provider != provider_before {
                    app.ai.models.clear();
                }
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
        if !fetching
            && has
            && crate::chrome::small_icon(ui, "update", tl!("Fetch the provider's model list"))
            && let Some(conn) = app.ai.selected_conn().cloned()
        {
            app.ai.models_job = Some(Job::spawn(move || localoffice_ai::list_models(&conn)));
        }
        if picked {
            app.ai.save();
        }
    });
}

/// Whether Clear is offered: with turns above and the panel quiet, since an answer still in
/// flight must keep its chat to land beside the question that asked for it.
fn clear_ready(ai: &AiState) -> bool {
    !ai.chat.is_empty() && ai.job.is_none()
}

/// The chat row's Clear, the small icons (the `chrome::small_icon` drawing) with the enabled
/// state greyed and detached while there is nothing to clear or a request is running.
fn clear_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let resp = ui.interact(r, clear_button_id(), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let c = if enabled { t.icon } else { t.text_disabled };
    crate::icons::paint(ui.painter(), r.shrink(3.0), "clearChat", c, t.accent);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tl!("Clear the conversation")));
    resp.on_hover_text(tl!("Clear the conversation")).clicked()
}

/// The Clear button's id, so tests can reach it where the row puts it.
fn clear_button_id() -> egui::Id {
    egui::Id::unique("ai-clear-chat")
}

/// An assistant turn drawn as light markdown: one wrapped block with the [`MdLine`]s back in
/// order — a heading a size up in the semibold face, list items behind a dim bullet, bold in
/// the house semibold, italic slanted, inline code in the monospace face over the house accent.
/// Plain prose wraps as one run, like any label.
fn show_assistant_turn(ui: &mut egui::Ui, text: &str, t: &Tokens) {
    let mut job = egui::text::LayoutJob::default();
    for (n, line) in md(text).into_iter().enumerate() {
        if n > 0 {
            job.append("\n", 0.0, egui::text::TextFormat::simple(theme::regular(13.0), t.text));
        }
        let (segs, heading) = match &line {
            MdLine::Item(segs) => {
                job.append("•  ", 0.0, egui::text::TextFormat::simple(theme::regular(13.0), t.text_dim));
                (segs, false)
            }
            MdLine::Heading(segs) => (segs, true),
            MdLine::Text(segs) => (segs, false),
        };
        for seg in segs {
            if seg.text.is_empty() {
                continue;
            }
            let format = match (heading, seg.style) {
                (_, MdStyle::Code) => egui::text::TextFormat::simple(egui::FontId::monospace(12.0), t.accent_text),
                (_, MdStyle::Bold) => egui::text::TextFormat::simple(theme::semibold(if heading { 15.0 } else { 13.0 }), t.text),
                (true, _) => egui::text::TextFormat::simple(theme::semibold(15.0), t.text),
                (false, MdStyle::Italic) => {
                    egui::text::TextFormat { font_id: theme::regular(13.0), color: t.text, italics: true, ..egui::text::TextFormat::default() }
                }
                (false, MdStyle::Plain) => egui::text::TextFormat::simple(theme::regular(13.0), t.text),
            };
            job.append(&seg.text, 0.0, format);
        }
    }
    if job.text.is_empty() {
        return;
    }
    job.wrap.max_width = ui.available_width();
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, t.text);
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
        // On the row's right end, out of the reading line: Clear, when it applies.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if clear_button(ui, clear_ready(&app.ai)) {
                app.ai.chat.clear();
            }
        });
    });
    ui.add_space(4.0);
    // A capped area: without it the chat asks for the whole screen while sizing and the window
    // grows past the user's resize. Bottom-up: the newest turn sits just above the input and the
    // rest scrolls upward, so a chat fills from the bottom of its box (the pdf's lesson).
    let chat_height = (ui.available_height() - 44.0).clamp(140.0, 460.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).max_height(chat_height).show(ui, |ui| {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if app.ai.job.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(egui::RichText::new(tl!("thinking…")).color(t.text_dim));
                });
                ui.add_space(6.0);
            }
            for turn in app.ai.chat.iter().rev() {
                ui.add_space(6.0);
                match turn {
                    Turn::User(text) => {
                        ui.label(egui::RichText::new(text).font(theme::regular(13.0)));
                        ui.label(egui::RichText::new(tl!("You")).color(t.text_dim).small());
                    }
                    Turn::Assistant(text) => {
                        show_assistant_turn(ui, text, t);
                        ui.label(egui::RichText::new(tl!("Assistant")).color(t.text_dim).small());
                        ui.separator();
                    }
                    Turn::Failed(text) => {
                        ui.label(egui::RichText::new(format!("✗ {text}")).color(t.red).small());
                        ui.add_space(4.0);
                    }
                }
            }
            if app.ai.chat.is_empty() && app.ai.job.is_none() {
                ui.label(egui::RichText::new(tl!("Ask about the document, or press Summarize.")).color(t.text_dim));
                ui.add_space(12.0);
            }
        });
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

    /// Visual QA (ignored in CI): renders the assistant panel with a markdown answer to a PNG
    /// for the vision pass, the pdf's pipeline. Run with
    /// `cargo test -p localoffice-ui-egui --lib ai_ui::tests::renders_the_panel_for_visual_qa -- --ignored`.
    #[test]
    #[ignore = "visual QA: renders the AI panel to a PNG for review"]
    fn renders_the_panel_for_visual_qa() {
        let mut a = app();
        // Hermetic: don't read the user's real connections; a saved answer with markdown.
        a.ai.loaded = true;
        a.ai.open = true;
        a.ai.chat = vec![
            Turn::User("resuma".into()),
            Turn::Assistant("The **Open Studio Handbook** in *plain* words:\n\n- **Objective:** a shared atelier.\n- `Rule`: clean as you go.\n\n### Notes\nAnyone can join.".into()),
        ];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_accessibility_check(false).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                ctx.set_theme(egui::Theme::Dark);
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..8 {
            h.step();
        }
        let img = h.render().expect("render");
        let out = std::env::temp_dir().join("localoffice-ai-markdown.png");
        img.save(&out).expect("save png");
        eprintln!("saved {}", out.display());
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

    /// One `poll` per job: the answer, the error and the model list each arrive once and are
    /// kept. Polling twice drained the channel and swallowed them (#ai).
    #[test]
    fn polling_keeps_the_answer_the_error_and_the_models() {
        let mut a = app();
        a.ai.job = Some(Job::spawn(|| Ok("olá".into())));
        wait_polled(&mut a);
        assert_eq!(a.ai.chat, vec![Turn::Assistant("olá".into())], "the answer lands");
        assert!(a.ai.job.is_none(), "the job is done");

        a.ai.chat.clear();
        a.ai.job = Some(Job::spawn(|| Err("provider down".into())));
        wait_polled(&mut a);
        assert_eq!(a.ai.chat.last(), Some(&Turn::Failed("provider down".into())), "the error lands too");

        a.ai.models_job = Some(Job::spawn(|| Ok(vec!["gemini-2.5-flash".into()])));
        wait_polled(&mut a);
        assert_eq!(a.ai.models, vec!["gemini-2.5-flash".to_string()], "the model list lands");
        assert!(a.ai.models_job.is_none(), "the fetch is done");
        assert!(a.ai.error.is_none());
    }

    /// Poll until both jobs are done: the spawned thread needs a moment to hand its result over.
    fn wait_polled(a: &mut WordApp) {
        for _ in 0..500 {
            poll(a);
            if a.ai.job.is_none() && a.ai.models_job.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// With no connection ready the typed question stays in the input box, not lost to a failed
    /// send, and the reason shows as a failed turn.
    #[test]
    fn a_send_without_a_ready_connection_keeps_the_question() {
        let mut a = app();
        a.ai.input = "o que este documento pede?".into();
        send(&mut a);
        assert_eq!(a.ai.input, "o que este documento pede?", "the question stays in the box");
        assert!(matches!(a.ai.chat.first(), Some(Turn::Failed(_))), "the reason shows as a turn");
        assert!(a.ai.job.is_none(), "nothing was sent");
    }

    /// The light markdown an assistant answer parses: the inline pairs split their runs, `- `
    /// opens a marker, `### ` a heading, an empty line is the paragraph break, and a marker
    /// without its pair stays literal text (`2 * 3` draws as typed).
    #[test]
    fn assistant_answers_take_light_markdown() {
        let s = |text: &str, style: MdStyle| MdSeg { text: text.into(), style };
        assert_eq!(
            md_segs("Pano: **feito**, *rascunho*, `código`, 2 * 3"),
            vec![
                s("Pano: ", MdStyle::Plain),
                s("feito", MdStyle::Bold),
                s(", ", MdStyle::Plain),
                s("rascunho", MdStyle::Italic),
                s(", ", MdStyle::Plain),
                s("código", MdStyle::Code),
                s(", 2 * 3", MdStyle::Plain),
            ],
            "the pairs split off, the stray `*` keeps its text"
        );
        assert_eq!(md_segs("a ** b ` c"), vec![s("a ** b ` c", MdStyle::Plain)], "an unclosed pair is not markup");
        assert_eq!(
            md_segs("um `code` e `outro`"),
            vec![s("um ", MdStyle::Plain), s("code", MdStyle::Code), s(" e ", MdStyle::Plain), s("outro", MdStyle::Code)]
        );
        assert_eq!(
            md("- **Objetivo:** sair cedo"),
            vec![MdLine::Item(vec![s("Objetivo:", MdStyle::Bold), s(" sair cedo", MdStyle::Plain)])],
            "the `- ` marker is read off and the run under it stays marked up"
        );
        assert_eq!(md("### Panorama"), vec![MdLine::Heading(vec![s("Panorama", MdStyle::Plain)])]);
        assert_eq!(md("## *Primeiro*"), vec![MdLine::Heading(vec![s("Primeiro", MdStyle::Italic)])]);
        assert_eq!(md("###no"), vec![MdLine::Text(vec![s("###no", MdStyle::Plain)])], "no space, no heading");
        assert_eq!(md("two hashes and no room"), vec![MdLine::Text(vec![s("two hashes and no room", MdStyle::Plain)])]);
        assert_eq!(
            md("um\n\ndois"),
            vec![MdLine::Text(vec![s("um", MdStyle::Plain)]), MdLine::Text(vec![]), MdLine::Text(vec![s("dois", MdStyle::Plain)])],
            "an empty line is a paragraph break"
        );
    }

    /// Clear only applies when the chat has turns and the panel is quiet: the answer of a
    /// request still in flight must arrive beside the question that asked for it.
    #[test]
    fn clear_waits_for_a_resting_nonempty_chat() {
        let mut a = app();
        assert!(!clear_ready(&a.ai), "an empty chat has nothing to clear");
        a.ai.chat.push(Turn::Assistant("**Feito**. `um`".into()));
        assert!(clear_ready(&a.ai), "turns above: Clear applies");
        a.ai.job = Some(Job::spawn(|| Ok("still coming".into())));
        assert!(!clear_ready(&a.ai), "not while the answer is in flight");
        wait_polled(&mut a);
        assert!(clear_ready(&a.ai), "and the panel is quiet again");
    }

    /// The Clear button, drawn on the actions row's right end, takes the whole conversation
    /// when clicked, and offers no click any more over an empty chat.
    #[test]
    fn the_clear_button_empties_the_chat() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.ai.open = true;
        a.ai.chat = vec![Turn::User("o que este documento pede?".into()), Turn::Assistant("### Panorama\n- **Objetivo:** revisar `-` e `a*`".into())];
        // The panel's combos name themselves through their `selected_text`, not a label, and
        // this check is what `chrome.rs`' harnesses pay for its chrome; off here.
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 750.0)).with_accessibility_check(false).build_ui_state(
            |ui, app: &mut WordApp| {
                // The panel's TextFormats name the app's families: install them like `WordApp`
                // does (the egui test root needs the faces declared to draw anything).
                let ctx = ui.ctx().clone();
                crate::theme::install_fonts(&ctx);
                crate::theme::apply(&ctx, crate::theme::Appearance::Light);
                show(app, &ctx);
            },
            a,
        );
        h.run_steps(3);
        h.get_by_label("Clear the conversation").click();
        h.run_steps(3);
        assert!(h.state().ai.chat.is_empty(), "clearing drops every turn, user's and assistant's alike");
        assert!(!clear_ready(&h.state().ai), "and the button is not offered again");
    }
}
