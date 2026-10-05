//! RIFF/WAVE reading and writing.
//!
//! Every WAV path in the app reads the header through
//! [`parse_wav_header_with_file_size`] and decodes samples through
//! [`decode_pcm`]: the in-memory loader, streaming playback, and the
//! recording finalize step. Before this module those paths each had their own
//! parser or decoder, and disagreed on which files they could open.
//!
//! Writers use [`header_bytes`], which writes plain RIFF up to 4 GB and RF64
//! (EBU Tech 3306) beyond it, in the same 80-byte layout either way so a
//! streaming recorder can rewrite the header in place when it stops.

use super::guano::{self, GuanoMetadata};
use crate::types::{RecorderBlock, WavDetails, WavMarker};

/// How each sample is stored in the `data` chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEncoding {
    /// Unsigned 8-bit, 128 = silence.
    U8,
    I16,
    I24,
    I32,
    F32,
    F64,
}

impl SampleEncoding {
    /// Bytes one sample takes in the file.
    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I16 => 2,
            Self::I24 => 3,
            Self::I32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

/// Parsed WAV header: enough to stream from disk without loading all samples.
#[derive(Clone, Debug)]
pub struct WavHeader {
    pub sample_rate: u32,
    pub channels: u16,
    /// Width of one sample in the file, in bits (8, 16, 24, 32 or 64). Fewer
    /// bits may be significant: see `details.valid_bits`.
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub encoding: SampleEncoding,
    /// Bytes per frame (one sample for every channel).
    pub block_align: u16,
    /// Byte offset of the first audio sample within the file.
    pub data_offset: u64,
    /// Byte length of the audio samples, a whole number of frames.
    pub data_size: u64,
    pub total_frames: u64,
    pub guano: Option<GuanoMetadata>,
    /// Cue-point markers from `cue ` + `LIST`/`adtl` chunks, if present.
    pub wav_markers: Vec<WavMarker>,
    pub details: WavDetails,
}

/// Parse a WAV header from the whole file.
pub fn parse_wav_header(bytes: &[u8]) -> Result<WavHeader, String> {
    parse_wav_header_with_file_size(bytes, Some(bytes.len() as u64))
}

/// Parse a WAV header from the first bytes of a file (typically 64 KB) or the
/// whole file. `file_size` is the size of the whole file when known; it lets
/// the parser catch a `data` size that runs past the end of the file or that
/// wrapped past 4 GB.
///
/// Reads RIFF and RF64, plain and WAVE_FORMAT_EXTENSIBLE `fmt ` chunks, and
/// the metadata block a Pettersson D500X writes inside its `data` chunk.
pub fn parse_wav_header_with_file_size(
    bytes: &[u8],
    file_size: Option<u64>,
) -> Result<WavHeader, String> {
    if bytes.len() < 12 {
        return Err("File too small for WAV header".into());
    }
    let magic = &bytes[0..4];
    if magic == b"RIFX" {
        return Err(msg::RIFX.into());
    }
    if (magic != b"RIFF" && magic != b"RF64") || &bytes[8..12] != b"WAVE" {
        return Err("Not a RIFF/WAVE or RF64/WAVE file".into());
    }
    let rf64 = magic == b"RF64";
    let riff_size = u32_at(bytes, 4) as u64;
    let len = bytes.len() as u64;
    // Size of the whole file. Without `file_size`, `bytes` is taken to be the
    // whole file.
    let file_len = file_size.unwrap_or(len);

    let mut notes = Vec::new();
    let mut fmt: Option<Fmt> = None;
    let mut data: Option<DataChunk> = None;
    let mut extra_data_chunks = 0u32;
    let mut ds64_data_size: Option<u64> = None;
    let mut guano: Option<GuanoMetadata> = None;
    let mut cue_points: Vec<(u32, u64)> = Vec::new();
    let mut labels: Vec<(u32, String)> = Vec::new();
    let mut notes_adtl: Vec<(u32, String)> = Vec::new();

    let mut pos = 12u64;
    while pos + 8 <= len {
        let p = pos as usize;
        let id = &bytes[p..p + 4];
        let size32 = u32_at(bytes, p + 4);
        let body_start = pos + 8;
        let mut size = size32 as u64;
        let fits = body_start + size <= len;
        let body = &bytes[body_start as usize..(body_start + size).min(len) as usize];

        match id {
            b"ds64" if body.len() >= 16 => {
                ds64_data_size = Some(u64_at(body, 8));
            }
            b"fmt " if fmt.is_none() => {
                if !fits {
                    return Err("fmt chunk too small or truncated".into());
                }
                fmt = Some(parse_fmt(body)?);
            }
            b"data" if data.is_none() => {
                if rf64 && size32 == 0xFFFF_FFFF {
                    size = ds64_data_size.unwrap_or(size);
                }
                let mut chunk = DataChunk {
                    audio_offset: body_start,
                    audio_size: size,
                    size_field: size32,
                    recorder_block: None,
                };
                // The D500X block sits at the front of the body and the size
                // counts only the samples after it, so the chunk is that much
                // longer than its size says.
                if let Some(block_len) = d500x_block_len(bytes, body_start, file_len) {
                    let block = parse_d500x_block(
                        &bytes[body_start as usize..(body_start + block_len) as usize],
                    );
                    chunk.audio_offset = body_start + block_len;
                    if body_start + block_len + size <= file_len {
                        // Size counts the samples only (as the recorder writes it).
                        notes.push(msg::d500x_size_short(block_len));
                        size += block_len;
                    } else {
                        // Size already counts the block (corrected by some other tool).
                        chunk.audio_size = size.saturating_sub(block_len);
                    }
                    chunk.recorder_block = Some(block);
                }
                data = Some(chunk);
                if body_start + size > len {
                    // The samples run past the bytes we were given: a header
                    // read, or a file cut short. Nothing more to walk.
                    break;
                }
            }
            b"data" => extra_data_chunks += 1,
            b"guan" if fits => guano = guano::parse_guano_chunk(body),
            b"cue " if fits && body.len() >= 4 => parse_cue(body, &mut cue_points),
            b"LIST" if fits && body.len() >= 4 && &body[0..4] == b"adtl" => {
                parse_adtl_subchunks(&body[4..], &mut labels, &mut notes_adtl);
            }
            _ => {}
        }

        // Advance to the next chunk. Bodies are padded to an even length.
        let next = body_start + size + (size & 1);
        if next <= pos {
            break;
        }
        pos = next;
    }

    let fmt = fmt.ok_or("No fmt chunk found in WAV header")?;
    let mut data = data.ok_or("No data chunk found in WAV header")?;
    let encoding = fmt.encoding;
    let block_align = fmt.block_align as u64;

    if extra_data_chunks > 0 {
        notes.push(msg::extra_data_chunks(extra_data_chunks));
    }

    let mut cut_short = false;
    if file_size.is_some() {
        let available = file_len.saturating_sub(data.audio_offset);
        let audio_end = data.audio_offset + data.audio_size;
        // A size slot of 0xFFFFFFFF in plain RIFF, or a size much smaller
        // than the rest of the file, is a u32 size that overflowed past 4 GB
        // (or a recorder that never filled it in). Read to the end of the
        // file, unless a chunk header follows the samples where the size says
        // they end: then the size is right and the file has more chunks.
        let wrapped = !rf64 && data.size_field == 0xFFFF_FFFF;
        let much_smaller = available > data.audio_size + 1024 && available > 1_000_000;
        if data.recorder_block.is_none()
            && (wrapped || much_smaller)
            && !plausible_chunk_at(bytes, audio_end + (data.audio_size & 1), file_len)
        {
            notes.push(msg::data_size_stretched(data.audio_size, available));
            data.audio_size = available;
        } else if data.audio_size > available {
            notes.push(msg::data_cut_short(data.audio_size, available));
            data.audio_size = available;
            cut_short = true;
        }

        // A file cut short has a RIFF size past its end too; one note covers it.
        if !rf64
            && !cut_short
            && riff_size != 0
            && riff_size != 0xFFFF_FFFF
            && riff_size + 8 > file_len
        {
            notes.push(msg::riff_size_past_end(riff_size + 8 - file_len));
        }
    }

    let data_size = data.audio_size / block_align * block_align;
    let total_frames = data_size / block_align;

    let wav_markers: Vec<WavMarker> = cue_points
        .iter()
        .map(|&(id, position)| WavMarker {
            id,
            position,
            label: labels
                .iter()
                .find(|(c, _)| *c == id)
                .map(|(_, t)| t.clone()),
            note: notes_adtl
                .iter()
                .find(|(c, _)| *c == id)
                .map(|(_, t)| t.clone()),
        })
        .collect();

    Ok(WavHeader {
        sample_rate: fmt.sample_rate,
        channels: fmt.channels,
        bits_per_sample: (encoding.bytes() * 8) as u16,
        is_float: encoding.is_float(),
        encoding,
        block_align: fmt.block_align,
        data_offset: data.audio_offset,
        data_size,
        total_frames,
        guano,
        wav_markers,
        details: WavDetails {
            valid_bits: fmt.valid_bits,
            extensible: fmt.extensible,
            rf64,
            channel_mask: fmt.channel_mask,
            recorder_block: data.recorder_block,
            notes,
        },
    })
}

struct DataChunk {
    /// Where the samples start (after a D500X block, if any).
    audio_offset: u64,
    audio_size: u64,
    /// The raw 32-bit size field.
    size_field: u32,
    recorder_block: Option<RecorderBlock>,
}

struct Fmt {
    encoding: SampleEncoding,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    valid_bits: Option<u16>,
    extensible: bool,
    channel_mask: Option<u32>,
}

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// The SubFormat GUID of an extensible header is
/// `{tttt0000-0000-0010-8000-00AA00389B71}`, where `tttt` is an ordinary
/// format tag. These are its bytes after the tag, as stored.
const KSDATAFORMAT_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

fn parse_fmt(fmt: &[u8]) -> Result<Fmt, String> {
    if fmt.len() < 16 {
        return Err("fmt chunk too small or truncated".into());
    }
    let mut tag = u16_at(fmt, 0);
    let channels = u16_at(fmt, 2);
    let sample_rate = u32_at(fmt, 4);
    let block_align = u16_at(fmt, 12);
    let bits = u16_at(fmt, 14);
    let mut declared_valid = None;
    let mut channel_mask = None;
    let extensible = tag == WAVE_FORMAT_EXTENSIBLE;
    if extensible {
        if fmt.len() < 40 {
            return Err("fmt chunk too small or truncated".into());
        }
        declared_valid = Some(u16_at(fmt, 18)).filter(|&v| v != 0);
        channel_mask = Some(u32_at(fmt, 20)).filter(|&m| m != 0);
        tag = u16_at(fmt, 24);
        if fmt[26..40] != KSDATAFORMAT_TAIL {
            return Err(msg::unknown_subformat());
        }
    }
    if channels == 0 {
        return Err("Invalid WAV: zero channels".into());
    }

    // The width of one sample comes from block_align, which also sets the
    // stride between frames. bits_per_sample can be less than that width
    // (12-bit audio in 16-bit samples); a block_align too small to hold
    // bits_per_sample is wrong, and the width falls back to bits rounded up.
    let from_bits = bits.div_ceil(8);
    let from_align = if block_align % channels == 0 {
        block_align / channels
    } else {
        0
    };
    let width = if from_align >= from_bits && from_align > 0 {
        from_align
    } else {
        from_bits
    };
    let encoding = match (tag, width) {
        (WAVE_FORMAT_PCM, 1) => SampleEncoding::U8,
        (WAVE_FORMAT_PCM, 2) => SampleEncoding::I16,
        (WAVE_FORMAT_PCM, 3) => SampleEncoding::I24,
        (WAVE_FORMAT_PCM, 4) => SampleEncoding::I32,
        (WAVE_FORMAT_IEEE_FLOAT, 4) => SampleEncoding::F32,
        (WAVE_FORMAT_IEEE_FLOAT, 8) => SampleEncoding::F64,
        (WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT, _) => {
            return Err(msg::unsupported_width(tag == WAVE_FORMAT_IEEE_FLOAT, bits));
        }
        _ => return Err(msg::unsupported_format(tag)),
    };
    let width_bits = width * 8;
    let valid = declared_valid.unwrap_or(bits);
    let valid_bits = (valid > 0 && valid < width_bits).then_some(valid);

    Ok(Fmt {
        encoding,
        channels,
        sample_rate,
        block_align: width * channels,
        valid_bits,
        extensible,
        channel_mask,
    })
}

/// Does a chunk header that fits in the file start at `pos`? Used to tell a
/// `data` size that is right (more chunks follow it) from one that wrapped.
fn plausible_chunk_at(bytes: &[u8], pos: u64, file_len: u64) -> bool {
    let p = pos as usize;
    if pos + 8 > bytes.len() as u64 {
        return false;
    }
    let id_ok = bytes[p..p + 4].iter().all(|b| (0x20..0x7F).contains(b));
    id_ok && pos + 8 + u32_at(bytes, p + 4) as u64 <= file_len
}

// ─── Pettersson D500X ───────────────────────────────────────────────────────

/// Where the D500X writes its firmware version, from the start of its block.
const D500X_FIRMWARE_AT: usize = 0xC4;
/// Length of the fixed-position fields, before the lines of text.
const D500X_FIXED: u64 = 0x1D4;

/// Length of a D500X metadata block at the start of a `data` body, if there
/// is one. The block starts with its own length (u32 LE, counting itself),
/// and has the firmware version (`D500X V2.2.6 ...`) at offset 0xC4. Layout
/// from Qubero's reading of a firmware 2.2.6 file.
fn d500x_block_len(bytes: &[u8], body_start: u64, file_len: u64) -> Option<u64> {
    let b = body_start as usize;
    if bytes.len() < b + D500X_FIXED as usize {
        return None;
    }
    if &bytes[b + D500X_FIRMWARE_AT..b + D500X_FIRMWARE_AT + 5] != b"D500X" {
        return None;
    }
    let block_len = u32_at(bytes, b) as u64;
    // Even, so the samples after it stay 2-byte aligned.
    let ok = block_len >= D500X_FIXED
        && block_len % 2 == 0
        && body_start + block_len <= file_len
        && b as u64 + block_len <= bytes.len() as u64;
    ok.then_some(block_len)
}

fn parse_d500x_block(block: &[u8]) -> RecorderBlock {
    let text = |from: usize, len: usize| -> String {
        let end = (from + len).min(block.len());
        let raw = &block[from.min(end)..end];
        let raw = raw.split(|&c| c == 0).next().unwrap_or(&[]);
        String::from_utf8_lossy(raw).trim().to_string()
    };
    let mut fields = Vec::new();
    let profile = text(0x12C, (D500X_FIXED as usize) - 0x12C);
    if !profile.is_empty() {
        fields.push(("Profile".to_string(), profile));
    }
    let settings = [text(0xF4, 24), text(0x10C, 24)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !settings.is_empty() {
        fields.push(("Profile settings".to_string(), settings));
    }
    // Lines of `Key:   value`, CR LF separated, ending at the first zero byte.
    let lines = &block[(D500X_FIXED as usize).min(block.len())..];
    let lines = lines.split(|&c| c == 0).next().unwrap_or(&[]);
    for line in String::from_utf8_lossy(lines).lines() {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if !k.is_empty() && !v.is_empty() {
                fields.push((k.to_string(), v.to_string()));
            }
        }
    }
    RecorderBlock {
        recorder: "Pettersson D500X",
        fields,
    }
}

// ─── cue / adtl ─────────────────────────────────────────────────────────────

fn parse_cue(cue: &[u8], out: &mut Vec<(u32, u64)>) {
    let num_points = u32_at(cue, 0);
    let mut cp = 4usize;
    for _ in 0..num_points {
        if cp + 24 > cue.len() {
            break;
        }
        // sample_offset is at offset 20 within the cue point struct
        out.push((u32_at(cue, cp), u32_at(cue, cp + 20) as u64));
        cp += 24;
    }
}

/// Parse `labl` and `note` sub-chunks from a LIST/adtl body.
fn parse_adtl_subchunks(
    data: &[u8],
    labels: &mut Vec<(u32, String)>,
    notes: &mut Vec<(u32, String)>,
) {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let sub_id = &data[pos..pos + 4];
        let sub_size = u32_at(data, pos + 4) as usize;
        let sub_start = pos + 8;
        let sub_end = sub_start.saturating_add(sub_size);
        if sub_end > data.len() || sub_size < 4 {
            break;
        }
        let cue_id = u32_at(data, sub_start);
        let text_bytes = &data[sub_start + 4..sub_end];
        let text = String::from_utf8_lossy(text_bytes)
            .trim_end_matches('\0')
            .to_string();
        match sub_id {
            b"labl" => labels.push((cue_id, text)),
            b"note" => notes.push((cue_id, text)),
            _ => {}
        }
        pos = sub_start + ((sub_size + 1) & !1);
    }
}

