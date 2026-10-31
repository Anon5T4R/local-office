#![cfg(feature = "dictation")]

//! Local speech-to-text (STT) via whisper.cpp (MIT).
//!
//! Dictation runs on local whisper.cpp models (via whisper-rs), downloaded on first use with
//! checksum verification and cached in the app's data directory for offline use. The first
//! Dictate click opens the model picker ([`models_available`], [`choose_model`]): `small`
//! (488 MB, recommended — near real-time on CPU) or `medium` (1.5 GB, more accurate but much
//! slower without a GPU). The choice is remembered in a one-line file beside the models.
//!
//! Dictation is start/stop: `tools.dictate` with `{"start": true}` begins a job ([`begin`]), a
//! second click with `{"start": false}` ends the capture ([`stop`]) and the job transcribes on
//! its background thread. The model loads per job and is dropped as soon as the text is ready,
//! so its weights never sit in RAM between dictations. The UI polls [`phase`] for feedback and
//! [`take_result`] for the finished text. A job refuses a second start, so repeated clicks
//! never run concurrent jobs.

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
/// Bytes read, written and hashed per step of the model download.
const DOWNLOAD_CHUNK: usize = 8192;
/// How much must arrive between download progress reports to the shared state.
const DOWNLOAD_REPORT_STEP: u64 = 10 * 1024 * 1024;

/// A whisper model Dictate can use: what it is called, where it downloads from and how big it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SttModel {
    /// `ggml-small.bin` (488 MB, multilingual): the recommended default — the fastest of the two
    /// on CPU by far.
    #[default]
    Small,
    /// `ggml-medium.bin` (1.5 GB, multilingual): more accurate, but far slower on CPU.
    Medium,
}

impl SttModel {
    /// The model file name inside the STT cache folder.
    fn file_name(self) -> &'static str {
        match self {
            Self::Small => "ggml-small.bin",
            Self::Medium => "ggml-medium.bin",
        }
    }

    /// Download URL (big-files-local release assets).
    fn url(self) -> &'static str {
        match self {
            Self::Small => "https://github.com/Anon5T4R/big-files-local/releases/download/v1.1.0/ggml-small.bin",
            Self::Medium => "https://github.com/Anon5T4R/big-files-local/releases/download/v1.0.0/ggml-medium.bin",
        }
    }

    /// Expected SHA256 of the model file (from whisper.cpp releases).
    fn sha256(self) -> &'static str {
        match self {
            Self::Small => "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            Self::Medium => "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        }
    }

    /// Download size in MB, for the picker UI.
    pub fn size_mb(self) -> u64 {
        match self {
            Self::Small => 488,
            Self::Medium => 1533,
        }
    }

    /// The label stored in the choice file (`small`/`medium`).
    fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
        }
    }

    /// The model a choice-file label names; unknown labels fall back to the caller's default.
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            _ => None,
        }
    }
}

/// The folder the models (and the choice file) live in.
fn stt_dir() -> Result<PathBuf, String> {
    Ok(dirs::data_dir().ok_or("could not determine data directory")?.join("LocalOffice").join(STT_MODEL_DIR))
}

/// Where the model file for `model` is cached.
fn model_path(model: SttModel) -> Result<PathBuf, String> {
    Ok(stt_dir()?.join(model.file_name()))
}

/// The sidecar that remembers a model passed its SHA256 verification, so the 1.5 GB file is not
/// re-hashed on every job.
fn sidecar_path(model_path: &Path) -> PathBuf {
    let mut name = model_path.as_os_str().to_os_string();
    name.push(".sha256.ok");
    PathBuf::from(name)
}

/// Every model Dictate can use, each with whether it is already on disk — the model picker's
/// rows.
pub fn models_available() -> Vec<(SttModel, bool)> {
    [SttModel::Small, SttModel::Medium].iter().map(|&m| (m, model_path(m).is_ok_and(|p| p.exists()))).collect()
}

/// Where the chosen model is remembered: a one-line label file beside the models. Dictation
/// state is a process-wide static (the job runs on its own thread, with no `Session`), so the
/// session preference store does not reach here; the file keeps the choice across restarts.
fn choice_path() -> Result<PathBuf, String> {
    Ok(stt_dir()?.join("choice.txt"))
}

