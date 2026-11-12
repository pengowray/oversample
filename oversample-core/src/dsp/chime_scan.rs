//! Finds AudioMoth chimes in a recording, and what they carry: the phone's date, time and UTC
//! offset, a location if the phone sent one, and a deployment ID.
//!
//! A chime is a short data-over-sound signal on an 18 kHz carrier that a phone plays to set a
//! recorder's clock. The TASCAM X8 bat detector firmware also uses it as a phone remote: the
//! deployment ID is `BATX`, a version byte, a command and an argument (see `chime::format`). The stop chime is
//! recorded at the end of the take, so a take can carry the phone's date, time and location
//! although the recorder has no GPS.
//!
//! Feed each channel with [`ChimeScanner::push`] in pieces of any length, so a caller can yield
//! between pieces, then call [`ChimeScanner::finish`].

use std::mem::MaybeUninit;

use chime::format::TimeMessage;
use chime::{Decoder, Event};

/// Two readings this close together, with the same contents, are one chime heard on two channels.
const SAME_CHIME_SECS: f64 = 0.1;

/// The X8 phone remote's command in a chime.
pub use chime::format::{RemoteCommand, RemoteMessage};

/// A chime that was read.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundChime {
    /// Channels it was read on, from 0.
    pub channels: Vec<u32>,
    /// Where its first start symbol began, in seconds from the start of the file. The time in
    /// the chime is the phone's time at this moment.
    pub start_secs: f64,
    /// Seconds since 1970-01-01 00:00:00 UTC.
    pub unix_time: u32,
    /// The phone's offset from UTC in minutes.
    pub timezone_minutes: i16,
    /// Latitude and longitude in degrees.
    pub location: Option<(f64, f64)>,
    pub deployment_id: Option<[u8; 8]>,
    /// The carrier's RMS level during the chime.
    pub level_dbfs: f32,
    /// The carrier over the noise in a 2 kHz band, if the noise could be measured.
    pub snr_db: Option<f32>,
}

impl FoundChime {
    /// The X8 phone remote message in the deployment ID, known or not.
    pub fn remote(&self) -> Option<RemoteMessage> {
        RemoteMessage::from_deployment_id(self.deployment_id.as_ref()?)
    }

    /// The X8 phone remote command in the deployment ID, if it is one this version knows.
    pub fn remote_command(&self) -> Option<RemoteCommand> {
        self.remote()?.known()
    }

