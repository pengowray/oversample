//! Crash-recovery for in-progress recordings.
//!
//! During a recording, raw PCM samples are streamed to a `<name>.wav.part`
//! file with a placeholder WAV header, and a `<name>.wav.meta.json` sidecar
//! captures the GUANO-relevant context (mic info, device, location, start
//! timestamp). On a clean stop the final encode is written and the partial
//! files are deleted. If the app crashes or is killed, the partial + sidecar
//! remain on disk; `recover_leftover_recordings` is called on next launch to
//! patch the WAV header from the file size, reattach GUANO from the sidecar,
//! and promote the file into the recordings directory.
//!
//! The header is `oversample_core::audio::wav::header_bytes`: 80 bytes with a
//! reserved `JUNK` chunk, so a recording that passes 4 GB is rewritten as
//! RF64 when it stops, without moving the samples. `.part` files from before
//! that have a 44-byte header and are recovered with sizes capped at 4 GB.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use oversample_core::audio::wav::{self as wavfmt, WavWriteFormat};

use crate::recording::NativeSampleFormat;

/// Bumped whenever the sidecar schema changes.
const META_VERSION: u32 = 1;

/// Sidecar file written at recording start. Stores everything needed to
/// reconstruct a full GUANO chunk during recovery.
#[derive(Serialize, Deserialize, Clone)]
pub struct RecoveryMeta {
    pub version: u32,
    pub filename: String,
    pub start_time_iso: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub channels: u16,
    pub connection_type: Option<String>,
    pub mic_name: Option<String>,
    pub mic_make: Option<String>,
    pub device_make: Option<String>,
    pub device_model: Option<String>,
    pub app_version: String,
    pub is_mobile: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub elevation: Option<f64>,
    pub accuracy: Option<f64>,
}

impl RecoveryMeta {
    /// Build the equivalent TauriGuanoParams for reconstruction.
    pub fn to_guano_params(
        &self,
    ) -> (
        crate::recording::TauriGuanoParams,
        Option<chrono::DateTime<chrono::Local>>,
    ) {
        use crate::recording::{RecordingLocation, TauriGuanoParams};
        let location = match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Some(RecordingLocation {
                latitude: lat,
                longitude: lon,
                elevation: self.elevation,
                accuracy: self.accuracy,
            }),
            _ => None,
        };
        let params = TauriGuanoParams {
            connection_type: self.connection_type.clone(),
            location,
            device_make: self.device_make.clone(),
            device_model: self.device_model.clone(),
            mic_name: self.mic_name.clone(),
            mic_make: self.mic_make.clone(),
            app_version: self.app_version.clone(),
            is_mobile: self.is_mobile,
            // Crash recovery can't reconstruct the pre-roll boundary from the
            // raw .part (the cue isn't stored separately), so leave it unset.
            preroll_secs: None,
        };
        let start = chrono::DateTime::parse_from_rfc3339(&self.start_time_iso)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Local));
        (params, start)
    }
}

/// Live state for the disk writer. Held in an Arc<Mutex> so the flush thread
/// and the stop command can both reach it.
pub struct RecoveryWriter {
    pub wav_path: PathBuf,
    pub meta_path: PathBuf,
    pub wav_file: File,
    pub data_bytes_written: u64,
    pub format: WavWriteFormat,
}

/// Container for recovery state that lives on MicState. Cloning bumps the Arc
/// ref count — used to hand a handle to the emitter thread while keeping the
/// same one on MicState for stop-path cleanup.
#[derive(Clone)]
pub struct RecoveryHandle {
    pub writer: Arc<Mutex<Option<RecoveryWriter>>>,
}

pub fn recovery_dir(app_data: &Path) -> PathBuf {
    app_data.join("recordings").join(".recovery")
}

pub fn recordings_dir(app_data: &Path) -> PathBuf {
    app_data.join("recordings")
}

