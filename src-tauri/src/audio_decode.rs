//! Native audio file decoding for Tauri.
//!
//! Provides Tauri commands to read audio file metadata and decode files
//! to mono f32 samples natively on a background thread, avoiding the
//! need to pass entire file bytes through the WASM boundary.

use serde::Serialize;
use std::io::Cursor;
use std::path::Path;

#[derive(Serialize, Clone, Debug)]
pub struct AudioFileInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub duration_secs: f64,
    pub total_mono_samples: usize,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub format: String,
    pub file_size: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct FullDecodeResult {
    pub info: AudioFileInfo,
    pub samples: Vec<f32>,
}

/// Read audio file metadata without decoding samples.
pub fn file_info(path: &str) -> Result<AudioFileInfo, String> {
    let path = Path::new(path);
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len();

    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" if is_w4v(&bytes) => w4v_info(&bytes, file_size),
        b"RIFF" => wav_info(&bytes, file_size),
        b"fLaC" => flac_info(&bytes, file_size),
        b"OggS" => ogg_info(&bytes, file_size),
        _ if is_m4a(&bytes) => m4a_info(&bytes, file_size),
        _ if is_mp3(&bytes) => mp3_info(&bytes, file_size),
        _ => Err("Unknown audio format (expected WAV, W4V, FLAC, OGG, MP3, or M4A)".into()),
    }
}

/// Decode entire audio file to mono f32 samples.
pub fn decode_full(path: &str) -> Result<FullDecodeResult, String> {
    let path = Path::new(path);
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len();

    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" if is_w4v(&bytes) => decode_w4v(&bytes, file_size),
        b"RIFF" => decode_wav(&bytes, file_size),
        b"fLaC" => decode_flac(&bytes, file_size),
        b"OggS" => decode_ogg(&bytes, file_size),
        _ if is_m4a(&bytes) => decode_m4a(&bytes, file_size),
        _ if is_mp3(&bytes) => decode_mp3(&bytes, file_size),
        _ => Err("Unknown audio format".into()),
    }
}

fn is_mp3(bytes: &[u8]) -> bool {
    if bytes.len() >= 3 && &bytes[0..3] == b"ID3" {
        return true;
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return true;
    }
    false
}

fn is_m4a(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

/// Wildlife Acoustics W4V format tag.
const W4V_FORMAT_TAG: u16 = 0x5741;
const W4V_BLOCK_SAMPLES: usize = 512;
const W4V_BLOCK_HEADER: usize = 8;

fn is_w4v(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[8..12] != b"WAVE" {
        return false;
    }
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(
            bytes[pos + 4..pos + 8].try_into().unwrap_or([0; 4]),
        ) as usize;
        if chunk_id == b"fmt " && chunk_size >= 2 && pos + 10 <= bytes.len() {
            let format_tag = u16::from_le_bytes([bytes[pos + 8], bytes[pos + 9]]);
            return format_tag == W4V_FORMAT_TAG;
        }
        pos = pos + 8 + ((chunk_size + 1) & !1);
    }
    false
}

fn mix_to_mono(samples: &[f32], channels: u32) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

// ── WAV ─────────────────────────────────────────────────────────────

fn wav_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV error: {e}"))?;
    let spec = reader.spec();
    let total_samples = reader.len() as usize;
    let channels = spec.channels as u32;
    let mono_samples = total_samples / channels as usize;
    Ok(AudioFileInfo {
        sample_rate: spec.sample_rate,
        channels,
        duration_secs: mono_samples as f64 / spec.sample_rate as f64,
        total_mono_samples: mono_samples,
        bits_per_sample: spec.bits_per_sample,
        is_float: matches!(spec.sample_format, hound::SampleFormat::Float),
        format: "WAV".into(),
        file_size,
    })
}

fn decode_wav(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV error: {e}"))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as u32;
    let bits_per_sample = spec.bits_per_sample;
    let is_float = matches!(spec.sample_format, hound::SampleFormat::Float);

    let all_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV sample error: {e}"))?,
        hound::SampleFormat::Int => {
            let max_val = (1u32 << (bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("WAV sample error: {e}"))?
                .into_iter()
                .map(|s| s as f32 / max_val)
                .collect()
        }
    };

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample,
            is_float,
            format: "WAV".into(),
            file_size,
        },
        samples,
    })
}

// ── FLAC ────────────────────────────────────────────────────────────

