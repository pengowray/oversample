//! Wildlife Acoustics metadata (`wamd` RIFF chunk), written by Song Meter and Echo Meter
//! recorders and by Kaleidoscope, usually after the `data` chunk and beside a GUANO
//! chunk.
//!
//! The chunk body is a list of fields, each a little-endian `u16` id, a little-endian
//! `u32` length and that many bytes. Text fields are UTF-8; the version and the time
//! expansion factor are `u16`. Ids 0x00 to 0x13 follow guano-py's `wamd2guano.py`.
//! SM4BAT firmware 2.x also writes 0x14 (the GPS position, in place of 0x06) and 0x15
//! (temperature, "16.25C"), seen in Xeno-canto recordings; other ids hold binary
//! settings and are left out.

use crate::types::RecorderBlock;

/// The recorder name shown as the metadata section's title.
pub const RECORDER: &str = "Wildlife Acoustics";

/// Display labels for the text fields, by id. The labels match the GUANO ones the
/// metadata panel uses, so the same rows get the same formatting.
fn label(id: u16) -> Option<&'static str> {
    Some(match id {
        0x01 => "Model",
        0x02 => "Serial",
        0x03 => "Firmware",
        0x04 => "File Prefix",
        0x05 => "Timestamp",
        0x07 => "GPS Track",
        0x08 => "Software",
        // Kaleidoscope's licence key, not the recording's licence: "License" would
        // be shown as a Creative Commons licence.
        0x09 => "Software License",
        0x0A => "Notes",
        0x0B => "Species (Auto)",
        0x0C => "Species (Manual)",
        0x0E => "Auto ID Stats",
        0x12 => "Microphone",
        0x13 => "Sensitivity",
        0x15 => "Temperature",
        _ => return None,
    })
}

/// The fields of a `wamd` chunk body: id and value bytes, in file order.
fn fields(body: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 6 <= body.len() {
        let id = u16::from_le_bytes([body[off], body[off + 1]]);
        let len = u32::from_le_bytes([body[off + 2], body[off + 3], body[off + 4], body[off + 5]])
            as usize;
        let start = off + 6;
        let Some(end) = start.checked_add(len).filter(|e| *e <= body.len()) else {
            break;
        };
        out.push((id, &body[start..end]));
        off = end;
    }
    out
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

fn u16_value(value: &[u8]) -> Option<u16> {
    (value.len() >= 2).then(|| u16::from_le_bytes([value[0], value[1]]))
}

/// The time expansion factor (field 0x0F), when more than 1.
pub fn time_expansion(body: &[u8]) -> Option<u16> {
    fields(body)
        .into_iter()
        .find(|(id, _)| *id == 0x0F)
        .and_then(|(_, v)| u16_value(v))
        .filter(|te| *te > 1)
}

/// Latitude, longitude and elevation (metres) from a GPS field (0x06 or 0x14). Song
/// Meters write `WGS84,51.50140,N,0.14190,W,12`, sometimes with no datum
/// (`,50.19523,N,66.54684,W`); Echo Meter Touch writes signed degrees,
/// `WGS84,9.7659036,-118.74552,57.21649169921875`.
pub fn parse_gps(value: &str) -> Option<(f64, f64, Option<f64>)> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    let num = |s: &str| s.parse::<f64>().ok();
    let (lat, lon, rest) = match parts.as_slice() {
        [_, lat, ns @ ("N" | "S"), lon, ew @ ("E" | "W"), rest @ ..] => {
            let lat = num(lat)? * if *ns == "S" { -1.0 } else { 1.0 };
            let lon = num(lon)? * if *ew == "W" { -1.0 } else { 1.0 };
            (lat, lon, rest)
        }
        [_, lat, lon, rest @ ..] => (num(lat)?, num(lon)?, rest),
        _ => return None,
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let elevation = rest.first().and_then(|s| num(s));
    Some((lat, lon, elevation))
}

/// The fields of a `wamd` chunk body for the metadata panel, or `None` when it has none
/// worth showing. Left out: the format version, empty text, the embedded voice note's
/// audio, the recorder program and run state and other binary fields, and alignment
/// padding.
pub fn parse_wamd_chunk(body: &[u8]) -> Option<RecorderBlock> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for (id, value) in fields(body) {
        match id {
            0x06 | 0x14 => {
                let raw = text(value);
                match parse_gps(&raw) {
                    Some((lat, lon, elevation)) => {
                        rows.push(("GPS Position".into(), format!("{lat}, {lon}")));
                        if let Some(e) = elevation {
                            rows.push(("Elevation".into(), format!("{e:.0} m")));
                        }
                    }
                    None if !raw.is_empty() => rows.push(("GPS Position".into(), raw)),
                    None => {}
                }
            }
            0x0D if !value.is_empty() => rows.push((
                "Voice Note".into(),
                format!("{} KB audio clip", value.len().div_ceil(1024)),
            )),
            0x0F => {
                if let Some(te) = u16_value(value) {
                    rows.push(("Time Expansion".into(), te.to_string()));
                }
            }
            id => {
                if let Some(l) = label(id) {
                    let t = text(value);
                    if !t.is_empty() {
                        rows.push((l.into(), t));
                    }
                }
            }
        }
    }
    (!rows.is_empty()).then_some(RecorderBlock {
        recorder: RECORDER,
        fields: rows,
    })
}

