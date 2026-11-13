//! Whether a recording is heterodyne audio, judged from the text that comes
//! with it: Xeno-canto's remarks, device, microphone and method fields, or
//! another source's description.
//!
//! A heterodyne bat detector mixes the call with a tone it is tuned to and
//! plays the difference, roughly 0 to 15 kHz. A recording of that output does
//! not show the bat's real frequencies. Unlike time expansion, the shift is
//! not a fixed factor, so the file alone does not give the real frequencies.
//!
//! [`infer_heterodyne_hint`] returns one of three levels, or `None`:
//!
//! | Level | Rules | Text example |
//! |---|---|---|
//! | [`SaysHeterodyne`](HeterodyneLevel::SaysHeterodyne) | R1 heterodyne word, R2 tuning phrase | "handheld hetrodyne detector set at 31,9 kHz" |
//! | [`HeterodyneOnlyDetector`](HeterodyneLevel::HeterodyneOnlyDetector) | R3 heterodyne-only detector named | "Magenta Bat5", "Pettersson D200" |
//! | [`MayBeHeterodyne`](HeterodyneLevel::MayBeHeterodyne) | R4 dual-mode detector, R5 detector speaker recorded | "Pettersson D980", "detectors speaker with mobilphone" |
//!
//! Then two steps can lower the level:
//! - C2: time-expansion, full-spectrum, frequency-division or zero-crossing
//!   wording ("other-mode wording") lowers it.
//! - C3: the file's header sample rate can remove the hint.
//!
//! Matching uses no regular expressions. Each field is split into clauses,
//! each clause into lowercase tokens, and the rules test tokens, token
//! prefixes and token sequences.
//!
//! The rules are ported from the Python reference implementation used to
//! calibrate them (`hetrules.py`, November 2026, not in this repository).
//! The port was checked against it on 11,502 Xeno-canto bat recordings.
//! On that set, with the sample-rate gate, 4 recordings get level A, 12 get
//! level B and 18 get level C, all at 48 kHz or less. All 16 recordings whose
//! text says the file is heterodyne output get level A or B. Heterodyne
//! uploads are rare, so the word lists for languages other than English, the
//! negation words and the listening phrases have no real hits behind them.

use std::collections::HashSet;

/// Free-text fields that describe a recording. An absent or empty field
/// counts as no text.
///
/// Callers must pass text in Unicode normalization form C (NFC). The word
/// lists are written with precomposed letters, so a decomposed "é" (an "e"
/// followed by a combining accent) does not match: "hétérodyne" in NFD
/// misses the heterodyne word list. In the WASM frontend, normalize each
/// field with JavaScript's `String.prototype.normalize("NFC")` first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordingText<'a> {
    /// Xeno-canto `rmk`.
    pub remarks: Option<&'a str>,
    /// Xeno-canto `dvc`.
    pub device: Option<&'a str>,
    /// Xeno-canto `mic`.
    pub microphone: Option<&'a str>,
    /// Xeno-canto `method`. It is a fixed pick-list on Xeno-canto, but it is
    /// matched like the other fields.
    pub method: Option<&'a str>,
    /// A free-text description from another source, such as GBIF,
    /// iNaturalist or Wikimedia Commons.
    pub description: Option<&'a str>,
}

impl<'a> RecordingText<'a> {
    /// The text of one field.
    pub fn get(&self, field: TextField) -> Option<&'a str> {
        match field {
            TextField::Remarks => self.remarks,
            TextField::Device => self.device,
            TextField::Microphone => self.microphone,
            TextField::Method => self.method,
            TextField::Description => self.description,
        }
    }
}

/// One of the fields of [`RecordingText`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextField {
    Remarks,
    Device,
    Microphone,
    Method,
    Description,
}

impl TextField {
    /// All fields, in the order the rules read them. When two fields give
    /// evidence at the same level, the earlier field's evidence is quoted.
    pub const ALL: [TextField; 5] = [
        TextField::Remarks,
        TextField::Device,
        TextField::Microphone,
        TextField::Method,
        TextField::Description,
    ];

    /// The device and microphone fields name equipment, so short model names
    /// ("Duet", "Griffin") and phones count only there.
    fn names_equipment(self) -> bool {
        matches!(self, TextField::Device | TextField::Microphone)
    }
}

/// How sure the text is that the audio is heterodyne output. Ordered from
/// weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeterodyneLevel {
    /// Level C. The text names a dual-mode detector, or says a detector's
    /// speaker or headphone output was recorded, without saying which mode
    /// was used. The audio may be heterodyne or time-expanded. Either way its
    /// frequencies may not be the bat's real frequencies. Only given when the
    /// sample rate is known and is 48 kHz or less.
    MayBeHeterodyne,
    /// Level B. The text names a detector that has only a heterodyne mode,
    /// such as a Magenta Bat5 or a Pettersson D200.
    HeterodyneOnlyDetector,
    /// Level A. The text says the audio is heterodyne: a word for heterodyne,
    /// or a detector tuned to a frequency in kHz.
    SaysHeterodyne,
}

/// The rule that produced the quoted evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeterodyneSource {
    /// R1: a word for heterodyne, in any of several languages, or "HET".
    HeterodyneWord,
    /// R2: a detector "tuned to" or "set at" a frequency between 10 and
    /// 150 kHz.
    TuningPhrase,
    /// R3: a detector with only a heterodyne mode.
    HeterodyneOnlyDetector,
    /// R4: a detector with a heterodyne mode and another mode.
    DualModeDetector,
    /// R5: a detector's speaker, headphone or output jack was recorded.
    DetectorOutput,
}

/// Why the text suggests heterodyne audio, with a quote to show the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeterodyneHint {
    pub level: HeterodyneLevel,
    pub source: HeterodyneSource,
    /// The field the quote is from.
    pub field: TextField,
    /// The clause around the match, in its original case, at most
    /// [`EXCERPT_MAX_CHARS`] characters plus the `…` marks. The text is copied
    /// from the field unchanged, except that `…` marks text cut from either
    /// end. Cuts fall between words where there is a space nearby.
    pub excerpt: String,
    /// The matched words exactly as they appear in the field, such as
    /// "hetrodyne" or "Magenta Bat5". Always a substring of `excerpt`.
    ///
    /// Widening a match to whole words adds at most 30 characters on each
    /// side, so a match inside a long run of letters and digits ends inside
    /// that run. A match longer than [`EXCERPT_MAX_CHARS`] is cut to that
    /// length around the matched tokens. When the tokens themselves are
    /// longer than that, the cut is around the heterodyne word in them.
    pub matched: String,
}

/// Longest excerpt, in characters, not counting the `…` marks. A longer match
/// is cut to this length, so no excerpt is longer.
pub const EXCERPT_MAX_CHARS: usize = 80;

/// Decides from a recording's text whether it is heterodyne audio.
///
/// `sample_rate` is the sample rate in the file header. It is part of the
/// decision:
/// - Above 96 kHz: no hint at any level. Such a file is full spectrum or
///   relabelled time expansion, and heterodyne wording in its text describes
///   listening or a nearby detector.
/// - 19 200, 25 000, 25 600, 30 000, 31 250, 38 400 or 50 000 Hz: only level A.
///   These rates are full-spectrum or time-expanded files saved with a header
///   rate one tenth of the real rate.
/// - Level C needs a known rate of 48 kHz or less.
/// - Levels A and B are given at 96 kHz or less, or when the rate is unknown.
///
/// The caller must not show the hint when the app already applies a
/// time-expansion factor other than 1 to the file. The user or the file's
/// metadata has then said the file is time-expanded, and that factor
/// already accounts for the frequencies.
pub fn infer_heterodyne_hint(
    text: &RecordingText,
    sample_rate: Option<u32>,
) -> Option<HeterodyneHint> {
    let analysis = analyze(text, sample_rate);
    let level = analysis.level?;
    let evidence = analysis.quoted_evidence()?;
    let field_text = text.get(evidence.field).unwrap_or("");
    let clause = analysis.fields[field_index(evidence.field)].clauses[evidence.clause].span;
    let matched = quoted_match(field_text, evidence);
    Some(HeterodyneHint {
        level,
        source: evidence.source,
        field: evidence.field,
        excerpt: excerpt(field_text, clause, matched),
        matched: field_text[matched.0..matched.1].to_string(),
    })
}

// ---------------------------------------------------------------- word lists
//
// The lists match the Python reference exactly. Sequences are written as
// space-separated tokens. An element ending in `*` matches any token that
// starts with the text before the `*`.

/// R1: substrings of a token that mean heterodyne. A token that contains one
/// counts, so "autoheterodyne" and "heterodyndetektor" match.
const HET_STEMS: &[&str] = &[
    // en heterodyne(d), nl, de Heterodyn, sv/da/no heterodyn(detektor),
    // pl heterodyna, cs heterodynní
    "heterodyn",
    // misspellings, es/pt heterodino/a, en heterodine
    "hetrodyn",
    "heterdyn",
    "hetorodyn",
    "heterodin",
    "hetrodin",
    // fr
    "hétérodyn",
    "hétérodin",
    "hètèrodyn",
    // pt heteródino
    "heteródin",
    // it eterodina
    "eterodin",
    // fi heterodyyni
    "heterodyy",
    // et
    "heterodüün",
    // ca heterodí
    "heterodí",
    // Magenta's own word, de Superhet
    "superhet",
    // ru/uk/kk, bg
    "гетеродин",
    "хетеродин",
    // de. Plain "Überlagerung" means overlap, so it is not listed.
    "überlagerungsempf",
    "überlagerungsdet",
    "überlagerungsverf",
    // de Mischerverfahren. Plain "Mischer" is a mixing desk.
    "mischerverf",
    "mischerprinz",
];

/// R1: heterodyne in scripts written without spaces (ja, ko, zh). These are
/// substring tests on the lowercased clause.
const HET_SUBSTRINGS: &[&str] = &[
    "ヘテロダイン",
    "헤테로다인",
    "外差式",
    "外差检测",
    "外差偵測",
    "外差探测",
];

/// R1: lowercase "het" is Dutch for "the". It counts only before one of these
/// tokens ("het detector")...
const HET_NEXT: &[&str] = &[
    "detector",
    "detectors",
    "detektor",
    "detektorn",
    "détecteur",
    "detecteur",
    "mode",
    "modus",
    "output",
    "channel",
    "setting",
    "tuned",
    "tuning",
    "freq",
    "frequency",
    "khz",
    "fd",
    "listening",
];
/// ...after one of these ("auto-het", "FD/HET", "TE/het"), or before a number
/// and "khz" or "k".
const HET_PREV: &[&str] = &["auto", "fd", "te"];

