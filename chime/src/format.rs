//! Definitions of the AudioMoth chime format: symbol timings, CRC, Hamming code
//! and the time message in the payload.
//!
//! The values come from `audiomothchime.js` and `audiomothchime_connector.js`
//! by Open Acoustic Devices (MIT licence, copies in the `batchi-chime` repository).

use core::fmt;

/// Carrier frequency in Hz.
pub const CARRIER_HZ: u32 = 18_000;

/// Length of the sin^2 rise, and of the sin^2 fall, of every symbol, in
/// microseconds.
pub const RAMP_US: u32 = 500;

/// Length of a data 0 symbol including both ramps, in microseconds.
pub const DATA_0_US: u32 = 5_000;

/// Length of a data 1 symbol including both ramps, in microseconds.
pub const DATA_1_US: u32 = 10_000;

/// Length of a start or stop symbol including both ramps, in microseconds.
pub const START_STOP_US: u32 = 7_500;

/// Number of start symbols at the beginning of a chime.
pub const START_SYMBOLS: usize = 16;

/// Number of stop symbols at the end of a chime.
pub const STOP_SYMBOLS: usize = 8;

/// Number of data symbols for each byte: two 7-bit Hamming codewords.
pub const SYMBOLS_PER_BYTE: usize = 14;

/// Number of CRC bytes after the payload.
pub const CRC_BYTES: usize = 2;

/// Size in bytes of the Unix time and time zone offset that start every payload.
pub const TIME_BYTES: usize = 6;

/// Size in bytes of the optional location field.
pub const LOCATION_BYTES: usize = 7;

/// Size in bytes of the optional deployment ID field.
pub const DEPLOYMENT_ID_BYTES: usize = 8;

/// Size in bytes of the longest payload: time, location and deployment ID.
pub const MAX_PAYLOAD_BYTES: usize = 21;

/// Size in bytes of the longest message: the longest payload and its CRC.
pub const MAX_MESSAGE_BYTES: usize = MAX_PAYLOAD_BYTES + CRC_BYTES;

/// Lowest time zone offset the encoder sends, in minutes: UTC-12:00.
pub const MIN_TIMEZONE_MINUTES: i16 = -720;

/// Highest time zone offset the encoder sends, in minutes: UTC+14:00.
pub const MAX_TIMEZONE_MINUTES: i16 = 840;

/// Amplitude of the carrier in the waveform the encoder produces.
pub const CARRIER_AMPLITUDE: f32 = 0.25;

/// Amplitude of the melody in the waveform the encoder produces.
pub const MELODY_AMPLITUDE: f32 = 0.5;

/// Hamming codeword for each nibble, as a 7-bit word. Bit j of the word is
/// `HAMMING_CODES[nibble][j]` in the JavaScript encoder, and bit 0 is sent first.
pub const HAMMING_CODEWORDS: [u8; 16] = [
    0, 7, 25, 30, 42, 45, 51, 52, 75, 76, 82, 85, 97, 102, 120, 127,
];

const CRC_POLYNOMIAL: u16 = 0x1021;

/// CRC-16/XMODEM of `bytes`: polynomial 0x1021, initial value 0, most significant
/// bit first. A chime sends the low byte of the result first.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        for bit in (0..8).rev() {
            crc = crc_shift(crc, (byte >> bit) & 1 != 0);
        }
    }
    for _ in 0..16 {
        crc = crc_shift(crc, false);
    }
    crc
}

/// True for a payload length that the encoder can send: the time alone, or with
/// location, deployment ID or both.
pub const fn is_payload_len(len: usize) -> bool {
    len == TIME_BYTES
        || len == TIME_BYTES + LOCATION_BYTES
        || len == TIME_BYTES + DEPLOYMENT_ID_BYTES
        || len == MAX_PAYLOAD_BYTES
}

fn crc_shift(crc: u16, input_bit: bool) -> u16 {
    let shifted = (crc << 1) | input_bit as u16;
    if crc & 0x8000 != 0 {
        shifted ^ CRC_POLYNOMIAL
    } else {
        shifted
    }
}

/// Decodes a 7-bit Hamming codeword, using the low 7 bits of `word`. Returns the
/// nibble of the nearest codeword and whether a bit was corrected.
///
/// The code has a minimum distance of 3 and every 7-bit word is within one bit
/// of exactly one codeword, so every word decodes to a nibble. A word with two
/// wrong bits decodes to the wrong nibble, and the CRC has to catch that.
pub fn hamming_decode(word: u8) -> (u8, bool) {
    let word = word & 0x7F;
    let mut nibble = 0u8;
    for (candidate, codeword) in HAMMING_CODEWORDS.iter().enumerate() {
        if (word ^ codeword).count_ones() <= 1 {
            nibble = candidate as u8;
            break;
        }
    }
    (nibble, word != HAMMING_CODEWORDS[nibble as usize])
}

/// The contents of a chime payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeMessage {
    /// Seconds since 1970-01-01 00:00:00 UTC at the start of the chime.
    pub unix_time: u32,
    /// The phone's offset from UTC in minutes, so that UTC+1 is 60.
    pub timezone_minutes: i16,
    /// The location field as sent: 28 bits of latitude in units of 1/1000000
    /// degree, then 28 bits of longitude in units of 1/500000 degree, both
    /// two's complement and least significant bit first.
    pub location: Option<[u8; LOCATION_BYTES]>,
    /// The deployment ID field as sent.
    pub deployment_id: Option<[u8; DEPLOYMENT_ID_BYTES]>,
}