/// The GUANO keys that hold the same fact as a row of [`parse_wamd_chunk`].
fn guano_keys(label: &str) -> &'static [&'static str] {
    match label {
        "Model" => &["Model"],
        "Serial" => &["Serial"],
        "Firmware" => &["Firmware Version"],
        "File Prefix" => &["WA|Song Meter|Prefix"],
        "Timestamp" => &["Timestamp"],
        "GPS Position" => &["Loc Position"],
        "Elevation" => &["Loc Elevation"],
        "Software" => &["WA|Kaleidoscope|Version"],
        "Notes" => &["Note"],
        "Species (Auto)" => &["Species Auto ID", "WA|Echo Meter|Auto ID"],
        "Species (Manual)" => &["Species Manual ID"],
        "Time Expansion" => &["TE"],
        "Microphone" => &["Microphone"],
        "Temperature" => &["Temperature Int", "Temperature Ext"],
        _ => &[],
    }
}

/// The numbers in a value, such as both coordinates of "50.19523, -66.54684" or the
/// 16.25 of "16.25C".
fn numbers(value: &str) -> Vec<f64> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|part| {
            let num: String = part
                .chars()
                .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'))
                .collect();
            num.parse().ok()
        })
        .collect()
}

/// Whether a GUANO block states the same value as a `wamd` row, so the metadata panel
/// can show the value once. Text must match exactly, except that timestamps may use a
/// space or `T` and Kaleidoscope's version may lack the word "Kaleidoscope"; numbers
/// (positions, elevation, temperature, time expansion) match within the rounding the
/// panel shows. A time expansion of 1 matches a GUANO block without `TE`, which GUANO
/// reads as 1.
pub fn same_in_guano(label: &str, value: &str, guano: &[(String, String)]) -> bool {
    let keys = guano_keys(label);
    let found = guano
        .iter()
        .find(|(k, v)| keys.contains(&k.trim()) && !v.trim().is_empty())
        .map(|(_, v)| v.trim());
    let Some(g) = found else {
        return label == "Time Expansion" && value == "1";
    };
    let close = |tolerance: f64| {
        let (a, b) = (numbers(value), numbers(g));
        !a.is_empty()
            && a.len() == b.len()
            && a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tolerance)
    };
    match label {
        "GPS Position" => close(1e-4),
        "Elevation" => close(0.5),
        "Temperature" => close(0.05),
        "Time Expansion" => close(1e-9),
        "Timestamp" => value.replacen(' ', "T", 1) == g.replacen(' ', "T", 1),
        "Software" => value == g || value.strip_prefix("Kaleidoscope ") == Some(g),
        _ => value == g,
    }
}

