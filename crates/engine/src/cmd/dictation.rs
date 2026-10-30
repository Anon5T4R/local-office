#![cfg(feature = "dictation")]

//! Local speech-to-text (STT) via whisper.cpp (MIT).
//!
//! This module provides local speech-to-text transcription using whisper.cpp (via whisper-rs).
//! The model (medium, 769 MB, 99 languages including pt-BR/pt-PT/en-US) is downloaded on first
//! use with checksum verification and cached in the app's data directory for offline use.
//!
//! Dictation is start/stop: `tools.dictate` with `{"start": true}` opens the microphone
//! ([`begin`]), a second click with `{"start": false}` ends the capture ([`stop`]) and the job
//! transcribes on its background thread. The UI polls [`phase`] for feedback and [`take_result`]
//! for the finished text. A job refuses a second start, so repeated clicks never run concurrent
//! jobs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::io;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};
use serde_json::Value;
use sha2::{Digest, Sha256};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// The directory name for STT models in the app's data directory.
const STT_MODEL_DIR: &str = "stt-models";
/// The whisper model to use: medium (1.5 GB, 99 languages, good quality).
const STT_MODEL_NAME: &str = "ggml-medium.bin";
/// Expected SHA256 of the whisper-medium model (from whisper.cpp releases).
const STT_MODEL_SHA256: &str = "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208";
/// Download URL for the model (from whisper.cpp releases on Hugging Face).
const STT_MODEL_URL: &str = "https://github.com/Anon5T4R/big-files-local/releases/download/v1.0.0/ggml-medium.bin";

/// Bytes read, written and hashed per step of the model download.
const DOWNLOAD_CHUNK: usize = 8192;
/// How much must arrive between download progress reports to the shared state.
const DOWNLOAD_REPORT_STEP: u64 = 10 * 1024 * 1024;

/// What a dictation job is doing; the UI polls this for feedback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DictationPhase {
    /// No job is running.
    #[default]
    Idle,
    /// The microphone stream is open and collecting samples.
    Recording,
    /// The whisper model is downloading (first use only): `bytes_done` of `total` bytes, with
    /// `total` 0 when the server gave no length.
    DownloadingModel { bytes_done: u64, total: u64 },
    /// The captured audio is being transcribed.
    Transcribing,
}

/// The shared dictation state: the running job's phase and its finished result.
struct Dictation {
    phase: DictationPhase,
    result: Option<Result<String, String>>,
}

/// The state a fresh job starts from (a `const`, because `Mutex::new` is a `const fn`).
const DICTATION_FRESH: Dictation = Dictation { phase: DictationPhase::Idle, result: None };

/// The whole dictation state: at most one job runs at a time, guarded by the phase.
static DICTATION: Mutex<Dictation> = Mutex::new(DICTATION_FRESH);

/// Set while the UI wants the recording job to stop and transcribe.
static STOP: AtomicBool = AtomicBool::new(false);

/// The shared state; a panicked worker leaves it usable (`PoisonError::into_inner`).
fn lock() -> MutexGuard<'static, Dictation> {
    DICTATION.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Replace the shared phase (the finished result is left alone).
fn set_phase(phase: DictationPhase) {
    lock().phase = phase;
}

/// Start a dictation job: the microphone opens and captures until [`stop`], then the job
/// transcribes in the background. Refused while a job runs, so repeated clicks never start
/// concurrent jobs.
pub fn begin() -> Result<(), String> {
    {
        let mut d = lock();
        if d.phase != DictationPhase::Idle {
            return Err("dictation already in progress".into());
        }
        d.result = None;
        d.phase = DictationPhase::Recording;
    }
    STOP.store(false, Ordering::SeqCst);
    std::thread::spawn(run_job);
    Ok(())
}

/// Ask the recording job to stop and transcribe what it captured.
pub fn stop() -> Result<(), String> {
    let d = lock();
    if d.phase != DictationPhase::Recording {
        return Err("not recording".into());
    }
    STOP.store(true, Ordering::SeqCst);
    Ok(())
}

/// The running job's phase, for the UI's feedback.
pub fn phase() -> DictationPhase {
    lock().phase
}

/// Take the finished job's result, if one is waiting.
pub fn take_result() -> Option<Result<String, String>> {
    lock().result.take()
}

/// The phase's name, for command results (scripts and MCP read these).
fn phase_name(phase: DictationPhase) -> &'static str {
    match phase {
        DictationPhase::Idle => "idle",
        DictationPhase::Recording => "recording",
        DictationPhase::DownloadingModel { .. } => "downloading-model",
        DictationPhase::Transcribing => "transcribing",
    }
}

/// The background job: record until asked to stop, then transcribe. Any failure lands in the
/// result and the phase returns to idle, ready for the next job.
fn run_job() {
    let result = record_and_transcribe();
    let mut d = lock();
    d.result = Some(result);
    d.phase = DictationPhase::Idle;
}

/// Record from the default microphone until [`stop`] is asked, resample to 16 kHz mono and
/// transcribe.
fn record_and_transcribe() -> Result<String, String> {
    let audio = record_until_stop()?;
    set_phase(DictationPhase::Transcribing);
    let recognizer = get_or_init_recognizer()?;
    set_phase(DictationPhase::Transcribing);
    log::info!("Transcribing {} samples...", audio.len());
    recognizer.transcribe(&audio, None)
}

