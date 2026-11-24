//! BYOK AI for LocalWriter: the user's own keys, one OpenAI-compatible client for every provider.
//!
//! Thin wrapper over the shared [`local_ai`] crate with LocalWriter's identity (config folder,
//! keychain service, OpenRouter headers). Nothing here talks to a LocalWriter server: there is
//! none. A connection is a provider preset (base URL), a model name the picker fetches from the
//! provider's own `/models`, and a secret the OS keychain holds. The saved shape is the shared
//! crate's [`Connection`], so the connections file is the suite's one file format.

use std::path::PathBuf;

pub mod actions;
pub mod prompts;

pub use local_ai::{Connection, Job, Msg, PROVIDERS, Provider, provider};

/// LocalWriter's identity in the shared `local-ai` crate: config folder and keychain service both
/// `"LocalWriter"`, and the OpenRouter referer points at this repository.
pub const APP: local_ai::App = local_ai::App { id: "LocalWriter", url: "https://github.com/Styx-Studios/Local-Writer" };

/// The identity under the product's old name (Local Word, then LocalOffice): the data folder and
/// the keychain service the connections and their keys were saved under before the rename.
pub const LEGACY_APP: local_ai::App = local_ai::App { id: "LocalOffice", url: APP.url };

/// Where the connections file lives: `<data dir>/LocalWriter/ai-connections.json`.
pub fn store_path() -> Option<PathBuf> {
    Connection::store_path(&APP)
}

/// The folder the connections file lives in (`<data dir>/LocalWriter`), when the system has a data
/// dir. The app moves the old folder here on the first start after the rename.
pub fn data_dir() -> Option<PathBuf> {
    store_path().and_then(|file| file.parent().map(std::path::Path::to_path_buf))
}

/// The folder the connections file lived in under the old name.
pub fn legacy_data_dir() -> Option<PathBuf> {
    Connection::store_path(&LEGACY_APP).and_then(|file| file.parent().map(std::path::Path::to_path_buf))
}

/// Copies the API keys saved under the old keychain service to the current one, for every saved
/// connection that has none yet. A key already under the current service is never replaced, the
/// old entries stay where they are, and a keychain that can't be read is skipped (logged, never
/// fatal). Returns how many keys were copied.
pub fn migrate_legacy_keys() -> usize {
    let all = match load_all() {
        Ok(all) => all,
        Err(e) => {
            log::warn!("legacy AI keys: the connections can't be read: {e}");
            return 0;
        }
    };
    migrate_keys(&all, |c| c.has_key(&APP), |c| c.key(&LEGACY_APP).unwrap_or_default(), |c, key| c.set_key(&APP, key))
}

/// [`migrate_legacy_keys`] with the keychain behind three closures (so it can be tested).
fn migrate_keys(
    all: &[Connection],
    has_current: impl Fn(&Connection) -> bool,
    legacy_key: impl Fn(&Connection) -> String,
    set_current: impl Fn(&Connection, &str) -> Result<(), String>,
) -> usize {
    let mut copied = 0;
    for connection in all {
        if has_current(connection) {
            continue;
        }
        let key = legacy_key(connection);
        if key.is_empty() {
            continue;
        }
        match set_current(connection, &key) {
            Ok(()) => {
                log::info!("copied the API key of connection `{}` to the Local Writer keychain entry", connection.id);
                copied += 1;
            }
            Err(e) => log::warn!("couldn't copy the API key of connection `{}`: {e}", connection.id),
        }
    }
    copied
}

/// Load every saved connection. A missing file is no connections, not an error; a broken one is an
/// error, so a silent wipe can never pretend the setup is gone.
pub fn load_all() -> Result<Vec<Connection>, String> {
    Connection::load_all(&APP)
}

/// Save every connection: write a sibling then rename over the real file.
pub fn save_all(all: &[Connection]) -> Result<(), String> {
    Connection::save_all(&APP, all)
}

/// Put this connection's key in the OS keychain.
pub fn set_key(conn: &Connection, key: &str) -> Result<(), String> {
    conn.set_key(&APP, key)
}

/// This connection's key, empty when none is saved.
pub fn key(conn: &Connection) -> Result<String, String> {
    conn.key(&APP)
}