// ─── Decoding ───────────────────────────────────────────────────────────────

/// Decode raw sample bytes into interleaved f32 in [-1, 1). Integer samples
/// are scaled by their full width, so audio with fewer valid bits (stored
/// left-justified, as the format requires) needs no special handling. A
/// trailing partial sample is ignored.
pub fn decode_pcm(bytes: &[u8], encoding: SampleEncoding) -> Vec<f32> {
    match encoding {
        SampleEncoding::U8 => bytes.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        SampleEncoding::I16 => bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect(),
        SampleEncoding::I24 => bytes
            .chunks_exact(3)
            .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        SampleEncoding::I32 => bytes
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        SampleEncoding::F32 => bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        SampleEncoding::F64 => bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32)
            .collect(),
    }
}

// ─── Writing ────────────────────────────────────────────────────────────────

/// Sample format of a WAV to write. Written as a plain 16-byte `fmt ` chunk
/// (PCM or IEEE float), which every reader accepts at any width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavWriteFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub is_float: bool,
}

impl WavWriteFormat {
    pub fn block_align(&self) -> u16 {
        self.channels * self.bits_per_sample.div_ceil(8)
    }
}

/// Length of the header [`header_bytes`] writes. The samples start here.
pub const WRITE_HEADER_LEN: usize = 80;