/// Raw `Option<_>` args passed from the WASM side at recording start. Both
/// `mic_start_recording` and `usb_start_recording` accept the same set so the
/// shared recovery-writer construction can live in one place. `shared_fd` is
/// handled at the call site (buffer ownership) rather than here.
pub struct StartArgs {
    pub filename: Option<String>,
    pub connection_type: Option<String>,
    pub mic_name: Option<String>,
    pub mic_make: Option<String>,
    pub device_make: Option<String>,
    pub device_model: Option<String>,
    pub app_version: Option<String>,
    pub loc_latitude: Option<f64>,
    pub loc_longitude: Option<f64>,
    pub loc_elevation: Option<f64>,
    pub loc_accuracy: Option<f64>,
    pub enable_recovery: Option<bool>,
}

/// Build a sidecar `RecoveryMeta` from the flat IPC args + stream info.
pub fn build_meta(
    args: &StartArgs,
    filename: &str,
    format: NativeSampleFormat,
    sample_rate: u32,
    channels: u16,
) -> RecoveryMeta {
    RecoveryMeta {
        version: META_VERSION,
        filename: filename.to_string(),
        start_time_iso: chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
        sample_rate,
        bits_per_sample: format.bits_per_sample(),
        is_float: format.is_float(),
        channels,
        connection_type: args.connection_type.clone(),
        mic_name: args.mic_name.clone(),
        mic_make: args.mic_make.clone(),
        device_make: args.device_make.clone(),
        device_model: args.device_model.clone(),
        app_version: args
            .app_version
            .clone()
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        is_mobile: cfg!(target_os = "android"),
        latitude: args.loc_latitude,
        longitude: args.loc_longitude,
        elevation: args.loc_elevation,
        accuracy: args.loc_accuracy,
    }
}

/// Create the recovery writer + sidecar for a starting recording. Returns
/// `None` when recovery is disabled, when the app_data_dir is unreachable, or
/// when file creation fails. Failures are logged but non-fatal — recording
/// always proceeds, with or without crash safety.
pub fn start_writer(
    app: &tauri::AppHandle,
    args: &StartArgs,
    format: NativeSampleFormat,
    sample_rate: u32,
    channels: u16,
    default_filename_prefix: &str,
) -> Option<RecoveryWriter> {
    if !args.enable_recovery.unwrap_or(false) {
        return None;
    }

    let app_data = match app.path().app_data_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "recovery: no app_data_dir ({}); recording continues without crash safety",
                e
            );
            return None;
        }
    };

    let filename = args.filename.clone().unwrap_or_else(|| {
        chrono::Local::now()
            .format(&format!("{}_%Y%m%d_%H%M%S.wav", default_filename_prefix))
            .to_string()
    });
    let meta = build_meta(args, &filename, format, sample_rate, channels);

    match create(&app_data, &filename, format, sample_rate, channels, &meta) {
        Ok(writer) => {
            eprintln!("recovery: writer installed for {}", filename);
            Some(writer)
        }
        Err(e) => {
            eprintln!(
                "recovery: failed to create writer ({}); recording continues without crash safety",
                e
            );
            None
        }
    }
}

/// Create the partial WAV + sidecar files, write the placeholder header, and
/// return a writer ready to receive appended samples.
pub fn create(
    app_data: &Path,
    filename: &str,
    format: NativeSampleFormat,
    sample_rate: u32,
    channels: u16,
    meta: &RecoveryMeta,
) -> std::io::Result<RecoveryWriter> {
    let dir = recovery_dir(app_data);
    std::fs::create_dir_all(&dir)?;

    let wav_path = dir.join(format!("{}.part", filename));
    let meta_path = dir.join(format!("{}.meta.json", filename));

    // Write the sidecar first so a crash between header write and samples still
    // leaves us with enough context to attempt recovery.
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&meta_path, json)?;

    let mut wav_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&wav_path)?;
    let format = WavWriteFormat {
        sample_rate,
        channels,
        bits_per_sample: format.bits_per_sample(),
        is_float: format.is_float(),
    };
    // Sizes stay 0 until the recording stops (or is recovered).
    wav_file.write_all(&wavfmt::header_bytes(&format, 0, 0))?;
    wav_file.flush()?;

    Ok(RecoveryWriter {
        wav_path,
        meta_path,
        wav_file,
        data_bytes_written: 0,
        format,
    })
}

