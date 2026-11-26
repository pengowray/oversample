//! Marks along the top edge of the overview for what the Call detection tab
//! found, so the parts of the file with calls can be seen at a glance.
//!
//! The in-built detector gives a tall green mark per bat call, a short green
//! mark across each call train and a short grey mark per other detection, the
//! same colours as its boxes on the spectrogram. BirdNET, Perch and
//! BattyBirdNET give a mark per stretch of windows with the same species, in
//! that species' colour from the label strip on the spectrogram.
//!
//! The marks sit in a band along the top edge: the bottom of the overview
//! holds the time ticks, their labels and the playhead dot, and the top has
//! only the bookmark dots, which are drawn over the band. The top of the
//! overview spectrogram is the highest frequencies, usually the emptiest part
//! of the picture.

use crate::classifier::summary::Span;
use crate::dsp::call_detect::{CallKind, CallTrain, DetectedCall};
use web_sys::CanvasRenderingContext2d;

/// Height of the band in canvas pixels. Marks are this tall or shorter.
pub const BAND_H: f64 = 4.0;

/// How one kind of mark is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkStyle {
    pub colour: &'static str,
    /// Pixels down from the top edge.
    pub height: f64,
}

/// A stretch of time to mark. `style` indexes [`OverviewMarks::styles`]; where
/// two marks share a pixel column, the lower `style` is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeMark {
    pub start_s: f64,
    pub end_s: f64,
    pub style: u8,
}

/// The marks for one file and method, in seconds from the start of the file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OverviewMarks {
    pub marks: Vec<TimeMark>,
    pub styles: Vec<MarkStyle>,
}

/// Pixel columns `x0..x1` that all show the mark style `style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnRun {
    pub x0: u32,
    pub x1: u32,
    pub style: u8,
}

/// The in-built detector's styles, in drawing priority.
const BAT_CALL: u8 = 0;
const TRAIN: u8 = 1;
const OTHER: u8 = 2;
const INBUILT_STYLES: [MarkStyle; 3] = [
    // The colours of `overlays::draw_calls` (`box_class`).
    MarkStyle {
        colour: crate::canvas::box_class::ANIMAL_STROKE,
        height: BAND_H,
    },
    MarkStyle {
        colour: crate::canvas::box_class::TRAIN_STROKE,
        height: 2.0,
    },
    MarkStyle {
        colour: crate::canvas::box_class::NOISE_STROKE,
        height: 2.0,
    },
];

/// Marks for the in-built detector's results: bat calls first, then trains,
/// then the other detections.
pub fn inbuilt_marks(calls: &[DetectedCall], trains: &[CallTrain]) -> OverviewMarks {
    let mut marks: Vec<TimeMark> = calls
        .iter()
        .map(|c| TimeMark {
            start_s: c.start_s,
            end_s: c.end_s,
            style: if c.kind == CallKind::Bat {
                BAT_CALL
            } else {
                OTHER
            },
        })
        .collect();
    marks.extend(trains.iter().map(|t| TimeMark {
        start_s: t.start_s,
        end_s: t.end_s,
        style: TRAIN,
    }));
    OverviewMarks {
        marks,
        styles: INBUILT_STYLES.to_vec(),
    }
}

/// Marks for the combined scan: the in-built detector's, with each bird box
/// (start and end, in seconds) marked as a bat call is: both are animal
/// calls.
pub fn combined_marks(
    calls: &[DetectedCall],
    trains: &[CallTrain],
    as_noise: &[bool],
    bird_boxes: impl IntoIterator<Item = (f64, f64)>,
) -> OverviewMarks {
    let mut out = inbuilt_marks(calls, trains);
    // The first marks are the calls, in order: those inside a modulated
    // carrier are marked as other detections.
    for (m, &noise) in out.marks.iter_mut().zip(as_noise) {
        if noise {
            m.style = OTHER;
        }
    }
    out.marks
        .extend(bird_boxes.into_iter().map(|(start_s, end_s)| TimeMark {
            start_s,
            end_s,
            style: BAT_CALL,
        }));
    out
}