fn flac_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let cursor = Cursor::new(bytes);
    let reader = claxon::FlacReader::new(cursor).map_err(|e| format!("FLAC error: {e}"))?;
    let info = reader.streaminfo();
    // info.samples is Option<u64> — total inter-channel frames. Use u64 arithmetic
    // to avoid overflow on 32-bit targets for files with > 2^32 frames.
    let total_frames = info.samples.unwrap_or(0);
    let mono_samples = total_frames as usize; // safe on 64-bit Tauri targets
    Ok(AudioFileInfo {
        sample_rate: info.sample_rate,
        channels: info.channels,
        duration_secs: total_frames as f64 / info.sample_rate as f64,
        total_mono_samples: mono_samples,
        bits_per_sample: info.bits_per_sample as u16,
        is_float: false,
        format: "FLAC".into(),
        file_size,
    })
}

fn decode_flac(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    let cursor = Cursor::new(bytes);
    let mut reader = claxon::FlacReader::new(cursor).map_err(|e| format!("FLAC error: {e}"))?;
    let info = reader.streaminfo();
    let sample_rate = info.sample_rate;
    let channels = info.channels;
    let bits = info.bits_per_sample;
    let max_val = (1u32 << (bits - 1)) as f32;

    let all_samples: Vec<f32> = reader
        .samples()
        .map(|s| s.map(|v| v as f32 / max_val))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("FLAC sample error: {e}"))?;

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: bits as u16,
            is_float: false,
            format: "FLAC".into(),
            file_size,
        },
        samples,
    })
}

// ── OGG ─────────────────────────────────────────────────────────────

fn ogg_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    // OGG requires full decode to know exact sample count
    let result = decode_ogg(bytes, file_size)?;
    Ok(result.info)
}

fn decode_ogg(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    use lewton::inside_ogg::OggStreamReader;

    let cursor = Cursor::new(bytes);
    let mut reader = OggStreamReader::new(cursor).map_err(|e| format!("OGG error: {e}"))?;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channels = reader.ident_hdr.audio_channels as u32;

    let mut all_samples: Vec<f32> = Vec::new();
    loop {
        match reader.read_dec_packet_itl() {
            Ok(Some(packet)) => {
                all_samples.extend(packet.iter().map(|&s| s as f32 / 32768.0));
            }
            Ok(None) => break,
            Err(e) => return Err(format!("OGG decode error: {e}")),
        }
    }

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "OGG".into(),
            file_size,
        },
        samples,
    })
}

// ── MP3 ─────────────────────────────────────────────────────────────

fn mp3_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    // MP3 requires full decode to know exact sample count
    let result = decode_mp3(bytes, file_size)?;
    Ok(result.info)
}

fn decode_mp3(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::{probe::Hint, FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let hint = Hint::new();
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("MP3 probe error: {e}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("No audio track found in MP3")?;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|cp| cp.audio())
        .ok_or("MP3 missing audio codec parameters")?;

    let sample_rate = audio_params.sample_rate.ok_or("MP3 missing sample rate")?;
    let channels = audio_params
        .channels
        .as_ref()
        .ok_or("MP3 missing channel info")?
        .count() as u32;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("MP3 decoder error: {e}"))?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("MP3 packet error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                decoded.copy_to_vec_interleaved(&mut scratch);
                all_samples.extend_from_slice(&scratch);
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("MP3 decode error: {e}")),
        }
    }

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "MP3".into(),
            file_size,
        },
        samples,
    })
}

// ── M4A (MPEG-4 / AAC / ALAC) ──────────────────────────────────────

fn m4a_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let result = decode_m4a(bytes, file_size)?;
    Ok(result.info)
}

fn decode_m4a(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::{probe::Hint, FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("M4A probe error: {e}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("No audio track found in M4A")?;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|cp| cp.audio())
        .ok_or("M4A missing audio codec parameters")?;

    let sample_rate = audio_params.sample_rate.ok_or("M4A missing sample rate")?;
    let channels = audio_params
        .channels
        .as_ref()
        .ok_or("M4A missing channel info")?
        .count() as u32;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("M4A decoder error: {e}"))?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => { decoder.reset(); continue; }
            Err(e) => return Err(format!("M4A packet error: {e}")),
        };
        if packet.track_id != track_id { continue; }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                decoded.copy_to_vec_interleaved(&mut scratch);
                all_samples.extend_from_slice(&scratch);
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("M4A decode error: {e}")),
        }
    }

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "M4A".into(),
            file_size,
        },
        samples,
    })
}

// ── W4V (Wildlife Acoustics) ───────────────────────────────────────