impl RecoveryWriter {
    /// Append already-encoded little-endian PCM bytes. Used by both the cpal
    /// and USB paths after they compute the new-samples slice under their
    /// respective buffer locks. Keeping the disk write here (and not inside
    /// the lock) means the audio callback is never blocked by I/O.
    pub fn append_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.wav_file.write_all(bytes)?;
        self.data_bytes_written += bytes.len() as u64;
        Ok(())
    }
}

/// Encode a slice of native-format samples to little-endian PCM bytes for the
/// WAV data chunk. Shared by both backends so the on-disk byte layout is
/// guaranteed consistent. Cheap for typical flush sizes (<1 MB).
pub fn encode_samples_i16(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn encode_samples_i24(samples: &[i32]) -> Vec<u8> {
    // cpal delivers 24-bit audio as i32 with zeros in the low byte. WAV i24
    // is packed, little-endian — take the top 3 bytes.
    let mut out = Vec::with_capacity(samples.len() * 3);
    for &s in samples {
        let v = (s >> 8) as i32;
        let b = v.to_le_bytes();
        out.extend_from_slice(&b[0..3]);
    }
    out
}

pub fn encode_samples_i32(samples: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn encode_samples_f32(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Format-aware helper for cpal's multi-format `RecordingBuffer`. Takes
/// everything in the native-format tail buffer, encodes it as little-endian
/// WAV PCM bytes, and leaves the buffer empty. Caller holds the buffer lock
/// during this call and is expected to drop the lock before writing the
/// returned bytes to disk (so the audio callback is not blocked by I/O).
///
/// In streaming-to-disk mode this is called every emitter tick, so the
/// buffer only holds whatever arrived since the last tick (~240 ms worth at
/// steady state). In to-memory mode this is not called and samples
/// accumulate as before.
pub fn drain_cpal_bytes(buf: &mut crate::recording::RecordingBuffer) -> Vec<u8> {
    match buf.format {
        NativeSampleFormat::I16 => {
            if buf.samples_i16.is_empty() {
                return Vec::new();
            }
            let samples = std::mem::take(&mut buf.samples_i16);
            encode_samples_i16(&samples)
        }
        NativeSampleFormat::I24 => {
            if buf.samples_i32.is_empty() {
                return Vec::new();
            }
            let samples = std::mem::take(&mut buf.samples_i32);
            encode_samples_i24(&samples)
        }
        NativeSampleFormat::I32 => {
            if buf.samples_i32.is_empty() {
                return Vec::new();
            }
            let samples = std::mem::take(&mut buf.samples_i32);
            encode_samples_i32(&samples)
        }
        NativeSampleFormat::F32 => {
            if buf.samples_f32.is_empty() {
                return Vec::new();
            }
            let samples = std::mem::take(&mut buf.samples_f32);
            encode_samples_f32(&samples)
        }
    }
}

/// Consume the writer: append any remaining tail bytes + the GUANO chunk,
/// patch the RIFF + data sizes, sync, and close. Returns the path to the
/// now-complete `.wav.part` file so the caller can rename / stream it to the
/// final destination. The sidecar is deleted (we've succeeded), so a failure
/// between this returning and the file being renamed would leave an
/// unrecognizable orphan — callers should do the rename immediately.
pub fn finalize_in_place_and_take(
    mut writer: RecoveryWriter,
    final_tail_bytes: &[u8],
    guano_text: &str,
) -> std::io::Result<PathBuf> {
    if !final_tail_bytes.is_empty() {
        writer.append_bytes(final_tail_bytes)?;
    }

    // Append the GUANO chunk at the end of file, then patch the header sizes
    // (the RIFF size counts the guan chunk too).
    writer.wav_file.seek(SeekFrom::End(0))?;
    let after_data = append_guano(&mut writer.wav_file, writer.data_bytes_written, guano_text)?;
    patch_sizes(
        &mut writer.wav_file,
        &writer.format,
        wavfmt::WRITE_HEADER_LEN as u64,
        writer.data_bytes_written,
        after_data,
    )?;

    // Durability: force kernel buffers to the device so a crash immediately
    // after this returns (e.g. shutdown) doesn't lose the recording.
    writer.wav_file.sync_data()?;

    let RecoveryWriter {
        wav_path,
        meta_path,
        wav_file,
        ..
    } = writer;
    drop(wav_file);
    let _ = std::fs::remove_file(&meta_path);
    Ok(wav_path)
}

/// Delete both the partial WAV and the sidecar. Called from the stop path
/// after the final, properly-encoded WAV has been written elsewhere.
pub fn cleanup(writer: RecoveryWriter) {
    // Drop the file handle before deleting (on Windows a held handle blocks
    // remove). Explicit drop of File via scope end.
    let RecoveryWriter {
        wav_path,
        meta_path,
        ..
    } = writer;
    let _ = std::fs::remove_file(&wav_path);
    let _ = std::fs::remove_file(&meta_path);
}

/// Write the data pad byte (when the sample data has an odd length) and a
/// GUANO chunk at the current position. Returns the bytes written, which the
/// RIFF size has to count.
fn append_guano(f: &mut File, data_bytes: u64, guano_text: &str) -> std::io::Result<u64> {
    let text_bytes = guano_text.as_bytes();
    let data_pad = wavfmt::data_pad_len(data_bytes);
    let text_pad = (text_bytes.len() % 2) as u64;
    if data_pad == 1 {
        f.write_all(&[0u8])?;
    }
    f.write_all(b"guan")?;
    f.write_all(&(text_bytes.len() as u32).to_le_bytes())?;
    f.write_all(text_bytes)?;
    if text_pad == 1 {
        f.write_all(&[0u8])?;
    }
    Ok(data_pad + 8 + text_bytes.len() as u64 + text_pad)
}

/// Write the final sizes into a recording's header. `data_offset` says which
/// header the file has: the current 80-byte one is rewritten whole (as RF64
/// past 4 GB); the old 44-byte one gets its two size fields patched, capped
/// at 4 GB because it has no room for a `ds64` chunk.
fn patch_sizes(
    f: &mut File,
    format: &WavWriteFormat,
    data_offset: u64,
    data_bytes: u64,
    bytes_after_data: u64,
) -> std::io::Result<()> {
    if data_offset == wavfmt::WRITE_HEADER_LEN as u64 {
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&wavfmt::header_bytes(format, data_bytes, bytes_after_data))?;
    } else {
        let riff_size = (data_offset - 8 + data_bytes + bytes_after_data).min(u32::MAX as u64);
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&(riff_size as u32).to_le_bytes())?;
        f.seek(SeekFrom::Start(data_offset - 4))?;
        f.write_all(&(data_bytes.min(u32::MAX as u64) as u32).to_le_bytes())?;
    }
    f.flush()
}

/// Recovered-recording report — returned to the WASM frontend. Canonical
/// definition lives in `oversample_ipc::mic` (shared with the frontend).
pub use oversample_ipc::mic::RecoveredRecording;

/// Scan the recovery directory and finalize any leftover `.wav.part` files.
/// Returns a list of recovered recordings (now moved into the recordings dir).
pub fn recover_leftover_recordings(app_data: &Path) -> Vec<RecoveredRecording> {
    let rec_dir = recovery_dir(app_data);
    let target_dir = recordings_dir(app_data);
    let mut out = Vec::new();

    if !rec_dir.exists() {
        return out;
    }

    let entries = match std::fs::read_dir(&rec_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };

    // Collect .part paths first; we'll delete sidecars as we go.
    let mut parts: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_part = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".wav.part"))
            .unwrap_or(false);
        if is_part {
            parts.push(path);
        }
    }

    for part_path in parts {
        match recover_one(&part_path, &rec_dir, &target_dir) {
            Ok(Some(r)) => out.push(r),
            Ok(None) => {}
            Err(e) => {
                eprintln!("recovery: {} failed: {}", part_path.display(), e);
            }
        }
    }

    out
}

