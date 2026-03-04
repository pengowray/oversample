//! Streaming playback engine.
//!
//! Instead of processing the entire selection through DSP before any audio
//! plays, this module processes and schedules audio in small chunks (~0.5s).
//! The user hears sound almost immediately while subsequent chunks are
//! processed in the background.

use web_sys::{AudioContext, AudioContextOptions};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use std::cell::RefCell;
use std::sync::Arc;

use crate::state::{PlaybackMode, FilterQuality};
use crate::dsp::heterodyne::heterodyne_mix;
use crate::dsp::pitch_shift::pitch_shift_realtime;
use crate::dsp::zc_divide::zc_divide;
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use crate::audio::playback::{apply_bandpass, apply_gain};

/// Number of source samples per chunk. ~0.5s at 192kHz, ~2s at 44.1kHz.
const CHUNK_SAMPLES: usize = 96_000;

/// Extra overlap samples prepended to each chunk for IIR filter warmup.
/// This lets filters (heterodyne lowpass, bandpass) settle before the
/// actual chunk data, avoiding clicks at boundaries.
const FILTER_WARMUP: usize = 4096;

/// How far ahead (in seconds) to stay buffered beyond current playback time.
const LOOKAHEAD_SECS: f64 = 1.5;

thread_local! {
    static STREAM_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    /// Master gain node for fade-out on stop (avoids clicks).
    static STREAM_GAIN: RefCell<Option<web_sys::GainNode>> = RefCell::new(None);
    /// Monotonically increasing generation counter to detect stale streams.
    static STREAM_GEN: RefCell<u32> = RefCell::new(0);
}

/// Snapshot of all playback parameters, frozen at play start so that
/// parameter changes mid-playback don't cause glitches.
pub(crate) struct PlaybackParams {
    pub mode: PlaybackMode,
    pub het_freq: f64,
    pub het_cutoff: f64,
    pub te_factor: f64,
    pub ps_factor: f64,
    pub zc_factor: f64,
    pub gain_db: f64,
    pub auto_gain: bool,
    pub filter_enabled: bool,
    pub filter_freq_low: f64,
    pub filter_freq_high: f64,
    pub filter_db_below: f64,
    pub filter_db_selected: f64,
    pub filter_db_harmonics: f64,
    pub filter_db_above: f64,
    pub filter_band_mode: u8,
    pub filter_quality: FilterQuality,
    pub sel_freq_low: f64,
    pub sel_freq_high: f64,
    pub has_selection: bool,
    pub notch_enabled: bool,
    pub notch_bands: Vec<crate::dsp::notch::NoiseBand>,
    pub notch_harmonic_suppression: f64,
    pub noise_reduce_enabled: bool,
    pub noise_reduce_strength: f64,
    pub noise_reduce_floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
}

/// Duration of fade-out when stopping playback (milliseconds).
const FADE_OUT_MS: f64 = 30.0;

/// Duration of fade-in when starting playback (milliseconds).
const FADE_IN_MS: f64 = 30.0;

/// Stop any active streaming playback with a short fade-out to avoid clicks.
pub(crate) fn stop_stream() {
    STREAM_GEN.with(|g| {
        let mut generation = g.borrow_mut();
        *generation = generation.wrapping_add(1);
    });

    // Take the old gain node and context; fade out then close asynchronously.
    let gain = STREAM_GAIN.with(|g| g.borrow_mut().take());
    let ctx = STREAM_CTX.with(|c| c.borrow_mut().take());

    if let (Some(gain_node), Some(old_ctx)) = (gain, ctx) {
        let now = old_ctx.current_time();
        let param = gain_node.gain();
        // Ramp from current value to 0 over FADE_OUT_MS
        let _ = param.cancel_scheduled_values(now);
        let _ = param.set_value_at_time(param.value(), now);
        let _ = param.linear_ramp_to_value_at_time(0.0, now + FADE_OUT_MS / 1000.0);
        // Close the context after the fade completes
        let fade_ms = (FADE_OUT_MS + 5.0) as i32; // small margin
        let cb = wasm_bindgen::closure::Closure::once(move || {
            let _ = old_ctx.close();
        });
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                fade_ms,
            );
        }
        cb.forget();
    }
}

/// Returns true if streaming playback is currently active.
#[allow(dead_code)]
pub(crate) fn is_streaming() -> bool {
    STREAM_CTX.with(|c| c.borrow().is_some())
}

