//! Streaming playback engine.
//!
//! Instead of processing the entire selection through DSP before any audio
//! plays, this module processes and schedules audio in small chunks (~0.5s).
//! The user hears sound almost immediately while subsequent chunks are
//! processed in the background.

use std::cell::RefCell;
use std::sync::Arc;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, AudioContextOptions};

use crate::audio::playback::{apply_bandpass, apply_gain};
use crate::audio::source::{AudioSource, ChannelView};
use crate::audio::streaming_source;
use crate::dsp::agc::{AgcConfig, AgcProcessor};
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast, BandMode};
use crate::dsp::heterodyne::{heterodyne_comb_mix, heterodyne_mix};
use crate::dsp::pitch_shift::{pitch_shift_realtime, PitchFactor};
use crate::dsp::zc_divide::zc_divide;
use crate::state::{FilterQuality, GainMode, PlaybackMode};

// Chunk size (~0.5s at 192kHz, ~2s at 44.1kHz) + filter warmup (lets the
// heterodyne lowpass / bandpass settle before the audible region, avoiding
// boundary clicks) are shared with offline export. See audio::chunk_params.
use super::chunk_params::{CHUNK_SAMPLES, FILTER_WARMUP};

/// Overlap samples for PV HQ mode. Each chunk extends by this amount past
/// its nominal end. The trailing overlap gets a Hann fade-out, while the
/// next chunk's leading overlap (same size) gets a Hann fade-in. Web Audio
/// sums them → smooth crossfade, eliminating boundary clicks without
/// needing the warmup-trim hack.
pub(crate) const PV_HQ_OVERLAP: usize = 8192;

/// Samples by which each chunk runs past its nominal end, to be crossfaded
/// with the start of the next chunk. 0 when chunks are trimmed and placed
/// end to end.
///
/// Pitch shift always crossfades, over one grain: two chunks line their
/// grains up differently, so their waveforms differ where they meet, and
/// placed end to end they would click. The phase vocoder crossfades over
/// [`PV_HQ_OVERLAP`] when its Quality is HQ.
pub(crate) fn chunk_overlap(params: &PlaybackParams) -> usize {
    match params.mode {
        PlaybackMode::PitchShift => crate::dsp::pitch_shift::GRAIN,
        PlaybackMode::PhaseVocoder if params.pv_hq => PV_HQ_OVERLAP,
        _ => 0,
    }
}

/// Where a chunk that nominally ends at `chunk_end` stops being kept and
/// stops being read: `(keep_end, read_end)`. Samples from `chunk_end` to
/// `keep_end` are the crossfade overlap. Samples from `keep_end` to
/// `read_end` are read so the shifter has audio after the part that is
/// kept, and are cut off after processing.
pub(crate) fn chunk_tail(
    params: &PlaybackParams,
    chunk_end: usize,
    end_sample: usize,
) -> (usize, usize) {
    let keep_end = (chunk_end + chunk_overlap(params)).min(end_sample);
    let read_end = match params.mode {
        PlaybackMode::PitchShift => (keep_end + FILTER_WARMUP).min(end_sample),
        PlaybackMode::PhaseVocoder if !params.pv_hq => (keep_end + FILTER_WARMUP).min(end_sample),
        _ => keep_end,
    };
    (keep_end, read_end)
}

/// Compensatory gain boost for PhaseVocoder mode (dB).
/// PV's STFT bin-shifting inherently loses ~6-12 dB compared to PitchShift.
pub(crate) const PV_MODE_BOOST_DB: f64 = 6.0;

/// How far ahead (in seconds) to stay buffered beyond current playback time.
const LOOKAHEAD_SECS: f64 = 1.5;

/// Number of chunks to pre-buffer before starting playback.
/// This prevents initial skips/gaps while the first chunks are being processed.
const PREBUFFER_CHUNKS: usize = 5; // 3 might be enough

