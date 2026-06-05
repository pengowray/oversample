// Re-export all DSP modules from oversample-core so `crate::dsp::fft`,
// `crate::dsp::filters`, etc. resolve. Glob (not a hand-maintained list) so this
// shim can never drift from the core module list — the previous explicit list
// had already fallen behind (audiomoth + effective_nyquist were missing).
pub use oversample_core::dsp::*;