fn recover_one(
    part_path: &Path,
    rec_dir: &Path,
    target_dir: &Path,
) -> std::io::Result<Option<RecoveredRecording>> {
    let file_size = part_path.metadata()?.len();
    let part_name = part_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad name"))?
        .to_string();
    // Strip ".part" → "<name>.wav"
    let wav_name = part_name.trim_end_matches(".part").to_string();
    let meta_path = rec_dir.join(format!("{}.meta.json", wav_name));

    // Find the format and where the samples start. No header, or no
    // samples after it = nothing useful. Delete and move on.
    let header = {
        use std::io::Read;
        let mut buf = Vec::new();
        File::open(part_path)?.take(512).read_to_end(&mut buf)?;
        buf
    };
    let located = wavfmt::locate_samples(&header).filter(|&(_, offset)| file_size > offset);
    let Some((format, data_offset)) = located else {
        let _ = std::fs::remove_file(part_path);
        let _ = std::fs::remove_file(&meta_path);
        return Ok(None);
    };

    // Load sidecar if present.
    let meta: Option<RecoveryMeta> = if meta_path.exists() {
        std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|s| serde_json::from_str::<RecoveryMeta>(&s).ok())
            .filter(|m| m.version == META_VERSION)
    } else {
        None
    };

    // Round the captured byte count down to a whole frame. A crash in the
    // middle of a write can leave a torn last sample (e.g. 1 byte of a 2-byte
    // i16, or 3 bytes of a 4-byte f32). Playing that would produce a single
    // garbled sample at the end.
    let raw_data_bytes = file_size - data_offset;
    let block_align = format.block_align().max(1) as u64;
    let data_bytes = (raw_data_bytes / block_align) * block_align;
    if data_bytes < raw_data_bytes {
        eprintln!(
            "recovery: {} had torn last sample ({} extra bytes), trimming to whole-sample boundary",
            part_path.display(),
            raw_data_bytes - data_bytes,
        );
    }

    let sample_count = data_bytes / block_align;
    let sample_rate = meta.as_ref().map_or(format.sample_rate, |m| m.sample_rate);
    let bits_per_sample = format.bits_per_sample;

    let duration_secs = if sample_rate > 0 {
        sample_count as f64 / sample_rate as f64
    } else {
        0.0
    };

    // Build GUANO. If we have a sidecar, use the real values; otherwise write a
    // minimal "recovered" note so the user can tell this file was reconstructed.
    let had_sidecar = meta.is_some();
    let guano_text = if let Some(m) = meta.as_ref() {
        let (params, start) = m.to_guano_params();
        // Reconstruct the stop-time timestamp from the recorded start + duration
        // so build_tauri_guano's "start_time = stop_time - duration" math
        // reproduces the original Timestamp value.
        let stop_time = start
            .map(|s| s + chrono::Duration::milliseconds((duration_secs * 1000.0) as i64))
            .unwrap_or_else(chrono::Local::now);
        let guano = crate::recording::build_tauri_guano(
            sample_rate,
            sample_count as usize,
            &wav_name,
            &stop_time,
            &params,
        );
        guano.to_text()
    } else {
        format!(
            "GUANO|Version: 1.0\nNote: Recovered after crash (no metadata sidecar)\nSamplerate: {}\nLength: {:.3}\nBits Per Sample: {}\n",
            sample_rate, duration_secs, bits_per_sample,
        )
    };
    // Trim the torn tail (if any), append GUANO, and write the final sizes,
    // in place: a long recording may not fit in memory.
    let after_data = {
        let mut f = OpenOptions::new().read(true).write(true).open(part_path)?;
        f.set_len(data_offset + data_bytes)?;
        f.seek(SeekFrom::End(0))?;
        let after = append_guano(&mut f, data_bytes, &guano_text)?;
        patch_sizes(&mut f, &format, data_offset, data_bytes, after)?;
        f.sync_data()?;
        after
    };
    let final_size = data_offset + data_bytes + after_data;

    // Move into the recordings dir with a "recovered_" prefix so the user can tell.
    std::fs::create_dir_all(target_dir)?;
    let final_name = format!("recovered_{}", wav_name);
    let final_path = target_dir.join(&final_name);
    if std::fs::rename(part_path, &final_path).is_err() {
        std::fs::copy(part_path, &final_path)?;
        let _ = std::fs::remove_file(part_path);
    }
    let _ = std::fs::remove_file(&meta_path);

    Ok(Some(RecoveredRecording {
        path: final_path.to_string_lossy().to_string(),
        filename: final_name,
        had_sidecar,
        sample_count,
        sample_rate,
        duration_secs,
        file_size_bytes: final_size,
    }))
}

impl Default for RecoveryHandle {
    fn default() -> Self {
        Self {
            writer: Arc::new(Mutex::new(None)),
        }
    }
}