/// Bytes of a `ds64` body without a table: RIFF size, data size, sample
/// count (u64 each), table length (u32).
const DS64_BODY_LEN: usize = 28;

/// The header of a WAV with `data_bytes` of samples followed by
/// `bytes_after_data` of other chunks (the data pad byte included; see
/// [`data_pad_len`]).
///
/// Writes `RIFF`, a 28-byte `JUNK` chunk, `fmt `, then the `data` chunk
/// header. When the file would pass 4 GB, writes `RF64` instead and turns the
/// `JUNK` chunk into a `ds64` chunk holding the 64-bit sizes (EBU Tech 3306).
/// Either way the header is [`WRITE_HEADER_LEN`] bytes, so a streaming
/// recorder can write it before the samples and rewrite it at the end.
pub fn header_bytes(fmt: &WavWriteFormat, data_bytes: u64, bytes_after_data: u64) -> Vec<u8> {
    let riff_size = (WRITE_HEADER_LEN as u64 - 8) + data_bytes + bytes_after_data;
    let rf64 = riff_size > u32::MAX as u64 || data_bytes > u32::MAX as u64;
    let block_align = fmt.block_align();

    let mut h = Vec::with_capacity(WRITE_HEADER_LEN);
    if rf64 {
        h.extend_from_slice(b"RF64");
        h.extend_from_slice(&u32::MAX.to_le_bytes());
        h.extend_from_slice(b"WAVE");
        h.extend_from_slice(b"ds64");
        h.extend_from_slice(&(DS64_BODY_LEN as u32).to_le_bytes());
        h.extend_from_slice(&riff_size.to_le_bytes());
        h.extend_from_slice(&data_bytes.to_le_bytes());
        let frames = data_bytes / block_align.max(1) as u64;
        h.extend_from_slice(&frames.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes()); // table length
    } else {
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(riff_size as u32).to_le_bytes());
        h.extend_from_slice(b"WAVE");
        // Reserved so the header can become RF64 without moving the samples.
        h.extend_from_slice(b"JUNK");
        h.extend_from_slice(&(DS64_BODY_LEN as u32).to_le_bytes());
        h.extend_from_slice(&[0u8; DS64_BODY_LEN]);
    }
    let tag = if fmt.is_float {
        WAVE_FORMAT_IEEE_FLOAT
    } else {
        WAVE_FORMAT_PCM
    };
    h.extend_from_slice(b"fmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&tag.to_le_bytes());
    h.extend_from_slice(&fmt.channels.to_le_bytes());
    h.extend_from_slice(&fmt.sample_rate.to_le_bytes());
    h.extend_from_slice(&(fmt.sample_rate * block_align as u32).to_le_bytes());
    h.extend_from_slice(&block_align.to_le_bytes());
    h.extend_from_slice(&fmt.bits_per_sample.to_le_bytes());
    h.extend_from_slice(b"data");
    let data_field = if rf64 { u32::MAX } else { data_bytes as u32 };
    h.extend_from_slice(&data_field.to_le_bytes());
    debug_assert_eq!(h.len(), WRITE_HEADER_LEN);
    h
}

