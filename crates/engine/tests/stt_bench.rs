//! Benchmark for the local STT (whisper) transcription path (`cmd/dictation.rs`).
//!
//! Measures `state.full()` wall time for the whisper `medium` model across a matrix of
//! `n_threads` × language × audio duration, to separate code-side costs (thread count,
//! language auto-detect) from what is inherent to the model on this CPU. Never runs
//! automatically: it needs the 1.5 GB model on disk and minutes of CPU.
//!
//! How to run (PowerShell):
//!
//! ```powershell
//! $env:PATH = "C:\Program Files\LLVM\bin;C:\Program Files\CMake\bin;$env:PATH"
//! $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
//! cargo test -p localoffice-engine --features dictation --release -- --ignored stt_bench --nocapture
//! ```
//!
//! Audio: by default a speech-like signal is synthesized (encoder cost is real, but decode
//! cost is understated because near-silence decodes few tokens). For realistic timings set
//! `STT_BENCH_WAV` to a 16-bit PCM WAV file (any sample rate/channels; resampled to 16 kHz
//! mono):
//!
//! ```powershell
//! $env:STT_BENCH_WAV = "$env:TEMP\opencode\stt_bench\speech.wav"
//! ```
//!
//! Model: `STT_BENCH_MODEL` names the file in the STT cache folder (default `ggml-medium.bin`,
//! e.g. `ggml-small.bin` for the small model).

#![cfg(feature = "dictation")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Where the model lives (the app's STT cache folder, `STT_BENCH_MODEL` picks the file).
fn model_path() -> PathBuf {
    let name = std::env::var("STT_BENCH_MODEL").unwrap_or_else(|_| "ggml-medium.bin".to_string());
    dirs::data_dir().expect("no data dir").join("LocalOffice").join("stt-models").join(name)
}

/// One timed transcription run; a fresh whisper state per run keeps measurements independent.
fn run_case(label: &str, ctx: &WhisperContext, audio: &[f32], threads: Option<i32>, language: Option<&str>) {
    let mut state = ctx.create_state().expect("create state");
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    if let Some(t) = threads {
        params.set_n_threads(t);
    }
    params.set_language(language);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_translate(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);

    let start = Instant::now();
    state.full(params, audio).expect("full failed");
    let elapsed = start.elapsed();

    let n_segments = state.full_n_segments();
    let mut text_len = 0;
    for i in 0..n_segments {
        if let Some(segment) = state.get_segment(i)
            && let Ok(text) = segment.to_str()
        {
            text_len += text.trim().chars().count();
        }
    }
    let lang = language.unwrap_or("auto");
    let threads_shown = threads.map_or_else(|| "default(4)".to_string(), |t| t.to_string());
    println!(
        "{label:<44} threads={threads_shown:<10} lang={lang:<4} audio={:.0}s  time={:>7.2}s  segments={n_segments:<3} text_chars={text_len}",
        audio.len() as f64 / 16_000.0,
        elapsed.as_secs_f64(),
    );
}

/// Loads `STT_BENCH_WAV` as 16 kHz mono f32, or synthesizes a speech-like signal.
fn prepare_audio() -> Vec<f32> {
    match std::env::var("STT_BENCH_WAV") {
        Ok(path) => {
            let (samples, rate, channels) = load_wav(Path::new(&path));
            println!("audio source: {path} ({rate} Hz, {channels} ch)");
            resample_16k_mono(&samples, rate, channels)
        }
        Err(_) => {
            println!("audio source: synthesized speech-like signal (set STT_BENCH_WAV for realistic decode cost)");
            synth_speech_like(35.0, 16_000)
        }
    }
}

