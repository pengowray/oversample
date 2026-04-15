use std::fs;
use std::path::{Path, PathBuf};
use crate::types::{XcGroupTaxonomy, XcRecording};

/// File hashes and size computed from audio bytes.
#[derive(Clone, Debug)]
pub struct FileHashes {
    pub size_bytes: u64,
    pub sha256: String,
    pub blake3: String,
    /// Multi-point spot hash: BLAKE3 of 16×1MB chunks across the audio data region.
    /// Matches the main app's Layer 2 spot_hash_b3.
    pub spot_hash_b3: String,
    /// Content hash: BLAKE3 of entire file with header bytes zeroed.
    /// Survives metadata/header edits while preserving audio content identity.
    pub content_hash: String,
    /// Audio data region byte offset (WAV: start of data chunk payload).
    pub data_offset: Option<u64>,
    /// Audio data region byte length.
    pub data_size: Option<u64>,
}

/// Hash data extracted from an XC sidecar JSON (stored under "_app" key,
/// with fallback to legacy top-level keys).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SidecarHashes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    /// Multi-point spot hash (16×1MB chunks, matches main app Layer 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spot_hash_b3: Option<String>,
    /// Content hash (BLAKE3 with header zeroed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Audio data region byte offset within the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_offset: Option<u64>,
    /// Audio data region byte length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_size: Option<u64>,
}

impl SidecarHashes {
    pub fn is_empty(&self) -> bool {
        self.blake3.is_none() && self.sha256.is_none() && self.file_size.is_none()
    }
}

/// Extract hash data from an XC sidecar JSON.
/// Tries `json["_app"]` first (new format), then falls back to top-level keys (legacy).
pub fn extract_sidecar_hashes(json: &serde_json::Value) -> SidecarHashes {
    let src = if json["_app"].is_object() {
        &json["_app"]
    } else {
        json
    };
    SidecarHashes {
        blake3: src["blake3"].as_str().map(|s| s.to_string()),
        sha256: src["sha256"].as_str().map(|s| s.to_string()),
        file_size: src["file_size"].as_u64(),
        spot_hash_b3: src["spot_hash_b3"].as_str().map(|s| s.to_string()),
        content_hash: src["content_hash"].as_str().map(|s| s.to_string()),
        data_offset: src["data_offset"].as_u64(),
        data_size: src["data_size"].as_u64(),
    }
}

/// Migrate a sidecar JSON from legacy format (hashes at top level) to new format
/// (hashes nested under "_app"). Returns true if migration was performed.
pub fn migrate_sidecar_json(json: &mut serde_json::Value) -> bool {
    // Already migrated?
    if json["_app"].is_object() {
        return false;
    }
    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return false,
    };

    let hash_keys = ["blake3", "sha256", "file_size", "spot_hash", "spot_hash_b3", "content_hash", "data_offset", "data_size"];
    let mut app_meta = serde_json::Map::new();
    let mut found_any = false;

    for key in &hash_keys {
        if let Some(val) = obj.remove(*key) {
            app_meta.insert((*key).to_string(), val);
            found_any = true;
        }
    }

    // Also move "retrieved" into the app sub-object (it's our metadata, not XC's)
    if let Some(val) = obj.remove("retrieved") {
        app_meta.insert("retrieved".to_string(), val);
    }

    if found_any || !app_meta.is_empty() {
        obj.insert("_app".to_string(), serde_json::Value::Object(app_meta));
        true
    } else {
        false
    }
}

/// Sanitize a string for use in filenames.
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect()
}

/// Build the base filename stem for a recording (no extension).
/// e.g. "XC928094 - Pond Myotis - Myotis dasycneme"
pub fn recording_stem(rec: &XcRecording) -> String {
    sanitize_filename(&format!("XC{} - {} - {} {}", rec.id, rec.en, rec.genus, rec.sp))
}