/// The first 4 bytes of a phone remote deployment ID for the TASCAM Portacapture X8 bat patch, in
/// the order the decoder gives them. `docs/PHONE_REMOTE.md` in the oversample-tascam-firmware
/// repository is the full format.
pub const REMOTE_MAGIC: [u8; 4] = *b"BATX";

/// The first layout of the 4 bytes after [`REMOTE_MAGIC`]: byte 4 is this version, byte 5 the
/// command and bytes 6 and 7 a 16-bit argument, low byte first. New commands keep the version; a
/// new version means bytes 5 to 7 have a different layout.
pub const REMOTE_VERSION: u8 = 1;

/// The layout with a check, for a receiver that shows a code: byte 4 is this version, byte 5 has
/// the command in its high 4 bits and the argument in its low 4 bits, and bytes 6 and 7 are the
/// check that [`remote_check`] makes from the code.
pub const REMOTE_VERSION_CODED: u8 = 2;

/// The number of digits in the code that the receiver shows and the phone page is given.
pub const REMOTE_CODE_DIGITS: usize = 6;

/// The check in bytes 6 and 7 of a version 2 deployment ID: the low 16 bits of a SipHash-2-4
/// hash, low byte first. The key is 16 bytes: the code's 6 ASCII digits, then 10 zero bytes. The
/// message is 12 bytes: [`REMOTE_MAGIC`], [`REMOTE_VERSION_CODED`], byte 5 of the deployment ID,
/// then the chime's Unix time (4 bytes) and UTC offset in minutes (2 bytes), both little-endian
/// as in the payload.
///
/// The time is in the message, so a recording of a chime has the wrong check when it is sent
/// again with another time. The check has 16 bits, so one guess in 65,536 is right, and a
/// recording of a few commands is enough to find the code by trying all 1,000,000 codes.
pub fn remote_check(
    code: &[u8; REMOTE_CODE_DIGITS],
    command_byte: u8,
    unix_time: u32,
    timezone_minutes: i16,
) -> [u8; 2] {
    // The key's second half is zero. The message is 12 bytes, so it is one whole word, then a
    // last word with its other 4 bytes and, in the top byte, its length.
    let [c0, c1, c2, c3, c4, c5] = *code;
    let key = [u64::from_le_bytes([c0, c1, c2, c3, c4, c5, 0, 0]), 0];
    let [t0, t1, t2, t3] = unix_time.to_le_bytes();
    let [z0, z1] = timezone_minutes.to_le_bytes();
    let [m0, m1, m2, m3] = REMOTE_MAGIC;
    let words = [
        u64::from_le_bytes([m0, m1, m2, m3, REMOTE_VERSION_CODED, command_byte, t0, t1]),
        u64::from_le_bytes([t2, t3, z0, z1, 0, 0, 0, 12]),
    ];
    let [check_0, check_1, ..] = siphash_2_4(key, &words).to_le_bytes();
    [check_0, check_1]
}

/// SipHash-2-4 with a 64-bit result (Aumasson and Bernstein, 2012), written for size: in the X8
/// patch it is smaller than the `siphasher` crate, which this file's tests check it against.
/// `key` is the 16-byte key as two little-endian words. `words` is the message as little-endian
/// words, the last one holding the bytes left over and, in its top byte, the message's length
/// in bytes modulo 256.
fn siphash_2_4(key: [u64; 2], words: &[u64]) -> u64 {
    #[inline(never)]
    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }
    let mut v = [
        key[0] ^ 0x736f_6d65_7073_6575,
        key[1] ^ 0x646f_7261_6e64_6f6d,
        key[0] ^ 0x6c79_6765_6e65_7261,
        key[1] ^ 0x7465_6462_7974_6573,
    ];
    for &word in words {
        v[3] ^= word;
        round(&mut v);
        round(&mut v);
        v[0] ^= word;
    }
    v[2] ^= 0xff;
    for _ in 0..4 {
        round(&mut v);
    }
    v[0] ^ v[1] ^ v[2] ^ v[3]
}

/// A phone remote message: a deployment ID that starts with [`REMOTE_MAGIC`]. It may hold a
/// version or command that this crate does not know, which the receiver reports as unknown.
///
/// The fields are named for version 1. In a version 2 message `command` is byte 5, which has the
/// command and the argument, and `argument` is the check: see [`RemoteMessage::coded`] and
/// [`RemoteMessage::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteMessage {
    pub version: u8,
    pub command: u8,
    pub argument: u16,
}

impl RemoteMessage {
    /// The message in a deployment ID, if the ID starts with [`REMOTE_MAGIC`].
    pub fn from_deployment_id(id: &[u8; DEPLOYMENT_ID_BYTES]) -> Option<Self> {
        if id[..4] != REMOTE_MAGIC {
            return None;
        }
        Some(Self {
            version: id[4],
            command: id[5],
            argument: u16::from_le_bytes([id[6], id[7]]),
        })
    }

