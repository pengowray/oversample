use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::audio::source::DEFAULT_ANALYSIS_WINDOW_SECS;
use crate::state::{AppState, RightSidebarTab};
use crate::dsp::bit_analysis::{self, BitAnalysis, BitCaution};
use crate::dsp::wsnr;
use std::sync::Arc;

#[component]
pub(crate) fn AnalysisPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Async analysis results — None means "not yet computed" or "computing"
    let analysis: RwSignal<Option<BitAnalysis>> = RwSignal::new(None);
    let wsnr_result: RwSignal<Option<wsnr::WsnrResult>> = RwSignal::new(None);
    let is_computing = RwSignal::new(false);
    let last_computed_idx: RwSignal<Option<usize>> = RwSignal::new(None);
    let compute_gen = RwSignal::new(0u32);
    // Whether the last analysis used full file (true) or first 30s (false).
    let analysis_is_full = RwSignal::new(false);
    // Whether the file is longer than the analysis window.
    let file_is_long = RwSignal::new(false);

    // Run analysis (default: first 30s; full_file=true for full scan)
    let run_analysis = move |full_file: bool| {
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let file = idx.and_then(|i| files.get(i).cloned());
        let Some(file) = file else { return; };

        analysis.set(None);
        wsnr_result.set(None);
        is_computing.set(true);
        last_computed_idx.set(idx);
        analysis_is_full.set(full_file);
        compute_gen.update(|g| *g += 1);
        let generation = compute_gen.get_untracked();

        let all_samples = file.audio.samples.clone(); // Arc clone, O(1)
        let sample_rate = file.audio.sample_rate;
        let bits_per_sample = file.audio.metadata.bits_per_sample;
        let is_float = file.audio.metadata.is_float;

        let max_samples = (DEFAULT_ANALYSIS_WINDOW_SECS * sample_rate as f64) as usize;
        let is_long = all_samples.len() > max_samples;
        file_is_long.set(is_long);

        let samples: Arc<Vec<f32>> = if full_file || !is_long {
            analysis_is_full.set(true);
            all_samples
        } else {
            Arc::new(all_samples[..max_samples].to_vec())
        };
        let duration_secs = samples.len() as f64 / sample_rate as f64;

        spawn_local(async move {
            yield_to_browser().await;
            if compute_gen.get_untracked() != generation { return; }

            let bits_result = bit_analysis::analyze_bits(
                &samples, bits_per_sample, is_float, duration_secs,
            );
            if compute_gen.get_untracked() != generation { return; }
            analysis.set(Some(bits_result));

            yield_to_browser().await;
            if compute_gen.get_untracked() != generation { return; }

            let wsnr_res = wsnr::analyze_wsnr(&samples, sample_rate);
            if compute_gen.get_untracked() != generation { return; }
            wsnr_result.set(Some(wsnr_res));

            is_computing.set(false);
        });
    };

    // Only compute expensive analysis when the Analysis tab is active
    Effect::new(move || {
        let tab = state.right_sidebar_tab.get();
        let _files = state.files.get();
        let idx = state.current_file_index.get();

        if tab != RightSidebarTab::Analysis {
            return;
        }

        // Already computed for this file
        if idx == last_computed_idx.get_untracked() && analysis.get_untracked().is_some() {
            return;
        }

        let files = state.files.get_untracked();
        let file = idx.and_then(|i| files.get(i).cloned());
        if file.is_none() {
            analysis.set(None);
            wsnr_result.set(None);
            last_computed_idx.set(None);
            is_computing.set(false);
            return;
        }

        run_analysis(false);
    });

    let xc_quality = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        idx.and_then(|i| files.get(i).cloned())
            .and_then(|file| file.xc_metadata)
            .and_then(|meta| {
                meta.iter()
                    .find(|(k, _)| k == "Quality")
                    .map(|(_, v)| v.clone())
            })
    });

    let report_text = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let file = idx.and_then(|i| files.get(i).cloned());

        let mut report = "=== Audio Analysis ===\n".to_string();

        if let Some(ref f) = file {
            let meta = &f.audio.metadata;
            let sr = f.audio.sample_rate;
            let sr_text = if sr % 1000 == 0 {
                format!("{} kHz", sr / 1000)
            } else {
                format!("{:.1} kHz", sr as f64 / 1000.0)
            };
            let ch_text = match f.audio.channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                n => format!("{} ch", n),
            };
            let bit_text = if meta.is_float {
                format!("{}-bit float", meta.bits_per_sample)
            } else {
                format!("{}-bit", meta.bits_per_sample)
            };
            let total_samples = f.audio.samples.len();
            let dur_text = format!("{:.3} s", f.audio.duration_secs);
            report.push_str(&format!(
                "\nFile\n  Sample rate: {}\n  Channels: {}\n  Bit depth: {}\n  Duration: {}\n  Samples: {}\n",
                sr_text, ch_text, bit_text, dur_text, total_samples
            ));

            // Signal stats — scan first 30s only for large files
            let max_scan = (DEFAULT_ANALYSIS_WINDOW_SECS * f.audio.sample_rate as f64) as usize;
            let scan_len = f.audio.samples.len().min(max_scan);
            let smp = &f.audio.samples[..scan_len];
            let len = smp.len();
            if len > 0 {
                let mut smin = f32::INFINITY;
                let mut smax = f32::NEG_INFINITY;
                let mut sum = 0.0f64;
                let mut sum_sq = 0.0f64;
                for &s in smp.iter() {
                    if s < smin { smin = s; }
                    if s > smax { smax = s; }
                    sum += s as f64;
                    sum_sq += (s as f64) * (s as f64);
                }
                let dc_bias = sum / len as f64;
                let rms = (sum_sq / len as f64).sqrt();
                let min_db = if smin.abs() > 0.0 { format!("{:.1} dB", 20.0 * (smin.abs() as f64).log10()) } else { "-\u{221e} dB".into() };
                let max_db = if smax.abs() > 0.0 { format!("{:.1} dB", 20.0 * (smax.abs() as f64).log10()) } else { "-\u{221e} dB".into() };
                let rms_db = if rms > 0.0 { format!("{:.1} dB", 20.0 * rms.log10()) } else { "-\u{221e} dB".into() };
                let dc_db = if dc_bias.abs() > 0.0 { format!("{:.1} dB", 20.0 * dc_bias.abs().log10()) } else { "-\u{221e} dB".into() };
                report.push_str(&format!(
                    "\nSignal\n  Min: {:.4} ({})\n  Max: {:.4} ({})\n  RMS: {}\n  DC bias: {}\n",
                    smin, min_db, smax, max_db, rms_db, dc_db
                ));
            }
        }

        // wSNR
        if let Some(ref w) = wsnr_result.get() {
            let grade = w.grade.label();
            report.push_str(&format!(
                "\nRecording Quality (wSNR): {}\n  SNR: {:.1} dB(ISO/ITU)\n  Signal: {:.1} dB (ISO 226)\n  Noise: {:.1} dB (ITU-R 468)\n",
                grade, w.snr_db, w.signal_db, w.noise_db
            ));
            if let Some(xc) = xc_quality.get() {
                report.push_str(&format!("  XC quality (metadata): {}\n", xc.trim()));
            }
            for msg in &w.warnings {
                report.push_str(&format!("  \u{26a0} {}\n", msg));
            }
        }

        // Bit analysis
        if let Some(ref a) = analysis.get() {
            let total = a.total_samples;
            let pos_total = a.positive_total;
            let neg_total = a.negative_total;
            let zero_total = a.zero_total;
            let non_silent = pos_total + neg_total;
            let is_asymmetric = non_silent > 0 && a.bit_cautions.first()
                .map(|c| c.iter().any(|x| matches!(x, BitCaution::SignBitSkewed)))
                .unwrap_or(false);

            let pos_pct = if total > 0 { format!("{:.0}%", pos_total as f64 / total as f64 * 100.0) } else { "0%".into() };
            let neg_pct = if total > 0 { format!("{:.0}%", neg_total as f64 / total as f64 * 100.0) } else { "0%".into() };
            let zero_pct = if total > 0 { format!("{:.0}%", zero_total as f64 / total as f64 * 100.0) } else { "0%".into() };
            let split_label = if is_asymmetric { "Asymmetric" } else { "Sample split" };
            let split_line = format!("  {}: {}+ {}− {}silence\n", split_label, pos_pct, neg_pct, zero_pct);

            report.push_str("\nBit Usage\n");

            if !a.is_float {
                let zero_padding = a.bits_per_sample - a.effective_bits;
                let effective_depth = a.bits_per_sample.saturating_sub(a.headroom_bits).saturating_sub(zero_padding);
                let headroom_db = a.headroom_bits as f64 * 20.0 * 2f64.log10();
                report.push_str(&format!("  Effective bit depth: {} bits\n", effective_depth));
                report.push_str(&format!("  Entropy estimate: ~{:.1} bits\n", a.effective_bits_f64));
                if a.headroom_bits > 0 {
                    report.push_str(&format!("  Headroom: {} bits ({:.1} dB)\n", a.headroom_bits, headroom_db));
                }
                if zero_padding > 0 {
                    report.push_str(&format!("  Zero padding: {} bits\n", zero_padding));
                }
                report.push_str(&format!("  {}\n", a.summary));
                if let Some(ref vc) = a.value_coverage {
                    report.push_str(&format!("  Value coverage: {:.1}% ({} of {})\n",
                        vc.coverage_pct, vc.unique_count, vc.value_space));
                    let ceiled = vc.resolution_bits.ceil() as u16;
                    report.push_str(&format!("  Value resolution: ~{:.1} bits ({}-bit)\n", vc.resolution_bits, ceiled));
                }
            } else {
                report.push_str(&format!("  {}\n", a.summary));
                report.push_str(&format!("  Entropy estimate: ~{:.1} bits\n", a.effective_bits_f64));
            }
            {
                let nf_db = a.noise_floor_db;
                let nf_bits = -nf_db / (20.0 * 2f64.log10());
                report.push_str(&format!("  Noise floor: {:.1} dBFS (~{:.1} bits)\n", nf_db, nf_bits));
            }
            report.push_str(&split_line);

            for w in &a.warnings {
                report.push_str(&format!("  ! {}\n", w));
            }
            let caution_list: Vec<String> = a.bit_cautions.iter().enumerate()
                .filter(|(_, cs)| !cs.is_empty())
                .map(|(i, cs)| {
                    let label = bit_analysis::bit_label(i, a.bits_per_sample, a.is_float);
                    let names: Vec<&str> = cs.iter().map(|c| match c {
                        BitCaution::SignBitSkewed => "asymmetric distribution",
                        BitCaution::Always1 => "always 1",
                        BitCaution::OnlyInFade => "only in fade",
                        BitCaution::VeryLowUsage => "very low usage",
                    }).collect();
                    format!("{} ({})", label, names.join(", "))
                })
                .collect();
            if !caution_list.is_empty() {
                report.push_str(&format!("  Cautions: {}\n", caution_list.join("; ")));
            }
        }

        report
    });

    view! {
        <div class="sidebar-panel">
            // Copy report button
            {move || {
                let has_file = {
                    let files = state.files.get();
                    let idx = state.current_file_index.get();
                    idx.and_then(|i| files.get(i)).is_some()
                };
                if has_file {
                    let text = report_text.get();
                    let on_copy = move |_: web_sys::MouseEvent| {
                        super::copy_to_clipboard(&text);
                    };
                    view! {
                        <div class="copy-report-row">
                            <button class="copy-report-btn" on:click=on_copy title="Copy full analysis report to clipboard">"Copy report"</button>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
            // File info + signal stats
            {move || {
                let files = state.files.get();
                let idx = state.current_file_index.get();
                let file = idx.and_then(|i| files.get(i).cloned());
                match file.as_ref() {
                    None => view! {
                        <div class="sidebar-panel-empty">"No file selected"</div>
                    }.into_any(),
                    Some(f) => {
                        let meta = &f.audio.metadata;
                        let sr = f.audio.sample_rate;
                        let sr_text = if sr % 1000 == 0 {
                            format!("{} kHz", sr / 1000)
                        } else {
                            format!("{:.1} kHz", sr as f64 / 1000.0)
                        };
                        let ch_text = match f.audio.channels {
                            1 => "Mono".to_string(),
                            2 => "Stereo".to_string(),
                            n => format!("{} ch", n),
                        };
                        let bit_text = if meta.is_float {
                            format!("{}-bit float", meta.bits_per_sample)
                        } else {
                            format!("{}-bit", meta.bits_per_sample)
                        };
                        let total_samples = f.audio.samples.len();
                        let dur_text = format!("{:.3} s", f.audio.duration_secs);
                        let samples_text = format!("{}", total_samples);

                        // Signal stats — scan first 30s only for large files
                        let max_scan = (DEFAULT_ANALYSIS_WINDOW_SECS * f.audio.sample_rate as f64) as usize;
                        let scan_len = f.audio.samples.len().min(max_scan);
                        let samples = &f.audio.samples[..scan_len];
                        let len = samples.len();
                        let (sig_min, sig_max, dc_bias, rms) = if len > 0 {
                            let mut smin = f32::INFINITY;
                            let mut smax = f32::NEG_INFINITY;
                            let mut sum = 0.0f64;
                            let mut sum_sq = 0.0f64;
                            for &s in samples.iter() {
                                if s < smin { smin = s; }
                                if s > smax { smax = s; }
                                sum += s as f64;
                                sum_sq += (s as f64) * (s as f64);
                            }
                            (smin, smax, sum / len as f64, (sum_sq / len as f64).sqrt())
                        } else {
                            (0.0f32, 0.0f32, 0.0f64, 0.0f64)
                        };
                        let min_db = if sig_min.abs() > 0.0 { format!("{:.1} dB", 20.0 * (sig_min.abs() as f64).log10()) } else { "-\u{221E} dB".into() };
                        let max_db = if sig_max.abs() > 0.0 { format!("{:.1} dB", 20.0 * (sig_max.abs() as f64).log10()) } else { "-\u{221E} dB".into() };
                        let rms_db = if rms > 0.0 { format!("{:.1} dB", 20.0 * rms.log10()) } else { "-\u{221E} dB".into() };
                        let dc_db = if dc_bias.abs() > 0.0 { format!("{:.1} dB", 20.0 * dc_bias.abs().log10()) } else { "-\u{221E} dB".into() };
                        let dc_raw_tooltip = format!("{:.6} (raw)", dc_bias);
                        // DC relative to RMS: gives perceptual sense of DC severity
                        let dc_rms_ratio = if rms > 0.0 { dc_bias.abs() / rms } else { 0.0 };
                        // Warning: notable DC if |dc| > 1% of full scale OR dc/rms > 5%, gated on N
                        let dc_notable = len > 10_000 && (dc_bias.abs() > 0.01 || dc_rms_ratio > 0.05);
                        let dc_warning = if dc_notable {
                            Some(format!("DC offset: {} \u{2014} {:.0}% of RMS level", dc_db, dc_rms_ratio * 100.0))
                        } else {
                            None
                        };

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"File"</div>
                                <div class="analysis-stats">
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{sr_text}</span>
                                        <span class="analysis-stat-label">"Sample rate"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{ch_text}</span>
                                        <span class="analysis-stat-label">"Channels"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{bit_text}</span>
                                        <span class="analysis-stat-label">"Bit depth"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{dur_text}</span>
                                        <span class="analysis-stat-label">"Duration"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{samples_text}</span>
                                        <span class="analysis-stat-label">"Samples"</span>
                                    </div>
                                </div>
                            </div>
                            <div class="setting-group">
                                <div class="setting-group-title">"Signal"</div>
                                <div class="analysis-stats">
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{format!("{:.4}", sig_min)}</span>
                                        <span class="analysis-stat-label" title=min_db>"Min"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{format!("{:.4}", sig_max)}</span>
                                        <span class="analysis-stat-label" title=max_db>"Max"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{rms_db}</span>
                                        <span class="analysis-stat-label">"RMS"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{dc_db}</span>
                                        <span class="analysis-stat-label" title=dc_raw_tooltip>"DC bias"</span>
                                    </div>
                                </div>
                                {dc_warning.map(|w| view! { <div class="analysis-warning">{w}</div> })}
                            </div>
                        }.into_any()
                    }
                }
            }}
            // Computing indicator + analysis scope badge
            {move || {
                if is_computing.get() {
                    view! { <div class="sidebar-panel-empty">"Computing analysis\u{2026}"</div> }.into_any()
                } else if file_is_long.get() && !analysis_is_full.get() && analysis.get().is_some() {
                    view! {
                        <div class="analysis-scope-row">
                            <span class="analysis-scope-badge">"First 30s"</span>
                            <button
                                class="analysis-full-btn"
                                on:click=move |_| run_analysis(true)
                            >
                                "Analyze full file"
                            </button>
                        </div>
                    }.into_any()
                } else if analysis_is_full.get() && file_is_long.get() && analysis.get().is_some() {
                    view! {
                        <div class="analysis-scope-row">
                            <span class="analysis-scope-badge analysis-scope-full">"Full file"</span>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
            // wSNR section
            {move || {
                match wsnr_result.get().as_ref() {
                    None => view! { <span></span> }.into_any(),
                    Some(w) => {
                        let grade_class = match w.grade {
                            wsnr::WsnrGrade::A => "wsnr-grade wsnr-grade-a",
                            wsnr::WsnrGrade::B => "wsnr-grade wsnr-grade-b",
                            wsnr::WsnrGrade::C => "wsnr-grade wsnr-grade-c",
                            wsnr::WsnrGrade::D => "wsnr-grade wsnr-grade-d",
                            wsnr::WsnrGrade::E => "wsnr-grade wsnr-grade-e",
                        };
                        let grade_label = w.grade.label().to_string();
                        let snr_text = format!("{:.1} dB(ISO/ITU)", w.snr_db);
                        let signal_text = format!("Signal: {:.1} dB (ISO 226)", w.signal_db);
                        let noise_text = format!("Noise: {:.1} dB (ITU-R 468)", w.noise_db);

                        let xc_comparison = xc_quality.get().map(|xc_q| {
                            let xc_letter = xc_q.trim().to_uppercase();
                            let xc_badge_class = match xc_letter.as_str() {
                                "A" => "wsnr-grade-sm wsnr-grade-a",
                                "B" => "wsnr-grade-sm wsnr-grade-b",
                                "C" => "wsnr-grade-sm wsnr-grade-c",
                                "D" => "wsnr-grade-sm wsnr-grade-d",
                                _ => "wsnr-grade-sm wsnr-grade-e",
                            };
                            //let computed = grade_label.clone();
                            let note = "";
                            /*
                            let note = if xc_letter == computed {
                                "(matches)".to_string()
                            } else {
                                format!("(estimated: {})", computed)
                            };
                            */
                            (xc_letter, xc_badge_class.to_string(), note)
                        });

                        let warnings: Vec<_> = w.warnings.iter().map(|msg| {
                            let msg = msg.clone();
                            view! { <div class="wsnr-warning">{msg}</div> }
                        }).collect();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Recording Quality"</div>
                                <div class="wsnr-result">
                                    <div class="wsnr-detail">Estimated wSNR:</div>
                                    <div class="wsnr-header">
                                        <span class=grade_class>{grade_label}</span>
                                        <span class="wsnr-snr">{snr_text}</span>
                                    </div>
                                    <div class="wsnr-detail">{signal_text}</div>
                                    <div class="wsnr-detail">{noise_text}</div>
                                    {if !warnings.is_empty() {
                                        view! { <div class="wsnr-warnings">{warnings}</div> }.into_any()
                                    } else {
                                        view! { <span></span> }.into_any()
                                    }}
                                    {xc_comparison.map(|(letter, badge_class, note)| view! {
                                        <div class="wsnr-comparison">
                                            "Metadata: "
                                            <span class=badge_class>{letter}</span>
                                            " (XC grade)" {note}
                                        </div>
                                    })}
                                </div>
                            </div>
                        }.into_any()
                    }
                }
            }}
            // Bit analysis section
            {move || {
                match analysis.get().as_ref() {
                    None => view! { <span></span> }.into_any(),
                    Some(a) => {
                        let bits = a.bits_per_sample as usize;
                        let cols = 4usize;
                        let total = a.total_samples;
                        let summary = a.summary.clone();
                        let warnings = a.warnings.clone();
                        let is_float = a.is_float;
                        let bits_per_sample = a.bits_per_sample;
                        let effective_bits = a.effective_bits;
                        let effective_bits_f64 = a.effective_bits_f64;
                        let headroom_bits = a.headroom_bits;
                        let noise_floor_db = a.noise_floor_db;
                        let value_coverage = a.value_coverage.clone();

                        // Positive/negative/zero split grids
                        let pos_total = a.positive_total;
                        let neg_total = a.negative_total;
                        let zero_total = a.zero_total;
                        let pos_counts = a.positive_counts.clone();
                        let neg_counts = a.negative_counts.clone();

                        let non_silent = pos_total + neg_total;
                        let is_asymmetric = non_silent > 0 && a.bit_cautions.first()
                            .map(|c| c.iter().any(|x| matches!(x, BitCaution::SignBitSkewed)))
                            .unwrap_or(false);

                        let pos_pct_f = if total > 0 { pos_total as f64 / total as f64 * 100.0 } else { 0.0 };
                        let neg_pct_f = if total > 0 { neg_total as f64 / total as f64 * 100.0 } else { 0.0 };
                        let zero_pct_f = if total > 0 { zero_total as f64 / total as f64 * 100.0 } else { 0.0 };
                        let pos_pct = format!("{:.0}%", pos_pct_f);
                        let neg_pct = format!("{:.0}%", neg_pct_f);
                        let zero_pct = format!("{:.0}%", zero_pct_f);
                        let pos_tooltip = format!("{} samples above 0", pos_total);
                        let neg_tooltip = format!("{} samples below 0", neg_total);
                        let zero_tooltip = format!("{} samples exactly zero", zero_total);

                        // Sample distribution line — always shown; prefixed "Asymmetric:" if skewed
                        let split_label = if is_asymmetric { "Asymmetric:" } else { "Sample split:" };
                        let split_text = format!("{} {} +, {} \u{2212}, {} silence",
                            split_label, pos_pct, neg_pct, zero_pct);
                        let split_tooltip = format!("{} positive, {} negative, {} silence",
                            pos_total, neg_total, zero_total);

                        // Integer-only bit depth stats (computed once for both display and ordering)
                        let zero_padding = if !is_float { bits_per_sample - effective_bits } else { 0 };
                        let effective_depth = if !is_float {
                            bits_per_sample.saturating_sub(headroom_bits).saturating_sub(zero_padding)
                        } else {
                            0
                        };
                        let headroom_db = headroom_bits as f64 * 20.0 * 2f64.log10();

                        let entropy_text = format!("Entropy estimate: ~{:.1} bits", effective_bits_f64);
                        let entropy_tooltip = format!("Estimated effective bit depth (Shannon entropy sum); nominal: {}-bit", bits_per_sample);

                        let warning_items: Vec<_> = warnings.iter().map(|w| {
                            let w = w.clone();
                            view! { <div class="bit-warning">{w}</div> }
                        }).collect();

                        let make_sign_grid = |sign_counts: &[usize], sign_total: usize, polarity: &str| -> Vec<_> {
                            (0..bits).map(|idx| {
                                let count = sign_counts[idx];
                                let label = bit_analysis::bit_label(idx, bits_per_sample, is_float);
                                let is_sign_bit = idx == 0;
                                // Sign bit is always 0% or 100% by definition — keep grey
                                if is_sign_bit {
                                    let value_text = if sign_total > 0 && count == sign_total {
                                        "100%".to_string()
                                    } else if sign_total > 0 {
                                        "0%".to_string()
                                    } else {
                                        "\u{2013}".to_string()
                                    };
                                    let sign_tooltip = if polarity == "positive" {
                                        "Sign bit: always 0 for positive samples".to_string()
                                    } else {
                                        "Sign bit: always 1 for negative samples".to_string()
                                    };
                                    return view! {
                                        <div class="bit-cell unused" title=sign_tooltip>
                                            <span class="bit-label">{label}</span>
                                            <span class="bit-value">{value_text}</span>
                                        </div>
                                    };
                                }
                                let used = count > 0;
                                // Zero-count non-sign bits are zero-padded (red); used bits get normal coloring
                                let cell_class = if sign_total > 0 && count == sign_total {
                                    "bit-cell used full"
                                } else if used {
                                    "bit-cell used"
                                } else {
                                    "bit-cell zero-padded"
                                };
                                let value_text = if count == 0 {
                                    "\u{2013}".to_string()
                                } else if sign_total > 0 {
                                    let pct = count as f64 / sign_total as f64 * 100.0;
                                    if count == sign_total {
                                        "100%".to_string()
                                    } else if pct >= 99.9 {
                                        "~100%".to_string()
                                    } else if pct >= 1.0 {
                                        format!("{:.0}%", pct)
                                    } else if count > 99 {
                                        "99+".into()
                                    } else {
                                        format!("{}", count)
                                    }
                                } else {
                                    "\u{2013}".to_string()
                                };
                                let tooltip = if sign_total > 0 {
                                    let pct = count as f64 / sign_total as f64 * 100.0;
                                    let missing = sign_total - count;
                                    if missing > 0 && pct >= 99.5 {
                                        format!("Bit {} is set in {} / {} {} samples ({:.1}%) — all but {}", label, count, sign_total, polarity, pct, missing)
                                    } else {
                                        format!("Bit {} is set in {} / {} {} samples ({:.1}%)", label, count, sign_total, polarity, pct)
                                    }
                                } else {
                                    format!("Bit {}: no {} samples", label, polarity)
                                };
                                view! {
                                    <div class=cell_class title=tooltip>
                                        <span class="bit-label">{label}</span>
                                        <span class="bit-value">{value_text}</span>
                                    </div>
                                }
                            }).collect()
                        };

                        let pos_grid = make_sign_grid(&pos_counts, pos_total, "positive");
                        let neg_grid = make_sign_grid(&neg_counts, neg_total, "negative");

                        let nf_bits = -noise_floor_db / (20.0 * 2f64.log10());
                        let noise_floor_text = format!("Noise floor: {:.1} dBFS (~{:.1} bits)", noise_floor_db, nf_bits);
                        let noise_floor_tooltip = "Minimum RMS level of 512-sample windows above digital silence (−80 dBFS); converted to equivalent bit depth at 6 dB/bit".to_string();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Bit Usage"</div>
                                // Stats block at top — effective depth first, then breakdown
                                {if !is_float {
                                    let summary_class = if effective_bits < bits_per_sample { "bit-warning" } else { "bit-depth-stat" };
                                    view! {
                                        <div>
                                            <div class="bit-depth-stat bit-depth-primary">{format!("Effective bit depth: {} bits", effective_depth)}</div>
                                            <div class="bit-depth-stat" title=entropy_tooltip>{entropy_text}</div>
                                            {if headroom_bits > 0 {
                                                view! { <div class="bit-depth-stat">{format!("Headroom: {} bit{} ({:.1} dB)", headroom_bits, if headroom_bits == 1 { "" } else { "s" }, headroom_db)}</div> }.into_any()
                                            } else { view! { <span></span> }.into_any() }}
                                            {if zero_padding > 0 {
                                                view! { <div class="bit-depth-stat">{format!("Zero padding: {} bit{}", zero_padding, if zero_padding == 1 { "" } else { "s" })}</div> }.into_any()
                                            } else { view! { <span></span> }.into_any() }}
                                            {match value_coverage {
                                                Some(ref vc) => {
                                                    let coverage_text = format!("Value coverage: {:.1}% ({} of {})",
                                                        vc.coverage_pct, vc.unique_count, vc.value_space);
                                                    let coverage_tooltip = format!("{} distinct sample values observed out of {} possible for {}-bit audio",
                                                        vc.unique_count, vc.value_space, bits_per_sample);
                                                    let resolution_text = format!("Value resolution: ~{:.1} bits ", vc.resolution_bits);
                                                    let resolution_tooltip = format!("log\u{2082}({}) = {:.2} — equivalent bit depth based on number of distinct values used",
                                                        vc.unique_count, vc.resolution_bits);
                                                    let ceiled = vc.resolution_bits.ceil() as u16;
                                                    let notable = (bits_per_sample == 16 && ceiled <= 12)
                                                        || (bits_per_sample == 24 && ceiled <= 16);
                                                    let suffix_text = format!("({}-bit)", ceiled);
                                                    let suffix_class = if notable { "bit-warning-inline" } else { "" };
                                                    view! {
                                                        <div>
                                                            <div class="bit-depth-stat" title=coverage_tooltip>{coverage_text}</div>
                                                            <div class="bit-depth-stat" title=resolution_tooltip>
                                                                {resolution_text}
                                                                <span class=suffix_class>{suffix_text}</span>
                                                            </div>
                                                        </div>
                                                    }.into_any()
                                                }
                                                None => view! { <span></span> }.into_any(),
                                            }}
                                            <div class=summary_class>{summary}</div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div>
                                            <div class="bit-depth-stat">{summary}</div>
                                            <div class="bit-depth-stat" title=entropy_tooltip>{entropy_text}</div>
                                        </div>
                                    }.into_any()
                                }}
                                <div class="bit-depth-stat" title=noise_floor_tooltip>{noise_floor_text}</div>
                                <div class=if is_asymmetric { "bit-warning" } else { "bit-depth-stat" } title=split_tooltip>{split_text}</div>
                                {warning_items}
                                <div class="bit-sign-header" title=pos_tooltip>{format!("Samples above zero ({})", pos_pct)}</div>
                                <div class="bit-grid" style=format!("grid-template-columns: repeat({}, 1fr);", cols)>
                                    {pos_grid}
                                </div>
                                <div class="bit-sign-header" title=neg_tooltip>{format!("Samples below zero ({})", neg_pct)}</div>
                                <div class="bit-grid" style=format!("grid-template-columns: repeat({}, 1fr);", cols)>
                                    {neg_grid}
                                </div>
                                <div class="bit-sign-header" title=zero_tooltip>{format!("Silence ({})", zero_pct)}</div>
                            </div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}

/// Yield once to the browser event loop via a zero-duration setTimeout.
async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = web_sys::window().unwrap();
        let cb = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.unchecked_ref(), 0,
        );
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