/// The pad byte after an odd-length `data` chunk: 1 or 0. A chunk written
/// after the samples starts after it.
pub fn data_pad_len(data_bytes: u64) -> u64 {
    data_bytes & 1
}

/// Set the RIFF size of an in-memory WAV after chunks were added or removed.
/// For RF64 the size goes in the `ds64` chunk. A plain RIFF that grows past
/// 4 GB keeps the largest size it can hold.
pub fn set_riff_size(wav: &mut [u8]) {
    if wav.len() < 12 {
        return;
    }
    let riff_size = wav.len() as u64 - 8;
    if &wav[0..4] == b"RF64" && wav.len() >= 28 && &wav[12..16] == b"ds64" {
        wav[20..28].copy_from_slice(&riff_size.to_le_bytes());
    } else {
        let clamped = riff_size.min(u32::MAX as u64) as u32;
        wav[4..8].copy_from_slice(&clamped.to_le_bytes());
    }
}

// ─── Byte helpers ───────────────────────────────────────────────────────────

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

// ─── User-facing text ───────────────────────────────────────────────────────

/// Messages shown to the user: load errors and the notes listed with a
/// file's metadata.
pub mod msg {
    pub const RIFX: &str = "RIFX (big-endian WAV) isn't supported.";

    /// Common names for WAV format tags Oversample can't decode.
    pub fn format_name(tag: u16) -> Option<&'static str> {
        Some(match tag {
            0x0002 => "Microsoft ADPCM",
            0x0006 => "A-law",
            0x0007 => "µ-law",
            0x0011 => "IMA ADPCM",
            0x0031 => "GSM 6.10",
            0x0050 => "MPEG audio",
            0x0055 => "MP3",
            0x0161 | 0x0162 | 0x0163 => "Windows Media Audio",
            0x2000 => "Dolby AC-3",
            _ => return None,
        })
    }

    pub fn unsupported_format(tag: u16) -> String {
        match format_name(tag) {
            Some(name) => format!(
                "Unsupported WAV encoding: {name}. Oversample opens uncompressed WAV files only (integer PCM or floating point)."
            ),
            None => format!(
                "Unsupported WAV encoding (format tag 0x{tag:04X}). Oversample opens uncompressed WAV files only (integer PCM or floating point)."
            ),
        }
    }

    pub fn unknown_subformat() -> String {
        "Unsupported WAV encoding (unknown extensible sub-format). Oversample opens uncompressed WAV files only (integer PCM or floating point).".into()
    }

    pub fn unsupported_width(float: bool, bits: u16) -> String {
        let kind = if float { "floating-point" } else { "integer" };
        format!("Unsupported WAV sample size: {bits}-bit {kind}.")
    }

    pub fn d500x_size_short(block_len: u64) -> String {
        format!(
            "D500X metadata block ({block_len} bytes) found at the start of the audio data. The block is skipped, and the audio is read in full."
        )
    }

    pub fn extra_data_chunks(n: u32) -> String {
        let s = if n == 1 { "" } else { "s" };
        format!("File has {n} extra audio data chunk{s}. Only the first one is read.")
    }

    pub fn data_size_stretched(declared: u64, available: u64) -> String {
        format!(
            "Audio data size in the header ({declared} bytes) is less than the file holds. Reading to the end of the file ({available} bytes)."
        )
    }

    pub fn data_cut_short(declared: u64, available: u64) -> String {
        format!(
            "File is shorter than its header says. The header lists {declared} bytes of audio, but only {available} bytes are present. The file may have been cut short."
        )
    }

    pub fn riff_size_past_end(over: u64) -> String {
        format!("RIFF size in the header is {over} bytes larger than the file.")
    }
}