/// Marks for a classifier run's species stretches (`summary::spans`), each in
/// the colour of its species on the spectrogram's label strip. `order` is
/// `summary::colour_order` for the run: species past the third share a grey.
pub fn classifier_marks(spans: &[Span], order: &[u16]) -> OverviewMarks {
    use crate::canvas::overlays::{species_colour, SPECIES_COLOURS};
    // One style per colour slot, plus the grey for the rest.
    let others = SPECIES_COLOURS.len();
    let styles = (0..=others)
        .map(|slot| MarkStyle {
            colour: species_colour(slot).0,
            height: BAND_H,
        })
        .collect();
    let marks = spans
        .iter()
        .map(|s| TimeMark {
            start_s: s.start_s,
            end_s: s.end_s,
            style: order
                .iter()
                .position(|&c| c == s.class)
                .unwrap_or(others)
                .min(others) as u8,
        })
        .collect();
    OverviewMarks { marks, styles }
}

/// Merge `marks` into runs of pixel columns for an overview `width` pixels
/// wide showing `span` seconds from `axis_start`. Each mark fills at least
/// one column, so a call shorter than a pixel still shows. A column covered
/// by more than one mark takes the lowest style. Neighbouring columns with
/// the same style make one run, so a file with thousands of calls draws at
/// most `width` rectangles.
pub fn merge_columns(marks: &[TimeMark], axis_start: f64, span: f64, width: u32) -> Vec<ColumnRun> {
    if width == 0 || span <= 0.0 || !span.is_finite() {
        return Vec::new();
    }
    let w = width as usize;
    let px_per_sec = width as f64 / span;
    let mut best = vec![u8::MAX; w];
    for m in marks {
        let x0 = ((m.start_s - axis_start) * px_per_sec).floor();
        let x1 = ((m.end_s - axis_start) * px_per_sec).ceil().max(x0 + 1.0);
        if !(x0.is_finite() && x1.is_finite()) || x1 <= 0.0 || x0 >= w as f64 {
            continue;
        }
        let (x0, x1) = (x0.max(0.0) as usize, (x1 as usize).min(w));
        for b in &mut best[x0..x1] {
            *b = (*b).min(m.style);
        }
    }
    let mut runs: Vec<ColumnRun> = Vec::new();
    for (x, &style) in best.iter().enumerate() {
        if style == u8::MAX {
            continue;
        }
        let x = x as u32;
        match runs.last_mut() {
            Some(r) if r.x1 == x && r.style == style => r.x1 = x + 1,
            _ => runs.push(ColumnRun {
                x0: x,
                x1: x + 1,
                style,
            }),
        }
    }
    runs
}

