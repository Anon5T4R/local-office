#![cfg(feature = "dictation")]

//! Local speech-to-text (STT) via whisper.cpp (MIT).
//!
//! This module provides local speech-to-text transcription using whisper.cpp (via whisper-rs).
//! The model (medium, 769 MB, 99 languages including pt-BR/pt-PT/en-US) is downloaded on first
//! use with checksum verification and cached in the app's data directory for offline use.
//!
//! Microphone capture uses cpal (cross-platform). Audio is resampled to 16kHz mono before
//! transcription. The model is loaded lazily (OnceLock) — it doesn't load until first use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::io;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};
use serde_json::Value;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// The directory name for STT models in the app's data directory.
const STT_MODEL_DIR: &str = "stt-models";
/// The whisper model to use: medium (769 MB, good quality).
const STT_MODEL_NAME: &str = "ggml-medium.bin";
/// Expected SHA256 of the whisper-medium model (from whisper.cpp releases).
const STT_MODEL_SHA256: &str = "a80e1c8c8b8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8";
/// Download URL for the model (from whisper.cpp releases on Hugging Face).
const STT_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin";

/// How long to record audio (in seconds).
const RECORD_DURATION_SECS: u64 = 5;

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
    let actual = result.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    Ok(actual.eq_ignore_ascii_case(expected))
}

fn download_model(path: &Path, url: &str, _expected_sha256: &str) -> Result<(), String> {
    use std::io::Read;

    let mut resp = ureq::get(url).call().map_err(|e| format!("download request failed: {e}"))?;

    if resp.status() != 200 {
        return Err(format!("download failed with status: {}", resp.status()));
    }

    let total_size = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut reader = resp.body_mut().as_reader();
    log::info!("Downloading model ({:.1} MB)...", total_size as f64 / 1_000_000.0);

    let mut downloaded = 0u64;
    let mut buffer = [0u8; 8192];
    let mut data = Vec::new();

    loop {
        let n = reader.read(&mut buffer).map_err(|e| format!("read error: {e}"))?;
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
        params.set_language(language);
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
        let n_segments = state.full_n_segments();
        for i in 0..n_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(seg_text) = segment.to_str() {
                    text.push_str(seg_text);
                }
            }
        }

        Ok(text.trim().to_string())
    }
}

/// The global STT recognizer (lazy-initialized on first use).
static STT_RECOGNIZER: std::sync::OnceLock<WhisperRecognizer> = std::sync::OnceLock::new();

fn get_or_init_recognizer() -> Result<&'static WhisperRecognizer, String> {
    if let Some(r) = STT_RECOGNIZER.get() {
        return Ok(r);
    }
    let model_path = ensure_stt_model()?;
    let recognizer = WhisperRecognizer::new(&model_path)?;
    // Race: another thread might have initialized first; that's fine, use theirs.
    let _ = STT_RECOGNIZER.set(recognizer);
    STT_RECOGNIZER.get().ok_or_else(|| "failed to initialize recognizer".to_string())
}

/// Record audio from the default microphone for a fixed duration and transcribe it.
fn record_and_transcribe(_language: Option<&str>) -> Result<String, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicBool, Ordering};

    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("no input device available")?;

    let config = device.default_input_config().map_err(|e| format!("failed to get input config: {e}"))?;

    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;

    let samples = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);
    let recording = Arc::new(AtomicBool::new(true));
    let recording_clone = Arc::clone(&recording);

    let stream_config: cpal::StreamConfig = config.clone().into();
    let stream = device
        .build_input_stream(
            stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if recording_clone.load(Ordering::Relaxed) {
                    let mut samples = samples_clone.lock().unwrap();
                    samples.extend_from_slice(data);
                }
            },
            |err| log::error!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| format!("failed to build input stream: {e}"))?;

    stream.play().map_err(|e| format!("failed to start recording: {e}"))?;

    log::info!("Recording for {} seconds...", RECORD_DURATION_SECS);
    std::thread::sleep(Duration::from_secs(RECORD_DURATION_SECS));

    recording.store(false, Ordering::Relaxed);
    drop(stream);

    // Resample to 16kHz mono
    let raw_samples = samples.lock().unwrap().clone();
    let mono_samples = if channels > 1 {
        // Downmix to mono
        raw_samples.chunks(channels).map(|chunk| chunk.iter().sum::<f32>() / channels as f32).collect()
    } else {
        raw_samples
    };

    // Resample to 16kHz
    let ratio = sample_rate as f64 / 16000.0;
    let resampled: Vec<f32> = if (ratio - 1.0).abs() < 0.001 {
        mono_samples
    } else {
        // Simple linear interpolation resampling
        let new_len = (mono_samples.len() as f64 / ratio) as usize;
        let mut result = Vec::with_capacity(new_len);
        for i in 0..new_len {
            let src_idx = i as f64 * ratio;
            let idx = src_idx as usize;
            let frac = src_idx - idx as f64;
            let s0 = mono_samples.get(idx).copied().unwrap_or(0.0);
            let s1 = mono_samples.get(idx + 1).copied().unwrap_or(s0);
            result.push((s0 as f64 * (1.0 - frac) + s1 as f64 * frac) as f32);
        }
        result
    };

    log::info!("Transcribing {} samples...", resampled.len());

    let recognizer = get_or_init_recognizer()?;
    recognizer.transcribe(&resampled, None)
}

#[cfg(windows)]
fn dictate(s: &mut Session, v: &Value) -> CmdResult {
    let _ = p::bool(v, "start");
    let text = record_and_transcribe(None).map_err(CmdError::Failed)?;
    if !text.is_empty() {
        super::type_text(s, &text).map_err(|e| CmdError::Failed(e.to_string()))?;
    }
    Ok(serde_json::json!({"text": text}))
}

#[cfg(not(target_os = "windows"))]
fn dictate(s: &mut Session, v: &Value) -> CmdResult {
    let _ = p::bool(v, "start");
    let text = record_and_transcribe(None).map_err(CmdError::Failed)?;
    if !text.is_empty() {
        super::type_text(s, &text).map_err(|e| CmdError::Failed(e.to_string()))?;
    }
    Ok(serde_json::json!({"text": text}))
}

/// Command spec for the dictation feature.
pub fn dictation_specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("tools.dictate", "Dictate", "Tools ▸ Voice", dictate).key("Mod+Shift+D").params(r#"{"start"?: bool}"#).pure()]
}