/// Determine audio file extension from the recording's original filename.
pub fn audio_extension(rec: &XcRecording) -> &str {
    rec.file_name
        .rsplit('.')
        .next()
        .unwrap_or("wav")
}

/// Path to the taxonomy cache file for a group.
pub fn taxonomy_path(root: &Path, group: &str, country: Option<&str>) -> PathBuf {
    let dir = root.join("taxonomy");
    let name = match country {
        Some(cnt) => format!("{}_{}.json", group, sanitize_filename(cnt)),
        None => format!("{}.json", group),
    };
    dir.join(name)
}

/// Load cached taxonomy for a group, if it exists.
pub fn load_taxonomy(root: &Path, group: &str, country: Option<&str>) -> Result<Option<XcGroupTaxonomy>, String> {
    let path = taxonomy_path(root, group, country);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let taxonomy: XcGroupTaxonomy = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
    Ok(Some(taxonomy))
}

/// Save taxonomy cache for a group.
pub fn save_taxonomy(root: &Path, group: &str, country: Option<&str>, data: &XcGroupTaxonomy) -> Result<(), String> {
    let path = taxonomy_path(root, group, country);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create dir {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| format!("Serialize error: {e}"))?;
    fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(())
}

/// Check how old the taxonomy cache is (returns human-readable string).
pub fn taxonomy_age_string(root: &Path, group: &str, country: Option<&str>) -> Option<String> {
    let taxonomy = load_taxonomy(root, group, country).ok()??;
    let updated = chrono::DateTime::parse_from_rfc3339(&taxonomy.last_updated).ok()?;
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(updated);

    let hours = duration.num_hours();
    if hours < 1 {
        Some("just now".to_string())
    } else if hours < 24 {
        Some(format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" }))
    } else {
        let days = duration.num_days();
        Some(format!("{days} day{} ago", if days == 1 { "" } else { "s" }))
    }
}

/// Check if a recording's audio is already cached.
pub fn is_recording_cached(root: &Path, id: u64) -> bool {
    let sounds_dir = root.join("sounds");
    if !sounds_dir.exists() {
        return false;
    }
    // Look for any file starting with "XC{id} -"
    let prefix = format!("XC{id} -");
    if let Ok(entries) = fs::read_dir(&sounds_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && !name.ends_with(".xc.json") {
                return true;
            }
        }
    }
    false
}

/// Find the cached audio file path for a recording.
pub fn cached_audio_path(root: &Path, id: u64) -> Option<PathBuf> {
    let sounds_dir = root.join("sounds");
    let prefix = format!("XC{id} -");
    if let Ok(entries) = fs::read_dir(&sounds_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && !name.ends_with(".xc.json") {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Detect WAV audio data region by scanning for the "data" chunk in a RIFF file.
/// Returns (data_offset, data_size) or (None, None) for non-WAV files.
pub fn detect_wav_data_region(data: &[u8]) -> (Option<u64>, Option<u64>) {
    // Check RIFF/WAVE header
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return (None, None);
    }
    // Scan chunks after the 12-byte RIFF header
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([
            data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7],
        ]) as u64;
        if chunk_id == b"data" {
            let offset = (pos + 8) as u64;
            return (Some(offset), Some(chunk_size));
        }
        // Advance to next chunk (chunks are word-aligned)
        let advance = 8 + chunk_size as usize;
        let advance = (advance + 1) & !1; // word-align
        pos += advance;
    }
    (None, None)
}

/// Size of each chunk for the multi-point spot hash (1 MB).
const SPOT_CHUNK_SIZE: u64 = 1_048_576;
/// Maximum number of chunks for the spot hash.
const NUM_SPOT_CHUNKS: u64 = 16;