/// A negation in the 3 tokens before a heterodyne word, model name or
/// other-mode phrase blocks it.
const NEG_BEFORE: &[&str] = &[
    "not", "non", "without", "never", "nicht", "kein", "keine", "ohne", "niet", "geen", "zonder",
    "sans", "sin", "senza", "inte", "ikke", "ej", "nie", "bez", "ni", "nor",
];
/// These negate only directly before the word ("no heterodyne"), because
/// Portuguese "no" means "in the".
const NEG_BEFORE_ADJACENT: &[&str] = &["no", "pas"];

/// R1 is blocked by a bleed or listening cue within this many tokens either
/// side of the heterodyne word.
const BLEED_WINDOW: usize = 6;
/// Bleed cues: a nearby detector's sound is in the file, or a filter removed
/// it.
const BLEED_TOKENS: &[&str] = &[
    "nearby",
    "another",
    "background",
    "avoid",
    "avoided",
    "leak",
    "leaking",
    "leaked",
    "bleed",
    "bleeding",
    "interference",
    "crosstalk",
    "highpass",
    "hpf",
];
/// Bleed cues matched as token prefixes (filter, gefiltert, filtrado).
const BLEED_PREFIX: &[&str] = &["filter", "gefilter", "filtr"];
/// Bleed cues of more than one token: a filter, the low end of the spectrum,
/// or someone else's detector.
const BLEED_SEQS: &[&str] = &[
    "high pass",
    "low frequenc*",
    "lower frequenc*",
    "very low",
    "other detector*",
    "second detector*",
    "from a colleague*",
    "colleague*",
    "someone else*",
];
/// Listening cues: the detector was used to find the bat, not to record it.
/// These also block R3 in the same clause, and R5.
const LISTEN_SEQS: &[&str] = &[
    "used to find",
    "used to locate",
    "used to detect",
    "used to spot",
    "used for find*",
    "used for listen*",
    "to find",
    "to locate",
    "to spot",
    "for finding",
    "for locating",
    "found with",
    "located with",
    "found using",
    "located using",
    "only for",
    "listened with",
    "listening with",
    "listen with",
    "monitor*",
    "for listening",
    "zum suchen",
    "zum finden",
    "pour trouver",
    "pour localiser",
    "para localizar",
    "om te vinden",
    "om te zoeken",
    "för att hitta",
];

/// C2: wording that says the file is in another mode. It counts anywhere in
/// the record, unless a negation comes in the 3 tokens before it.
const OTHER_MODE_SEQS: &[&str] = &[
    // time expansion
    "time expan*",
    "timeexpan*",
    "time exp",
    "tidsexpan*",
    "tidsekspan*",
    "zeitdehn*",
    "zeit dehn*",
    "tijdexpan*",
    "tijdsexpan*",
    "tijd expan*",
    "expansion de temps",
    "expansion du temps",
    "expansión de tiempo",
    "expansion de tiempo",
    "expansión temporal",
    "espansione temporale",
    "expansão de tempo",
    "ekspansja czasu",
    // slowed down, or a slow-down factor
    "x 10",
    "10 x",
    "x 20",
    "20 x",
    "slowed",
    "slow down",
    "slowed down",
    "retarded",
    // full spectrum
    "full spectrum",
    "fullspectrum",
    "full spec",
    "vollspektrum",
    "volledig spectrum",
    "spectre complet",
    "espectro completo",
    // frequency division
    "frequency division",
    "frequency divided",
    "freq division",
    "frequentiedeling",
    "frequenzteil*",
    "frequenzteiler*",
    "division de fréquence",
    "división de frecuencia",
    "frekvensdel*",
    "divided by 10",
    // zero crossing
    "zero crossing",
    "zero crossings",
];

/// R2: verbs for tuning a detector.
const TUNE_VERBS: &[&str] = &[
    "tuned",
    "tuning",
    "set",
    "setting",
    "adjusted",
    "réglé",
    "réglée",
    "regle",
    "eingestellt",
    "abgestimmt",
    "ingesteld",
    "afgestemd",
    "sintonizado",
    "sintonizada",
    "sintonizzato",
    "inställd",
    "inställt",
    "nastaven",
    "nastavený",
];
/// R2 is blocked when one of these is near the verb: the kHz value is then a
/// filter, trigger or sample-rate setting.
const TUNE_BLOCK: &[&str] = &[
    "filter",
    "trigger",
    "sample",
    "sampling",
    "rate",
    "hpf",
    "lpf",
    "pass",
    "highpass",
    "lowpass",
    "gain",
    "threshold",
    "trig",
    "samplerate",
    "sr",
];
/// R2 and R5: a token starting with one of these is a detector word.
const DETECTOR_PREFIX: &[&str] = &[
    "detector",
    "detektor",
    "détecteur",
    "detecteur",
    "batdetector",
    "ultrasounddetector",
    "ultrasonicdetector",
    "fladdermusdetektor",
    "flagermusdetektor",
    "flaggermusdetektor",
    "batdetektor",
    "vleermuisdetector",
];
/// R5: a detector's sound output.
const SPEAKER_TOKENS: &[&str] = &[
    "speaker",
    "spaker",
    "speakers",
    "loudspeaker",
    "högtalare",
    "högtalaren",
    "luidspreker",
    "lautsprecher",
    "altavoz",
    "altoparlante",
    "headphone",
    "headphones",
    "earphone",
    "earphones",
    "output",
    "jack",
];
/// R5: a detector's speaker as two tokens, or a recorder held close to it.
const SPEAKER_SEQS: &[&str] = &["haut parleur", "held close", "held next"];
/// R5, device and microphone fields only: a phone recorded the detector.
const PHONE_TOKENS: &[&str] = &[
    "phone",
    "mobilphone",
    "iphone",
    "smartphone",
    "mobile",
    "telefon",
    "telefoon",
    "mobiltelefon",
];

/// A detector model: the sequences that name it, and a label for it. Only the
/// first sequence that matches in a clause counts, at its first position.
type Model = (&'static [&'static str], &'static str);

/// R3: detectors with only a heterodyne mode. Bare "bat box" (a roost box),
/// "bat 4", "bat 5", "baton" and "scout" are left out on purpose.
const HET_ONLY_MODELS: &[Model] = &[
    (&["magenta bat"], "Magenta Bat4/Bat5"),
    (&["magenta mk*"], "Magenta MkII kit"),
    (&["magenta 4", "magenta 5"], "Magenta Bat4/Bat5"),
    (
        &["magenta detector*", "magenta kit", "magenta bat detector*"],
        "Magenta",
    ),
    (&["ssf bat", "ssf 2", "ssf 3"], "SSF BAT2/BAT3"),
    (&["autobat"], "SSF BAT3 AutoBat"),
    (
        &[
            "batbox iii*",
            "batbox 3",
            "bat box iii*",
            "batbox iiid",
            "batbox 3 d",
        ],
        "Batbox III",
    ),
    (&["batscanner", "bat scanner"], "Elekon Batscanner"),
    (&["batz"], "Ciel BATz"),
    (&["cdb 103"], "Ciel CDB103"),
    (
        &[
            "pettersson d 100",
            "petterson d 100",
            "pettersson d 200",
            "petterson d 200",
            "pettersson ultrasound detector d 200",
            "pettersson ultrasound detector d 100",
        ],
        "Pettersson D100/D200",
    ),
];
/// R3: bare model numbers. Every position counts, but only when the record
/// also has "pettersson", "petterson" or a detector word, and neither of the
/// 2 tokens before is in [`BARE_BLOCK_PREV`]. Bare "d 100" also needs no
/// "pcm d 100" (Sony PCM-D100) anywhere in the record.
const BARE_HET_ONLY: &[Model] = &[
    (&["d 200"], "Pettersson D200"),
    (&["d 100"], "Pettersson D100"),
];
/// Makers of recorders and cameras with D100 or D200 models.
const BARE_BLOCK_PREV: &[&str] = &["pcm", "sony", "nikon", "canon", "tascam", "zoom", "dr"];

/// R4: detectors with a heterodyne mode and another mode.
const DUAL_MODELS: &[Model] = &[
    (&["batbox duet", "bat box duet", "bb duet"], "Batbox Duet"),
    (&["batbox griffin", "bat box griffin"], "Batbox Griffin"),
    (
        &[
            "d 230", "d 240", "d 240 x", "240 x", "d 980", "d 1000", "d 1000 x",
        ],
        "Pettersson D230/D240x/D980/D1000X",
    ),
    (&["cdb 301"], "Ciel CDB301"),
    (
        &["micro trio", "microtrio", "micro duo", "microduo"],
        "Ciel micro trio/duo",
    ),
    (&["pipistrelle mini"], "Apodemus Pipistrelle Mini"),
];
/// R4, device and microphone fields only: short words that are detector
/// models there and ordinary words elsewhere.
const DUAL_DEVICE_ONLY: &[Model] = &[(&["duet"], "Batbox Duet"), (&["griffin"], "Batbox Griffin")];

/// Full-spectrum recorders. R5 is blocked in a clause that names one. A
/// heterodyne-only detector drops to level C when the device or microphone
/// field also names one, because the detector was then probably only used
/// for listening.
const FS_RECORDERS: &[&str] = &[
    "echo meter",
    "echometer",
    "em 3",
    "emt",
    "emt 2",
    "batlogger",
    "bat logger",
    "anabat",
    "walkabout",
    "audiomoth",
    "audio moth",
    "ultramic",
    "dodotronic",
    "m 500",
    "u 256",
    "u 384",
    "pippyg",
    "teensy",
    "teensybat",
    "batcorder",
    "sm 4 bat",
    "mini bat",
    "minibat",
    "d 500 x",
    "d 500",
    "peersonic",
    "avisoft",
    "ultrasoundgate",
    "wurb",
    "batango",
    "song meter mini",
    "elekon logger",
];

/// Header rates of full-spectrum or time-expanded files saved with a rate
/// one tenth of the real one.
const TE_HEADER_RATES: &[u32] = &[19_200, 25_000, 25_600, 30_000, 31_250, 38_400, 50_000];

// ---------------------------------------------------------------- tokenising

/// A clause of one field, with its tokens.
struct Clause {
    /// Byte range of the clause in the field text.
    span: (usize, usize),
    /// Lowercase tokens.
    words: Vec<String>,
    /// Byte range of each token in the field text.
    spans: Vec<(usize, usize)>,
}

struct FieldText<'a> {
    field: TextField,
    text: &'a str,
    clauses: Vec<Clause>,
}

impl<'a> FieldText<'a> {
    fn new(field: TextField, text: &'a str) -> Self {
        let clauses = clause_spans(text)
            .into_iter()
            .map(|span| {
                let (words, spans) = tokenize(text, span);
                Clause { span, words, spans }
            })
            .collect();
        FieldText {
            field,
            text,
            clauses,
        }
    }
}

fn field_index(field: TextField) -> usize {
    TextField::ALL.iter().position(|&f| f == field).unwrap_or(0)
}