thread_local! {
    static STREAM_CTX: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
    /// Master gain node for fade-out on stop (avoids clicks).
    static STREAM_GAIN: RefCell<Option<web_sys::GainNode>> = const { RefCell::new(None) };
    /// Monotonically increasing generation counter to detect stale streams.
    static STREAM_GEN: RefCell<u32> = const { RefCell::new(0) };
    /// Web Audio time at which the most recently scheduled buffer ends.
    /// Compared against the audio context's current_time() from the playhead
    /// animation to detect buffer underruns.
    static SCHEDULED_END: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    /// Set once the chunk loop has scheduled the final chunk of the selection.
    /// While this is true the already-scheduled tail is draining normally and
    /// `audio_buffer_ahead_secs` should not be interpreted as an underrun.
    static SCHEDULER_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Upper sample bound for the scheduler this iteration.
///
/// For a play-to-EOF stream (estimated-length MP3/OGG, no bounded selection) we
/// follow the decode frontier: `total_samples()` grows as on-demand prefetch
/// advances and is finalized to the true count at real EOF, so playback covers
/// the whole file even before the background decode has reconciled the length
/// (audit lows #8/#12). For an exact length or a bounded selection, clamp to
/// `end_sample`, re-reading `total_samples()` so an over-estimated tail trims to
/// real EOF instead of scheduling trailing zeros.
fn playback_end_bound(
    source: &Arc<dyn AudioSource>,
    end_sample: usize,
    play_to_eof: bool,
) -> usize {
    if play_to_eof {
        source.total_samples() as usize
    } else {
        end_sample.min(source.total_samples() as usize)
    }
}

/// True once the scheduler has finished (reached the real end of the stream) AND
/// all scheduled audio has played out. The play-to-EOF playhead uses this to stop
/// when the audio actually ends, since the true duration of an estimated-length
/// streaming file isn't known up front.
pub fn playback_drained() -> bool {
    if !SCHEDULER_DONE.with(|d| d.get()) {
        return false;
    }
    STREAM_CTX.with(|ctx| match ctx.borrow().as_ref() {
        Some(audio_ctx) => audio_ctx.current_time() >= SCHEDULED_END.with(|s| s.get()),
        None => true,
    })
}

/// How many seconds of audio are scheduled beyond the audio context's
/// current_time. Returns None if no stream is active. Negative means the
/// scheduler has fallen behind and audio is running out (buffer underrun).
pub fn audio_buffer_ahead_secs() -> Option<f64> {
    // Once the scheduler has queued the final chunk, the remaining lead time
    // only shrinks — that's expected end-of-stream drain, not an underrun.
    if SCHEDULER_DONE.with(|d| d.get()) {
        return None;
    }
    STREAM_CTX.with(|ctx| {
        let borrow = ctx.borrow();
        let audio_ctx = borrow.as_ref()?;
        let sched = SCHEDULED_END.with(|s| s.get());
        if sched <= 0.0 {
            return None;
        }
        Some(sched - audio_ctx.current_time())
    })
}

/// Snapshot of all playback parameters, frozen at play start so that
/// parameter changes mid-playback don't cause glitches.
#[derive(Debug)]
pub(crate) struct PlaybackParams {
    pub mode: PlaybackMode,
    pub het_freq: f64,
    pub het_cutoff: f64,
    /// Heterodyne carrier count (1 = single, >1 = comb).
    pub het_comb_count: u32,
    /// Spacing (Hz) between comb carriers.
    pub het_comb_spacing: f64,
    pub te_factor: f64,
    pub ps_factor: PitchFactor,
    pub pv_factor: PitchFactor,
    pub pv_hq: bool,
    /// Output-side heterodyne shift (Hz) applied AFTER pitch shifting
    /// in PS / PV modes. Compound mapping: `out = |in/factor − shift|`.
    /// Stored in output-Hz space (typically 0–5 kHz) so the LP cutoff
    /// stays narrow. 0 = no shift (pure multiplicative pitch divide).
    pub ps_shift_hz: f64,
    pub zc_factor: f64,
    /// Lowest bat frequency (Hz): ZC division filters from 1 kHz below it.
    pub zc_lowest_bat_hz: f32,
    pub gain_db: f64,
    pub gain_mode: GainMode,
    pub auto_peak_gain_db: f64,
    /// Manual gain (dB) for live monitoring. Applied at the tail of the
    /// live DSP pipeline; ignored by file playback.
    pub live_gain_db: f64,
    pub filter_enabled: bool,
    pub filter_freq_low: f64,
    pub filter_freq_high: f64,
    pub filter_db_below: f64,
    pub filter_db_selected: f64,
    pub filter_db_harmonics: f64,
    pub filter_db_above: f64,
    pub filter_band_mode: BandMode,
    pub filter_quality: FilterQuality,
    pub sel_freq_low: f64,
    pub sel_freq_high: f64,
    pub has_selection: bool,
    pub notch_enabled: bool,
    pub notch_bands: Vec<crate::dsp::notch::NoiseBand>,
    pub noise_reduce_enabled: bool,
    pub noise_reduce_strength: f64,
    pub noise_reduce_floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
}

fn selection_bandpass_active(sample_rate: u32, params: &PlaybackParams) -> bool {
    params.has_selection
        && matches!(
            params.mode,
            PlaybackMode::Normal
                | PlaybackMode::TimeExpansion
                | PlaybackMode::PitchShift
                | PlaybackMode::PhaseVocoder
                | PlaybackMode::ZeroCrossing
        )
        && (params.sel_freq_low > 0.0 || params.sel_freq_high < (sample_rate as f64 / 2.0))
}

/// The fixed gain (dB) of playback and export, before any AGC: none when
/// gain is Off, the slider for Manual and Adaptive, the measured peak gain
/// plus the slider for Peak. The phase vocoder's level boost applies in every
/// mode.
pub(crate) fn fixed_gain_db(
    mode: PlaybackMode,
    gain_mode: GainMode,
    slider_db: f64,
    auto_peak_db: f64,
) -> f64 {
    let mode_boost = if matches!(mode, PlaybackMode::PhaseVocoder) {
        PV_MODE_BOOST_DB
    } else {
        0.0
    };
    mode_boost
        + match gain_mode {
            GainMode::Off => 0.0,
            GainMode::Manual | GainMode::Adaptive => slider_db,
            GainMode::AutoPeak => auto_peak_db + slider_db,
        }
}

/// Duration of fade-in when starting playback (milliseconds).
const FADE_IN_MS: f64 = 30.0;

/// Stop any active streaming playback with a short fade-out to avoid clicks.
///
/// The AudioContext is kept alive for reuse — only the gain node is disconnected.
/// This avoids exhausting the browser's AudioContext limit (~6 in Chrome) when
/// the user presses play repeatedly.
pub(crate) fn stop_stream() {
    STREAM_GEN.with(|g| {
        let mut generation = g.borrow_mut();
        *generation = generation.wrapping_add(1);
    });
    SCHEDULED_END.with(|s| s.set(0.0));
    SCHEDULER_DONE.with(|d| d.set(false));

    // Take the gain node but keep the context alive for reuse.
    let gain = STREAM_GAIN.with(|g| g.borrow_mut().take());
    let ctx = STREAM_CTX.with(|c| c.borrow().clone());

    if let (Some(gain_node), Some(ref old_ctx)) = (gain, ctx) {
        let now = old_ctx.current_time();
        let param = gain_node.gain();
        // Mute immediately to avoid overlap with new stream
        let _ = param.cancel_scheduled_values(now);
        let _ = param.set_value_at_time(0.0, now);
        // Disconnect gain node to release all scheduled buffer sources.
        // The generation counter prevents the chunk loop from scheduling more.
        let _ = gain_node.disconnect();
    }
}

/// Returns true if streaming playback is currently active.
#[allow(dead_code)]
pub(crate) fn is_streaming() -> bool {
    STREAM_CTX.with(|c| c.borrow().is_some())
}

/// Playback-speed multiplier for a sign-encoded time-expansion factor.
///
/// Positive factors slow playback down (expand time); negative speed it up;
/// `|factor| <= 1` is full speed. `te_factor` stays a raw `f64` (it crosses the
/// IPC boundary as a plain number); this just centralizes the decode that
/// `playback.rs` and `start_stream` previously re-derived by hand.
pub(crate) fn te_speed_ratio(te_factor: f64) -> f64 {
    let abs_f = te_factor.abs().max(1.0);
    if te_factor > 0.0 {
        1.0 / abs_f
    } else {
        abs_f
    }
}

/// The lowest and highest sample rates TE playback gives the audio output.
const TE_OUTPUT_RATE_RANGE: (u32, u32) = (8_000, 384_000);

/// The sample rate TE playback gives the audio output for a file at
/// `sample_rate`: the file's rate times [`te_speed_ratio`], kept inside
/// 8 to 384 kHz. A 44.1 kHz file at ×10 plays at 8 kHz, not 4.41 kHz.
pub(crate) fn te_output_rate(sample_rate: u32, te_factor: f64) -> u32 {
    let rate = sample_rate as f64 * te_speed_ratio(te_factor);
    (rate as u32).clamp(TE_OUTPUT_RATE_RANGE.0, TE_OUTPUT_RATE_RANGE.1)
}

/// The TE factor playback really applies to a file at `sample_rate`, in
/// `te_factor`'s convention (above 1 slower, below -1 faster, 1 unchanged).
/// It is `te_factor` unless [`te_output_rate`] had to limit the rate.
/// Playback, the playhead and the axis labels (`crate::heard_axis`) all take
/// the factor from the output rate, so they cannot disagree.
pub(crate) fn te_applied_factor(sample_rate: u32, te_factor: f64) -> f64 {
    let out = te_output_rate(sample_rate, te_factor) as f64;
    let rate = sample_rate as f64;
    if out < rate {
        rate / out
    } else if out > rate {
        -(out / rate)
    } else {
        1.0
    }
}

/// How fast TE playback of a file at `sample_rate` runs, as a multiple of
/// the recording's own speed: [`te_speed_ratio`] of [`te_applied_factor`].
pub(crate) fn te_playback_speed(sample_rate: u32, te_factor: f64) -> f64 {
    te_speed_ratio(te_applied_factor(sample_rate, te_factor))
}

/// Start streaming playback of a sample range.
///
/// `source` provides sample data; `channel_view` selects which channel(s) to play.
/// Returns the final playback sample rate (may differ from source for TE mode),
/// or `None` if AudioContext creation failed.
pub(crate) fn start_stream(
    source: Arc<dyn AudioSource>,
    channel_view: ChannelView,
    sample_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
    play_to_eof: bool,
) -> Option<u32> {
    stop_stream();

    let final_rate = match params.mode {
        PlaybackMode::TimeExpansion => te_output_rate(sample_rate, params.te_factor),
        _ => sample_rate,
    };

    // Stereo output: stereo source + Stereo view (all modes, not just Normal)
    let stereo_out = source.channel_count() >= 2 && channel_view == ChannelView::Stereo;

    // Reuse existing AudioContext if its sample rate matches; otherwise create new.
    let ctx = STREAM_CTX.with(|c| {
        let existing = c.borrow().clone();
        if let Some(ref ctx) = existing {
            if (ctx.sample_rate() - final_rate as f32).abs() < 1.0 {
                let _ = ctx.resume();
                return Some(ctx.clone());
            }
            // Sample rate changed — must close old and create new
            let _ = ctx.close();
            *c.borrow_mut() = None;
        }
        let opts = AudioContextOptions::new();
        opts.set_sample_rate(final_rate as f32);
        let new_ctx = AudioContext::new_with_context_options(&opts)
            .or_else(|_| AudioContext::new())
            .ok()?;
        *c.borrow_mut() = Some(new_ctx.clone());
        Some(new_ctx)
    });
    let Some(ctx) = ctx else {
        web_sys::console::warn_1(&"Playback failed: could not create AudioContext".into());
        return None;
    };

    // Create a master gain node for fade-out on stop
    let gain_node = ctx.create_gain().unwrap();
    let _ = gain_node.connect_with_audio_node(&ctx.destination());

    let generation = STREAM_GEN.with(|g| *g.borrow());
    STREAM_GAIN.with(|g| *g.borrow_mut() = Some(gain_node.clone()));

    // Spawn the async chunk-processing loop
    wasm_bindgen_futures::spawn_local(chunk_loop(
        ctx,
        gain_node,
        generation,
        source,
        channel_view,
        stereo_out,
        sample_rate,
        final_rate,
        start_sample,
        end_sample,
        params,
        play_to_eof,
    ));

    Some(final_rate)
}

/// Async loop that processes and schedules audio chunks.
async fn chunk_loop(
    ctx: AudioContext,
    gain_node: web_sys::GainNode,
    generation: u32,
    source: Arc<dyn AudioSource>,
    channel_view: ChannelView,
    stereo_out: bool,
    source_rate: u32,
    final_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
    play_to_eof: bool,
) {
    let mut pos = start_sample;
    let crossfade_mode = chunk_overlap(&params) > 0;

    // Gain computation depends on mode:
    // - Off: no gain at all (0 dB)
    // - Manual: gain_db slider only
    // - AutoPeak: pre-scan peak normalization + gain_db slider on top
    // - Adaptive: AGC leveler with smooth envelope following (applied in process_one_chunk)
    let is_adaptive = params.gain_mode == GainMode::Adaptive;
    let manual_boost = params.gain_db; // slider value, additive for all modes

    let auto_peak_gain: f64 = params.auto_peak_gain_db;

    // AGC processor for Adaptive mode — persists across chunks for smooth gain transitions
    let agc = if is_adaptive {
        Some(RefCell::new(AgcProcessor::new(
            AgcConfig::default(),
            final_rate,
        )))
    } else {
        None
    };

    // AGC (Adaptive) is applied per-sample in process_one_chunk, after this.
    let global_gain = fixed_gain_db(params.mode, params.gain_mode, manual_boost, auto_peak_gain);

    // ── Pre-buffer phase ─────────────────────────────────────────────────────
    // Process PREBUFFER_CHUNKS chunks before scheduling anything, so Web Audio
    // has a comfortable lead and the first audible chunk doesn't skip.
    let mut prebuf: Vec<ChunkAudio> = Vec::with_capacity(PREBUFFER_CHUNKS);

    for _ in 0..PREBUFFER_CHUNKS {
        let effective_end = playback_end_bound(&source, end_sample, play_to_eof);
        if pos >= effective_end {
            break;
        }
        let current_gen = STREAM_GEN.with(|g| *g.borrow());
        if current_gen != generation {
            return;
        }

        let (chunk, new_pos) = process_one_chunk(
            &source,
            channel_view,
            stereo_out,
            source_rate,
            &params,
            global_gain,
            agc.as_ref(),
            pos,
            start_sample,
            effective_end,
        )
        .await;
        pos = new_pos;

        prebuf.push(chunk);

        // Yield between pre-buffer chunks so the UI stays responsive
        yield_to_browser().await;
    }

    // Now schedule all pre-buffered chunks at once, starting slightly in the future
    let mut scheduled_time = ctx.current_time() + 0.02;
    SCHEDULED_END.with(|s| s.set(scheduled_time));

    // Fade in from silence to avoid click when crossfading with old stream
    {
        let param = gain_node.gain();
        let _ = param.set_value_at_time(0.0, scheduled_time);
        let _ = param.linear_ramp_to_value_at_time(1.0, scheduled_time + FADE_IN_MS / 1000.0);
    }

    // With crossfaded chunks, stride duration is CHUNK_SAMPLES/rate (the
    // overlap is extra audio that blends with the next chunk via crossfade).
    let stride_duration = CHUNK_SAMPLES as f64 / final_rate as f64;

    let advance = |chunk: &ChunkAudio| {
        if crossfade_mode {
            // Advance by stride only; the trailing overlap is summed with the
            // next chunk's fade-in by Web Audio.
            stride_duration
        } else {
            chunk.frames() as f64 / final_rate as f64
        }
    };

    for chunk in prebuf {
        if chunk.frames() > 0 {
            chunk.schedule(&ctx, &gain_node, final_rate, scheduled_time);
            scheduled_time += advance(&chunk);
            SCHEDULED_END.with(|s| s.set(scheduled_time));
        }
    }

    // ── Streaming phase ──────────────────────────────────────────────────────
    // Continue processing remaining chunks with lookahead throttling.
    loop {
        // Recompute the bound each iteration: follow the decode frontier for a
        // play-to-EOF stream, or clamp to end_sample otherwise (see
        // `playback_end_bound`). No-op for exact-length sources.
        let effective_end = playback_end_bound(&source, end_sample, play_to_eof);
        if pos >= effective_end {
            break;
        }
        let current_gen = STREAM_GEN.with(|g| *g.borrow());
        if current_gen != generation {
            break;
        }

        let (chunk, new_pos) = process_one_chunk(
            &source,
            channel_view,
            stereo_out,
            source_rate,
            &params,
            global_gain,
            agc.as_ref(),
            pos,
            start_sample,
            effective_end,
        )
        .await;
        pos = new_pos;

        if chunk.frames() > 0 {
            chunk.schedule(&ctx, &gain_node, final_rate, scheduled_time);
            scheduled_time += advance(&chunk);
            SCHEDULED_END.with(|s| s.set(scheduled_time));
        }

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

    // Scheduler finished naturally — mark so the playhead animation treats the
    // draining tail as healthy rather than an underrun. Guard on generation so
    // a concurrent stop_stream (which already reset the flag) wins.
    let current_gen = STREAM_GEN.with(|g| *g.borrow());
    if current_gen == generation {
        SCHEDULER_DONE.with(|d| d.set(true));
    }
}

/// Audio for one scheduled chunk.
enum ChunkAudio {
    Mono(Vec<f32>),
    Stereo { left: Vec<f32>, right: Vec<f32> },
}

impl ChunkAudio {
    fn frames(&self) -> usize {
        match self {
            ChunkAudio::Mono(samples) => samples.len(),
            ChunkAudio::Stereo { left, right } => left.len().min(right.len()),
        }
    }

    fn schedule(&self, ctx: &AudioContext, dest: &web_sys::GainNode, rate: u32, when: f64) {
        match self {
            ChunkAudio::Mono(samples) => schedule_buffer(ctx, dest, samples, rate, when),
            ChunkAudio::Stereo { left, right } => {
                schedule_buffer_stereo(ctx, dest, left, right, rate, when)
            }
        }
    }
}

/// Process a single chunk: prefetch, read, filter, DSP, trim, gain.
/// With `stereo_out`, left and right each go through the whole pipeline on their
/// own. Returns the chunk and the next read position.
async fn process_one_chunk(
    source: &Arc<dyn AudioSource>,
    channel_view: ChannelView,
    stereo_out: bool,
    source_rate: u32,
    params: &PlaybackParams,
    global_gain: f64,
    agc: Option<&RefCell<AgcProcessor>>,
    pos: usize,
    start_sample: usize,
    end_sample: usize,
) -> (ChunkAudio, usize) {
    let overlap = chunk_overlap(params);

    let warmup_start = pos.saturating_sub(FILTER_WARMUP);
    let chunk_end = (pos + CHUNK_SAMPLES).min(end_sample);
    let warmup_len = pos - warmup_start;

    // With crossfaded chunks, the chunk is kept past its nominal end by the
    // overlap, which is crossfaded with the next chunk's start. Anything
    // read after that is cut off again once it has been processed.
    let (keep_end, trailing_end) = chunk_tail(params, chunk_end, end_sample);
    let trailing_len = keep_end - chunk_end;
    let discard_len = trailing_end - keep_end;

    // One read per channel view: L and R in stereo, otherwise the chosen view.
    let views: &[ChannelView] = if stereo_out {
        &[ChannelView::Channel(0), ChannelView::Channel(1)]
    } else {
        std::slice::from_ref(&channel_view)
    };
    let read_len = trailing_end - warmup_start;
    let raw_views =
        streaming_source::read_views_ready(source.as_ref(), views, warmup_start as u64, read_len)
            .await
            .views
            // If part of the region couldn't be loaded, play what is there (missing
            // chunks are silent) rather than stalling the chunk schedule.
            .unwrap_or_else(|| {
                views
                    .iter()
                    .map(|&cv| source.read_region(cv, warmup_start as u64, read_len))
                    .collect()
            });
    let mut raw_views = raw_views.into_iter();

    // Hann fades for the crossfade overlap, applied after the warmup is trimmed.
    let core_len = chunk_end - pos; // nominal chunk length (without overlap)
    let apply_pv_hq_fading = |buf: &mut Vec<f32>| {
        // Hann fade-in on leading overlap (first chunk's start is clean)
        if pos > start_sample {
            let fade_in_len = overlap.min(core_len).min(buf.len());
            for (i, sample) in buf.iter_mut().enumerate().take(fade_in_len) {
                let t = i as f32 / fade_in_len as f32;
                let w = 0.5 * (1.0 - (std::f32::consts::PI * t).cos());
                *sample *= w;
            }
        }
        // Hann fade-out on trailing overlap
        if trailing_len > 0 {
            let fade_out_start = buf.len().saturating_sub(trailing_len);
            let fade_out_len = buf.len() - fade_out_start;
            for i in 0..fade_out_len {
                let t = i as f32 / fade_out_len as f32;
                let w = 0.5 * (1.0 + (std::f32::consts::PI * t).cos());
                buf[fade_out_start + i] *= w;
            }
        }
    };

    // Read one channel view and run it through filters, DSP, trim and gain.
    let process_view = |raw: Vec<f32>| -> Vec<f32> {
        let filtered = apply_filters(&raw, source_rate, params);
        let processed = apply_dsp_mode(&filtered, source_rate, params);
        // Trim the warmup and what was read past the part that is kept.
        let trim_end = processed.len().saturating_sub(discard_len);
        let mut out = if warmup_len < trim_end {
            processed[warmup_len..trim_end].to_vec()
        } else {
            processed
        };
        if overlap > 0 {
            apply_pv_hq_fading(&mut out);
        }
        apply_gain(&mut out, global_gain);
        out
    };

    let chunk = if stereo_out {
        let mut left = process_view(raw_views.next().unwrap_or_default());
        let mut right = process_view(raw_views.next().unwrap_or_default());
        if let Some(agc_cell) = agc {
            agc_cell.borrow_mut().process_stereo(&mut left, &mut right);
        }
        ChunkAudio::Stereo { left, right }
    } else {
        let mut samples = process_view(raw_views.next().unwrap_or_default());
        if let Some(agc_cell) = agc {
            agc_cell.borrow_mut().process(&mut samples);
        }
        ChunkAudio::Mono(samples)
    };

    (chunk, chunk_end)
}

pub(crate) fn apply_filters(
    samples: &[f32],
    sample_rate: u32,
    params: &PlaybackParams,
) -> Vec<f32> {
    let mut result = if params.filter_enabled {
        match params.filter_quality {
            FilterQuality::Fast => apply_eq_filter_fast(
                samples,
                sample_rate,
                params.filter_freq_low,
                params.filter_freq_high,
                params.filter_db_below,
                params.filter_db_selected,
                params.filter_db_harmonics,
                params.filter_db_above,
                params.filter_band_mode,
            ),
            FilterQuality::Spectral => apply_eq_filter(
                samples,
                sample_rate,
                params.filter_freq_low,
                params.filter_freq_high,
                params.filter_db_below,
                params.filter_db_selected,
                params.filter_db_harmonics,
                params.filter_db_above,
                params.filter_band_mode,
            ),
        }
    } else if selection_bandpass_active(sample_rate, params) {
        apply_bandpass(
            samples,
            sample_rate,
            params.sel_freq_low,
            params.sel_freq_high,
        )
    } else {
        samples.to_vec()
    };

    // Apply notch filters after EQ/bandpass
    if params.notch_enabled && !params.notch_bands.is_empty() {
        result = crate::dsp::notch::apply_notch_filters(&result, sample_rate, &params.notch_bands);
    }

    // Apply spectral subtraction after notch
    if params.noise_reduce_enabled {
        if let Some(ref floor) = params.noise_reduce_floor {
            result = crate::dsp::spectral_sub::apply_spectral_subtraction(
                &result,
                sample_rate,
                floor,
                params.noise_reduce_strength,
                0.05,
            );
        }
    }

    result
}

/// Compute the carrier frequencies for the heterodyne mode. `base` is the
/// LOWEST carrier; the comb tiles UPWARD from it:
/// `base, base + spacing, …, base + (count-1)·spacing`. With count = 1 this is
/// just `[base]` (classic single-carrier). Anchoring at the bottom — rather
/// than centering on `base` — lets the user tune the low edge and grow the
/// comb up to cover the focus band from below.
pub(crate) fn het_carriers(base: f64, spacing: f64, count: u32) -> Vec<f64> {
    let n = count.max(1);
    if n == 1 || spacing <= 0.0 {
        return vec![base];
    }
    (0..n).map(|i| base + i as f64 * spacing).collect()
}

/// The selection's frequency band, when it has one. A selection that only
/// covers a time range (its band is 0 Hz to Nyquist) has none, so it doesn't
/// retune HET.
pub(crate) fn selection_freq_band(
    sel_freq_low: f64,
    sel_freq_high: f64,
    has_selection: bool,
    nyquist: f64,
) -> Option<(f64, f64)> {
    (has_selection && (sel_freq_low > 0.0 || sel_freq_high < nyquist))
        .then_some((sel_freq_low, sel_freq_high))
}

/// The carriers HET mixes with, in playback and in the XForm display: one
/// carrier at `het_freq`, or a comb tiling upward from it. With a selection
/// band, a single carrier sits at the band's centre and a comb starts at its
/// low edge.
pub(crate) fn het_carriers_for(
    het_freq: f64,
    comb_count: u32,
    comb_spacing: f64,
    band: Option<(f64, f64)>,
) -> Vec<f64> {
    if comb_count <= 1 {
        vec![band.map_or(het_freq, |(lo, hi)| (lo + hi) / 2.0)]
    } else {
        het_carriers(
            band.map_or(het_freq, |(lo, _)| lo),
            comb_spacing,
            comb_count,
        )
    }
}

/// Post-pitch heterodyne shift for PS / PV modes. The shift value is
/// expressed in OUTPUT-Hz space, so it's small (typically 0–5 kHz) and
/// the LP cutoff stays narrow, which keeps the low-pass well-behaved.
///
/// Mathematically equivalent to a pre-pitch shift of `shift × factor`:
///     `|in/factor − shift_out| ≡ |in − shift_out·factor| / factor`
/// We pick the post-pitch ordering because the LP cutoff (just above the
/// final output band) is much smaller and easier to design than the
/// pre-pitch ordering's wide-band LP near Nyquist.
fn ps_post_shift(samples: &[f32], sample_rate: u32, params: &PlaybackParams) -> Vec<f32> {
    let shift = params.ps_shift_hz.abs();
    if shift < 1.0 {
        return samples.to_vec();
    }
    let factor = match params.mode {
        PlaybackMode::PitchShift => params.ps_factor,
        PlaybackMode::PhaseVocoder => params.pv_factor,
        _ => return samples.to_vec(),
    }
    .magnitude();
    let cutoff = ps_post_shift_cutoff(params.filter_freq_high, factor, sample_rate);
    heterodyne_mix(samples, sample_rate, shift, cutoff)
}

/// Low-pass cutoff for [`ps_post_shift`]: just above the final output band
/// (the bandpass upper edge / factor) with a safety margin, at least 20 kHz
/// (the post-pitch signal already has its spectrum compressed by `factor`)
/// and at most 90% of Nyquist. The Nyquist limit wins: at 44.1 kHz, 90% of
/// Nyquist is 19.8 kHz, and `f64::clamp(20_000.0, 19_845.0)` panics.
fn ps_post_shift_cutoff(filter_freq_high: f64, factor: f64, sample_rate: u32) -> f64 {
    let nyquist = sample_rate as f64 * 0.5;
    (filter_freq_high / factor + 5_000.0)
        .max(20_000.0)
        .min(nyquist * 0.9)
}

pub(crate) fn apply_dsp_mode(
    samples: &[f32],
    sample_rate: u32,
    params: &PlaybackParams,
) -> Vec<f32> {
    match params.mode {
        PlaybackMode::Normal => samples.to_vec(),
        PlaybackMode::Heterodyne => {
            let band = selection_freq_band(
                params.sel_freq_low,
                params.sel_freq_high,
                params.has_selection,
                sample_rate as f64 / 2.0,
            );
            let carriers = het_carriers_for(
                params.het_freq,
                params.het_comb_count,
                params.het_comb_spacing,
                band,
            );
            heterodyne_comb_mix(samples, sample_rate, &carriers, params.het_cutoff)
        }
        PlaybackMode::TimeExpansion => {
            // Rate change handled by AudioContext sample rate, not sample transform
            samples.to_vec()
        }
        PlaybackMode::PitchShift => {
            let pitched = pitch_shift_realtime(samples, params.ps_factor);
            ps_post_shift(&pitched, sample_rate, params)
        }
        PlaybackMode::PhaseVocoder => {
            let pitched =
                crate::dsp::phase_vocoder::phase_vocoder_pitch_shift(samples, params.pv_factor);
            ps_post_shift(&pitched, sample_rate, params)
        }
        PlaybackMode::ZeroCrossing => zc_divide(
            samples,
            sample_rate,
            params.zc_factor as u32,
            params.filter_enabled,
            params.zc_lowest_bat_hz,
        ),
    }
}

/// A sample rate every browser accepts for an audio buffer: the Web Audio
/// specification requires support up to 96 kHz, and no more.
const SAFE_BUFFER_RATE: u32 = 96_000;

/// How many input samples go into one output sample when a buffer at
/// `sample_rate` has to be lowered to [`SAFE_BUFFER_RATE`] or under.
fn safe_rate_divisor(sample_rate: u32) -> u32 {
    sample_rate.div_ceil(SAFE_BUFFER_RATE).max(1)
}

/// An audio buffer holding `channels` (all the same length) at `sample_rate`.
///
/// Chrome takes buffers up to 768 kHz. Firefox refuses the rates of most bat
/// recordings, and playback of a 500 kHz file was silent there in every mode
/// but TE. When the browser refuses the rate, the samples are low-passed and
/// thinned to a rate it must accept. The buffer lasts as long either way, so
/// the scheduling does not change. Sound above half the lower rate is lost,
/// which no speaker would have played.
fn make_buffer(
    ctx: &AudioContext,
    channels: &[&[f32]],
    sample_rate: u32,
) -> Option<web_sys::AudioBuffer> {
    let len = channels.iter().map(|c| c.len()).min()?;
    let count = channels.len() as u32;
    if let Ok(buffer) = ctx.create_buffer(count, len as u32, sample_rate as f32) {
        for (i, ch) in channels.iter().enumerate() {
            let _ = buffer.copy_to_channel(&ch[..len], i as i32);
        }
        return Some(buffer);
    }
    let m = safe_rate_divisor(sample_rate);
    if m <= 1 {
        return None;
    }
    let thinned: Vec<Vec<f32>> = channels
        .iter()
        .map(|ch| crate::dsp::filters::decimate(&ch[..len], sample_rate, sample_rate / m))
        .collect();
    let out_len = thinned.iter().map(|c| c.len()).min()?;
    let buffer = ctx
        .create_buffer(count, out_len as u32, sample_rate as f32 / m as f32)
        .ok()?;
    for (i, ch) in thinned.iter().enumerate() {
        let _ = buffer.copy_to_channel(&ch[..out_len], i as i32);
    }
    Some(buffer)
}

fn schedule_buffer(
    ctx: &AudioContext,
    dest: &web_sys::GainNode,
    samples: &[f32],
    sample_rate: u32,
    when: f64,
) {
    let Some(buffer) = make_buffer(ctx, &[samples], sample_rate) else {
        return;
    };
    let Ok(source) = ctx.create_buffer_source() else {
        return;
    };
    source.set_buffer(Some(&buffer));
    let _ = source.connect_with_audio_node(dest);
    let _ = source.start_with_when(when);
}

fn schedule_buffer_stereo(
    ctx: &AudioContext,
    dest: &web_sys::GainNode,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    when: f64,
) {
    let Some(buffer) = make_buffer(ctx, &[left, right], sample_rate) else {
        return;
    };
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

#[cfg(test)]
mod tests {

    /// The factor TE applies is the one asked for while the output rate is
    /// inside 8 to 384 kHz, and the one the limited rate gives outside it.
    #[test]
    fn te_applied_factor_follows_the_output_rate() {
        use super::{te_applied_factor, te_output_rate, te_playback_speed};
        assert_eq!(te_output_rate(256_000, 10.0), 25_600);
        assert_eq!(te_applied_factor(256_000, 10.0), 10.0);
        assert_eq!(te_applied_factor(256_000, 1.0), 1.0);
        assert_eq!(te_applied_factor(48_000, -4.0), -4.0);
        // 44.1 kHz at x10 would be 4.41 kHz: limited to 8 kHz.
        assert_eq!(te_output_rate(44_100, 10.0), 8_000);
        assert!((te_applied_factor(44_100, 10.0) - 5.5125).abs() < 1e-12);
        assert!((te_playback_speed(44_100, 10.0) - 8_000.0 / 44_100.0).abs() < 1e-12);
        // 256 kHz sped up x2 would be 512 kHz: limited to 384 kHz.
        assert_eq!(te_output_rate(256_000, -2.0), 384_000);
        assert!((te_applied_factor(256_000, -2.0) + 1.5).abs() < 1e-12);
        assert!((te_playback_speed(256_000, -2.0) - 1.5).abs() < 1e-12);
    }

    /// A refused buffer rate is divided down to 96 kHz or under.
    #[test]
    fn a_refused_buffer_rate_is_divided_down_to_a_safe_one() {
        use super::{safe_rate_divisor, SAFE_BUFFER_RATE};
        for rate in [44_100, 96_000, 192_000, 256_000, 384_000, 500_000] {
            let m = safe_rate_divisor(rate);
            assert!(rate / m <= SAFE_BUFFER_RATE, "{rate} / {m}");
            assert!(m == 1 || rate / (m - 1) > SAFE_BUFFER_RATE, "{rate} / {m}");
        }
        assert_eq!(safe_rate_divisor(500_000), 6);
        assert_eq!(safe_rate_divisor(96_000), 1);
    }

    #[test]
    fn ps_post_shift_cutoff_fits_under_nyquist() {
        use super::ps_post_shift_cutoff;
        // 192 kHz: 20 kHz floor, or the band edge / factor + 5 kHz.
        let near = |a: f64, b: f64| (a - b).abs() < 0.01;
        assert!(near(
            ps_post_shift_cutoff(60_000.0, 10.0, 192_000),
            20_000.0
        ));
        assert!(near(
            ps_post_shift_cutoff(400_000.0, 10.0, 192_000),
            45_000.0
        ));
        // At 44.1 kHz and 16 kHz the 20 kHz floor is above 90% of Nyquist.
        assert!(near(ps_post_shift_cutoff(22_050.0, 10.0, 44_100), 19_845.0));
        assert!(near(ps_post_shift_cutoff(8_000.0, 2.0, 16_000), 7_200.0));
    }

    #[test]
    fn time_only_selection_does_not_retune_het() {
        use super::{het_carriers_for, selection_freq_band};
        let nyq = 192_000.0;
        assert_eq!(selection_freq_band(0.0, nyq, true, nyq), None);
        assert_eq!(selection_freq_band(30_000.0, 50_000.0, false, nyq), None);
        assert_eq!(
            selection_freq_band(30_000.0, 50_000.0, true, nyq),
            Some((30_000.0, 50_000.0))
        );
        assert_eq!(
            het_carriers_for(45_000.0, 1, 30_000.0, None),
            vec![45_000.0]
        );
        assert_eq!(
            het_carriers_for(45_000.0, 1, 30_000.0, Some((30_000.0, 50_000.0))),
            vec![40_000.0]
        );
        assert_eq!(
            het_carriers_for(31_000.0, 3, 30_000.0, None),
            vec![31_000.0, 61_000.0, 91_000.0]
        );
        assert_eq!(
            het_carriers_for(31_000.0, 2, 30_000.0, Some((20_000.0, 80_000.0))),
            vec![20_000.0, 50_000.0]
        );
    }

    use super::het_carriers;

    #[test]
    fn het_carriers_single_is_just_base() {
        assert_eq!(het_carriers(45_000.0, 30_000.0, 1), vec![45_000.0]);
        // Zero count clamps to 1; zero/non-positive spacing collapses to one carrier.
        assert_eq!(het_carriers(45_000.0, 30_000.0, 0), vec![45_000.0]);
        assert_eq!(het_carriers(45_000.0, 0.0, 4), vec![45_000.0]);
    }

    #[test]
    fn het_carriers_tile_upward_from_base() {
        // Bottom-anchored: the first carrier is the base, the rest tile upward.
        assert_eq!(
            het_carriers(20_000.0, 10_000.0, 3),
            vec![20_000.0, 30_000.0, 40_000.0]
        );
        assert_eq!(
            het_carriers(30_000.0, 12_000.0, 2),
            vec![30_000.0, 42_000.0]
        );
        // The base is always the lowest carrier — the comb never dips below it.
        let c = het_carriers(30_000.0, 12_000.0, 4);
        let lowest = c.iter().cloned().fold(f64::INFINITY, f64::min);
        assert_eq!(lowest, 30_000.0);
    }
}