    /// The whole deployment ID, as the decoder gives it.
    pub fn deployment_id(self) -> [u8; DEPLOYMENT_ID_BYTES] {
        let [low, high] = self.argument.to_le_bytes();
        let [m0, m1, m2, m3] = REMOTE_MAGIC;
        [m0, m1, m2, m3, self.version, self.command, low, high]
    }

    /// The command of a version 1 message, if this crate knows the command and its argument.
    /// None for a version 2 message: see [`RemoteMessage::coded`].
    pub fn known(self) -> Option<RemoteCommand> {
        if self.version != REMOTE_VERSION {
            return None;
        }
        RemoteCommand::from_code(self.command, self.argument)
    }

    /// The command of a version 2 message, if this crate knows the command and its argument. The
    /// check is not looked at: see [`RemoteMessage::check_matches`].
    pub fn coded(self) -> Option<RemoteCommand> {
        if self.version != REMOTE_VERSION_CODED {
            return None;
        }
        RemoteCommand::from_code(self.command >> 4, (self.command & 0x0f).into())
    }

    /// The command of a version 1 or version 2 message that this crate knows, whatever the
    /// check of a version 2 message is.
    pub fn command(self) -> Option<RemoteCommand> {
        self.known().or(self.coded())
    }

    /// The check in a version 2 message, as bytes 6 and 7 of the deployment ID.
    pub fn check(self) -> Option<[u8; 2]> {
        (self.version == REMOTE_VERSION_CODED).then(|| self.argument.to_le_bytes())
    }

    /// Whether this is a version 2 message whose check is the one that `code` gives for the
    /// chime's time and UTC offset. The command may be one this crate does not know.
    pub fn check_matches(
        self,
        code: &[u8; REMOTE_CODE_DIGITS],
        unix_time: u32,
        timezone_minutes: i16,
    ) -> bool {
        self.check()
            == Some(remote_check(
                code,
                self.command,
                unix_time,
                timezone_minutes,
            ))
    }
}

/// A command for the X8 bat patch's phone remote. Only [`RemoteCommand::Listen`] has an argument;
/// the others are sent with argument 0, and any other argument makes the message unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteCommand {
    /// Start recording.
    Start,
    /// Stop recording.
    Stop,
    /// Set the clock to the chime's time. Start, stop and listening mode chimes set the clock too,
    /// but only if the receiver is set to.
    SetClock,
    /// Play this listening mode in the headphones.
    Listen(ListeningMode),
}

impl RemoteCommand {
    pub const ALL: [RemoteCommand; 9] = [
        RemoteCommand::Start,
        RemoteCommand::Stop,
        RemoteCommand::SetClock,
        RemoteCommand::Listen(ListeningMode::Het),
        RemoteCommand::Listen(ListeningMode::Comb),
        RemoteCommand::Listen(ListeningMode::Fd),
        RemoteCommand::Listen(ListeningMode::Te),
        RemoteCommand::Listen(ListeningMode::Pitch),
        RemoteCommand::Listen(ListeningMode::Thru),
    ];

    /// The command with this command byte and argument, if this crate knows both.
    fn from_code(code: u8, argument: u16) -> Option<Self> {
        match (code, argument) {
            (1, 0) => Some(RemoteCommand::Start),
            (2, 0) => Some(RemoteCommand::Stop),
            (3, 0) => Some(RemoteCommand::SetClock),
            (4, 1..=6) => Some(RemoteCommand::Listen(
                ListeningMode::ALL[argument as usize - 1],
            )),
            _ => None,
        }
    }

    /// The command byte. 0 is never a command.
    pub const fn code(self) -> u8 {
        match self {
            RemoteCommand::Start => 1,
            RemoteCommand::Stop => 2,
            RemoteCommand::SetClock => 3,
            RemoteCommand::Listen(_) => 4,
        }
    }

    /// The 16-bit argument.
    pub const fn argument(self) -> u16 {
        match self {
            RemoteCommand::Listen(mode) => mode.code(),
            _ => 0,
        }
    }

    /// The message that sends this command.
    pub const fn message(self) -> RemoteMessage {
        RemoteMessage {
            version: REMOTE_VERSION,
            command: self.code(),
            argument: self.argument(),
        }
    }

    /// The version 1 command in a deployment ID, if it holds one this crate knows.
    pub fn from_deployment_id(id: &[u8; DEPLOYMENT_ID_BYTES]) -> Option<Self> {
        RemoteMessage::from_deployment_id(id)?.known()
    }

    /// The whole version 1 deployment ID, as the decoder gives it.
    pub fn deployment_id(self) -> [u8; DEPLOYMENT_ID_BYTES] {
        self.message().deployment_id()
    }

    /// Byte 5 of a version 2 deployment ID: the command in the high 4 bits and the argument in
    /// the low 4 bits.
    pub const fn coded_byte(self) -> u8 {
        self.code() << 4 | self.argument() as u8
    }

    /// The whole version 2 deployment ID, as the decoder gives it, for a chime that carries
    /// `unix_time` and `timezone_minutes`, with the check that `code` gives.
    pub fn coded_deployment_id(
        self,
        code: &[u8; REMOTE_CODE_DIGITS],
        unix_time: u32,
        timezone_minutes: i16,
    ) -> [u8; DEPLOYMENT_ID_BYTES] {
        let [m0, m1, m2, m3] = REMOTE_MAGIC;
        let byte = self.coded_byte();
        let [c0, c1] = remote_check(code, byte, unix_time, timezone_minutes);
        [m0, m1, m2, m3, REMOTE_VERSION_CODED, byte, c0, c1]
    }
}