/// Whitespace as the Python reference's `str.isspace` sees it: Unicode white
/// space plus the ASCII information separators U+001C to U+001F, which
/// Rust's `char::is_whitespace` does not count.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Splits text into clauses at line breaks, `;`, `!`, `?`, and at `.` when
/// whitespace (as [`is_space`] defines it) or the end of the text follows it.
/// "44.100 Hz" and "31,9 kHz" stay in one clause. Blank clauses are dropped,
/// so clause numbers count only clauses with text.
fn clause_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        let split = match ch {
            '\r' | '\n' | ';' | '!' | '?' => true,
            '.' => chars.peek().is_none_or(|&(_, next)| is_space(next)),
            _ => false,
        };
        if split {
            out.push((start, i));
            start = i + ch.len_utf8();
        }
    }
    out.push((start, text.len()));
    out.retain(|&(s, e)| !text[s..e].chars().all(is_space));
    out
}

/// Lowercase tokens of one clause, with their byte ranges in `text`.
///
/// Tokens are split at every character that is not alphanumeric, then again
/// where ASCII digits meet other characters: "Bat5" gives `bat 5`, "D240x"
/// gives `d 240 x` and "31,9" gives `31 9`. Each character is lowercased
/// before the tests, as the reference lowercases the whole text first.
///
/// Two differences from the Python reference are deliberate:
/// - Rust's `char::is_alphanumeric` also accepts characters that Python's
///   `str.isalnum` rejects: combining vowel signs (categories Mn and Mc,
///   such as the Devanagari and Thai vowel signs) and circled letters
///   (category So, such as "Ⓡ"). The reference splits Hindi "हेटेरोडाइन"
///   (heterodyne) into 5 tokens at its vowel signs, while this port keeps
///   it as one. Both split at a virama ("्") and at a Thai tone mark, which
///   neither test accepts. "MagentaⓇ" is one token here, so it does not
///   match "magenta".
/// - `char::to_lowercase` lowercases each character alone, so a word-final
///   "Σ" becomes "σ". Python's `str.lower` gives the final form "ς". No word
///   list has Greek words, so only the evidence text differs.
fn tokenize(text: &str, (start, end): (usize, usize)) -> (Vec<String>, Vec<(usize, usize)>) {
    let mut words = Vec::new();
    let mut spans = Vec::new();
    let mut cur = String::new();
    let mut cur_span = (0, 0);
    let mut prev_digit: Option<bool> = None;
    for (offset, ch) in text[start..end].char_indices() {
        let pos = start + offset;
        let ch_end = pos + ch.len_utf8();
        for lc in ch.to_lowercase() {
            if lc.is_alphanumeric() {
                let digit = lc.is_ascii_digit();
                if !cur.is_empty() && prev_digit.is_some_and(|p| p != digit) {
                    words.push(std::mem::take(&mut cur));
                    spans.push(cur_span);
                }
                if cur.is_empty() {
                    cur_span.0 = pos;
                }
                cur.push(lc);
                cur_span.1 = ch_end;
                prev_digit = Some(digit);
            } else {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    spans.push(cur_span);
                }
                prev_digit = None;
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
        spans.push(cur_span);
    }
    (words, spans)
}

/// A number token: ASCII digits only. A Unicode digit test would accept "²".
fn is_num(tok: &str) -> bool {
    !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit())
}

fn seq_len(seq: &str) -> usize {
    seq.split(' ').count()
}

/// Start positions where the space-separated sequence `seq` matches
/// consecutive tokens. An element ending in `*` is a prefix match.
fn find_seq<'t>(toks: &'t [String], seq: &'t str) -> impl Iterator<Item = usize> + 't {
    let n = seq_len(seq);
    (0..(toks.len() + 1).saturating_sub(n)).filter(move |&i| {
        seq.split(' ')
            .zip(&toks[i..i + n])
            .all(|(s, t)| match s.strip_suffix('*') {
                Some(prefix) => t.starts_with(prefix),
                None => t == s,
            })
    })
}

/// The first sequence in `seqs` that matches anywhere, and its first
/// position. List order wins over text order.
fn any_seq(toks: &[String], seqs: &[&'static str]) -> Option<(&'static str, usize)> {
    seqs.iter()
        .find_map(|&seq| find_seq(toks, seq).next().map(|i| (seq, i)))
}

fn in_list(list: &[&str], tok: &str) -> bool {
    list.contains(&tok)
}

/// A negation in the 3 tokens before position `i`, or "no"/"pas" right
/// before it.
fn is_negated(toks: &[String], i: usize) -> bool {
    toks[i.saturating_sub(3)..i]
        .iter()
        .any(|t| in_list(NEG_BEFORE, t))
        || (i >= 1 && in_list(NEG_BEFORE_ADJACENT, &toks[i - 1]))
}

fn bleed_or_listen(toks: &[String]) -> bool {
    toks.iter()
        .any(|t| in_list(BLEED_TOKENS, t) || BLEED_PREFIX.iter().any(|p| t.starts_with(p)))
        || any_seq(toks, BLEED_SEQS).is_some()
        || any_seq(toks, LISTEN_SEQS).is_some()
}

fn has_detector_word(toks: &[String]) -> bool {
    toks.iter()
        .any(|t| DETECTOR_PREFIX.iter().any(|p| t.starts_with(p)))
}

// ---------------------------------------------------------------- matching

struct Evidence {
    level: HeterodyneLevel,
    source: HeterodyneSource,
    field: TextField,
    /// Index into the field's clauses.
    clause: usize,
    /// Byte range of the matched tokens in the field text. Only the quoted
    /// evidence is widened to whole words, in [`quoted_match`].
    span: (usize, usize),
    /// Whether the quote widens `span` to whole words. False for an
    /// uppercase "HET", which is a whole word already, and for a match in a
    /// script written without spaces, where the whole word would be the
    /// whole run of text.
    widen: bool,
    /// What matched, written the way the Python reference writes it: the
    /// lowercase token or tokens, or "label [sequence]" for a model. The
    /// cross-check against the reference compares this.
    #[cfg_attr(not(test), allow(dead_code))]
    key: String,
}

/// A non-negated other-mode phrase.
struct OtherMode {
    field: TextField,
    clause: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    phrase: &'static str,
}

struct Analysis<'a> {
    fields: Vec<FieldText<'a>>,
    evidence: Vec<Evidence>,
    other_mode: Vec<OtherMode>,
    /// The field and clause of each entry in `other_mode`, for lookups.
    other_mode_clauses: HashSet<(TextField, usize)>,
    /// The last full-spectrum recorder named in the device or microphone
    /// field.
    fs_named: Option<&'static str>,
    /// Highest level of any evidence.
    raw_level: Option<HeterodyneLevel>,
    /// After C2, the other-mode downgrades.
    text_level: Option<HeterodyneLevel>,
    /// After C3, the sample-rate gate.
    level: Option<HeterodyneLevel>,
}

impl Analysis<'_> {
    fn in_other_mode_clause(&self, e: &Evidence) -> bool {
        self.other_mode_clauses.contains(&(e.field, e.clause))
    }

    /// The evidence to quote: the first at the highest level, preferring a
    /// clause without other-mode wording.
    fn quoted_evidence(&self) -> Option<&Evidence> {
        let raw = self.raw_level?;
        let mut top = self.evidence.iter().filter(|e| e.level == raw);
        let first = top.clone().next()?;
        Some(top.find(|e| !self.in_other_mode_clause(e)).unwrap_or(first))
    }
}

/// An R1 match. `index` is the token position, or `None` for an uppercase
/// "HET" or a CJK match, which have no negation check, search the whole
/// clause for bleed cues, and are quoted without widening.
struct HetWord {
    index: Option<usize>,
    key: String,
    /// Byte range of the matched tokens or text in the field text.
    span: (usize, usize),
}

fn het_words(text: &str, clause: &Clause) -> Vec<HetWord> {
    let toks = &clause.words;
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if HET_STEMS.iter().any(|s| t.contains(s)) {
            out.push(HetWord {
                index: Some(i),
                key: t.clone(),
                span: token_span(clause, i, i),
            });
        } else if t == "het" {
            let next = toks.get(i + 1).map_or("", String::as_str);
            let next2 = toks.get(i + 2).map_or("", String::as_str);
            let prev = if i > 0 { toks[i - 1].as_str() } else { "" };
            let next_word = in_list(HET_NEXT, next);
            let prev_word = in_list(HET_PREV, prev);
            let khz = is_num(next) && (next2 == "khz" || next2 == "k");
            if next_word || prev_word || khz {
                let first = if prev_word { i - 1 } else { i };
                let last = if khz {
                    i + 2
                } else if next_word {
                    i + 1
                } else {
                    i
                };
                out.push(HetWord {
                    index: Some(i),
                    key: format!("het {next}"),
                    span: token_span(clause, first, last),
                });
            }
        }
    }
    // Uppercase "HET", but not in an all-caps clause, where it may be Dutch
    // "HET" (the).
    let ctext = &text[clause.span.0..clause.span.1];
    if let Some(at) = upper_het(ctext) {
        if ctext.chars().any(char::is_lowercase) {
            let start = clause.span.0 + at;
            out.push(HetWord {
                index: None,
                key: "HET".to_string(),
                span: (start, start + 3),
            });
        }
    }
    let lower = ctext.to_lowercase();
    for &s in HET_SUBSTRINGS {
        if lower.contains(s) {
            // These scripts have no case, so the original has the same text.
            let span = ctext.find(s).map_or(clause.span, |at| {
                (clause.span.0 + at, clause.span.0 + at + s.len())
            });
            out.push(HetWord {
                index: None,
                key: s.to_string(),
                span,
            });
        }
    }
    out
}

/// Byte offset of the first original-case token that is exactly "HET".
/// Tokens here split only at characters that are not alphanumeric, using
/// the same Rust test as [`tokenize`], which accepts more characters than
/// the Python reference.
fn upper_het(clause: &str) -> Option<usize> {
    let mut start = None;
    for (i, ch) in clause.char_indices().chain([(clause.len(), ' ')]) {
        if ch.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            if &clause[s..i] == "HET" {
                return Some(s);
            }
        }
    }
    None
}

/// R2: a tuning verb, then within 5 tokens a number from 10 to 150, an
/// optional 1-2 digit decimal ("31,9"), and "khz" or "k" "hz". Returns the
/// positions of the verb and of the "khz" or "k" token.
fn tuning_phrase(toks: &[String]) -> Option<(usize, usize)> {
    let len = toks.len();
    for (i, t) in toks.iter().enumerate() {
        if !in_list(TUNE_VERBS, t) {
            continue;
        }
        let window = &toks[i.saturating_sub(4)..(i + 8).min(len)];
        if window.iter().any(|w| in_list(TUNE_BLOCK, w)) {
            continue;
        }
        for j in i + 1..(i + 6).min(len) {
            let in_range = is_num(&toks[j])
                && toks[j]
                    .parse::<u64>()
                    .is_ok_and(|n| (10..=150).contains(&n));
            if !in_range {
                continue;
            }
            let mut k = j + 1;
            if k < len && is_num(&toks[k]) && toks[k].len() <= 2 {
                k += 1;
            }
            if k < len
                && (toks[k] == "khz" || (toks[k] == "k" && k + 1 < len && toks[k + 1] == "hz"))
            {
                return Some((i, k));
            }
        }
    }
    None
}

