//! BYOK AI for LocalOffice: the user's own keys, one OpenAI-compatible client for every provider.
//!
//! Thin wrapper over the shared [`local_ai`] crate with LocalOffice's identity (config folder,
//! keychain service, OpenRouter headers). Nothing here talks to a LocalOffice server: there is
//! none. A connection is a provider preset (base URL), a model name the picker fetches from the
//! provider's own `/models`, and a secret the OS keychain holds. The saved shape is the shared
//! crate's [`Connection`], so the connections file is the suite's one file format.

use std::path::PathBuf;

pub use local_ai::{Connection, Job, Msg, PROVIDERS, Provider, provider};

/// LocalOffice's identity in the shared `local-ai` crate: config folder and keychain service both
/// `"LocalOffice"`, and the OpenRouter referer points at this repository.
pub const APP: local_ai::App = local_ai::App { id: "LocalOffice", url: "https://github.com/Anon5T4R/local-office" };

/// Where the connections file lives: `<data dir>/LocalOffice/ai-connections.json`.
pub fn store_path() -> Option<PathBuf> {
    Connection::store_path(&APP)
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

/// The provider's model list, sorted for the picker.
pub fn list_models(conn: &Connection) -> Result<Vec<String>, String> {
    local_ai::list_models(&APP, conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_path_lives_under_the_apps_folder() {
        assert!(store_path().unwrap().ends_with(r"LocalOffice\ai-connections.json"));
    }

    #[test]
    fn the_identity_is_localoffices() {
        assert_eq!(APP.id, "LocalOffice");
        assert_eq!(APP.url, "https://github.com/Anon5T4R/local-office");
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