/// A listening mode of the X8 bat patch, the argument of [`RemoteCommand::Listen`]. The codes
/// follow the order of the X8's mode buttons, from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListeningMode {
    /// Heterodyne.
    Het,
    /// Heterodyne at several frequencies at once.
    Comb,
    /// Frequency division.
    Fd,
    /// Time expansion.
    Te,
    /// Pitch shift.
    Pitch,
    /// The input as it is, at 1:1.
    Thru,
}

impl ListeningMode {
    pub const ALL: [ListeningMode; 6] = [
        ListeningMode::Het,
        ListeningMode::Comb,
        ListeningMode::Fd,
        ListeningMode::Te,
        ListeningMode::Pitch,
        ListeningMode::Thru,
    ];

    /// The argument that selects this mode.
    pub const fn code(self) -> u16 {
        match self {
            ListeningMode::Het => 1,
            ListeningMode::Comb => 2,
            ListeningMode::Fd => 3,
            ListeningMode::Te => 4,
            ListeningMode::Pitch => 5,
            ListeningMode::Thru => 6,
        }
    }

    /// The name on the X8's mode button.
    pub const fn name(self) -> &'static str {
        match self {
            ListeningMode::Het => "HET",
            ListeningMode::Comb => "COMB",
            ListeningMode::Fd => "FD",
            ListeningMode::Te => "TE",
            ListeningMode::Pitch => "PITCH",
            ListeningMode::Thru => "1:1",
        }
    }
}

impl TimeMessage {
    /// Parses a payload without its CRC. The valid lengths are 6, 13 (with
    /// location), 14 (with deployment ID) and 21 bytes (with both, location
    /// first). Returns `None` for any other length, and for a time zone outside
    /// [`MIN_TIMEZONE_MINUTES`] to [`MAX_TIMEZONE_MINUTES`], which the encoder
    /// never sends.
    pub fn parse(payload: &[u8]) -> Option<TimeMessage> {
        if !is_payload_len(payload.len()) {
            return None;
        }
        let has_location = matches!(payload.len(), 13 | 21);
        let has_deployment_id = matches!(payload.len(), 14 | 21);

        let unix_time = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let timezone_minutes = i16::from_le_bytes([payload[4], payload[5]]);
        if !(MIN_TIMEZONE_MINUTES..=MAX_TIMEZONE_MINUTES).contains(&timezone_minutes) {
            return None;
        }

        let mut offset = TIME_BYTES;
        let mut location = None;
        if has_location {
            let mut field = [0u8; LOCATION_BYTES];
            field.copy_from_slice(&payload[offset..offset + LOCATION_BYTES]);
            location = Some(field);
            offset += LOCATION_BYTES;
        }
        let mut deployment_id = None;
        if has_deployment_id {
            let mut field = [0u8; DEPLOYMENT_ID_BYTES];
            field.copy_from_slice(&payload[offset..offset + DEPLOYMENT_ID_BYTES]);
            deployment_id = Some(field);
        }

        Some(TimeMessage {
            unix_time,
            timezone_minutes,
            location,
            deployment_id,
        })
    }

    /// The X8 bat patch's phone remote message in the deployment ID, known or not.
    pub fn remote(&self) -> Option<RemoteMessage> {
        RemoteMessage::from_deployment_id(self.deployment_id.as_ref()?)
    }

    /// The X8 bat patch's version 1 phone remote command in the deployment ID, if it holds one
    /// this crate knows.
    pub fn remote_command(&self) -> Option<RemoteCommand> {
        self.remote()?.known()
    }

    /// Whether the deployment ID is a version 2 phone remote message whose check is the one that
    /// `code` gives for this chime's time and UTC offset.
    pub fn remote_check_matches(&self, code: &[u8; REMOTE_CODE_DIGITS]) -> bool {
        self.remote()
            .is_some_and(|remote| remote.check_matches(code, self.unix_time, self.timezone_minutes))
    }

    /// The location as latitude and longitude in degrees, if the chime has one.
    pub fn latitude_longitude(&self) -> Option<(f64, f64)> {
        let b = self.location?.map(u32::from);
        // Move each 28-bit field to the top of an i32, so that shifting back extends its sign.
        let latitude = ((b[0] | b[1] << 8 | b[2] << 16 | (b[3] & 0x0f) << 24) << 4) as i32 >> 4;
        let longitude = ((b[3] >> 4 | b[4] << 4 | b[5] << 12 | b[6] << 20) << 4) as i32 >> 4;
        Some((
            latitude as f64 / LATITUDE_UNITS_PER_DEGREE,
            longitude as f64 / LONGITUDE_UNITS_PER_DEGREE,
        ))
    }

    /// The phone's local time at the start of the chime.
    pub fn local(&self) -> LocalTime {
        LocalTime::from_epoch_seconds(self.unix_time as i64 + self.timezone_minutes as i64 * 60)
    }
}

const LATITUDE_UNITS_PER_DEGREE: f64 = 1_000_000.0;
const LONGITUDE_UNITS_PER_DEGREE: f64 = 500_000.0;

const SECONDS_IN_DAY: i64 = 86_400;