/// The model the next job uses: the last one picked in the model picker, or `Small` (the
/// recommended default) before any choice was made.
fn load_choice() -> Result<SttModel, String> {
    let path = choice_path()?;
    let mut label = String::new();
    if path.exists() {
        use std::io::Read;
        io::open_reader(&path)?.read_to_string(&mut label).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(SttModel::from_label(label.trim()).unwrap_or_default())
}

/// Remember `model` as the Dictate model (the model picker's OK). While a job waits in
/// [`DictationPhase::NeedsModel`] this also resumes it: the model downloads or recording starts
/// at once, without the UI having to call [`begin`] again.
pub fn choose_model(model: SttModel) -> Result<(), String> {
    let path = choice_path()?;
    io::create_dir_all(&stt_dir()?)?;
    {
        use std::io::Write;
        io::open_writer(&path)?.write_all(model.label().as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if phase() == DictationPhase::NeedsModel {
        start_job()?;
    }
    Ok(())
}

/// The shared dictation state: the running job's phase and its finished result.
struct Dictation {
    phase: DictationPhase,
    result: Option<Result<String, String>>,
}

/// What a dictation job is doing; the UI polls this for feedback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DictationPhase {
    /// No job is running.
    #[default]
    Idle,
    /// The first Dictate click found no model downloaded and none chosen yet: the UI asks which
    /// one to fetch before the job can start.
    NeedsModel,
    /// The microphone stream is open and collecting samples.
    Recording,
    /// The whisper model is downloading (first use only): `bytes_done` of `total` bytes, with
    /// `total` 0 when the server gave no length.
    DownloadingModel { bytes_done: u64, total: u64 },
    /// The captured audio is being transcribed.
    Transcribing,
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

/// Start a dictation job: the model downloads if needed, then the microphone opens and captures
/// until [`stop`], then the job transcribes in the background. With no model downloaded and none
/// chosen yet the job waits in [`DictationPhase::NeedsModel`] for the model picker instead.
/// Refused while a job runs, so repeated clicks never start concurrent jobs.
pub fn begin() -> Result<(), String> {
    start_job()
}

/// The phase logic [`begin`] and [`choose_model`] share: from the chosen model and the files on
/// disk, either park the job at [`DictationPhase::NeedsModel`] (no model on disk, none chosen),
/// or mark the phase and spawn the worker.
fn start_job() -> Result<(), String> {
    let (model, needs_download) = {
        let mut d = lock();
        if !matches!(d.phase, DictationPhase::Idle | DictationPhase::NeedsModel) {
            return Err("dictation already in progress".into());
        }
        let model = load_choice()?;
        let downloaded = model_path(model).is_ok_and(|p| p.exists());
        if downloaded {
            d.result = None;
            d.phase = DictationPhase::Recording;
        } else if choice_path()?.exists() {
            // Chosen before but not on disk (a fresh machine, a deleted cache): fetch it.
            d.result = None;
            d.phase = DictationPhase::DownloadingModel { bytes_done: 0, total: 0 };
        } else {
            d.phase = DictationPhase::NeedsModel;
            return Ok(());
        }
        (model, !downloaded)
    };
    STOP.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let result = if needs_download { ensure_model(model).map(|_| ()) } else { Ok(()) }.and_then(|()| record_and_transcribe(model));
        let mut d = lock();
        d.result = Some(result);
        d.phase = DictationPhase::Idle;
    });
    Ok(())
}

/// Back out of [`DictationPhase::NeedsModel`] (the model picker's Cancel): no job was running.
pub fn cancel_model_choice() {
    let mut d = lock();
    if d.phase == DictationPhase::NeedsModel {
        d.phase = DictationPhase::Idle;
    }
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
        DictationPhase::NeedsModel => "needs-model",
        DictationPhase::Recording => "recording",
        DictationPhase::DownloadingModel { .. } => "downloading-model",
        DictationPhase::Transcribing => "transcribing",
    }
}