/// Search the bytes that follow a WAV file's `data` chunk for a `wamd` chunk, as
/// [`super::guano::parse_guano_after_data`] does for GUANO.
pub fn parse_wamd_after_data(bytes: &[u8], data_size: u64) -> Option<RecorderBlock> {
    let starts: &[usize] = if data_size & 1 == 1 { &[1, 0] } else { &[0] };
    starts.iter().find_map(|&start| {
        super::wav::riff_chunks(bytes, start)
            .find(|c| c.id == b"wamd" && c.complete())
            .and_then(|c| parse_wamd_chunk(c.body))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: u16, value: &[u8]) -> Vec<u8> {
        let mut f = id.to_le_bytes().to_vec();
        f.extend((value.len() as u32).to_le_bytes());
        f.extend(value);
        f
    }

    /// The `wamd` chunk of an observation.org bat recording processed by Kaleidoscope:
    /// version 1 and the software name.
    const KALEIDOSCOPE: &[u8] =
        b"\x00\x00\x02\x00\x00\x00\x01\x00\x08\x00\x13\x00\x00\x00Kaleidoscope 5.6.0c";

    #[test]
    fn kaleidoscope_chunk() {
        let block = parse_wamd_chunk(KALEIDOSCOPE).unwrap();
        assert_eq!(block.recorder, "Wildlife Acoustics");
        assert_eq!(
            block.fields,
            [("Software".to_string(), "Kaleidoscope 5.6.0c".to_string())]
        );
        assert_eq!(time_expansion(KALEIDOSCOPE), None);
    }

    #[test]
    fn song_meter_chunk() {
        let body = [
            field(0x00, &1u16.to_le_bytes()),
            field(0x01, b"SM4BAT-FS\0"),
            field(0x02, b"S4U01234"),
            field(0x03, b"2.3.0"),
            field(0x05, b"2019-08-15 22:12:11-05:00"),
            field(0x06, b"WGS84,51.50140,N,0.14190,W,12"),
            field(0x0B, b"PIPPIP"),
            field(0x0D, &[0u8; 3000]),
            field(0x0F, &10u16.to_le_bytes()),
            field(0x10, &[1, 2, 3]),
            field(0xFFFF, &[0, 0]),
            field(0x12, b"U2"),
        ]
        .concat();
        let block = parse_wamd_chunk(&body).unwrap();
        let rows: Vec<(&str, &str)> = block
            .fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Model", "SM4BAT-FS"),
                ("Serial", "S4U01234"),
                ("Firmware", "2.3.0"),
                ("Timestamp", "2019-08-15 22:12:11-05:00"),
                ("GPS Position", "51.5014, -0.1419"),
                ("Elevation", "12 m"),
                ("Species (Auto)", "PIPPIP"),
                ("Voice Note", "3 KB audio clip"),
                ("Time Expansion", "10"),
                ("Microphone", "U2"),
            ]
        );
        assert_eq!(time_expansion(&body), Some(10));
    }

    fn rows(body: &[u8]) -> Vec<(String, String)> {
        parse_wamd_chunk(body).unwrap().fields
    }

    /// Fields of an SM4BAT-FS recording on Xeno-canto (XC883320, W4V, firmware 2.4.6),
    /// without its binary program and run state.
    #[test]
    fn real_sm4bat_fields() {
        let body = [
            field(0x00, &1u16.to_le_bytes()),
            field(0x01, b"SM4BAT-FS"),
            field(0x02, b"S4U19393"),
            field(0x03, b"2.4.6"),
            field(0x04, b"S4U19393"),
            field(0x05, b"2023-08-10 21:17:04-04:00"),
            field(0x06, b",50.19523,N,66.54684,W"),
            field(0x08, b"Kaleidoscope 5.6.6"),
            field(0x0A, b"\n\n"),
            field(0x0C, b"LACI"),
            field(0x12, b"U2"),
            field(0x13, b"13.0"),
            field(0x15, b"16.25C"),
            field(0x10, b"SM4P\x01\x00V\x9c"),
        ]
        .concat();
        let got = rows(&body);
        let want = [
            ("Model", "SM4BAT-FS"),
            ("Serial", "S4U19393"),
            ("Firmware", "2.4.6"),
            ("File Prefix", "S4U19393"),
            ("Timestamp", "2023-08-10 21:17:04-04:00"),
            ("GPS Position", "50.19523, -66.54684"),
            ("Software", "Kaleidoscope 5.6.6"),
            ("Species (Manual)", "LACI"),
            ("Microphone", "U2"),
            ("Sensitivity", "13.0"),
            ("Temperature", "16.25C"),
        ];
        assert_eq!(
            got.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>(),
            want
        );

        // Firmware 2.x: the position in 0x14, TE 1, the licence key, padding.
        let body = [
            field(0x0F, &1u16.to_le_bytes()),
            field(0x09, b"KP-XXXX"),
            field(0x14, b"WGS84,31.30751,N,81.49842,W"),
            field(0x22, b"\x01\x00\x01\x01MABUDA1\x00"),
            field(0xFFFF, b"\x00"),
        ]
        .concat();
        let got = rows(&body);
        assert_eq!(
            got,
            [
                ("Time Expansion".to_string(), "1".to_string()),
                ("Software License".to_string(), "KP-XXXX".to_string()),
                (
                    "GPS Position".to_string(),
                    "31.30751, -81.49842".to_string()
                ),
            ]
        );
        assert_eq!(time_expansion(&body), None);
    }

    /// Fields of an Echo Meter Touch 2 Pro recording on Xeno-canto (XC1058382).
    #[test]
    fn real_echo_meter_fields() {
        let body = [
            field(0x00, &1u16.to_le_bytes()),
            field(0x01, b"Echo Meter Touch 2 Pro Android"),
            field(0x02, b""),
            field(0x03, b"App 3.0.4"),
            field(0x05, b"2025-11-01 09:34:04+0800"),
            field(0x06, b"WGS84,9.7659036,-118.74552,57.21649169921875"),
            field(0x0B, b"HYPSAV"),
        ]
        .concat();
        let got = rows(&body);
        assert_eq!(
            got.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Model", "Echo Meter Touch 2 Pro Android"),
                ("Firmware", "App 3.0.4"),
                ("Timestamp", "2025-11-01 09:34:04+0800"),
                ("GPS Position", "9.7659036, -118.74552"),
                ("Elevation", "57 m"),
                ("Species (Auto)", "HYPSAV"),
            ]
        );
    }

    #[test]
    fn values_also_in_guano() {
        let guano: Vec<(String, String)> = [
            ("Firmware Version", "2.4.6"),
            ("Loc Position", "50.19523 -66.54684"),
            ("Loc Elevation", "57.21649169921875"),
            ("Model", "SM4BAT-FS"),
            ("Note", ""),
            ("Species Manual ID", "LACI"),
            ("Temperature Int", "16.25"),
            ("Timestamp", "2023-08-10T21:17:04-04:00"),
            ("WA|Kaleidoscope|Version", "5.6.6"),
            ("WA|Song Meter|Prefix", "S4U19393"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        for (label, value) in [
            ("Model", "SM4BAT-FS"),
            ("Firmware", "2.4.6"),
            ("File Prefix", "S4U19393"),
            ("Timestamp", "2023-08-10 21:17:04-04:00"),
            ("GPS Position", "50.19523, -66.54684"),
            ("Elevation", "57 m"),
            ("Software", "Kaleidoscope 5.6.6"),
            ("Species (Manual)", "LACI"),
            ("Temperature", "16.25C"),
            ("Time Expansion", "1"),
        ] {
            assert!(same_in_guano(label, value, &guano), "{label}");
        }
        for (label, value) in [
            // Echo Meter Touch wrote the longitude's sign wrong in wamd.
            ("GPS Position", "50.19523, 66.54684"),
            ("Model", "SM4BAT"),
            ("Serial", "S4U19393"),
            ("Notes", "rain"),
            ("Microphone", "U2"),
            ("Sensitivity", "13.0"),
            ("Time Expansion", "10"),
            ("Software", "Kaleidoscope 5.6.0"),
        ] {
            assert!(!same_in_guano(label, value, &guano), "{label}");
        }
    }

    #[test]
    fn gps_layouts() {
        assert_eq!(
            parse_gps("WGS84,33.86000,S,151.20000,E"),
            Some((-33.86, 151.2, None))
        );
        assert_eq!(
            parse_gps("WGS84, 51.5, -0.14, 30.4"),
            Some((51.5, -0.14, Some(30.4)))
        );
        assert_eq!(parse_gps("WGS84,,"), None);
        assert_eq!(parse_gps("WGS84,95,N,0,E"), None);
    }

    #[test]
    fn damaged_chunks() {
        // A length past the end stops the walk; what came before is kept.
        let mut body = field(0x01, b"SM4");
        body.extend(field(0x02, b"S4U"));
        body.truncate(body.len() - 1);
        let block = parse_wamd_chunk(&body).unwrap();
        assert_eq!(block.fields, [("Model".to_string(), "SM4".to_string())]);
        assert_eq!(parse_wamd_chunk(&[0xFF; 5]), None);
        assert_eq!(parse_wamd_chunk(&[]), None);
        let huge = [0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, b'x'];
        assert_eq!(parse_wamd_chunk(&huge), None);
    }

    #[test]
    fn found_after_the_data() {
        // An odd-sized data chunk: the wamd chunk starts after a pad byte.
        let mut tail = vec![0u8];
        tail.extend(b"wamd");
        tail.extend((KALEIDOSCOPE.len() as u32).to_le_bytes());
        tail.extend(KALEIDOSCOPE);
        assert!(parse_wamd_after_data(&tail, 3).is_some());
        assert!(parse_wamd_after_data(&tail[1..], 4).is_some());
    }
}