/// Draw `marks` along the top edge of an overview `width` pixels wide that
/// shows `span` seconds from `axis_start`.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    marks: &OverviewMarks,
    axis_start: f64,
    span: f64,
    width: u32,
) {
    let runs = merge_columns(&marks.marks, axis_start, span, width);
    if runs.is_empty() {
        return;
    }
    // A dark line under each mark, so it shows on a bright overview.
    ctx.set_fill_style_str("rgba(0, 0, 0, 0.6)");
    for r in &runs {
        let Some(style) = marks.styles.get(r.style as usize) else {
            continue;
        };
        ctx.fill_rect(r.x0 as f64, style.height, (r.x1 - r.x0) as f64, 1.0);
    }
    for r in &runs {
        let Some(style) = marks.styles.get(r.style as usize) else {
            continue;
        };
        ctx.set_fill_style_str(style.colour);
        ctx.fill_rect(r.x0 as f64, 0.0, (r.x1 - r.x0) as f64, style.height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(start_s: f64, end_s: f64, style: u8) -> TimeMark {
        TimeMark {
            start_s,
            end_s,
            style,
        }
    }

    fn run(x0: u32, x1: u32, style: u8) -> ColumnRun {
        ColumnRun { x0, x1, style }
    }

    #[test]
    fn a_call_shorter_than_a_pixel_fills_one_column() {
        // 100 s over 100 px: 1 s per column. A 2 ms call at 10.5 s.
        let runs = merge_columns(&[mark(10.5, 10.502, 0)], 0.0, 100.0, 100);
        assert_eq!(runs, [run(10, 11, 0)]);
    }

    #[test]
    fn many_calls_in_one_column_give_one_run() {
        let marks: Vec<_> = (0..1000)
            .map(|i| {
                mark(
                    20.0 + i as f64 * 0.0009,
                    20.0 + i as f64 * 0.0009 + 0.0005,
                    0,
                )
            })
            .collect();
        assert_eq!(merge_columns(&marks, 0.0, 100.0, 100), [run(20, 21, 0)]);
    }

    #[test]
    fn neighbouring_columns_with_the_same_style_join() {
        let marks = [mark(1.2, 1.8, 0), mark(2.1, 2.4, 0), mark(5.0, 5.5, 0)];
        assert_eq!(
            merge_columns(&marks, 0.0, 10.0, 10),
            [run(1, 3, 0), run(5, 6, 0)]
        );
    }

    #[test]
    fn the_lower_style_wins_a_shared_column() {
        // A train over columns 2 to 7 with a bat call in column 4.
        let marks = [mark(2.0, 8.0, TRAIN), mark(4.2, 4.3, BAT_CALL)];
        assert_eq!(
            merge_columns(&marks, 0.0, 10.0, 10),
            [run(2, 4, TRAIN), run(4, 5, BAT_CALL), run(5, 8, TRAIN)]
        );
        // The order of the marks does not matter.
        let reversed = [marks[1], marks[0]];
        assert_eq!(
            merge_columns(&reversed, 0.0, 10.0, 10),
            merge_columns(&marks, 0.0, 10.0, 10)
        );
    }

    #[test]
    fn marks_are_clipped_to_the_overview() {
        let marks = [
            mark(-5.0, 1.5, 0),  // starts before the window
            mark(9.5, 20.0, 1),  // ends after it
            mark(30.0, 31.0, 0), // after it
            mark(-3.0, -2.0, 0), // before it
        ];
        assert_eq!(
            merge_columns(&marks, 0.0, 10.0, 10),
            [run(0, 2, 0), run(9, 10, 1)]
        );
    }

    #[test]
    fn the_window_start_is_followed() {
        // A window showing 100 s to 110 s.
        let runs = merge_columns(&[mark(103.0, 104.0, 0)], 100.0, 10.0, 10);
        assert_eq!(runs, [run(3, 4, 0)]);
    }

    #[test]
    fn nothing_is_drawn_without_a_width_or_a_span() {
        let marks = [mark(1.0, 2.0, 0)];
        assert!(merge_columns(&marks, 0.0, 10.0, 0).is_empty());
        assert!(merge_columns(&marks, 0.0, 0.0, 10).is_empty());
    }

    #[test]
    fn inbuilt_marks_rank_bat_calls_over_trains_over_other_detections() {
        use crate::dsp::call_detect::{group_trains, CallEvent, TrainParams};
        let call = |at_s: f64, kind: CallKind| DetectedCall {
            event: CallEvent {
                start_sample: 0,
                end_sample: 0,
                peak_freq_hz: 45_000.0,
                peak_db_above_floor: 30.0,
                audible_db_above_floor: -3.0,
                min_freq_hz: 43_000.0,
                max_freq_hz: 55_000.0,
            },
            start_s: at_s,
            end_s: at_s + 0.005,
            kind,
            contour: Vec::new(),
            steady_s: 0.0,
        };
        let calls = vec![
            call(1.0, CallKind::Bat),
            call(1.1, CallKind::Bat),
            call(1.2, CallKind::Bat),
            call(5.0, CallKind::Tone),
        ];
        let trains = group_trains(&calls, &TrainParams::default());
        let m = inbuilt_marks(&calls, &trains);
        let styles: Vec<u8> = m.marks.iter().map(|m| m.style).collect();
        assert_eq!(styles[..4], [BAT_CALL, BAT_CALL, BAT_CALL, OTHER]);
        assert_eq!(styles[4..], vec![TRAIN; trains.len()][..]);
        assert!(m.styles[usize::from(BAT_CALL)].height > m.styles[usize::from(TRAIN)].height);
    }

    #[test]
    fn classifier_marks_use_the_species_colour_slots() {
        let span = |class: u16, start_s: f64| Span {
            class,
            start_s,
            end_s: start_s + 3.0,
            best: 0.9,
            windows: 1,
        };
        // Colour order: class 7, then 3, 5, 9.
        let order = [7, 3, 5, 9];
        let m = classifier_marks(&[span(3, 0.0), span(9, 3.0), span(7, 6.0)], &order);
        let styles: Vec<u8> = m.marks.iter().map(|m| m.style).collect();
        // Class 9 is the fourth species, so it takes the shared grey.
        assert_eq!(styles, [1, 3, 0]);
        assert_eq!(m.styles.len(), 4);
        assert_eq!(
            m.styles[0].colour,
            crate::canvas::overlays::species_colour(0).0
        );
        assert_eq!(m.styles[3].colour, crate::canvas::overlays::SPECIES_OTHER.0);
    }
}