fn parse_w4v_riff(bytes: &[u8]) -> Result<(u32, u16, u16, u64, u64, u64), String> {
    // Returns (sample_rate, channels, block_align, data_offset, data_size, fact_samples)
    if bytes.len() < 12 || &bytes[8..12] != b"WAVE" {
        return Err("Not a RIFF/WAVE file".into());
    }
    let mut pos = 12usize;
    let mut fmt_info: Option<(u32, u16, u16)> = None;
    let mut data_offset: Option<u64> = None;
    let mut data_size: Option<u64> = None;
    let mut fact_samples: u64 = 0;

    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(
            bytes[pos + 4..pos + 8].try_into().map_err(|_| "chunk size")?,
        ) as u64;
        let body = pos + 8;
        let fits = body as u64 + chunk_size <= bytes.len() as u64;

        match chunk_id {
            b"fmt " if chunk_size >= 14 && fits => {
                let f = &bytes[body..];
                let sr = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
                let ch = u16::from_le_bytes([f[2], f[3]]);
                let ba = u16::from_le_bytes([f[12], f[13]]);
                fmt_info = Some((sr, ch, ba));
            }
            b"fact" if chunk_size >= 4 && fits => {
                let f = &bytes[body..];
                fact_samples = u32::from_le_bytes([f[0], f[1], f[2], f[3]]) as u64;
            }
            b"data" => {
                data_offset = Some(body as u64);
                data_size = Some(chunk_size);
                break;
            }
            _ => {}
        }
        pos = body + ((chunk_size as usize + 1) & !1);
    }

    let (sr, ch, ba) = fmt_info.ok_or("No fmt chunk")?;
    let d_off = data_offset.ok_or("No data chunk")?;
    let d_sz = data_size.ok_or("No data chunk")?;
    let fact = if fact_samples > 0 {
        fact_samples
    } else {
        (d_sz / ba as u64) * W4V_BLOCK_SAMPLES as u64
    };
    Ok((sr, ch, ba, d_off, d_sz, fact))
}

fn decode_w4v_blocks(bytes: &[u8], block_align: u16, data_offset: u64, data_size: u64) -> Vec<f32> {
    let ba = block_align as usize;
    let data_bytes = ba - W4V_BLOCK_HEADER;
    let bits = data_bytes * 8 / W4V_BLOCK_SAMPLES;
    let num_blocks = data_size as usize / ba;
    let start = data_offset as usize;
    let sign_bit = 1usize << (bits - 1);
    let mask = (1usize << bits) - 1;

    let mut samples = Vec::with_capacity(num_blocks * W4V_BLOCK_SAMPLES);

    for bi in 0..num_blocks {
        let off = start + bi * ba;
        if off + ba > bytes.len() {
            break;
        }
        let block = &bytes[off..off + ba];
        let predictor = i16::from_le_bytes([block[0], block[1]]) as i32;
        let scale = block[2] as i32;
        let data = &block[W4V_BLOCK_HEADER..];

        let mut bit_pos = 0usize;
        for _ in 0..W4V_BLOCK_SAMPLES {
            let byte_idx = bit_pos / 8;
            let bit_off = bit_pos % 8;

            let raw = if byte_idx + 1 < data.len() {
                let two = ((data[byte_idx] as usize) << 8) | (data[byte_idx + 1] as usize);
                let shift = 16usize.wrapping_sub(bits + bit_off);
                if shift < 16 {
                    (two >> shift) & mask
                } else if byte_idx + 2 < data.len() {
                    let three = (two << 8) | (data[byte_idx + 2] as usize);
                    (three >> (24 - bits - bit_off)) & mask
                } else {
                    0
                }
            } else if byte_idx < data.len() {
                let shift = 8usize.wrapping_sub(bits + bit_off);
                if shift < 8 { (data[byte_idx] as usize >> shift) & mask } else { 0 }
            } else {
                0
            };
            bit_pos += bits;

            let signed_val = if raw & sign_bit != 0 {
                raw as i32 - (1i32 << bits)
            } else {
                raw as i32
            };
            let s = (predictor + signed_val * scale).clamp(-32768, 32767);
            samples.push(s as f32 / 32768.0);
        }
    }
    samples
}

fn w4v_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let (sr, ch, _ba, _d_off, _d_sz, fact) = parse_w4v_riff(bytes)?;
    let mono_samples = fact as usize / ch.max(1) as usize;
    Ok(AudioFileInfo {
        sample_rate: sr,
        channels: ch as u32,
        duration_secs: mono_samples as f64 / sr as f64,
        total_mono_samples: mono_samples,
        bits_per_sample: 16,
        is_float: false,
        format: "W4V".into(),
        file_size,
    })
}

fn decode_w4v(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    let (sr, ch, ba, d_off, d_sz, _fact) = parse_w4v_riff(bytes)?;
    let all_samples = decode_w4v_blocks(bytes, ba, d_off, d_sz);
    let samples = mix_to_mono(&all_samples, ch as u32);
    let duration_secs = samples.len() as f64 / sr as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate: sr,
            channels: ch as u32,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "W4V".into(),
            file_size,
        },
        samples,
    })
}