/// Capture the microphone until [`stop`] is asked. The stream's callback accumulates while the
/// flag is clear; the worker sleeps in short steps and checks it, so the UI's stop arrives
/// within one step and no thread ever blocks the UI.
fn record_until_stop() -> Result<Vec<f32>, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("no input device available")?;
    let config = device.default_input_config().map_err(|e| format!("failed to get input config: {e}"))?;
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;

    let samples = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);
    let stream_config: cpal::StreamConfig = config.into();
    let stream = device
        .build_input_stream(
            stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !STOP.load(Ordering::SeqCst) {
                    samples_clone.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend_from_slice(data);
                }
            },
            |err| log::error!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| format!("failed to build input stream: {e}"))?;
    stream.play().map_err(|e| format!("failed to start recording: {e}"))?;

    while !STOP.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(stream);

    Ok(resample_16k_mono(&samples.lock().unwrap_or_else(std::sync::PoisonError::into_inner), sample_rate, channels))
}

/// Downmix the capture to mono and resample to 16 kHz, which whisper expects.
fn resample_16k_mono(raw: &[f32], sample_rate: u32, channels: usize) -> Vec<f32> {
    let mono = if channels > 1 { raw.chunks(channels).map(|chunk| chunk.iter().sum::<f32>() / channels as f32).collect() } else { raw.to_vec() };
    let ratio = f64::from(sample_rate) / 16000.0;
    if (ratio - 1.0).abs() < 0.001 {
        return mono;
    }
    let new_len = (mono.len() as f64 / ratio) as usize;
    (0..new_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let idx = src as usize;
            let frac = src - idx as f64;
            let s0 = mono.get(idx).copied().unwrap_or(0.0);
            let s1 = mono.get(idx + 1).copied().unwrap_or(s0);
            (s0 as f64 * (1.0 - frac) + s1 as f64 * frac) as f32
        })
        .collect()
}

/// Ensures the STT model is available locally, downloading if necessary.
/// Returns the path to the model file.
fn ensure_stt_model() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir().ok_or("could not determine data directory")?.join("LocalOffice").join(STT_MODEL_DIR);

    io::create_dir_all(&data_dir)?;

    let model_path = data_dir.join(STT_MODEL_NAME);

    // If model exists, verify checksum
    if model_path.exists() {
        if sha256_matches(&model_path, STT_MODEL_SHA256)? {
            return Ok(model_path);
        }
        log::warn!("STT model checksum mismatch, re-downloading");
        io::remove_file(&model_path)?;
    }

    // Download model
    log::info!("Downloading STT model ({})...", STT_MODEL_NAME);
    download_model(&model_path, STT_MODEL_URL, STT_MODEL_SHA256)?;

    Ok(model_path)
}

/// The hex text of a SHA256 digest.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Whether the file at `path` hashes to `expected` (hex, case-insensitive). Hashed in chunks:
/// the 1.5 GB model never sits whole in memory.
fn sha256_matches(path: &Path, expected: &str) -> Result<bool, String> {
    use std::io::Read;

    let mut file = io::open_reader(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; DOWNLOAD_CHUNK];
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()).eq_ignore_ascii_case(expected))
}

/// Downloads the model to a temporary file beside `path` — streamed to disk in chunks and
/// hashed on the way, so neither the bytes nor their checksum ever sit whole in memory — then
/// verifies the checksum and moves the file into place. Progress goes to the shared phase for
/// the UI.
fn download_model(path: &Path, url: &str, expected_sha256: &str) -> Result<(), String> {
    use std::io::{Read, Write};

    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            // Every read of the body must make progress; the 1.5 GB may legitimately take
            // longer than any whole-call budget.
            .timeout_per_read(Some(Duration::from_secs(120)))
            .build(),
    );
    let mut resp = agent.get(url).call().map_err(|e| format!("download request failed: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("download failed with status: {}", resp.status()));
    }

    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut reader = resp.body_mut().as_reader();
    log::info!("Downloading model ({:.1} MB)...", total as f64 / 1_000_000.0);

    set_phase(DictationPhase::DownloadingModel { bytes_done: 0, total });
    let temp = path.with_extension("part");
    let mut file = io::open_writer(&temp)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; DOWNLOAD_CHUNK];
    let mut downloaded = 0u64;
    let mut reported = 0u64;
    loop {
        let n = reader.read(&mut buffer).map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|e| format!("{}: {e}", temp.display()))?;
        downloaded += n as u64;
        if downloaded - reported >= DOWNLOAD_REPORT_STEP {
            reported = downloaded;
            set_phase(DictationPhase::DownloadingModel { bytes_done: downloaded, total });
        }
    }
    drop(file);

    if !hex(&hasher.finalize()).eq_ignore_ascii_case(expected_sha256) {
        let _ = io::remove_file(&temp);
        return Err("model checksum verification failed after download".into());
    }
    io::replace_file(&temp, path)?;

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
        let ctx = self.ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
            if let Some(segment) = state.get_segment(i)
                && let Ok(seg_text) = segment.to_str()
            {
                text.push_str(seg_text);
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

fn dictate(_s: &mut Session, v: &Value) -> CmdResult {
    let start = p::bool(v, "start").unwrap_or(true);
    if start {
        // Start recording in the background; the result comes through `take_result`.
        begin().map_err(CmdError::Failed)?;
        Ok(serde_json::json!({"status": "recording", "phase": phase_name(phase())}))
    } else {
        // Signal the stop; transcription continues in the background.
        stop().map_err(CmdError::Failed)?;
        Ok(serde_json::json!({"status": "stopping", "phase": phase_name(phase())}))
    }
}

/// Command spec for the dictation feature.
pub fn dictation_specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("tools.dictate", "Dictate", "Tools ▸ Voice", dictate).params(r#"{"start"?: bool}"#).pure()]
}