/// Minimal RIFF/WAVE reader for 16-bit PCM: returns (mono f32 samples, rate, channels).
fn load_wav(path: &Path) -> (Vec<f32>, u32, u16) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(&bytes[0..4], b"RIFF", "{}: not a RIFF file", path.display());
    assert_eq!(&bytes[8..12], b"WAVE", "{}: not a WAVE file", path.display());
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let (mut pos, mut data) = (12usize, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
        match id {
            b"fmt " => {
                channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    assert_eq!(bits, 16, "only 16-bit PCM supported, got {bits}");
    let data = data.expect("no data chunk");
    let frames: Vec<i16> = data.chunks_exact(2).map(|c| i16::from_le_bytes(c.try_into().unwrap())).collect();
    (frames.into_iter().map(|s| f32::from(s) / 32_768.0).collect(), rate, channels)
}

/// Downmix to mono and linear-resample to 16 kHz (mirrors `cmd/dictation.rs`).
fn resample_16k_mono(raw: &[f32], sample_rate: u32, channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> =
        if channels > 1 { raw.chunks(channels).map(|chunk| chunk.iter().sum::<f32>() / channels as f32).collect() } else { raw.to_vec() };
    let ratio = f64::from(sample_rate) / 16_000.0;
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
            s0 * (1.0 - frac) as f32 + s1 * frac as f32
        })
        .collect()
}

/// Speech-like signal: modulated multi-harmonic tone bursts with pauses.
fn synth_speech_like(seconds: f64, rate: u32) -> Vec<f32> {
    let len = (seconds * f64::from(rate)) as usize;
    (0..len)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            // 0.5 s voiced / 0.15 s pause, so whisper sees segment boundaries.
            let voiced = (t % 0.65) < 0.5;
            if !voiced {
                0.0
            } else {
                let f = 140.0 + 90.0 * (2.0 * std::f64::consts::PI * 0.8 * t).sin();
                let tone = (2.0 * std::f64::consts::PI * f * t).sin()
                    + 0.5 * (2.0 * std::f64::consts::PI * 2.0 * f * t).sin()
                    + 0.25 * (2.0 * std::f64::consts::PI * 3.0 * f * t).sin();
                (0.35 * tone * (2.0 * std::f64::consts::PI * 4.0 * t).sin()) as f32
            }
        })
        .collect()
}

/// Slices the first `seconds` of audio.
fn slice(samples: &[f32], seconds: f64) -> Vec<f32> {
    samples[..samples.len().min((seconds * 16_000.0) as usize)].to_vec()
}

#[test]
#[ignore = "benchmark: needs a whisper model in the STT cache and minutes of CPU; see the module docs to run"]
fn stt_bench() {
    let path = model_path();
    assert!(path.exists(), "model not found at {} (run the app once or download it)", path.display());
    let parallelism = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
    println!("available_parallelism={parallelism}");

    let start = Instant::now();
    let ctx = WhisperContext::new_with_params(&path, WhisperContextParameters::default()).expect("load model");
    println!("model load: {:.2}s\n", start.elapsed().as_secs_f64());

    let full = prepare_audio();

    // Phase A: thread sweep at 5 s with an explicit language (decode cost only varies with
    // token count, which is the same for every entry here).
    println!("--- Phase A: threads (5 s audio) ---");
    let audio_5s = slice(&full, 5.0);
    for threads in [None, Some(1), Some(6), Some(12)] {
        run_case("threads sweep", &ctx, &audio_5s, threads, Some("en"));
    }

    // Phase B: language cost at 5 s (auto-detect adds a whole extra encoder pass).
    println!("\n--- Phase B: language (5 s audio) ---");
    for language in [None, Some("en"), Some("pt")] {
        run_case("language sweep", &ctx, &audio_5s, None, language);
    }

    // Phase C: long clip — production config vs explicit language vs fully tuned (every
    // logical core, as the patched `transcribe()` now uses).
    println!("\n--- Phase C: 30 s audio ---");
    let audio_30s = slice(&full, 30.0);
    run_case("production config", &ctx, &audio_30s, None, None);
    run_case("explicit language", &ctx, &audio_30s, None, Some("en"));
    run_case("all cores + explicit language", &ctx, &audio_30s, Some(std::thread::available_parallelism().map_or(4, |n| n.get() as i32)), Some("en"));
}