/// A calendar date and time of day with no time zone attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    /// Converts a count of seconds since 1970-01-01 00:00:00 to a date and time.
    /// A negative count gives a date before 1970. No time zone is applied.
    pub fn from_epoch_seconds(seconds: i64) -> LocalTime {
        let (year, month, day) = civil_from_days(seconds.div_euclid(SECONDS_IN_DAY));
        let second_of_day = seconds.rem_euclid(SECONDS_IN_DAY);
        LocalTime {
            year,
            month,
            day,
            hour: (second_of_day / 3600) as u8,
            minute: (second_of_day / 60 % 60) as u8,
            second: (second_of_day % 60) as u8,
        }
    }

    /// Converts this date and time to a count of seconds since
    /// 1970-01-01 00:00:00. No time zone is applied.
    pub fn to_epoch_seconds(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * SECONDS_IN_DAY
            + self.hour as i64 * 3600
            + self.minute as i64 * 60
            + self.second as i64
    }
}

impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

// The two calendar conversions below are Howard Hinnant's algorithms for the
// proleptic Gregorian calendar. A year here starts on 1 March, so the leap day is
// the last day of the year. An era is 400 years, or 146097 days.

fn civil_from_days(days_since_1970: i64) -> (i32, u8, u8) {
    let shifted = days_since_1970 + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u8, day as u8)
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let month = month as i64;
    let year = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod location_tests {
    use super::*;

    /// As `encodeLocation` in `audiomothchime_connector.js` packs them.
    fn message_at(latitude: f64, longitude: f64) -> TimeMessage {
        let lat = libm::round(latitude * LATITUDE_UNITS_PER_DEGREE) as i32;
        let lon = libm::round(longitude * LONGITUDE_UNITS_PER_DEGREE) as i32;
        let location = [
            lat as u8,
            (lat >> 8) as u8,
            (lat >> 16) as u8,
            ((lat >> 24) & 0x0f) as u8 | ((lon & 0x0f) << 4) as u8,
            (lon >> 4) as u8,
            (lon >> 12) as u8,
            (lon >> 20) as u8,
        ];
        TimeMessage {
            unix_time: 0,
            timezone_minutes: 0,
            location: Some(location),
            deployment_id: None,
        }
    }

    #[test]
    fn location_comes_back_in_all_four_quarters_of_the_world() {
        for (latitude, longitude) in [
            (-33.87, 151.21),
            (51.5, -0.12),
            (-54.8, -68.3),
            (90.0, 180.0),
            (-90.0, -180.0),
            (0.0, 0.0),
        ] {
            let (lat, lon) = message_at(latitude, longitude)
                .latitude_longitude()
                .unwrap();
            assert!((lat - latitude).abs() < 1e-6, "{lat} {latitude}");
            assert!((lon - longitude).abs() < 3e-6, "{lon} {longitude}");
        }
    }

    #[test]
    fn no_location_field_gives_none() {
        let message = TimeMessage {
            unix_time: 0,
            timezone_minutes: 0,
            location: None,
            deployment_id: None,
        };
        assert_eq!(message.latitude_longitude(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;

    /// The byte-at-a-time CRC in AudioMoth's `audioconfig.c`, which the JavaScript
    /// encoder's bit-serial form must agree with.
    fn firmware_crc(bytes: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &byte in bytes {
            crc = crc.rotate_left(8);
            crc ^= byte as u16;
            crc ^= (crc & 0xFF) >> 4;
            crc ^= crc << 12;
            crc ^= (crc & 0xFF) << 5;
        }
        crc
    }

    /// `hammingConversion` from AudioMoth's `audioconfig.c`.
    const FIRMWARE_HAMMING_TABLE: [u8; 128] = [
        0, 0, 0, 1, 0, 1, 1, 1, //
        0, 2, 4, 8, 9, 5, 3, 1, //
        0, 2, 10, 6, 7, 11, 3, 1, //
        2, 2, 3, 2, 3, 2, 3, 3, //
        0, 12, 4, 6, 7, 5, 13, 1, //
        4, 5, 4, 4, 5, 5, 4, 5, //
        7, 6, 6, 6, 7, 7, 7, 6, //
        14, 2, 4, 6, 7, 5, 3, 15, //
        0, 12, 10, 8, 9, 11, 13, 1, //
        9, 8, 8, 8, 9, 9, 9, 8, //
        10, 11, 10, 10, 11, 11, 10, 11, //
        14, 2, 10, 8, 9, 11, 3, 15, //
        12, 12, 13, 12, 13, 12, 13, 13, //
        14, 12, 4, 8, 9, 5, 13, 15, //
        14, 12, 10, 6, 7, 11, 13, 15, //
        14, 14, 14, 15, 14, 15, 15, 15,
    ];

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn crc_of_nothing_is_zero() {
        assert_eq!(crc16(&[]), 0);
    }

    #[test]
    fn crc_matches_the_firmware_form() {
        let samples: [&[u8]; 4] = [
            b"123456789",
            &[0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02],
            &[0xFF; 21],
            &[0x00, 0x80, 0x7F, 0x01, 0xFE],
        ];
        for sample in samples {
            assert_eq!(crc16(sample), firmware_crc(sample));
        }
    }

    #[test]
    fn all_codewords_decode_to_their_nibble() {
        for (nibble, &codeword) in HAMMING_CODEWORDS.iter().enumerate() {
            assert_eq!(hamming_decode(codeword), (nibble as u8, false));
        }
    }

    #[test]
    fn every_single_bit_flip_is_corrected() {
        for (nibble, &codeword) in HAMMING_CODEWORDS.iter().enumerate() {
            for bit in 0..7 {
                let (decoded, corrected) = hamming_decode(codeword ^ (1 << bit));
                assert_eq!(decoded, nibble as u8, "codeword {codeword}, bit {bit}");
                assert!(corrected, "codeword {codeword}, bit {bit}");
            }
        }
    }

    #[test]
    fn hamming_decode_matches_the_firmware_table() {
        for word in 0..128u8 {
            assert_eq!(
                hamming_decode(word).0,
                FIRMWARE_HAMMING_TABLE[word as usize],
                "word {word}"
            );
        }
    }

    #[test]
    fn hamming_decode_ignores_bit_7() {
        assert_eq!(hamming_decode(0x80 | 25), (2, false));
    }

    #[test]
    fn parse_time_only() {
        let payload = [0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02];
        let message = TimeMessage::parse(&payload).unwrap();
        assert_eq!(
            message,
            TimeMessage {
                unix_time: 1_793_000_000,
                timezone_minutes: 660,
                location: None,
                deployment_id: None,
            }
        );
    }

    #[test]
    fn parse_all_optional_field_layouts() {
        let mut payload = [0u8; 21];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte = i as u8 + 100;
        }
        // A time zone of 0, because 104 and 105 are out of range.
        payload[4] = 0;
        payload[5] = 0;

        let with_location = TimeMessage::parse(&payload[..13]).unwrap();
        assert_eq!(
            with_location.location,
            Some([106, 107, 108, 109, 110, 111, 112])
        );
        assert_eq!(with_location.deployment_id, None);

        let with_deployment = TimeMessage::parse(&payload[..14]).unwrap();
        assert_eq!(with_deployment.location, None);
        assert_eq!(
            with_deployment.deployment_id,
            Some([106, 107, 108, 109, 110, 111, 112, 113])
        );

        let with_both = TimeMessage::parse(&payload).unwrap();
        assert_eq!(
            with_both.location,
            Some([106, 107, 108, 109, 110, 111, 112])
        );
        assert_eq!(
            with_both.deployment_id,
            Some([113, 114, 115, 116, 117, 118, 119, 120])
        );
    }

    #[test]
    fn parse_rejects_other_lengths() {
        let payload = [0u8; 40];
        for length in 0..=40 {
            let expected_valid = matches!(length, 6 | 13 | 14 | 21);
            assert_eq!(
                TimeMessage::parse(&payload[..length]).is_some(),
                expected_valid,
                "{length}"
            );
        }
    }

    #[test]
    fn parse_rejects_time_zones_the_encoder_never_sends() {
        let cases: [(i16, bool); 8] = [
            (-30000, false),
            (-721, false),
            (-720, true),
            (0, true),
            (840, true),
            (841, false),
            (30000, false),
            (i16::MIN, false),
        ];
        for (minutes, expected_valid) in cases {
            let bytes = minutes.to_le_bytes();
            let payload = [1, 2, 3, 4, bytes[0], bytes[1]];
            assert_eq!(
                TimeMessage::parse(&payload).is_some(),
                expected_valid,
                "{minutes}"
            );
        }
    }

    #[test]
    fn time_zones_are_twos_complement() {
        // The two time zone bytes are written out by hand so that this test does
        // not depend on i16::to_le_bytes.
        let cases: [(i16, [u8; 2]); 5] = [
            (-300, [0xD4, 0xFE]),
            (660, [0x94, 0x02]),
            (345, [0x59, 0x01]),
            (-720, [0x30, 0xFD]),
            (840, [0x48, 0x03]),
        ];
        for (minutes, bytes) in cases {
            let payload = [0, 0, 0, 0, bytes[0], bytes[1]];
            assert_eq!(
                TimeMessage::parse(&payload).unwrap().timezone_minutes,
                minutes
            );
        }
    }

    /// Year, month, day, hour, minute and second.
    type DateAndTime = (i32, u8, u8, u8, u8, u8);

    /// Unix time, time zone in minutes and the expected local time. The expected
    /// values are from Python:
    /// `datetime(1970, 1, 1) + timedelta(seconds=unix + tz * 60)`.
    const LOCAL_TIME_CASES: [(u32, i16, DateAndTime); 21] = [
        (0, 0, (1970, 1, 1, 0, 0, 0)),
        (0, -300, (1969, 12, 31, 19, 0, 0)),
        (1793000000, 660, (2026, 10, 26, 18, 33, 20)),
        (1582934400, 0, (2020, 2, 29, 0, 0, 0)),
        (1582934399, 0, (2020, 2, 28, 23, 59, 59)),
        (1582934400, -1, (2020, 2, 28, 23, 59, 0)),
        (1709164800, 0, (2024, 2, 29, 0, 0, 0)),
        (1709251199, 0, (2024, 2, 29, 23, 59, 59)),
        (1709251200, 0, (2024, 3, 1, 0, 0, 0)),
        (1735689599, 0, (2024, 12, 31, 23, 59, 59)),
        (1735689600, -300, (2024, 12, 31, 19, 0, 0)),
        (1735603200, 840, (2024, 12, 31, 14, 0, 0)),
        (1704067199, 660, (2024, 1, 1, 10, 59, 59)),
        (951782400, 0, (2000, 2, 29, 0, 0, 0)),
        (951868799, 0, (2000, 2, 29, 23, 59, 59)),
        (4107542400, 0, (2100, 3, 1, 0, 0, 0)),
        (4294967295, 840, (2106, 2, 7, 20, 28, 15)),
        (4102444799, 60, (2100, 1, 1, 0, 59, 59)),
        (1677628799, 345, (2023, 3, 1, 5, 44, 59)),
        (1646092800, -720, (2022, 2, 28, 12, 0, 0)),
        (978307199, 0, (2000, 12, 31, 23, 59, 59)),
    ];

    #[test]
    fn local_time_matches_python_datetime() {
        for (unix_time, timezone_minutes, expected) in LOCAL_TIME_CASES {
            let message = TimeMessage {
                unix_time,
                timezone_minutes,
                location: None,
                deployment_id: None,
            };
            let local = message.local();
            let (year, month, day, hour, minute, second) = expected;
            assert_eq!(
                local,
                LocalTime {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    second
                },
                "unix {unix_time}, time zone {timezone_minutes}"
            );
        }
    }

    #[test]
    fn epoch_seconds_round_trip() {
        for (unix_time, timezone_minutes, _) in LOCAL_TIME_CASES {
            let seconds = unix_time as i64 + timezone_minutes as i64 * 60;
            assert_eq!(
                LocalTime::from_epoch_seconds(seconds).to_epoch_seconds(),
                seconds
            );
        }
    }

    #[test]
    fn local_time_display() {
        let time = LocalTime {
            year: 2026,
            month: 3,
            day: 5,
            hour: 7,
            minute: 8,
            second: 9,
        };
        assert_eq!(format!("{time}"), "2026-03-05 07:08:09");
    }

    #[test]
    fn remote_commands_round_trip() {
        for command in RemoteCommand::ALL {
            assert_eq!(
                RemoteCommand::from_code(command.code(), command.argument()),
                Some(command)
            );
            assert_eq!(
                RemoteCommand::from_deployment_id(&command.deployment_id()),
                Some(command)
            );
        }
        assert_eq!(
            &RemoteCommand::Stop.deployment_id(),
            b"BATX\x01\x02\x00\x00"
        );
        assert_eq!(
            &RemoteCommand::SetClock.deployment_id(),
            b"BATX\x01\x03\x00\x00"
        );
        assert_eq!(
            &RemoteCommand::Listen(ListeningMode::Thru).deployment_id(),
            b"BATX\x01\x04\x06\x00"
        );
        assert_eq!(RemoteCommand::from_deployment_id(b"AUDIOMTH"), None);
    }

    #[test]
    fn unknown_remote_messages_are_read_but_not_known() {
        for id in [
            b"BATX\x01\x05\x00\x00",
            b"BATX\x02\x01\x00\x00",
            b"BATX\x01\x01\x05\x00",
            b"BATX\x01\x03\x01\x00",
            b"BATX\x01\x04\x00\x00",
            b"BATX\x01\x04\x07\x00",
            b"BATX\x01\x00\x00\x00",
        ] {
            let message = RemoteMessage::from_deployment_id(id).unwrap();
            assert_eq!(message.deployment_id(), *id);
            assert_eq!(message.known(), None, "{id:?}");
        }
        assert_eq!(
            RemoteMessage::from_deployment_id(b"BATX\x01\x01\x34\x12")
                .unwrap()
                .argument,
            0x1234
        );
        assert_eq!(
            RemoteMessage::from_deployment_id(b"BATY\x01\x01\x00\x00"),
            None
        );
    }

    /// SipHash-2-4 of a message of any length, with the key as 16 bytes.
    fn siphash_bytes(key: &[u8; 16], data: &[u8]) -> u64 {
        let word = |bytes: &[u8]| {
            let mut word = [0u8; 8];
            word[..bytes.len()].copy_from_slice(bytes);
            u64::from_le_bytes(word)
        };
        let whole = data.chunks_exact(8);
        let last = word(whole.remainder()) | (data.len() as u64) << 56;
        let words: std::vec::Vec<u64> = whole.map(word).chain([last]).collect();
        siphash_2_4([word(&key[..8]), word(&key[8..])], &words)
    }

    /// The vector in the SipHash paper's appendix, and each shorter message against the
    /// `siphasher` crate.
    #[test]
    fn siphash_matches_the_reference() {
        use core::hash::Hasher;

        let key: [u8; 16] = core::array::from_fn(|i| i as u8);
        let message: [u8; 15] = core::array::from_fn(|i| i as u8);
        assert_eq!(siphash_bytes(&key, &message), 0xa129_ca61_49be_45e5);
        let long: [u8; 40] = core::array::from_fn(|i| (i * 7) as u8);
        for len in 0..=long.len() {
            let mut hasher = siphasher::sip::SipHasher24::new_with_key(&key);
            hasher.write(&long[..len]);
            assert_eq!(
                siphash_bytes(&key, &long[..len]),
                hasher.finish(),
                "{len} bytes"
            );
        }
    }

    /// The whole hash of each vector that the phone page's tests also use.
    #[test]
    fn the_check_is_the_low_16_bits_of_the_hash() {
        let mut key = [0u8; 16];
        key[..6].copy_from_slice(b"493817");
        let mut message = std::vec::Vec::from(*b"BATX\x02\x10");
        message.extend_from_slice(&1_794_700_800u32.to_le_bytes());
        message.extend_from_slice(&600i16.to_le_bytes());
        assert_eq!(siphash_bytes(&key, &message), 0xc2a8_f456_2886_f3ba);
        message[5] = 0x46;
        message[6..10].copy_from_slice(&1_794_700_801u32.to_le_bytes());
        message[10..].copy_from_slice(&(-300i16).to_le_bytes());
        assert_eq!(siphash_bytes(&key, &message), 0x3f47_b561_45d0_adb0);
        let mut zero_key = [0u8; 16];
        zero_key[..6].copy_from_slice(b"000000");
        assert_eq!(
            siphash_bytes(&zero_key, b"BATX\x02\x30\0\0\0\0\0\0"),
            0x82c3_7daf_f2bd_0a6b
        );
    }

    /// The vectors that the phone page's tests also use.
    #[test]
    fn coded_remote_commands_match_the_test_vectors() {
        let vectors: [(&[u8; 6], u32, i16, RemoteCommand, [u8; 8]); 3] = [
            (
                b"493817",
                1_794_700_800,
                600,
                RemoteCommand::Start,
                [0x42, 0x41, 0x54, 0x58, 0x02, 0x10, 0xba, 0xf3],
            ),
            (
                b"493817",
                1_794_700_801,
                -300,
                RemoteCommand::Listen(ListeningMode::Thru),
                [0x42, 0x41, 0x54, 0x58, 0x02, 0x46, 0xb0, 0xad],
            ),
            (
                b"000000",
                0,
                0,
                RemoteCommand::SetClock,
                [0x42, 0x41, 0x54, 0x58, 0x02, 0x30, 0x6b, 0x0a],
            ),
        ];
        for (code, unix_time, timezone_minutes, command, id) in vectors {
            assert_eq!(
                command.coded_deployment_id(code, unix_time, timezone_minutes),
                id
            );
            let message = RemoteMessage::from_deployment_id(&id).unwrap();
            assert_eq!(message.deployment_id(), id);
            assert_eq!(message.coded(), Some(command));
            assert_eq!(message.command(), Some(command));
            assert_eq!(message.known(), None);
            assert_eq!(message.check(), Some([id[6], id[7]]));
            assert!(message.check_matches(code, unix_time, timezone_minutes));
            // Another code, time or UTC offset gives another check.
            assert!(!message.check_matches(b"493818", unix_time, timezone_minutes));
            assert!(!message.check_matches(code, unix_time + 1, timezone_minutes));
            assert!(!message.check_matches(code, unix_time, timezone_minutes + 60));
        }
    }

    #[test]
    fn every_command_round_trips_in_version_2() {
        for command in RemoteCommand::ALL {
            let id = command.coded_deployment_id(b"012345", 1_794_700_800, 0);
            let message = RemoteMessage::from_deployment_id(&id).unwrap();
            assert_eq!(message.coded(), Some(command));
            assert!(message.check_matches(b"012345", 1_794_700_800, 0));
            // Another command with the same check is refused.
            let mut other = id;
            other[5] ^= 0x30;
            let other = RemoteMessage::from_deployment_id(&other).unwrap();
            assert!(!other.check_matches(b"012345", 1_794_700_800, 0));
        }
    }

    #[test]
    fn a_coded_chime_is_checked_against_its_own_time() {
        let id = RemoteCommand::Stop.coded_deployment_id(b"493817", 1_794_700_800, 600);
        let mut payload = std::vec::Vec::new();
        payload.extend_from_slice(&1_794_700_800u32.to_le_bytes());
        payload.extend_from_slice(&600i16.to_le_bytes());
        payload.extend_from_slice(&id);
        let message = TimeMessage::parse(&payload).unwrap();
        assert!(message.remote_check_matches(b"493817"));
        assert!(!message.remote_check_matches(b"493816"));
        // The same deployment ID in a chime with a later time.
        payload[0] += 1;
        let later = TimeMessage::parse(&payload).unwrap();
        assert!(!later.remote_check_matches(b"493817"));
        // Version 1 has no check.
        assert_eq!(later.remote_command(), None);
        let version_1 = RemoteMessage::from_deployment_id(&RemoteCommand::Stop.deployment_id());
        assert_eq!(version_1.unwrap().check(), None);
        assert!(!version_1.unwrap().check_matches(b"493817", 0, 0));
    }

    #[test]
    fn unknown_version_2_messages_are_not_known() {
        // Command 0, command 5, Start with an argument, and listening modes 0 and 7.
        for byte in [0x00, 0x50, 0x11, 0x40, 0x47] {
            let id = [0x42, 0x41, 0x54, 0x58, 0x02, byte, 0, 0];
            let message = RemoteMessage::from_deployment_id(&id).unwrap();
            assert_eq!(message.command(), None, "{byte:#04x}");
        }
        // A version this crate does not know.
        let message = RemoteMessage::from_deployment_id(b"BATX\x03\x10\x00\x00").unwrap();
        assert_eq!(message.command(), None);
        assert_eq!(message.check(), None);
    }
}