/// Record from the default microphone until [`stop`] is asked, resample to 16 kHz mono and
/// transcribe with `model`.
fn record_and_transcribe(model: SttModel) -> Result<String, String> {
    let audio = record_until_stop()?;
    set_phase(DictationPhase::Transcribing);
    log::info!("Loading {}...", model.file_name());
    let recognizer = WhisperRecognizer::new(&model_path(model)?)?;
    log::info!("Transcribing {} samples...", audio.len());
    let text = recognizer.transcribe(&audio, None);
    // `recognizer` drops here: the model's weights (488 MB / 1.5 GB) leave RAM as soon as the
    // text is ready, and the next job loads the file afresh.
    text
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

/// Makes sure the model file is on disk and matches its checksum, downloading it when missing
/// or corrupted. Returns the path. A passed verification is remembered in a `.sha256.ok`
/// sidecar so the multi-hundred-MB file is not re-hashed on every job; the sidecar is written
/// again after each fresh download.
fn ensure_model(model: SttModel) -> Result<PathBuf, String> {
    let path = model_path(model)?;
    io::create_dir_all(&stt_dir()?)?;
    let sidecar = sidecar_path(&path);
    if path.exists() {
        if sidecar_content_equals(&sidecar, model.sha256()) {
            return Ok(path);
        }
        if sha256_matches(&path, model.sha256())? {
            write_sidecar(&sidecar, model.sha256());
            return Ok(path);
        }
        log::warn!("STT model checksum mismatch, re-downloading");
        io::remove_file(&path)?;
        let _ = io::remove_file(&sidecar);
    }

    log::info!("Downloading STT model ({})...", model.file_name());
    download_model(&path, model.url(), model.sha256())?;
    write_sidecar(&sidecar, model.sha256());
    log::info!("Model downloaded and verified successfully");
    Ok(path)
}

/// Whether the sidecar names exactly `expected` (an absent or unreadable sidecar just means
/// "verify again").
fn sidecar_content_equals(sidecar: &Path, expected: &str) -> bool {
    let mut content = String::new();
    io::open_reader(sidecar)
        .and_then(|mut r| {
            use std::io::Read;
            r.read_to_string(&mut content).map_err(|e| format!("{}: {e}", sidecar.display()))
        })
        .is_ok_and(|_| content.trim().eq_ignore_ascii_case(expected))
}

/// Remember that the model file at the sibling path passed verification.
fn write_sidecar(sidecar: &Path, sha256: &str) {
    // A cache marker: failing to write it only costs a re-hash on the next job.
    let _ = io::open_writer(sidecar).map(|mut w| {
        use std::io::Write;
        w.write_all(sha256.as_bytes()).map_err(|e| format!("{}: {e}", sidecar.display()))
    });
}

/// The hex text of a SHA256 digest.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Whether the file at `path` hashes to `expected` (hex, case-insensitive). Hashed in chunks:
/// the multi-hundred-MB models never sit whole in memory.
fn sha256_matches(path: &Path, expected: &str) -> Result<bool, String> {
    let mut file = io::open_reader(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; DOWNLOAD_CHUNK];
    loop {
        let n = std::io::Read::read(&mut file, &mut buffer).map_err(|e| format!("{}: {e}", path.display()))?;
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
            // Every read of the body must make progress; the model may legitimately take
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

    Ok(())
}

/// The per-job whisper wrapper: the context owns the model's weights and frees them on drop.
struct WhisperRecognizer {
    ctx: WhisperContext,
}

impl WhisperRecognizer {
    fn new(model_path: &Path) -> Result<Self, String> {
        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(model_path, params).map_err(|e| format!("failed to load whisper model: {e}"))?;
        Ok(Self { ctx })
    }

    /// Transcribe audio samples (16 kHz mono `f32` PCM).
    pub fn transcribe(&self, audio: &[f32], language: Option<&str>) -> Result<String, String> {
        let mut state = self.ctx.create_state().map_err(|e| format!("failed to create whisper state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // Every logical core, not whisper.cpp's `min(4, cores)` default — the encoder scales
        // with cores, and the bench (crates/engine/tests/stt_bench.rs) measured the default
        // leaving the 12-thread machine mostly idle.
        params.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get() as i32));
        // `None` keeps whisper's language auto-detect, which the bench measured at a real cost
        // (one extra encoder pass), kept anyway: dictation is multilingual.
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

fn dictate(_s: &mut Session, v: &Value) -> CmdResult {
    let start = p::bool(v, "start").unwrap_or(true);
    if start {
        // Start the job in the background; the result comes through `take_result`. With no
        // model chosen yet the job parks in `needs-model` for the UI's picker instead.
        begin().map_err(CmdError::Failed)?;
        let phase = phase();
        let status = if phase == DictationPhase::NeedsModel { "needs-model" } else { "recording" };
        Ok(serde_json::json!({"status": status, "phase": phase_name(phase)}))
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