/// Compute BLAKE3 multi-point spot hash (16×1MB chunks across audio data region).
/// Matches the algorithm in the main app's file_identity.rs.
pub fn compute_spot_hash_b3(data: &[u8], data_offset: Option<u64>, data_size: Option<u64>) -> String {
    let file_size = data.len() as u64;
    let audio_start = data_offset.unwrap_or(0);
    let audio_len = data_size.unwrap_or(file_size.saturating_sub(audio_start));

    if audio_len == 0 {
        return blake3::hash(&[]).to_hex().to_string();
    }

    let num_chunks = NUM_SPOT_CHUNKS.min((audio_len / SPOT_CHUNK_SIZE).max(1));
    let mut chunk_hashes = Vec::with_capacity(num_chunks as usize);

    for i in 0..num_chunks {
        let chunk_start = audio_start + i * (audio_len / num_chunks);
        let remaining = audio_len - (chunk_start - audio_start);
        let chunk_len = SPOT_CHUNK_SIZE.min(remaining);
        let s = chunk_start as usize;
        let e = (chunk_start + chunk_len).min(file_size) as usize;
        chunk_hashes.push(blake3::hash(&data[s..e]));
    }

    // Combine chunk hashes
    let mut combined = Vec::with_capacity(chunk_hashes.len() * 32);
    for h in &chunk_hashes {
        combined.extend_from_slice(h.as_bytes());
    }
    blake3::hash(&combined).to_hex().to_string()
}

/// Compute content hash: BLAKE3 of entire file with header bytes zeroed.
pub fn compute_content_hash(data: &[u8], data_offset: Option<u64>) -> String {
    let header_end = data_offset.unwrap_or(0) as usize;
    let mut hasher = blake3::Hasher::new();
    if header_end > 0 && header_end < data.len() {
        let zeroed = vec![0u8; header_end];
        hasher.update(&zeroed);
        hasher.update(&data[header_end..]);
    } else {
        hasher.update(data);
    }
    hasher.finalize().to_hex().to_string()
}

/// Compute hashes and size from audio bytes.
pub fn compute_file_hashes(data: &[u8]) -> FileHashes {
    use sha2::Digest;

    let size_bytes = data.len() as u64;

    // Detect WAV data region for audio-aware hashing
    let (data_offset, data_size) = detect_wav_data_region(data);

    // SHA-256
    let sha256 = {
        let mut hasher = sha2::Sha256::new();
        hasher.update(data);
        hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };

    // BLAKE3 (full file)
    let blake3 = blake3::hash(data).to_hex().to_string();

    // Multi-point spot hash (16×1MB chunks, matches main app Layer 2)
    let spot_hash_b3 = compute_spot_hash_b3(data, data_offset, data_size);

    // Content hash (BLAKE3 with header zeroed)
    let content_hash = compute_content_hash(data, data_offset);

    FileHashes { size_bytes, sha256, blake3, spot_hash_b3, content_hash, data_offset, data_size }
}

/// Build the XC metadata sidecar JSON for a recording.
pub fn build_metadata_json(rec: &XcRecording) -> serde_json::Value {
    let attribution = format!(
        "{}, XC{}. Accessible at www.xeno-canto.org/{}",
        rec.rec, rec.id, rec.id
    );
    let now = chrono::Utc::now().format("%Y-%m-%d").to_string();

    serde_json::json!({
        "source": "xeno-canto",
        "xc_id": rec.id_num(),
        "url": &rec.url,
        "file": &rec.file_url,
        "file-name": &rec.file_name,
        "gen": &rec.genus,
        "sp": &rec.sp,
        "ssp": &rec.ssp,
        "en": &rec.en,
        "grp": &rec.grp,
        "rec": &rec.rec,
        "cnt": &rec.cnt,
        "loc": &rec.loc,
        "lat": &rec.lat,
        "lon": &rec.lon,
        "alt": &rec.alt,
        "type": &rec.sound_type,
        "sex": &rec.sex,
        "stage": &rec.stage,
        "method": &rec.method,
        "date": &rec.date,
        "time": &rec.time,
        "uploaded": &rec.uploaded,
        "also": &rec.also,
        "rmk": &rec.rmk,
        "animal-seen": &rec.animal_seen,
        "playback-used": &rec.playback_used,
        "temp": &rec.temp,
        "regnr": &rec.regnr,
        "auto": &rec.auto_rec,
        "dvc": &rec.dvc,
        "mic": &rec.mic,
        "q": &rec.q,
        "length": &rec.length,
        "smp": rec.smp.parse::<u64>().ok(),
        "lic": &rec.lic,
        "attribution": attribution,
        "retrieved": now,
    })
}