/// Start streaming playback of a sample range.
///
/// Returns the final playback sample rate (may differ from source for TE mode).
pub(crate) fn start_stream(
    source_samples: Arc<Vec<f32>>,
    sample_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
) -> u32 {
    stop_stream();

    let final_rate = match params.mode {
        PlaybackMode::TimeExpansion => {
            let abs_f = params.te_factor.abs().max(1.0);
            let rate = if params.te_factor > 0.0 {
                sample_rate as f64 / abs_f
            } else {
                sample_rate as f64 * abs_f
            };
            (rate as u32).clamp(8000, 384_000)
        }
        _ => sample_rate,
    };

    // Create AudioContext at the final playback rate
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(final_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .or_else(|_| AudioContext::new())
        .unwrap();

    // Create a master gain node for fade-out on stop
    let gain_node = ctx.create_gain().unwrap();
    let _ = gain_node.connect_with_audio_node(&ctx.destination());

    let generation = STREAM_GEN.with(|g| *g.borrow());
    STREAM_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    STREAM_GAIN.with(|g| *g.borrow_mut() = Some(gain_node.clone()));

    // Spawn the async chunk-processing loop
    wasm_bindgen_futures::spawn_local(chunk_loop(
        ctx,
        gain_node,
        generation,
        source_samples,
        sample_rate,
        final_rate,
        start_sample,
        end_sample,
        params,
    ));

    final_rate
}

/// Async loop that processes and schedules audio chunks.
async fn chunk_loop(
    ctx: AudioContext,
    gain_node: web_sys::GainNode,
    generation: u32,
    source: Arc<Vec<f32>>,
    source_rate: u32,
    final_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
) {
    let mut pos = start_sample;
    // Small initial delay so the first chunk has time to be created
    let mut scheduled_time = ctx.current_time() + 0.02;

    // Fade in from silence to avoid click when crossfading with old stream
    {
        let param = gain_node.gain();
        let _ = param.set_value_at_time(0.0, scheduled_time);
        let _ = param.linear_ramp_to_value_at_time(1.0, scheduled_time + FADE_IN_MS / 1000.0);
    }

    // For auto-gain: pre-scan up to ~15s of the selection so quiet intros
    // don't cause excessive gain, without stalling on very long files.
    let cached_gain: Option<f64> = if params.auto_gain {
        let max_scan = (source_rate as usize) * 15; // ~15 seconds
        let scan_end = end_sample.min(start_sample + max_scan);
        let peak = source[start_sample..scan_end]
            .iter()
            .fold(0.0f32, |mx, s| mx.max(s.abs()));
        if peak < 1e-10 {
            Some(0.0)
        } else {
            let peak_db = 20.0 * (peak as f64).log10();
            Some((-3.0 - peak_db).min(30.0))
        }
    } else {
        None
    };

    while pos < end_sample {
        // Check if this stream has been cancelled
        let current_gen = STREAM_GEN.with(|g| *g.borrow());
        if current_gen != generation {
            break;
        }

        // Determine chunk boundaries with filter warmup overlap
        let warmup_start = if pos > start_sample {
            pos.saturating_sub(FILTER_WARMUP)
        } else {
            pos
        };
        let chunk_end = (pos + CHUNK_SAMPLES).min(end_sample);
        let warmup_len = pos - warmup_start;

        // For PitchShift, add trailing overlap to avoid OLA edge artifacts
        // at the end of each chunk (incomplete window coverage → clicks).
        let trailing_end = if matches!(params.mode, PlaybackMode::PitchShift) {
            (chunk_end + FILTER_WARMUP).min(end_sample)
        } else {
            chunk_end
        };
        let trailing_len = trailing_end - chunk_end;

        let chunk_with_warmup = &source[warmup_start..trailing_end];

        // Apply EQ/bandpass filter
        let filtered = apply_filters(chunk_with_warmup, source_rate, &params);

        // Apply DSP mode transform
        let processed = apply_dsp_mode(&filtered, source_rate, &params);

        // Trim warmup and trailing overlap from the output.
        // All DSP modes preserve input length (output.len() == input.len()),
        // so warmup_len and trailing_len map 1:1 to output positions.
        let trim_start = warmup_len;
        let trim_end = processed.len().saturating_sub(trailing_len);
        let trimmed = if trim_start < trim_end {
            &processed[trim_start..trim_end]
        } else {
            &processed[..]
        };

        // Apply gain
        let mut final_samples = trimmed.to_vec();
        let gain = cached_gain.unwrap_or(params.gain_db);
        apply_gain(&mut final_samples, gain);

        // Schedule this chunk in Web Audio
        if !final_samples.is_empty() {
            schedule_buffer(&ctx, &gain_node, &final_samples, final_rate, scheduled_time);
            let chunk_duration = final_samples.len() as f64 / final_rate as f64;
            scheduled_time += chunk_duration;
        }

        pos = chunk_end;

        // Yield to browser so UI stays responsive
        yield_to_browser().await;

        // If we're well ahead of playback, sleep before processing next chunk
        let now = ctx.current_time();
        if scheduled_time - now > LOOKAHEAD_SECS {
            let sleep_ms = ((scheduled_time - now - LOOKAHEAD_SECS * 0.5) * 1000.0) as u32;
            if sleep_ms > 10 {
                sleep(sleep_ms).await;
            }
        }
    }
}

fn apply_filters(samples: &[f32], sample_rate: u32, params: &PlaybackParams) -> Vec<f32> {
    let mut result = if params.filter_enabled {
        match params.filter_quality {
            FilterQuality::Fast => apply_eq_filter_fast(
                samples, sample_rate,
                params.filter_freq_low, params.filter_freq_high,
                params.filter_db_below, params.filter_db_selected,
                params.filter_db_harmonics, params.filter_db_above,
                params.filter_band_mode,
            ),
            FilterQuality::HQ => apply_eq_filter(
                samples, sample_rate,
                params.filter_freq_low, params.filter_freq_high,
                params.filter_db_below, params.filter_db_selected,
                params.filter_db_harmonics, params.filter_db_above,
                params.filter_band_mode,
            ),
        }
    } else if params.has_selection
        && matches!(
            params.mode,
            PlaybackMode::Normal
                | PlaybackMode::TimeExpansion
                | PlaybackMode::PitchShift
                | PlaybackMode::ZeroCrossing
        )
        && (params.sel_freq_low > 0.0
            || params.sel_freq_high < (sample_rate as f64 / 2.0))
    {
        apply_bandpass(samples, sample_rate, params.sel_freq_low, params.sel_freq_high)
    } else {
        samples.to_vec()
    };

    // Apply notch filters after EQ/bandpass
    if params.notch_enabled && !params.notch_bands.is_empty() {
        result = crate::dsp::notch::apply_notch_filters(
            &result, sample_rate, &params.notch_bands,
            params.notch_harmonic_suppression,
        );
    }

    // Apply spectral subtraction after notch
    if params.noise_reduce_enabled {
        if let Some(ref floor) = params.noise_reduce_floor {
            result = crate::dsp::spectral_sub::apply_spectral_subtraction(
                &result, sample_rate, floor, params.noise_reduce_strength, 0.05,
                params.notch_harmonic_suppression,
            );
        }
    }

    result
}

fn apply_dsp_mode(samples: &[f32], sample_rate: u32, params: &PlaybackParams) -> Vec<f32> {
    match params.mode {
        PlaybackMode::Normal => samples.to_vec(),
        PlaybackMode::Heterodyne => {
            let effective_lo =
                if params.has_selection && (params.sel_freq_low > 0.0 || params.sel_freq_high > 0.0)
                {
                    (params.sel_freq_low + params.sel_freq_high) / 2.0
                } else {
                    params.het_freq
                };
            heterodyne_mix(samples, sample_rate, effective_lo, params.het_cutoff)
        }
        PlaybackMode::TimeExpansion => {
            // Rate change handled by AudioContext sample rate, not sample transform
            samples.to_vec()
        }
        PlaybackMode::PitchShift => pitch_shift_realtime(samples, params.ps_factor),
        PlaybackMode::ZeroCrossing => {
            zc_divide(samples, sample_rate, params.zc_factor as u32, params.filter_enabled)
        }
    }
}

fn schedule_buffer(ctx: &AudioContext, dest: &web_sys::GainNode, samples: &[f32], sample_rate: u32, when: f64) {
    let Ok(buffer) = ctx.create_buffer(1, samples.len() as u32, sample_rate as f32) else {
        return;
    };
    let _ = buffer.copy_to_channel(samples, 0);
    let Ok(source) = ctx.create_buffer_source() else {
        return;
    };
    source.set_buffer(Some(&buffer));
    let _ = source.connect_with_audio_node(dest);
    let _ = source.start_with_when(when);
}

async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback(&resolve);
        }
    });
    let _ = JsFuture::from(promise).await;
}

async fn sleep(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        }
    });
    let _ = JsFuture::from(promise).await;
}