    /// The phone's local time as ISO 8601 with its UTC offset, such as
    /// `2026-10-26T17:33:40+10:00`.
    pub fn local_time_iso(&self) -> String {
        let local = chime::LocalTime::from_epoch_seconds(
            self.unix_time as i64 + self.timezone_minutes as i64 * 60,
        );
        let sign = if self.timezone_minutes < 0 { '-' } else { '+' };
        let offset = self.timezone_minutes.unsigned_abs();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
            offset / 60,
            offset % 60
        )
    }

    /// The deployment ID as 16 hex digits, in the order the chime sends it.
    pub fn deployment_id_hex(&self) -> Option<String> {
        self.deployment_id
            .map(|id| id.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChimeScanResult {
    pub chimes: Vec<FoundChime>,
    /// Chimes that were heard but could not be read, on any channel.
    pub unreadable: u32,
}

/// Decoders for one stretch of a file, one per channel.
pub struct ChimeScanner {
    decoders: Vec<Box<Decoder>>,
    /// Samples pushed to each channel so far.
    pushed: Vec<u64>,
    sample_rate: u32,
    /// The file's frame at which the stretch starts.
    start_frame: u64,
    result: ChimeScanResult,
}

impl ChimeScanner {
    /// Decoders for `channels` channels from `start_frame` on. `None` below 40 kHz, where the
    /// 18 kHz carrier cannot be recorded.
    pub fn new(sample_rate: u32, channels: usize, start_frame: u64) -> Option<Self> {
        let mut decoders = Vec::with_capacity(channels);
        for _ in 0..channels {
            // Built in place: a decoder is about 19 KB, too large to pass by value comfortably.
            let mut slot: Box<MaybeUninit<Decoder>> = Box::new_uninit();
            Decoder::init_in_place(&mut slot, sample_rate)?;
            // SAFETY: `init_in_place` returned Some, so it wrote the whole decoder.
            decoders.push(unsafe { slot.assume_init() });
        }
        Some(Self {
            decoders,
            pushed: vec![0; channels],
            sample_rate,
            start_frame,
            result: ChimeScanResult::default(),
        })
    }

    /// The next samples of `channel`.
    pub fn push(&mut self, channel: usize, samples: &[f32]) {
        let Some(decoder) = self.decoders.get_mut(channel) else {
            return;
        };
        for &x in samples {
            if let Some(event) = decoder.push(x) {
                match event {
                    Event::Chime(chime) => {
                        let Some(message) = TimeMessage::parse(&chime.payload[..chime.payload_len])
                        else {
                            continue;
                        };
                        let start_frame = self.start_frame as f64 + chime.start_sample as f64;
                        let found = FoundChime {
                            channels: vec![channel as u32],
                            start_secs: start_frame / self.sample_rate as f64,
                            unix_time: message.unix_time,
                            timezone_minutes: message.timezone_minutes,
                            location: message.latitude_longitude(),
                            deployment_id: message.deployment_id,
                            level_dbfs: chime.level_dbfs,
                            snr_db: (!chime.snr_db.is_nan()).then_some(chime.snr_db),
                        };
                        add(&mut self.result.chimes, found);
                    }
                    Event::Unreadable(_) => self.result.unreadable += 1,
                }
            }
        }
        self.pushed[channel] += samples.len() as u64;
    }

    pub fn finish(mut self) -> ChimeScanResult {
        self.result
            .chimes
            .sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
        self.result
    }
}

/// Add a reading, or the channel to an earlier reading of the same chime.
fn add(chimes: &mut Vec<FoundChime>, found: FoundChime) {
    let same = chimes.iter_mut().find(|c| {
        (c.start_secs - found.start_secs).abs() < SAME_CHIME_SECS
            && c.unix_time == found.unix_time
            && c.timezone_minutes == found.timezone_minutes
            && c.location == found.location
            && c.deployment_id == found.deployment_id
    });
    match same {
        Some(earlier) => {
            for channel in found.channels {
                if !earlier.channels.contains(&channel) {
                    earlier.channels.push(channel);
                }
            }
            // The louder reading's level and SNR.
            if found.level_dbfs > earlier.level_dbfs {
                earlier.level_dbfs = found.level_dbfs;
                earlier.snr_db = found.snr_db;
            }
        }
        None => chimes.push(found),
    }
}

/// Merge the results of scans of different stretches of one file.
pub fn merge(results: impl IntoIterator<Item = ChimeScanResult>) -> ChimeScanResult {
    let mut merged = ChimeScanResult::default();
    for result in results {
        merged.unreadable += result.unreadable;
        for chime in result.chimes {
            add(&mut merged.chimes, chime);
        }
    }
    merged
        .chimes
        .sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
    merged
}

/// The stretches of a file to scan by default, as (start frame, frames): the first and last
/// `edge_secs`, where the X8 phone remote's start and stop chimes are, or the whole file if
/// that is not much longer.
pub fn edge_ranges(total_frames: u64, sample_rate: u32, edge_secs: f64) -> Vec<(u64, u64)> {
    let edge = (edge_secs * sample_rate as f64) as u64;
    if total_frames <= 2 * edge {
        vec![(0, total_frames)]
    } else {
        vec![(0, edge), (total_frames - edge, edge)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 192_000;

    /// A chime's 18 kHz symbols with no melody, from the chime crate's encoder, at `gain` times the
    /// level the phone page plays them.
    fn render(payload: &[u8], gain: f32) -> Vec<f32> {
        let carrier = chime::encode::render(RATE, payload, &[]).carrier;
        carrier
            .iter()
            .map(|&c| (c * (gain * chime::format::CARRIER_AMPLITUDE) as f64) as f32)
            .collect()
    }

    fn payload(unix: u32, tz: i16, location: Option<[u8; 7]>, id: Option<&[u8; 8]>) -> Vec<u8> {
        let mut p = unix.to_le_bytes().to_vec();
        p.extend_from_slice(&tz.to_le_bytes());
        if let Some(l) = location {
            p.extend_from_slice(&l);
        }
        if let Some(id) = id {
            p.extend_from_slice(id);
        }
        p
    }

    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((*seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 2e-4
            })
            .collect()
    }

    #[test]
    fn a_stop_chime_gives_its_time_location_and_command() {
        // -33.87, 151.21, packed as the phone page's library packs them.
        let location = [0x50, 0x2F, 0xFB, 0x8D, 0x40, 0x1A, 0x48];
        let chime = render(
            &payload(
                1_793_000_020,
                600,
                Some(location),
                Some(&RemoteCommand::Stop.deployment_id()),
            ),
            0.1,
        );
        let mut seed = 1;
        let mut x = noise(RATE as usize * 2 + chime.len(), &mut seed);
        for (a, b) in x[RATE as usize..].iter_mut().zip(&chime) {
            *a += b;
        }
        // As the last stretch of a longer file, fed in pieces, on two channels.
        let start_frame = 1_000 * RATE as u64;
        let mut scanner = ChimeScanner::new(RATE, 2, start_frame).unwrap();
        for piece in x.chunks(50_000) {
            scanner.push(0, piece);
            scanner.push(1, piece);
        }
        let result = scanner.finish();
        assert_eq!(result.chimes.len(), 1, "{result:?}");
        let found = &result.chimes[0];
        assert_eq!(found.channels, [0, 1]);
        assert!(
            (found.start_secs - 1_001.0).abs() < 0.01,
            "{}",
            found.start_secs
        );
        assert_eq!(found.local_time_iso(), "2026-10-26T17:33:40+10:00");
        let (lat, lon) = found.location.unwrap();
        assert!(
            (lat + 33.87).abs() < 1e-5 && (lon - 151.21).abs() < 1e-5,
            "{lat} {lon}"
        );
        assert_eq!(found.remote_command(), Some(RemoteCommand::Stop));
        assert_eq!(
            found.deployment_id_hex().as_deref(),
            Some("4241545801020000")
        );
        assert!(found.snr_db.is_some());
    }

    #[test]
    fn a_clock_chime_has_no_command_and_negative_offsets_print() {
        let chime = render(&payload(1_793_000_000, -210, None, None), 0.1);
        let mut seed = 2;
        let mut x = noise(RATE as usize + chime.len(), &mut seed);
        for (a, b) in x[RATE as usize / 2..].iter_mut().zip(&chime) {
            *a += b;
        }
        let mut scanner = ChimeScanner::new(RATE, 1, 0).unwrap();
        scanner.push(0, &x);
        let result = scanner.finish();
        assert_eq!(result.chimes.len(), 1);
        assert_eq!(result.chimes[0].remote_command(), None);
        assert_eq!(result.chimes[0].location, None);
        assert_eq!(
            result.chimes[0].local_time_iso(),
            "2026-10-26T04:03:20-03:30"
        );
    }

    #[test]
    fn low_sample_rates_cannot_be_scanned() {
        assert!(ChimeScanner::new(32_000, 1, 0).is_none());
        assert!(ChimeScanner::new(48_000, 1, 0).is_some());
    }

    #[test]
    fn edge_ranges_cover_short_files_whole() {
        assert_eq!(edge_ranges(50 * 48_000, 48_000, 30.0), [(0, 50 * 48_000)]);
        assert_eq!(
            edge_ranges(100 * 48_000, 48_000, 30.0),
            [(0, 30 * 48_000), (70 * 48_000, 30 * 48_000)]
        );
    }

    #[test]
    fn merging_keeps_one_reading_per_chime() {
        let found = |channel: u32, start_secs: f64, level_dbfs: f32| FoundChime {
            channels: vec![channel],
            start_secs,
            unix_time: 5,
            timezone_minutes: 0,
            location: None,
            deployment_id: None,
            level_dbfs,
            snr_db: None,
        };
        let a = ChimeScanResult {
            chimes: vec![found(0, 10.0, -70.0)],
            unreadable: 1,
        };
        let b = ChimeScanResult {
            chimes: vec![found(1, 10.02, -60.0), found(1, 50.0, -60.0)],
            unreadable: 2,
        };
        let merged = merge([a, b]);
        assert_eq!(merged.unreadable, 3);
        assert_eq!(merged.chimes.len(), 2);
        assert_eq!(merged.chimes[0].channels, [0, 1]);
        assert_eq!(merged.chimes[0].level_dbfs, -60.0);
    }
}