/// Build the XC metadata sidecar JSON for a recording, including file hashes.
/// Hashes are nested under a `"_app"` key to distinguish from XC API fields.
pub fn build_metadata_json_with_hashes(rec: &XcRecording, hashes: &FileHashes) -> serde_json::Value {
    let mut json = build_metadata_json(rec);
    if let Some(obj) = json.as_object_mut() {
        // Remove top-level "retrieved" — it goes under _app
        let retrieved = obj.remove("retrieved");
        let mut bm = serde_json::Map::new();
        bm.insert("file_size".into(), serde_json::json!(hashes.size_bytes));
        bm.insert("sha256".into(), serde_json::json!(hashes.sha256));
        bm.insert("blake3".into(), serde_json::json!(hashes.blake3));
        bm.insert("spot_hash_b3".into(), serde_json::json!(hashes.spot_hash_b3));
        bm.insert("content_hash".into(), serde_json::json!(hashes.content_hash));
        if let Some(offset) = hashes.data_offset {
            bm.insert("data_offset".into(), serde_json::json!(offset));
        }
        if let Some(size) = hashes.data_size {
            bm.insert("data_size".into(), serde_json::json!(size));
        }
        if let Some(r) = retrieved {
            bm.insert("retrieved".into(), r);
        }
        obj.insert("_app".into(), serde_json::Value::Object(bm));
    }
    json
}

/// Save a recording's audio and metadata to the cache.
/// Returns the path to the saved audio file.
pub fn save_recording(
    root: &Path,
    rec: &XcRecording,
    audio_bytes: &[u8],
) -> Result<PathBuf, String> {
    // Validate audio bytes before writing anything
    if audio_bytes.is_empty() {
        return Err("Downloaded file is empty".into());
    }
    if audio_bytes.len() < 44 {
        return Err(format!(
            "Downloaded file is too small ({} bytes) — probably not a valid audio file",
            audio_bytes.len()
        ));
    }

    let sounds_dir = root.join("sounds");
    fs::create_dir_all(&sounds_dir)
        .map_err(|e| format!("Failed to create sounds dir: {e}"))?;

    let stem = recording_stem(rec);
    let ext = audio_extension(rec);
    let audio_filename = format!("{stem}.{ext}");
    let meta_filename = format!("{stem}.xc.json");

    // Write audio
    let audio_path = sounds_dir.join(&audio_filename);
    fs::write(&audio_path, audio_bytes)
        .map_err(|e| format!("Failed to write audio: {e}"))?;

    // Write metadata sidecar (with file hashes)
    let meta_path = sounds_dir.join(&meta_filename);
    let hashes = compute_file_hashes(audio_bytes);
    let metadata = build_metadata_json_with_hashes(rec, &hashes);
    let json_str = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Serialize error: {e}"))?;
    fs::write(&meta_path, format!("{json_str}\n"))
        .map_err(|e| format!("Failed to write metadata: {e}"))?;

    // Update index.json (only after audio + metadata written successfully)
    update_index(root, rec, &audio_filename, &meta_filename)?;

    Ok(audio_path)
}