/// Whether the OS keychain holds a key for this connection.
pub fn has_key(conn: &Connection) -> bool {
    conn.has_key(&APP)
}

/// Forget this connection's key.
pub fn clear_key(conn: &Connection) -> Result<(), String> {
    conn.clear_key(&APP)
}

/// A chat request: the whole conversation, the document context already folded into the turns by
/// the caller.
pub fn chat(conn: &Connection, history: &[Msg]) -> Result<String, String> {
    local_ai::chat(&APP, conn, history)
}

/// One streamed answer: `on_delta` receives each piece as it arrives and the whole text comes back.
pub fn chat_stream(conn: &Connection, history: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String> {
    local_ai::chat_stream(&APP, conn, history, on_delta)
}

/// [`chat_stream`] with the key handed in instead of read from the OS keychain: for the tests that
/// exercise the transport end to end (a loopback provider, a real provider) without the keychain.
pub fn chat_stream_with_key(conn: &Connection, key: &str, history: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String> {
    local_ai::chat_stream_with_key(&APP, conn, key, history, on_delta)
}

/// A streaming answer on a worker thread: the pieces arrive on `deltas` as the provider sends them,
/// and the joined text (or the error) on `done`. Both channels close when the request ends.
pub struct Stream {
    pub deltas: std::sync::mpsc::Receiver<String>,
    pub done: std::sync::mpsc::Receiver<Result<String, String>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stream {
    /// Starts the request on a worker thread. The UI polls [`Stream::poll`] every frame.
    pub fn start(conn: Connection, history: Vec<Msg>) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::channel;
        let (deltas_tx, deltas) = channel();
        let (done_tx, done) = channel();
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("ai-chat".into())
            .spawn(move || {
                let result = local_ai::chat_stream(&APP, &conn, &history, |delta| {
                    // A cancelled request stops forwarding: closing the panel must not keep
                    // spending the user's tokens.
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = deltas_tx.send(delta.to_owned());
                });
                let _ = done_tx.send(result);
            })
            .ok();
        Self { deltas, done, cancelled }
    }

    /// What arrived since the last frame, and the final result once there is one.
    pub fn poll(&self) -> (Vec<String>, Option<Result<String, String>>) {
        let mut pieces = Vec::new();
        while let Ok(delta) = self.deltas.try_recv() {
            pieces.push(delta);
        }
        (pieces, self.done.try_recv().ok())
    }

    /// Tells the worker to stop forwarding (the answer in flight is discarded by the caller).
    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The provider's model list, sorted for the picker.
pub fn list_models(conn: &Connection) -> Result<Vec<String>, String> {
    local_ai::list_models(&APP, conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider on the loopback: it answers the first POST with `body` as an SSE response and
    /// sends back the request's head. The CI gate for the streaming path with no network and no
    /// keychain.
    fn serve_sse(body: String) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut reader = BufReader::new(stream.try_clone().expect("a reading handle"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            // The body carries the model and the messages: read it so the test can check them.
            let length = head
                .lines()
                .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(str::trim).map(str::to_owned))
                .and_then(|length| length.parse::<usize>().ok())
                .unwrap_or(0);
            let mut posted = vec![0u8; length];
            let _ = reader.read_exact(&mut posted);
            let _ = tx.send(format!("{head}\n{}", String::from_utf8_lossy(&posted)));
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}");
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        (format!("http://127.0.0.1:{port}/v1"), rx)
    }

    /// One streaming chunk in the OpenAI shape.
    fn delta(text: &str) -> String {
        let content = serde_json::to_string(text).expect("a JSON string");
        format!("data: {{\"choices\":[{{\"delta\":{{\"content\":{content}}}}}]}}\n\n")
    }

    /// The streaming path end to end over SSE: the pieces arrive in order, the answer joins, and
    /// the artifact contract reads the whole answer into a document.
    #[test]
    fn a_streamed_answer_becomes_a_document() {
        let answer = "{\"title\":\"Protocolo\",\"blocks\":[\
            {\"kind\":\"heading\",\"level\":1,\"text\":\"Sepse\"},\
            {\"kind\":\"paragraph\",\"text\":\"Definição.\"}]}";
        let cut = answer.char_indices().nth(answer.chars().count() / 2).map(|(i, _)| i).unwrap_or(answer.len());
        let body = format!("{}{}data: [DONE]\n\n", delta(&answer[..cut]), delta(&answer[cut..]));
        let (base_url, received) = serve_sse(body);
        let request = actions::Request::GenerateDoc { topic: "sepse".into(), language: "português".into(), shape: None };
        let messages = actions::messages(&request, "pt-br").expect("messages");
        let conn = Connection { id: "loopback".into(), provider: "custom".into(), label: "Loopback".into(), base_url, model: "mock".into() };
        let mut pieces = Vec::new();
        let whole =
            chat_stream_with_key(&conn, "test-key", &messages, &mut |piece: &str| pieces.push(piece.to_owned())).expect("the streamed answer");
        assert_eq!(whole, answer, "the pieces join into the answer");
        assert_eq!(pieces, vec![answer[..cut].to_owned(), answer[cut..].to_owned()], "the pieces arrived in order");
        let head = received.recv().expect("the request");
        assert!(head.contains("POST /v1/chat/completions"), "{head}");
        assert!(head.contains("\"stream\":true"), "{head}");
        match actions::interpret(&request, &whole).expect("a document") {
            actions::Outcome::Doc { title, blocks } => {
                assert_eq!(title, "Protocolo");
                assert_eq!(blocks.len(), 2);
            }
        }
    }

    #[test]
    fn the_store_path_lives_under_the_apps_folder() {
        // Path separators differ per OS; the identity folder and file name are what matter.
        let path = store_path().unwrap();
        assert!(path.to_string_lossy().contains("LocalWriter"));
        assert!(path.ends_with("ai-connections.json"));
    }

    #[test]
    fn the_identity_is_localwriters() {
        assert_eq!(APP.id, "LocalWriter");
        assert_eq!(APP.url, "https://github.com/Styx-Studios/Local-Writer");
    }

    fn connection(id: &str) -> Connection {
        Connection { id: id.into(), provider: "custom".into(), label: id.into(), base_url: String::new(), model: String::new() }
    }

    #[test]
    fn the_old_identity_only_differs_in_the_name() {
        assert_eq!(LEGACY_APP.id, "LocalOffice");
        assert_eq!(LEGACY_APP.url, APP.url);
        assert_ne!(data_dir(), legacy_data_dir());
    }

    #[test]
    fn legacy_keys_are_copied_once_and_never_overwrite() {
        use std::cell::RefCell;
        use std::collections::HashMap;

        let all = [connection("a"), connection("b"), connection("c"), connection("d")];
        let current: RefCell<HashMap<String, String>> = RefCell::new(HashMap::from([("b".to_string(), "new-b".to_string())]));
        let legacy: HashMap<&str, &str> = HashMap::from([("a", "old-a"), ("b", "old-b"), ("d", "")]);
        let run = || {
            migrate_keys(
                &all,
                |c| current.borrow().get(&c.id).is_some_and(|k| !k.is_empty()),
                |c| legacy.get(c.id.as_str()).map(|k| (*k).to_string()).unwrap_or_default(),
                |c, key| {
                    current.borrow_mut().insert(c.id.clone(), key.to_string());
                    Ok(())
                },
            )
        };
        assert_eq!(run(), 1, "only `a` has an old key and no new one");
        assert_eq!(current.borrow().get("a").map(String::as_str), Some("old-a"));
        assert_eq!(current.borrow().get("b").map(String::as_str), Some("new-b"), "an existing key is kept");
        assert!(!current.borrow().contains_key("c") && !current.borrow().contains_key("d"));
        assert_eq!(run(), 0, "the second start has nothing to copy");
    }

    #[test]
    fn a_keychain_that_refuses_is_not_fatal() {
        let all = [connection("a")];
        assert_eq!(migrate_keys(&all, |_| false, |_| "k".into(), |_, _| Err("locked".into())), 0);
    }

    #[test]
    fn a_connection_is_the_shared_shape() {
        let conn = Connection {
            id: "conn-1".into(),
            provider: "gemini".into(),
            label: "Gemini".into(),
            base_url: String::new(),
            model: "gemini-2.5-flash".into(),
        };
        // The provider preset supplies the base URL when the connection leaves it empty.
        assert_eq!(provider(&conn.provider).map(|p| p.base_url), Some("https://generativelanguage.googleapis.com/v1beta/openai"));
    }
}
