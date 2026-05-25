// Re-export all DSP modules from oversample-core.
// Individual module re-exports so `crate::dsp::fft`, `crate::dsp::filters`, etc. work.
pub use oversample_core::dsp::{
    agc, bit_analysis, fft, filters, harmonics, heterodyne, notch,
    phase_vocoder, pitch_shift, spectral_sub, zc_divide, wsnr,
    zero_crossing, chromagram, psd, pulse_detect, resonators,
    lsb_autocorr, pipistrelle,
};