/// Find the cached metadata sidecar path for a recording.
pub fn cached_metadata_path(root: &Path, id: u64) -> Option<PathBuf> {
    let sounds_dir = root.join("sounds");
    let prefix = format!("XC{id} -");
    if let Ok(entries) = fs::read_dir(&sounds_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.ends_with(".xc.json") {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Delete a recording's audio, metadata sidecar, and index entry.
/// Accepts either an XC ID or a filename. Returns names of deleted files.
pub fn delete_recording(root: &Path, id: u64) -> Result<Vec<String>, String> {
    let sounds_dir = root.join("sounds");
    let prefix = format!("XC{id} -");
    let mut deleted = Vec::new();

    if let Ok(entries) = fs::read_dir(&sounds_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) {
                fs::remove_file(entry.path())
                    .map_err(|e| format!("Failed to delete {}: {e}", name))?;
                deleted.push(name);
            }
        }
    }

    if deleted.is_empty() {
        return Err(format!("No cached files found for XC{id}"));
    }

    // Remove from index.json
    remove_from_index(root, id)?;

    Ok(deleted)
}

/// Remove a recording entry from index.json by XC ID.
fn remove_from_index(root: &Path, id: u64) -> Result<(), String> {
    let index_path = root.join("index.json");
    if !index_path.exists() {
        return Ok(());
    }

    let mut index = read_index(root);
    let sounds = index["sounds"]
        .as_array_mut()
        .ok_or("index.json 'sounds' is not an array")?;

    let before = sounds.len();
    sounds.retain(|s| s["xc_id"].as_u64() != Some(id));

    if sounds.len() == before {
        return Ok(()); // wasn't in the index
    }

    let tmp_path = root.join("index.json.tmp");
    let json_str = serde_json::to_string_pretty(&index)
        .map_err(|e| format!("Serialize error: {e}"))?;
    fs::write(&tmp_path, format!("{json_str}\n"))
        .map_err(|e| format!("Failed to write index.json.tmp: {e}"))?;
    fs::rename(&tmp_path, &index_path)
        .map_err(|e| format!("Failed to finalize index.json: {e}"))?;

    Ok(())
}

/// Read and parse the cache index, falling back gracefully on errors.
fn read_index(root: &Path) -> serde_json::Value {
    let index_path = root.join("index.json");
    let tmp_path = root.join("index.json.tmp");

    // Try main index first, then tmp fallback (in case rename didn't complete)
    for path in [&index_path, &tmp_path] {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if val["sounds"].is_array() {
                        return val;
                    }
                }
            }
        }
    }

    // Both missing or corrupt — start fresh
    serde_json::json!({ "version": 1, "sounds": [] })
}

/// Update (or create) index.json with a new recording entry.
/// Writes atomically via a temp file to prevent corruption on crash.
fn update_index(
    root: &Path,
    rec: &XcRecording,
    audio_filename: &str,
    meta_filename: &str,
) -> Result<(), String> {
    let index_path = root.join("index.json");
    let tmp_path = root.join("index.json.tmp");

    let mut index = read_index(root);

    let sounds = index["sounds"]
        .as_array_mut()
        .ok_or("index.json 'sounds' is not an array")?;

    // Skip if already present
    let id_num = rec.id_num();
    if sounds.iter().any(|s| s["xc_id"].as_u64() == Some(id_num)) {
        return Ok(());
    }

    sounds.push(serde_json::json!({
        "filename": audio_filename,
        "metadata": meta_filename,
        "xc_id": id_num,
        "en": rec.en,
        "species": format!("{} {}", rec.genus, rec.sp),
        "source": "xeno-canto"
    }));

    let json_str = serde_json::to_string_pretty(&index)
        .map_err(|e| format!("Serialize error: {e}"))?;

    // Write to temp file first, then rename for atomic update
    fs::write(&tmp_path, format!("{json_str}\n"))
        .map_err(|e| format!("Failed to write index.json.tmp: {e}"))?;
    fs::rename(&tmp_path, &index_path)
        .map_err(|e| format!("Failed to finalize index.json: {e}"))?;

    Ok(())
}