/// A model match: its label, token position and the sequence that matched.
struct ModelHit {
    label: &'static str,
    index: usize,
    seq: &'static str,
}

fn model_hits(toks: &[String], models: &[Model]) -> Vec<ModelHit> {
    models
        .iter()
        .filter_map(|&(seqs, label)| {
            any_seq(toks, seqs).map(|(seq, index)| ModelHit { label, index, seq })
        })
        .collect()
}

fn bare_het_only(toks: &[String]) -> Vec<ModelHit> {
    let mut out = Vec::new();
    for &(seqs, label) in BARE_HET_ONLY {
        let seq = seqs[0];
        for index in find_seq(toks, seq) {
            let blocked = toks[index.saturating_sub(2)..index]
                .iter()
                .any(|p| in_list(BARE_BLOCK_PREV, p));
            if !blocked {
                out.push(ModelHit { label, index, seq });
            }
        }
    }
    out
}

fn other_mode_hits(fields: &[FieldText]) -> Vec<OtherMode> {
    let mut out = Vec::new();
    for f in fields {
        for (ci, clause) in f.clauses.iter().enumerate() {
            for &phrase in OTHER_MODE_SEQS {
                for i in find_seq(&clause.words, phrase) {
                    if !is_negated(&clause.words, i) {
                        out.push(OtherMode {
                            field: f.field,
                            clause: ci,
                            phrase,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Byte range of the tokens `first..=last` in the field text.
fn token_span(clause: &Clause, first: usize, last: usize) -> (usize, usize) {
    (clause.spans[first].0, clause.spans[last].1)
}

fn analyze<'a>(text: &RecordingText<'a>, sample_rate: Option<u32>) -> Analysis<'a> {
    use HeterodyneLevel::*;
    use HeterodyneSource as Src;

    let fields: Vec<FieldText<'a>> = TextField::ALL
        .iter()
        .map(|&f| FieldText::new(f, text.get(f).unwrap_or("")))
        .collect();
    let all_clauses = || fields.iter().flat_map(|f| &f.clauses);

    let other_mode = other_mode_hits(&fields);
    let detector_word_anywhere = all_clauses().any(|c| has_detector_word(&c.words));
    let bare_ok = detector_word_anywhere
        || all_clauses()
            .flat_map(|c| &c.words)
            .any(|t| t == "pettersson" || t == "petterson");
    let sony_pcm_d100 = all_clauses().any(|c| find_seq(&c.words, "pcm d 100").next().is_some());

    let mut evidence = Vec::new();
    for f in &fields {
        for (ci, clause) in f.clauses.iter().enumerate() {
            let toks = &clause.words;
            let mut push = |level, source, span, widen, key| {
                evidence.push(Evidence {
                    level,
                    source,
                    field: f.field,
                    clause: ci,
                    span,
                    widen,
                    key,
                })
            };

            // R1: a heterodyne word, unless negated or near a bleed or
            // listening cue.
            for hit in het_words(f.text, clause) {
                let negated = hit.index.is_some_and(|i| is_negated(toks, i));
                let window = match hit.index {
                    Some(i) => {
                        &toks
                            [i.saturating_sub(BLEED_WINDOW)..(i + BLEED_WINDOW + 1).min(toks.len())]
                    }
                    None => &toks[..],
                };
                if !negated && !bleed_or_listen(window) {
                    let widen = hit.index.is_some();
                    push(
                        SaysHeterodyne,
                        Src::HeterodyneWord,
                        hit.span,
                        widen,
                        hit.key,
                    );
                }
            }

            // R2: a tuning phrase, with a detector word anywhere in the record.
            if let Some((i, k)) = tuning_phrase(toks) {
                if detector_word_anywhere {
                    let end = if toks[k] == "k" { k + 1 } else { k };
                    push(
                        SaysHeterodyne,
                        Src::TuningPhrase,
                        token_span(clause, i, end),
                        true,
                        toks[i..=k].join(" "),
                    );
                }
            }

            // R3: a heterodyne-only detector, unless negated or with a
            // listening cue in the clause.
            let mut het_only = model_hits(toks, HET_ONLY_MODELS);
            if bare_ok {
                het_only.extend(
                    bare_het_only(toks)
                        .into_iter()
                        .filter(|m| !(sony_pcm_d100 && m.seq == "d 100")),
                );
            }
            // The listening cue is searched for once per clause, not once
            // per model match: a clause can name thousands of bare D200s.
            let listening = !het_only.is_empty() && any_seq(toks, LISTEN_SEQS).is_some();
            for m in het_only {
                if listening || is_negated(toks, m.index) {
                    continue;
                }
                push(
                    HeterodyneOnlyDetector,
                    Src::HeterodyneOnlyDetector,
                    token_span(clause, m.index, m.index + seq_len(m.seq) - 1),
                    true,
                    format!("{} [{}]", m.label, m.seq),
                );
            }

            // R4: a dual-mode detector, unless negated.
            let mut dual = model_hits(toks, DUAL_MODELS);
            if f.field.names_equipment() {
                dual.extend(model_hits(toks, DUAL_DEVICE_ONLY));
            }
            for m in dual {
                if is_negated(toks, m.index) {
                    continue;
                }
                push(
                    MayBeHeterodyne,
                    Src::DualModeDetector,
                    token_span(clause, m.index, m.index + seq_len(m.seq) - 1),
                    true,
                    format!("{} [{}]", m.label, m.seq),
                );
            }

            // R5: a detector's speaker or output recorded, in a clause with a
            // detector word, no bleed or listening cue and no full-spectrum
            // recorder.
            if has_detector_word(toks)
                && !bleed_or_listen(toks)
                && any_seq(toks, FS_RECORDERS).is_none()
            {
                let speaker = toks.iter().position(|t| in_list(SPEAKER_TOKENS, t));
                let held = any_seq(toks, SPEAKER_SEQS);
                let phone = toks.iter().position(|t| in_list(PHONE_TOKENS, t));
                let hit = match (speaker, held, phone) {
                    (Some(i), _, _) => Some((i, 1, toks[i].clone())),
                    (None, Some((seq, i)), _) => Some((i, seq_len(seq), seq.to_string())),
                    (None, None, Some(i)) if f.field.names_equipment() => {
                        Some((i, 1, toks[i].clone()))
                    }
                    _ => None,
                };
                if let Some((i, n, key)) = hit {
                    push(
                        MayBeHeterodyne,
                        Src::DetectorOutput,
                        token_span(clause, i, i + n - 1),
                        true,
                        key,
                    );
                }
            }
        }
    }

    let mut fs_named = None;
    for f in fields.iter().filter(|f| f.field.names_equipment()) {
        for clause in &f.clauses {
            if let Some((seq, _)) = any_seq(&clause.words, FS_RECORDERS) {
                fs_named = Some(seq);
            }
        }
    }

    let mut analysis = Analysis {
        raw_level: evidence.iter().map(|e| e.level).max(),
        fields,
        evidence,
        other_mode_clauses: other_mode.iter().map(|o| (o.field, o.clause)).collect(),
        other_mode,
        fs_named,
        text_level: None,
        level: None,
    };

    // C2: other-mode wording, and a full-spectrum recorder named with a
    // heterodyne-only detector.
    analysis.text_level = match analysis.raw_level {
        // Level A drops only when every clause that gave level A also has
        // other-mode wording ("Heterodyne, time-expanded").
        Some(SaysHeterodyne) => {
            let all_other_mode = analysis
                .evidence
                .iter()
                .filter(|e| e.level == SaysHeterodyne)
                .all(|e| analysis.in_other_mode_clause(e));
            Some(if all_other_mode {
                MayBeHeterodyne
            } else {
                SaysHeterodyne
            })
        }
        Some(HeterodyneOnlyDetector) => Some(
            if !analysis.other_mode.is_empty() || analysis.fs_named.is_some() {
                MayBeHeterodyne
            } else {
                HeterodyneOnlyDetector
            },
        ),
        Some(MayBeHeterodyne) if !analysis.other_mode.is_empty() => None,
        level => level,
    };
    analysis.level = sample_rate_gate(analysis.text_level, sample_rate);
    analysis
}

/// C3: the levels each header sample rate allows. See [`infer_heterodyne_hint`].
fn sample_rate_gate(
    level: Option<HeterodyneLevel>,
    sample_rate: Option<u32>,
) -> Option<HeterodyneLevel> {
    use HeterodyneLevel::*;
    let level = level?;
    let allowed = match sample_rate {
        None => level != MayBeHeterodyne,
        Some(sr) if sr > 96_000 => false,
        Some(sr) if TE_HEADER_RATES.contains(&sr) => level == SaysHeterodyne,
        Some(sr) => level != MayBeHeterodyne || sr <= 48_000,
    };
    allowed.then_some(level)
}

// ---------------------------------------------------------------- excerpt

/// How far a cut point may move to land between words. Text without spaces,
/// such as Japanese, is cut at the character.
const SNAP_CHARS: usize = 15;

/// Most characters that widening a match to whole words adds on each side.
const WIDEN_MAX_CHARS: usize = 30;

/// Byte range of the text to quote as the match. The evidence's tokens are
/// widened to whole words when the evidence allows it. If that is longer
/// than [`EXCERPT_MAX_CHARS`], it is cut to that length around the tokens.
/// When the tokens themselves are longer than the limit, the cut is around
/// the heterodyne word in them, and tokens with no heterodyne word keep
/// their first [`EXCERPT_MAX_CHARS`] characters.
fn quoted_match(text: &str, evidence: &Evidence) -> (usize, usize) {
    let span = if evidence.widen {
        whole_words(text, evidence.span)
    } else {
        evidence.span
    };
    if text[span.0..span.1].chars().count() <= EXCERPT_MAX_CHARS {
        return span;
    }
    let tokens_fit = text[evidence.span.0..evidence.span.1].chars().count() <= EXCERPT_MAX_CHARS;
    let focus = match evidence.source {
        HeterodyneSource::HeterodyneWord if !tokens_fit => stem_span(text, evidence.span),
        _ => None,
    };
    cut_around(
        text,
        span,
        focus.unwrap_or(evidence.span),
        EXCERPT_MAX_CHARS,
    )
}

/// Byte range of the first heterodyne stem (in [`HET_STEMS`] order) inside
/// the token at `span`. `None` when the token has no stem, as for the tokens
/// of "het detector".
fn stem_span(text: &str, span: (usize, usize)) -> Option<(usize, usize)> {
    let token = &text[span.0..span.1];
    let lower: String = token.chars().flat_map(char::to_lowercase).collect();
    let (at, len) = HET_STEMS
        .iter()
        .find_map(|s| lower.find(s).map(|at| (at, s.len())))?;
    // Map the stem's byte range in `lower` back to whole characters of
    // `token`. One character can lowercase to more than one byte or char.
    let (mut start, mut end) = (None, None);
    let mut low = 0;
    for (o, ch) in token.char_indices() {
        let next = low + ch.to_lowercase().map(char::len_utf8).sum::<usize>();
        if start.is_none() && next > at {
            start = Some(o);
        }
        if next >= at + len {
            end = Some(o + ch.len_utf8());
            break;
        }
        low = next;
    }
    Some((span.0 + start?, span.0 + end?))
}

/// A byte range of at most `max` characters inside `outer` that contains
/// `focus`. The characters left over are split evenly before and after the
/// focus, and when one side runs out of text the other side gets the rest.
/// A focus longer than `max` keeps only its first `max` characters.
fn cut_around(
    text: &str,
    outer: (usize, usize),
    focus: (usize, usize),
    max: usize,
) -> (usize, usize) {
    let focus_chars = text[focus.0..focus.1].chars().count();
    if focus_chars >= max {
        let end = text[focus.0..focus.1]
            .char_indices()
            .nth(max)
            .map_or(focus.1, |(o, _)| focus.0 + o);
        return (focus.0, end);
    }
    let room = max - focus_chars;
    let before = text[outer.0..focus.0].chars().count();
    let after = text[focus.1..outer.1].chars().count();
    let take_after = after.min(room - before.min(room / 2));
    let take_before = before.min(room - take_after);
    // Skip the characters before the focus that do not fit.
    let start = text[outer.0..focus.0]
        .char_indices()
        .nth(before - take_before)
        .map_or(focus.0, |(o, _)| outer.0 + o);
    let end = text[focus.1..outer.1]
        .char_indices()
        .nth(take_after)
        .map_or(outer.1, |(o, _)| focus.1 + o);
    (start, end)
}

/// The clause around `span`, cut to at most [`EXCERPT_MAX_CHARS`] characters
/// at word boundaries, with `…` where text was cut. White space at either
/// end, as [`is_space`] defines it, is left out, and cuts snap to the same
/// white space. `span` must be at most [`EXCERPT_MAX_CHARS`] characters, as
/// [`quoted_match`] makes it.
fn excerpt(text: &str, clause: (usize, usize), span: (usize, usize)) -> String {
    let raw = &text[clause.0..clause.1];
    let start = clause.0 + (raw.len() - raw.trim_start_matches(is_space).len());
    let end = clause.0 + raw.trim_end_matches(is_space).len();
    if text[start..end].chars().count() <= EXCERPT_MAX_CHARS {
        return text[start..end].to_string();
    }

    let (ms, me) = (span.0.max(start), span.1.min(end));
    let (at_start, at_end) = cut_around(text, (start, end), (ms, me), EXCERPT_MAX_CHARS);
    let mut out = String::new();
    let mut cut_start = start;
    if at_start > start {
        cut_start = snap_forward(text, at_start, ms);
        out.push('…');
    }
    let mut cut_end = end;
    if at_end < end {
        // The `min` keeps the limit valid if `span` was too long.
        cut_end = snap_back(text, at_end, me.min(at_end));
    }
    out.push_str(&text[cut_start..cut_end]);
    if cut_end < end {
        out.push('…');
    }
    out
}

/// Moves a start cut at `at` forward to the start of the next word, if one
/// starts within [`SNAP_CHARS`] and no later than `limit`.
fn snap_forward(text: &str, at: usize, limit: usize) -> usize {
    let region = &text[at..limit];
    let at_word_start = text[..at].chars().next_back().is_some_and(is_space);
    let from = if at_word_start {
        0
    } else {
        match region
            .char_indices()
            .take(SNAP_CHARS + 1)
            .find(|&(_, c)| is_space(c))
        {
            Some((o, _)) => o,
            None => return at,
        }
    };
    limit - region[from..].trim_start_matches(is_space).len()
}

/// Moves an end cut at `at` back to the end of the previous word, if one ends
/// within [`SNAP_CHARS`] and after `limit`.
fn snap_back(text: &str, at: usize, limit: usize) -> usize {
    let next_is_space = text[at..].chars().next().is_some_and(is_space);
    let region = &text[limit..at];
    let to = if next_is_space {
        region.len()
    } else {
        match region
            .char_indices()
            .rev()
            .take(SNAP_CHARS + 1)
            .find(|&(_, c)| is_space(c))
        {
            Some((o, _)) => o,
            None => return at,
        }
    };
    limit + region[..to].trim_end_matches(is_space).len()
}

/// Widens a token span to the whole run of alphanumeric characters around
/// it, so a match on "bat" in "Bat5" quotes "Bat5". It adds at most
/// [`WIDEN_MAX_CHARS`] characters on each side.
fn whole_words(text: &str, (mut start, mut end): (usize, usize)) -> (usize, usize) {
    for c in text[..start]
        .chars()
        .rev()
        .take(WIDEN_MAX_CHARS)
        .take_while(|c| c.is_alphanumeric())
    {
        start -= c.len_utf8();
    }
    for c in text[end..]
        .chars()
        .take(WIDEN_MAX_CHARS)
        .take_while(|c| c.is_alphanumeric())
    {
        end += c.len_utf8();
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use HeterodyneLevel::*;

    const A: Option<HeterodyneLevel> = Some(SaysHeterodyne);
    const B: Option<HeterodyneLevel> = Some(HeterodyneOnlyDetector);
    const C: Option<HeterodyneLevel> = Some(MayBeHeterodyne);
    const NONE: Option<HeterodyneLevel> = None;

    /// A fixture from the Python reference's `fixtures.py`.
    struct Fixture {
        id: &'static str,
        text: RecordingText<'static>,
        sample_rate: u32,
        /// Level after the sample-rate gate.
        level: Option<HeterodyneLevel>,
        /// Level before the gate, when the fixture states it.
        text_level: Option<Option<HeterodyneLevel>>,
        note: &'static str,
    }

    fn fx(id: &'static str, sample_rate: u32, level: Option<HeterodyneLevel>) -> Fixture {
        Fixture {
            id,
            text: RecordingText::default(),
            sample_rate,
            level,
            text_level: None,
            note: "",
        }
    }

    impl Fixture {
        fn rmk(mut self, s: &'static str) -> Self {
            self.text.remarks = Some(s);
            self
        }
        fn dvc(mut self, s: &'static str) -> Self {
            self.text.device = Some(s);
            self
        }
        fn mic(mut self, s: &'static str) -> Self {
            self.text.microphone = Some(s);
            self
        }
        fn desc(mut self, s: &'static str) -> Self {
            self.text.description = Some(s);
            self
        }
        fn text_level(mut self, l: Option<HeterodyneLevel>) -> Self {
            self.text_level = Some(l);
            self
        }
        fn note(mut self, s: &'static str) -> Self {
            self.note = s;
            self
        }
    }

    /// Real cases, copied from Xeno-canto records (ids are XC numbers).
    fn real_fixtures() -> Vec<Fixture> {
        vec![
            fx("1104195", 44100, A)
                .rmk("Ultrasound into audible sound by heterodyne")
                .mic("Pettersson D240X")
                .dvc("Tascam DR05")
                .note("het word beats dual-mode model"),
            fx("1034529", 48000, A)
                .rmk("The sound was recordet from a handheld hetrodyne detector set at 31,9 kHz.\r\nEnergy maximum estimated from other passages to 30-31 kHz.")
                .dvc("iphone")
                .mic("no external microphone")
                .note("misspelling + tuning"),
            fx("1047523", 48000, A)
                .rmk("The recording was with an iphone held close to a Pettersson ultrasound detector D200, which was set to 23,5 kHz. \r\nThe eco location sound was strong even below 20 kHz")
                .dvc("iphone")
                .mic("no external microphone")
                .note("tuning + D200"),
            fx("1031342", 48000, A)
                .rmk("The sound was detected with a handheld bat detector of type Pettersson D200.\r\nThe output from the speaker was recorded directly with an Iphone held close.\r\nIt was set on 51,3 kHz")
                .dvc("iphone")
                .note("'detected with' must not negate"),
            fx("924727", 48000, B)
                .rmk("Recording with Magenta Bat5")
                .note("model in remark"),
            fx("831342", 44100, B)
                .dvc("Magenta Bat5")
                .rmk("Semiopen area around a house")
                .note("model in dvc"),
            fx("995835", 48000, B)
                .dvc("Zoom F6 …")
                .mic("Magenta Bat 5 …")
                .rmk("Pipistrelle bat calls … up to five bats feeding over pond …")
                .note("detector in mic, recorder in dvc"),
            fx("825295", 48000, B)
                .dvc("Olympus LS 14 ...")
                .mic("Magenta Bat 5 ... ")
                .rmk("Pipistrelle bat ... International Bat Night ... 26th - 27th August ... out with Magenta Bat 5 bat detector on tripod to Olympus LS 14 ... "),
            fx("933225", 44100, C)
                .dvc("ultrasonic detector and mobile phone")
                .mic("recorded sound from ultrasonic detectors spaker with mobilphone")
                .rmk("recorded sound from ultrasonic detectors spaker with mobilphone\r\nI have 2 more recording from the same evening")
                .note("detector speaker, mode not stated"),
            fx("1062066", 44100, C)
                .dvc("Pettersson D980")
                .mic("solid-dielectric capacitance")
                .rmk("Hand released recordings with a good proportion of free flight at the end.")
                .note("dual-mode at 44.1k; lengths say TE"),
            fx("1091356", 192000, NONE)
                .rmk("Lower frequencies are from nearby heterodyne detector; animals filmed while attacking fish in a puddle")
                .dvc("Audiomoth 192kHz")
                .note("bleed"),
            fx("1091133", 300000, NONE)
                .rmk("Bat filmed too as it flew at about 2-4m away with social call. At very low frequencies heterodyne detector output.")
                .dvc("Avisoft 116Hm")
                .note("bleed"),
            fx("1183210", 384000, NONE)
                .rmk("SNR 36dB(ISO/ITU)\r\nOutside the cave\r\n\r\nHigh-pass filter 10kHz (to avoid heterodyne-detector's sound)")
                .mic("Dodotronic ultramic 384k_Evo")
                .note("filter"),
            fx("819607", 256000, NONE)
                .rmk("original recording free flight recording vegetation at canopy level, recorded near a body of water recorded in real-time. Heterodyne,  time-expanded")
                .dvc("Echometer")
                .mic("Echometer omnidireccional")
                .text_level(C)
                .note("listening modes of an FS recorder"),
            fx("1082453", 256000, NONE)
                .dvc("Heterodyne with timeexpansion")
                .text_level(C)
                .note("het + TE in device field"),
            fx("1004707", 256000, NONE)
                .rmk("Used the Echo Touch2 bat detector from Wildlife Acoustics.  It amplifies the calls and plays them back so they are audible to humans.")
                .dvc("iphone 16")
                .mic("Echo Touch2 bat detector")
                .note("FS detector + phone"),
            fx("803754", 192000, NONE)
                .dvc("Sony PCM-D100")
                .mic("Sonorous Objects SO.2 Ultrasonic stereo mic (Primo EM258)")
                .rmk("d100 + so.2")
                .note("Sony PCM-D100, not Pettersson D100"),
            // The remark is shortened: the original starts with the name of
            // the person who identified the bat.
            fx("820057", 44100, NONE)
                .dvc("Sony PCM-D100")
                .mic("Built-in")
                .rmk("Clearing in mixed forest. Sunset 21:13. Extract of soundscape recording. Volume +20 dB.")
                .note("D100 trap at low rate"),
            fx("1013548", 256000, NONE)
                .rmk("Recorded just after the bats had returned to their roost (a bat box) in the early morning.")
                .dvc("Echo Meter Touch 2")
                .note("roost box, not Batbox Ltd"),
            fx("834242", 500000, NONE)
                .dvc("D500x Bat box")
                .mic("Pettersson")
                .note("D500X label"),
            fx("1024353", 44100, NONE)
                .dvc("Pettersson 240x + Tascam")
                .rmk("Near pond. It is about the lower sound in the (10x retarded) recording, not the foraging Common pipistrelle bat.")
                .note("dual-mode + 10x wording"),
            fx("759031", 32000, NONE)
                .dvc("Pettersson D240x")
                .rmk("Chiroptera Test\r\nPettersson D240x original recording (=time expanded 10x, sampling rate 44.100 Hz), just a new sampling rate: 32.000 Hz")
                .note("TE wording"),
            fx("1063198", 96000, NONE)
                .rmk("\"op dit moment een grotere vleermuis voorbij vliegend\". De lange CF-pulsen zijn mogelijk van het scherpstellen van mijn camera.")
                .dvc("Sony PCM-M10")
                .note("Dutch 'het' = 'the'"),
            fx("837336", 441000, NONE)
                .dvc("Pettersson D240x")
                .rmk("im Vordergrund Echoortung Plecotus sp.\r\n\r\nSchlappinerjoch 2200m, Detektor-Nacht vom 20.8. um 20:30h MESZ bis 21.8.")
                .text_level(C)
                .note("German 'Detektor-Nacht'; D240x suppressed by smp"),
            fx("933465", 500000, NONE)
                .rmk("Även inspelad med annan detektor ca 70 m bort, en minut senare.")
                .note("Swedish 'another detector'"),
            fx("828980", 256000, NONE)
                .rmk("Jag startade detektorn när de flög ut från the house.")
                .note("Swedish generic 'detektorn'"),
            fx("909786", 48000, NONE)
                .rmk("Il semble que cela pourrait-être une pipistrelle commune. Elle chassait à l'aube en bordure d'un étang.")
                .dvc("SD MixPre 3II")
                .mic("sennheizer MKH40")
                .note("French, audible recording"),
            fx("1033741", 24000, NONE)
                .dvc("Sm4")
                .mic("Sm4")
                .rmk("Resident species, only bat that my recorder can hear. Recorder set at 6db.")
                .note("'set at' without kHz"),
            fx("822312", 192000, NONE)
                .dvc("Audiomoth 192 kHz")
                .mic("MEMS")
                .rmk("Audiomoth on 10m tall pole, bat scanning the Audiomoth.")
                .note("'bat scanning' is not Batscanner"),
            fx("806265", 44100, NONE)
                .dvc("Tascam DR-05X")
                .rmk("Passed close to head, pretty sure of species from use of bat detector, however I did pick up Pipistrellus pygmaeus too.")
                .note("detector used for ID"),
            fx("807175", 44100, NONE)
                .dvc("parabol")
                .rmk("A bat of a kind...Any suggestions? many sounds like this evening and early morning. Direct recorded with parabol without detector.")
                .note("'without detector'"),
            fx("1100677", 44100, NONE)
                .dvc("Echo Meter with ipad")
                .mic("echo meter")
                .rmk("I believe the recording is at normal tempo (not slowed down).")
                .note("negated other-mode wording"),
            fx("1099114", 48000, NONE)
                .dvc("Apodemus pippyg")
                .rmk("Rate slowed down by 8 times")
                .note("slowed FS"),
            fx("892810", 38400, NONE)
                .dvc("EM3")
                .mic("built-in")
                .note("x10 header rate"),
            fx("1035085", 48000, NONE)
                .dvc("iphone")
                .mic("no external microphone")
                .rmk("Bat flying in the woods adjacent to the suburbs of the City of Sundsvall")
                .note("probably het (same recordist as D200 files); text cannot tell"),
        ]
    }

    /// Made-up cases for wording the real data does not have.
    fn synthetic_fixtures() -> Vec<Fixture> {
        vec![
            fx("S-nl", 44100, A).rmk("Opname met heterodyne detector, afgestemd op 45 kHz"),
            fx("S-de", 48000, A).rmk("Aufnahme mit Heterodyn-Detektor, eingestellt auf 40 kHz"),
            fx("S-fr", 44100, A).rmk("Enregistré avec un détecteur hétérodyne réglé sur 45 kHz"),
            fx("S-es", 44100, A).rmk("Grabado con detector heterodino"),
            fx("S-it", 44100, A).rmk("Registrato con bat detector eterodina"),
            fx("S-sv", 44100, A).rmk("Inspelad med heterodyndetektor"),
            fx("S-ja", 48000, A).rmk("ヘテロダイン式バットディテクターで録音"),
            fx("S-ru", 44100, A).rmk("Записано гетеродинным детектором"),
            fx("S-auto", 44100, A)
                .rmk("SSF Bat3 in autoheterodyne")
                .note("compound stem"),
            fx("S-HET", 44100, A)
                .rmk("Batbox Duet, HET channel only")
                .note("uppercase HET"),
            fx("S-not", 44100, NONE).rmk("Not heterodyne: full spectrum recording"),
            fx("S-find", 44100, NONE).rmk("Full spectrum. Heterodyne only used to find the bat."),
            fx("S-mag-find", 44100, NONE)
                .rmk("Magenta Bat4 used to find bats, recorded with Echo Meter Touch"),
            fx("S-duet", 44100, C).rmk("Recorded through the headphone output of my Batbox Duet"),
            fx("S-ssf-te", 44100, C)
                .rmk("SSF Bat2 output, time expanded 10x afterwards")
                .note("het-only + other mode"),
            fx("S-bg", 44100, A)
                .rmk("Recorded the heterodyne output of my Magenta Bat4 with a Zoom H1n while cars passed in the background")
                .note("bleed word far from het word"),
            fx("S-desc", 44100, A)
                .desc("Heterodyne bat detector recording of a common pipistrelle, tuned to 45 kHz")
                .note("free-text Description field"),
            fx("S-sony", 44100, B)
                .rmk("Pettersson bat detector D200")
                .dvc("Sony PCM M10")
                .note("Sony recorder must not block D200"),
            fx("S-d200-fs", 44100, C)
                .dvc("Pettersson D200 + Echo Meter Touch 2")
                .note("het-only + FS recorder in dvc"),
        ]
    }

    fn check_fixture(f: &Fixture) {
        let a = analyze(&f.text, Some(f.sample_rate));
        assert_eq!(a.level, f.level, "{} final level ({})", f.id, f.note);
        if let Some(text_level) = f.text_level {
            assert_eq!(a.text_level, text_level, "{} text level ({})", f.id, f.note);
        }
        let hint = infer_heterodyne_hint(&f.text, Some(f.sample_rate));
        assert_eq!(hint.as_ref().map(|h| h.level), f.level, "{}", f.id);
        if let Some(h) = hint {
            let field = f.text.get(h.field).unwrap();
            assert!(field.contains(&h.matched), "{}: {:?}", f.id, h);
            assert!(h.excerpt.contains(&h.matched), "{}: {:?}", f.id, h);
            assert!(
                field.contains(h.excerpt.trim_matches('…')),
                "{}: {:?}",
                f.id,
                h
            );
        }
    }

    #[test]
    fn real_fixtures_match_reference() {
        let fixtures = real_fixtures();
        assert_eq!(fixtures.len(), 35);
        for f in &fixtures {
            check_fixture(f);
        }
    }

    #[test]
    fn synthetic_fixtures_match_reference() {
        let fixtures = synthetic_fixtures();
        assert_eq!(fixtures.len(), 19);
        for f in &fixtures {
            check_fixture(f);
        }
    }

    /// Evidence in the format of the Python reference's `classify` result:
    /// level (3 = A, 2 = B, 1 = C), rule, field, matched text, clause index.
    type RefEvidence = (u8, &'static str, &'static str, &'static str, usize);
    /// Other-mode hits in the reference's format: field, clause, phrase.
    type RefOtherMode = (&'static str, usize, &'static str);

    #[test]
    fn evidence_lists_match_reference() {
        const R1: &str = "R1 het word";
        const R2: &str = "R2 tuned to NN kHz + detector";
        const R3: &str = "R3 het-only detector";
        const R4: &str = "R4 dual-mode detector";
        const R5: &str = "R5 detector output recorded";
        const D2: &str = "Pettersson D230/D240x/D980/D1000X [d 240]";
        // Copied from the Python reference's classify(), run on the same
        // fixture text.
        let cases: [(&str, Vec<RefEvidence>, Vec<RefOtherMode>); 9] = [
            (
                "1104195",
                vec![(3, R1, "rmk", "heterodyne", 0), (1, R4, "mic", D2, 0)],
                vec![],
            ),
            (
                "1047523",
                vec![
                    (3, R2, "rmk", "set to 23 5 khz", 0),
                    (
                        2,
                        R3,
                        "rmk",
                        "Pettersson D100/D200 [pettersson ultrasound detector d 200]",
                        0,
                    ),
                    (2, R3, "rmk", "Pettersson D200 [d 200]", 0),
                    (1, R5, "rmk", "held close", 0),
                ],
                vec![],
            ),
            (
                "1031342",
                vec![
                    (2, R3, "rmk", "Pettersson D100/D200 [pettersson d 200]", 0),
                    (2, R3, "rmk", "Pettersson D200 [d 200]", 0),
                    (3, R2, "rmk", "set on 51 3 khz", 2),
                ],
                vec![],
            ),
            (
                "933225",
                vec![
                    (1, R5, "rmk", "spaker", 0),
                    (1, R5, "dvc", "mobile", 0),
                    (1, R5, "mic", "spaker", 0),
                ],
                vec![],
            ),
            (
                "759031",
                vec![(1, R4, "rmk", D2, 1), (1, R4, "dvc", D2, 0)],
                vec![("rmk", 1, "time expan*"), ("rmk", 1, "10 x")],
            ),
            ("1100677", vec![], vec![]),
            ("S-ja", vec![(3, R1, "rmk", "ヘテロダイン", 0)], vec![]),
            (
                "S-HET",
                vec![
                    (3, R1, "rmk", "het channel", 0),
                    (3, R1, "rmk", "HET", 0),
                    (1, R4, "rmk", "Batbox Duet [batbox duet]", 0),
                ],
                vec![],
            ),
            (
                "S-bg",
                vec![
                    (3, R1, "rmk", "heterodyne", 0),
                    (2, R3, "rmk", "Magenta Bat4/Bat5 [magenta bat]", 0),
                ],
                vec![],
            ),
        ];
        let field_key = |f: TextField| match f {
            TextField::Remarks => "rmk",
            TextField::Device => "dvc",
            TextField::Microphone => "mic",
            TextField::Method => "method",
            TextField::Description => "desc",
        };
        let rule = |s: HeterodyneSource| match s {
            HeterodyneSource::HeterodyneWord => R1,
            HeterodyneSource::TuningPhrase => R2,
            HeterodyneSource::HeterodyneOnlyDetector => R3,
            HeterodyneSource::DualModeDetector => R4,
            HeterodyneSource::DetectorOutput => R5,
        };
        let fixtures: Vec<Fixture> = real_fixtures()
            .into_iter()
            .chain(synthetic_fixtures())
            .collect();
        for (id, evidence, other_mode) in cases {
            let f = fixtures.iter().find(|f| f.id == id).unwrap();
            let a = analyze(&f.text, Some(f.sample_rate));
            let got: Vec<(u8, &str, &str, &str, usize)> = a
                .evidence
                .iter()
                .map(|e| {
                    (
                        e.level as u8 + 1,
                        rule(e.source),
                        field_key(e.field),
                        e.key.as_str(),
                        e.clause,
                    )
                })
                .collect();
            assert_eq!(got, evidence, "{id} evidence");
            let got: Vec<RefOtherMode> = a
                .other_mode
                .iter()
                .map(|o| (field_key(o.field), o.clause, o.phrase))
                .collect();
            assert_eq!(got, other_mode, "{id} other-mode wording");
        }
    }

    fn remarks(s: &str) -> RecordingText<'_> {
        RecordingText {
            remarks: Some(s),
            ..Default::default()
        }
    }

    fn words(s: &str) -> Vec<String> {
        tokenize(s, (0, s.len())).0
    }

    #[test]
    fn tokens_split_at_digits_and_punctuation() {
        assert_eq!(words("Bat5"), ["bat", "5"]);
        assert_eq!(words("D240x"), ["d", "240", "x"]);
        assert_eq!(words("SM4BAT"), ["sm", "4", "bat"]);
        assert_eq!(words("31,9 kHz"), ["31", "9", "khz"]);
        assert_eq!(
            words("heterodyne-detector's"),
            ["heterodyne", "detector", "s"]
        );
        // "²" is alphanumeric but not an ASCII digit, so it stays in the word.
        assert_eq!(words("x² Hétérodyne"), ["x²", "hétérodyne"]);
    }

    #[test]
    fn tokens_differ_from_the_reference_on_purpose() {
        // Vowel signs and circled letters stay in the word. The Python
        // reference splits at them: "ह ट र ड इन" and "magenta bat 5".
        assert_eq!(words("हेटेरोडाइन"), ["हेटेरोडाइन"]);
        assert_eq!(words("MagentaⓇ Bat5"), ["magentaⓡ", "bat", "5"]);
        // Both split at a Thai tone mark. The reference also drops the
        // vowel sign before it: "เคร อง".
        assert_eq!(words("เครื่อง"), ["เครื", "อง"]);
        // A word-final capital sigma lowercases to "σ", not "ς".
        assert_eq!(words("ΟΔΟΣ"), ["οδοσ"]);
    }

    #[test]
    fn token_spans_point_at_original_text() {
        let s = "Magenta BAT5, réglé";
        let (w, spans) = tokenize(s, (0, s.len()));
        let original: Vec<&str> = spans.iter().map(|&(a, b)| &s[a..b]).collect();
        assert_eq!(w, ["magenta", "bat", "5", "réglé"]);
        assert_eq!(original, ["Magenta", "BAT", "5", "réglé"]);
    }

    #[test]
    fn clauses_split_at_sentence_ends_only() {
        let s = "Set at 31,9 kHz.\r\nRate 44.100 Hz; next! ok?  \n. end.";
        let clauses: Vec<&str> = clause_spans(s).iter().map(|&(a, b)| &s[a..b]).collect();
        assert_eq!(
            clauses,
            ["Set at 31,9 kHz", "Rate 44.100 Hz", " next", " ok", " end"]
        );
    }

    #[test]
    fn ascii_separators_count_as_whitespace_in_clauses() {
        // U+001C after "." ends the clause, as in the Python reference, so
        // the time-expansion wording is in another clause and level A stays.
        let s = "Heterodyne.\u{1c}Time expansion x10";
        let clauses: Vec<&str> = clause_spans(s).iter().map(|&(a, b)| &s[a..b]).collect();
        assert_eq!(clauses, ["Heterodyne", "\u{1c}Time expansion x10"]);
        assert_eq!(analyze(&remarks(s), Some(44100)).level, A);
        // A clause of only these separators is blank.
        assert_eq!(clause_spans("a;\u{1d}\u{1f} ;b").len(), 2);
    }

    #[test]
    fn excerpts_leave_out_ascii_separators_at_their_ends() {
        // The excerpt leaves out the U+001F that starts the clause after
        // ".", and a U+001E at the end.
        let hint = infer_heterodyne_hint(&remarks("Note.\x1fHeterodyne"), None).unwrap();
        assert_eq!(hint.excerpt, "Heterodyne");
        let hint = infer_heterodyne_hint(&remarks("Heterodyne 45 kHz\x1e"), None).unwrap();
        assert_eq!(hint.excerpt, "Heterodyne 45 kHz");

        // A long clause with U+001F between its words is cut between words,
        // with no U+001F next to the `…`.
        let s = format!("{}heterodyne", "words\x1f".repeat(30));
        let hint = infer_heterodyne_hint(&remarks(&s), None).unwrap();
        assert_eq!(
            hint.excerpt,
            format!("…{}heterodyne", "words\x1f".repeat(11))
        );
        let s = format!("heterodyne{}", "\x1fwords".repeat(30));
        let hint = infer_heterodyne_hint(&remarks(&s), None).unwrap();
        assert_eq!(
            hint.excerpt,
            format!("heterodyne{}…", "\x1fwords".repeat(11))
        );
    }

    #[test]
    fn sequences_match_prefixes_and_list_order() {
        let toks = words("Pettersson D240x and D980");
        assert_eq!(find_seq(&toks, "d 240").collect::<Vec<_>>(), [1]);
        assert_eq!(find_seq(&toks, "pett*").collect::<Vec<_>>(), [0]);
        assert_eq!(find_seq(&toks, "a b c d e f g").count(), 0);
        // The first sequence in the list wins, even when another sequence
        // matches earlier in the text.
        assert_eq!(any_seq(&toks, &["d 980", "d 240"]), Some(("d 980", 5)));
    }

    #[test]
    fn het_needs_a_neighbour_word() {
        assert_eq!(
            analyze(&remarks("van het scherpstellen"), None).raw_level,
            None
        );
        for s in [
            "het detector at 45",
            "auto-het",
            "FD/het",
            "het 45 kHz",
            "het 45k",
        ] {
            assert_eq!(analyze(&remarks(s), None).raw_level, A, "{s}");
        }
        // Uppercase HET counts, except in an all-caps clause.
        assert_eq!(analyze(&remarks("Duet on HET"), None).raw_level, A);
        assert_eq!(analyze(&remarks("HET IS DONKER"), None).raw_level, None);
    }

    #[test]
    fn negation_and_bleed_block_het_words() {
        for s in [
            "not heterodyne",
            "without a heterodyne",
            "no heterodyne",
            "nearby heterodyne detector",
            "heterodyne only used to find the bat",
        ] {
            assert_eq!(analyze(&remarks(s), Some(44100)).raw_level, None, "{s}");
        }
        // Portuguese "no" (in the) negates only directly before.
        assert_eq!(
            analyze(&remarks("no modo heterodino"), Some(44100)).raw_level,
            A
        );
        // A bleed word more than 6 tokens away does not block.
        let far = "heterodyne output one two three four five six background";
        assert_eq!(analyze(&remarks(far), Some(44100)).raw_level, A);
    }

    #[test]
    fn tuning_phrase_needs_khz_and_a_detector_word() {
        let tuned = |s: &str| analyze(&remarks(s), Some(44100)).raw_level;
        assert_eq!(tuned("bat detector set to 45 kHz"), A);
        assert_eq!(tuned("bat detector set to 23,5 k hz"), A);
        assert_eq!(tuned("set to 45 kHz"), None);
        assert_eq!(tuned("bat detector set to 6 dB"), None);
        assert_eq!(tuned("bat detector set to 200 kHz"), None);
        assert_eq!(tuned("bat detector, high pass filter set to 15 kHz"), None);
    }

    #[test]
    fn bare_model_numbers_need_context() {
        let level = |rmk: &str, dvc: &str| {
            let t = RecordingText {
                remarks: Some(rmk),
                device: Some(dvc),
                ..Default::default()
            };
            analyze(&t, Some(44100)).level
        };
        assert_eq!(level("D200", ""), None);
        assert_eq!(level("D200", "bat detector"), B);
        assert_eq!(level("Sony D200 recorder", "bat detector"), None);
        // A Sony PCM-D100 anywhere blocks bare D100 but not D200.
        assert_eq!(level("d100 + detector", "Sony PCM-D100"), None);
        assert_eq!(level("d200 + detector", "Sony PCM-D100"), B);
    }

    #[test]
    fn duet_and_griffin_count_only_in_device_fields() {
        let t = RecordingText {
            remarks: Some("a duet of two bats"),
            ..Default::default()
        };
        assert_eq!(analyze(&t, Some(44100)).raw_level, None);
        let t = RecordingText {
            device: Some("Duet"),
            ..Default::default()
        };
        assert_eq!(analyze(&t, Some(44100)).level, C);
    }

    #[test]
    fn sample_rate_gate_branches() {
        let gate = sample_rate_gate;
        // Unknown rate: A and B pass, C needs a known rate.
        assert_eq!(gate(A, None), A);
        assert_eq!(gate(B, None), B);
        assert_eq!(gate(C, None), None);
        assert_eq!(gate(None, None), None);
        // Above 96 kHz: nothing passes.
        for l in [A, B, C] {
            assert_eq!(gate(l, Some(96_001)), None);
            assert_eq!(gate(l, Some(384_000)), None);
        }
        // x10 header rates: only A.
        for &sr in TE_HEADER_RATES {
            assert_eq!(gate(A, Some(sr)), A, "{sr}");
            assert_eq!(gate(B, Some(sr)), None, "{sr}");
            assert_eq!(gate(C, Some(sr)), None, "{sr}");
        }
        // 48 to 96 kHz: A and B pass, C does not.
        assert_eq!(gate(A, Some(96_000)), A);
        assert_eq!(gate(B, Some(96_000)), B);
        assert_eq!(gate(C, Some(96_000)), None);
        assert_eq!(gate(C, Some(48_001)), None);
        // 48 kHz or less: all pass.
        for l in [A, B, C] {
            assert_eq!(gate(l, Some(48_000)), l);
            assert_eq!(gate(l, Some(44_100)), l);
        }
    }

    #[test]
    fn says_het_drops_only_when_every_clause_has_other_mode_wording() {
        // Other-mode wording in another clause: stays A.
        let a = analyze(
            &remarks("Heterodyne output. Calls time expanded later."),
            Some(44100),
        );
        assert_eq!(a.text_level, A);
        // Two level A clauses, one with other-mode wording: stays A, and the
        // quote comes from the clause without it.
        let t = remarks("Heterodyne, time expanded. Recorded from the heterodyne output.");
        assert_eq!(analyze(&t, Some(44100)).text_level, A);
        let hint = infer_heterodyne_hint(&t, Some(44100)).unwrap();
        assert_eq!(hint.excerpt, "Recorded from the heterodyne output");
        // Every level A clause has other-mode wording: drops to C, not lower.
        let t = remarks("Heterodyne, time expanded");
        assert_eq!(analyze(&t, Some(44100)).text_level, C);
        let hint = infer_heterodyne_hint(&t, Some(44100)).unwrap();
        assert_eq!(hint.level, MayBeHeterodyne);
        assert_eq!(hint.source, HeterodyneSource::HeterodyneWord);
    }

    #[test]
    fn het_only_detector_downgrades() {
        let level = |t: RecordingText| analyze(&t, Some(44100)).text_level;
        // Other-mode wording anywhere: B drops to C.
        assert_eq!(
            level(RecordingText {
                device: Some("Magenta Bat5"),
                remarks: Some("Slowed down afterwards"),
                ..Default::default()
            }),
            C
        );
        // A full-spectrum recorder in the device field: B drops to C.
        assert_eq!(
            level(RecordingText {
                device: Some("Magenta Bat5"),
                microphone: Some("AudioMoth"),
                ..Default::default()
            }),
            C
        );
        // A full-spectrum recorder named only in the remarks: stays B.
        assert_eq!(
            level(RecordingText {
                device: Some("Magenta Bat5"),
                remarks: Some("An AudioMoth was out too"),
                ..Default::default()
            }),
            B
        );
    }

    #[test]
    fn maybe_het_drops_to_none_unless_other_mode_is_negated() {
        let level = |rmk: &str| {
            let t = RecordingText {
                device: Some("Pettersson D240x"),
                remarks: Some(rmk),
                ..Default::default()
            };
            analyze(&t, Some(44100)).text_level
        };
        assert_eq!(level("time expanded 10x"), None);
        assert_eq!(level("not time expanded"), C);
        assert_eq!(level("recorded at night"), C);
    }

    #[test]
    fn hint_quotes_original_text() {
        let t = RecordingText {
            remarks: Some("The sound was recordet from a handheld hetrodyne detector set at 31,9 kHz.\r\nEnergy maximum"),
            device: Some("iphone"),
            ..Default::default()
        };
        let hint = infer_heterodyne_hint(&t, Some(48000)).unwrap();
        assert_eq!(
            hint,
            HeterodyneHint {
                level: SaysHeterodyne,
                source: HeterodyneSource::HeterodyneWord,
                field: TextField::Remarks,
                excerpt:
                    "The sound was recordet from a handheld hetrodyne detector set at 31,9 kHz"
                        .into(),
                matched: "hetrodyne".into(),
            }
        );

        let t = RecordingText {
            microphone: Some("Magenta Bat 5 …"),
            ..Default::default()
        };
        let hint = infer_heterodyne_hint(&t, Some(48000)).unwrap();
        assert_eq!(hint.source, HeterodyneSource::HeterodyneOnlyDetector);
        assert_eq!(hint.field, TextField::Microphone);
        assert_eq!(hint.matched, "Magenta Bat");
        assert_eq!(hint.excerpt, "Magenta Bat 5 …");

        // A match that ends inside a word quotes the whole word.
        let t = RecordingText {
            device: Some("Magenta Bat5"),
            ..Default::default()
        };
        let hint = infer_heterodyne_hint(&t, Some(48000)).unwrap();
        assert_eq!(hint.matched, "Magenta Bat5");
    }

    #[test]
    fn tuning_quote_includes_the_unit() {
        let t = remarks("bat detector tuned to 45 k Hz");
        let hint = infer_heterodyne_hint(&t, Some(44100)).unwrap();
        assert_eq!(hint.source, HeterodyneSource::TuningPhrase);
        assert_eq!(hint.matched, "tuned to 45 k Hz");
    }

    #[test]
    fn long_clauses_are_cut_around_the_match() {
        let s = "The recording was with an iphone held close to a Pettersson ultrasound detector D200, which was set to 23,5 kHz. \r\nThe eco location sound was strong";
        let hint = infer_heterodyne_hint(&remarks(s), Some(48000)).unwrap();
        assert_eq!(hint.source, HeterodyneSource::TuningPhrase);
        assert_eq!(hint.matched, "set to 23,5 kHz");
        assert!(hint.excerpt.starts_with('…'), "{}", hint.excerpt);
        assert!(
            hint.excerpt.ends_with("which was set to 23,5 kHz"),
            "{}",
            hint.excerpt
        );
        assert!(hint.excerpt.chars().count() <= EXCERPT_MAX_CHARS + 1);
        // The cut lands between words.
        let first_word = hint
            .excerpt
            .trim_start_matches('…')
            .split(' ')
            .next()
            .unwrap();
        assert!(s.contains(&format!(" {first_word} ")), "{}", hint.excerpt);

        // Cut on both sides.
        let s = format!(
            "{} heterodyne {}",
            "word ".repeat(30).trim_end(),
            "more ".repeat(30).trim_end()
        );
        let hint = infer_heterodyne_hint(&remarks(&s), None).unwrap();
        assert!(hint.excerpt.starts_with("…word "), "{}", hint.excerpt);
        assert!(hint.excerpt.ends_with(" more…"), "{}", hint.excerpt);
        assert!(hint.excerpt.contains(" heterodyne "));
        assert!(hint.excerpt.chars().count() <= EXCERPT_MAX_CHARS + 2);
    }

    #[test]
    fn text_without_spaces_is_cut_at_characters() {
        let s = format!("{}ヘテロダイン式{}", "あ".repeat(60), "い".repeat(60));
        let hint = infer_heterodyne_hint(&remarks(&s), Some(48000)).unwrap();
        assert_eq!(hint.matched, "ヘテロダイン");
        assert!(hint.excerpt.starts_with('…') && hint.excerpt.ends_with('…'));
        assert_eq!(hint.excerpt.chars().count(), EXCERPT_MAX_CHARS + 2);
    }

    /// Checks the length limits of a hint for remarks `s`, and returns it.
    fn bounded_hint(s: &str) -> HeterodyneHint {
        let hint = infer_heterodyne_hint(&remarks(s), Some(44100)).unwrap();
        let core = hint.excerpt.trim_start_matches('…').trim_end_matches('…');
        assert!(
            core.chars().count() <= EXCERPT_MAX_CHARS,
            "{}",
            hint.excerpt
        );
        assert!(hint.matched.chars().count() <= EXCERPT_MAX_CHARS);
        assert!(hint.excerpt.contains(&hint.matched));
        assert!(s.contains(core));
        hint
    }

    #[test]
    fn long_runs_without_spaces_are_quoted_in_part() {
        // Widening stops 30 characters after the matched token.
        let s = "heterodyne1".repeat(100_000 / 11);
        let hint = bounded_hint(&s);
        assert_eq!(hint.matched, s[..40]);
        assert_eq!(hint.excerpt, format!("{}…", &s[..80]));

        // A token longer than the limit is cut around the heterodyne word.
        let s = format!("{}heterodyne{}", "a".repeat(50_000), "a".repeat(50_000));
        let hint = bounded_hint(&s);
        assert_eq!(hint.matched.chars().count(), EXCERPT_MAX_CHARS);
        assert!(hint.matched.contains("heterodyne"), "{}", hint.matched);
        assert_eq!(hint.excerpt, format!("…{}…", hint.matched));

        // A token that fits in the limit is quoted whole, and the cut takes
        // keeps the same number of widened digits on each side.
        let token = format!("{}heterodyne", "x".repeat(50));
        let s = format!("{}{token}{}", "1".repeat(30), "2".repeat(30));
        let hint = bounded_hint(&s);
        assert_eq!(
            hint.matched,
            format!("{}{token}{}", "1".repeat(10), "2".repeat(10))
        );
        assert_eq!(hint.excerpt, format!("…{}…", hint.matched));

        // The cut follows the word, not the middle of the token.
        let s = format!("{}heterodyne{}", "a".repeat(10), "a".repeat(99_990));
        let hint = bounded_hint(&s);
        assert!(hint.matched.starts_with(&s[..20]), "{}", hint.matched);
        assert!(hint.excerpt.ends_with('…') && !hint.excerpt.starts_with('…'));

        // A model name in a long run: the match keeps the model name.
        let s = format!("Pettersson {}", "D200".repeat(25_000));
        let hint = bounded_hint(&s);
        assert!(hint.matched.starts_with("Pettersson D200D200"));
    }

    #[test]
    fn stem_is_found_where_lowercase_has_other_byte_lengths() {
        // The Kelvin sign "K" (3 bytes) lowercases to "k" (1 byte).
        let s = format!("{0}HETERODYNE{0}", "\u{212a}".repeat(60));
        let (a, b) = stem_span(&s, (0, s.len())).unwrap();
        assert_eq!(&s[a..b], "HETERODYN");
        let hint = bounded_hint(&s);
        assert!(hint.matched.contains("HETERODYNE"), "{}", hint.matched);
    }

    #[test]
    fn empty_text_gives_no_hint() {
        assert_eq!(
            infer_heterodyne_hint(&RecordingText::default(), Some(44100)),
            None
        );
        assert_eq!(infer_heterodyne_hint(&remarks("  \r\n. "), None), None);
    }
}
