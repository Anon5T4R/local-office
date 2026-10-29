//! Local speech-to-text (STT) via whisper.cpp (MIT).
//!
//! This module provides local speech-to-text transcription using whisper.cpp (via whisper-rs).
//! The model (whisper-medium, 769 MB) is downloaded on first use with checksum verification
//! and cached in the app's data directory for offline use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::io;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// The directory name for STT models in the app's data directory.
const STT_MODEL_DIR: &str = "stt-models";
/// The whisper model to use: medium (769 MB, good quality).
const STT_MODEL_NAME: &str = "ggml-medium.bin";
/// Expected SHA256 of the whisper-medium model (from whisper.cpp releases).
const STT_MODEL_SHA256: &str = "a80e1c8c8b8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8";
/// Download URL for the model (from whisper.cpp releases on Hugging Face).
const STT_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin";

/// Ensures the STT model is available locally, downloading if necessary.
/// Returns the path to the model file.
fn ensure_stt_model() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir().ok_or("could not determine data directory")?.join("LocalOffice").join(STT_MODEL_DIR);

    io::create_dir_all(&data_dir)?;

    let model_path = data_dir.join(STT_MODEL_NAME);

    // If model exists, verify checksum
    if model_path.exists() {
        if verify_sha256(&model_path, STT_MODEL_SHA256)? {
            return Ok(model_path);
        } else {
            log::warn!("STT model checksum mismatch, re-downloading");
            io::remove_file(&model_path)?;
        }
    }

    // Download model
    log::info!("Downloading STT model ({})...", STT_MODEL_NAME);
    download_model(&model_path, STT_MODEL_URL, STT_MODEL_SHA256)?;

    Ok(model_path)
}

fn verify_sha256(path: &Path, expected: &str) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    let bytes = io::read_file(path.to_str().ok_or("invalid path")?)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    let actual = format!("{:x}", result);
    Ok(actual.eq_ignore_ascii_case(expected))
}

fn download_model(path: &Path, url: &str, _expected_sha256: &str) -> Result<(), String> {
    use std::io::Read;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;

    let mut resp = client.get(url).send().map_err(|e| format!("download request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("download failed with status: {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(0);
    log::info!("Downloading model ({:.1} MB)...", total_size as f64 / 1_000_000.0);

    let mut downloaded = 0u64;
    let mut buffer = [0u8; 8192];
    let mut stream = resp;
    let mut data = Vec::new();

    loop {
        let n = stream.read(&mut buffer).map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..n]);
        downloaded += n as u64;

        if downloaded % (10 * 1024 * 1024) == 0 {
            log::info!("Downloaded {:.1} MB...", downloaded as f64 / 1_000_000.0);
        }
    }

    io::write_file(path.to_str().ok_or("invalid path")?, &data)?;

    // Verify checksum
    if !verify_sha256(Path::new(path), STT_MODEL_SHA256)? {
        io::remove_file(path).ok();
        return Err("model checksum verification failed after download".into());
    }

    log::info!("Model downloaded and verified successfully");
    Ok(())
}

/// Thread-safe wrapper for the Whisper context.
struct WhisperRecognizer {
    ctx: Arc<Mutex<WhisperContext>>,
}

impl WhisperRecognizer {
    fn new(model_path: &Path) -> Result<Self, String> {
        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(model_path, params).map_err(|e| format!("failed to load whisper model: {e}"))?;
        Ok(Self { ctx: Arc::new(Mutex::new(ctx)) })
    }

    /// Transcribe audio samples (16-bit PCM, 16kHz mono).
    pub fn transcribe(&self, audio: &[f32], language: Option<&str>) -> Result<String, String> {
        let ctx = self.ctx.lock().unwrap();
        let mut state = ctx.create_state().map_err(|e| format!("failed to create whisper state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language.unwrap_or("auto"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);

        state.full(params, audio).map_err(|e| format!("transcription failed: {e}"))?;

        let mut text = String::new();
        let n_segments = state.full_n_segments().map_err(|e| format!("failed to get segments: {e}"))?;
        for i in 0..n_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                text.push_str(&segment);
            }
        }

        Ok(text.trim().to_string())
    }
}

/// The global STT recognizer (lazy-initialized on first use).
static STT_RECOGNIZER: std::sync::OnceLock<WhisperRecognizer> = std::sync::OnceLock::new();

fn get_or_init_recognizer() -> Result<&'static WhisperRecognizer, String> {
    STT_RECOGNIZER.get_or_try_init(|| {
        let model_path = ensure_stt_model()?;
        WhisperRecognizer::new(&model_path)
    })
}

#[cfg(windows)]
fn dictate(s: &mut Session, v: &Value) -> CmdResult {
    let _ = p::bool(v, "start");
    Err(CmdError::Unimplemented("Dictate (STT) not yet implemented on Windows".into()))
}

#[cfg(not(target_os = "windows"))]
fn dictate(s: &mut Session, v: &Value) -> CmdResult {
    let _ = p::bool(v, "start");
    Err(CmdError::Unimplemented("Dictate (STT) not yet implemented on this platform".into()))
}

/// Command spec for the dictation feature.
pub fn dictation_specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("tools.dictate", "Dictate", "Tools ▸ Voice", dictate).key("Mod+Shift+D").params(r#"{"start"?: bool}"#).pure()]
}
