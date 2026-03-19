//! Automatic Gain Control (AGC) — smooth leveler with limiter.
//!
//! Replaces the old per-chunk adaptive gain with smooth, sample-level
//! envelope following. The processor maintains state across chunks for
//! seamless transitions.
//!
//! Design: a leveler that targets a specific output level (-3 dBFS by default).
//! Quiet signals get boosted, loud signals get reduced, with smooth attack/release
//! envelope following. A noise gate prevents boosting silence, and a hard limiter
//! prevents clipping.

/// Configuration for the AGC processor.
#[derive(Clone, Copy, Debug)]
pub struct AgcConfig {
    /// Target output level in dBFS. The AGC tries to keep the envelope near this level.
    pub target_db: f64,
    /// Maximum boost in dB. Prevents extreme amplification of very quiet signals.
    pub max_boost_db: f64,
    /// Maximum cut in dB. Limits gain reduction for very loud signals.
    pub max_cut_db: f64,
    /// Attack time in milliseconds — how fast gain reduces when signal gets louder.
    pub attack_ms: f64,
    /// Release time in milliseconds — how fast gain increases when signal gets quieter.
    pub release_ms: f64,
    /// Noise gate threshold in dBFS. Below this, gain ramps to 0 (avoid amplifying silence).
    pub gate_threshold_db: f64,
    /// Limiter ceiling in dBFS. Hard limit on output level.
    pub limiter_ceiling_db: f64,
}

impl Default for AgcConfig {
    fn default() -> Self {
        Self {
            target_db: -3.0,
            max_boost_db: 60.0,
            max_cut_db: 20.0,
            attack_ms: 5.0,
            release_ms: 100.0,
            gate_threshold_db: -50.0,
            limiter_ceiling_db: -1.0,
        }
    }
}

/// Stateful AGC processor. Persists across audio chunks for smooth gain transitions.
pub struct AgcProcessor {
    config: AgcConfig,
    /// Smoothed envelope level in dB.
    envelope_db: f64,
    /// Smoothed gain in dB (separate smoothing from envelope for stability).
    smooth_gain_db: f64,
    /// Exponential smoothing coefficient for attack (envelope rises).
    attack_coeff: f64,
    /// Exponential smoothing coefficient for release (envelope falls).
    release_coeff: f64,
    /// Coefficient for gain smoothing (slower than envelope to avoid pumping).
    gain_smooth_coeff: f64,
    /// Linear limiter ceiling.
    ceiling_linear: f32,
}

impl AgcProcessor {
    pub fn new(config: AgcConfig, sample_rate: u32) -> Self {
        let rate = sample_rate as f64;
        let attack_coeff = (-1.0 / (config.attack_ms * 0.001 * rate)).exp();
        let release_coeff = (-1.0 / (config.release_ms * 0.001 * rate)).exp();
        // Gain changes are smoothed more slowly than the envelope to reduce pumping
        let gain_smooth_ms = config.release_ms * 2.0;
        let gain_smooth_coeff = (-1.0 / (gain_smooth_ms * 0.001 * rate)).exp();
        let ceiling_linear = 10.0_f64.powf(config.limiter_ceiling_db / 20.0) as f32;

        Self {
            config,
            envelope_db: -100.0,
            smooth_gain_db: 0.0,
            attack_coeff,
            release_coeff,
            gain_smooth_coeff,
            ceiling_linear,
        }
    }

    /// Process samples in-place with AGC (leveler + limiter).
    pub fn process(&mut self, samples: &mut [f32]) {
        let target = self.config.target_db;
        let max_boost = self.config.max_boost_db;
        let max_cut = self.config.max_cut_db;
        let gate = self.config.gate_threshold_db;
        let attack = self.attack_coeff;
        let release = self.release_coeff;
        let gain_smooth = self.gain_smooth_coeff;
        let ceiling = self.ceiling_linear;

        for sample in samples.iter_mut() {
            // Detect instantaneous level
            let abs_val = sample.abs();
            let instant_db = if abs_val > 1e-10 {
                20.0 * (abs_val as f64).log10()
            } else {
                -100.0
            };

            // Smooth envelope (peak follower with attack/release)
            if instant_db > self.envelope_db {
                self.envelope_db = attack * self.envelope_db + (1.0 - attack) * instant_db;
            } else {
                self.envelope_db = release * self.envelope_db + (1.0 - release) * instant_db;
            }

            // Compute desired gain to bring envelope to target
            let desired_gain_db = if self.envelope_db < gate {
                // Below noise gate: ramp gain to 0 (don't amplify silence/noise)
                0.0
            } else {
                (target - self.envelope_db).clamp(-max_cut, max_boost)
            };

            // Smooth the gain changes to reduce pumping
            self.smooth_gain_db = gain_smooth * self.smooth_gain_db
                + (1.0 - gain_smooth) * desired_gain_db;

            // Apply gain
            let gain_linear = 10.0_f64.powf(self.smooth_gain_db / 20.0);
            *sample = (*sample as f64 * gain_linear) as f32;

            // Hard limiter
            if *sample > ceiling {
                *sample = ceiling;
            } else if *sample < -ceiling {
                *sample = -ceiling;
            }
        }
    }

    /// Process stereo channels with linked envelope detection.
    /// Uses the louder of the two channels for envelope/gain computation,
    /// then applies the same gain to both — preserving the stereo image
    /// and avoiding cross-channel pumping.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        let target = self.config.target_db;
        let max_boost = self.config.max_boost_db;
        let max_cut = self.config.max_cut_db;
        let gate = self.config.gate_threshold_db;
        let attack = self.attack_coeff;
        let release = self.release_coeff;
        let gain_smooth = self.gain_smooth_coeff;
        let ceiling = self.ceiling_linear;

        let len = left.len().min(right.len());
        for i in 0..len {
            // Detect level from the louder channel
            let abs_val = left[i].abs().max(right[i].abs());
            let instant_db = if abs_val > 1e-10 {
                20.0 * (abs_val as f64).log10()
            } else {
                -100.0
            };

            // Smooth envelope
            if instant_db > self.envelope_db {
                self.envelope_db = attack * self.envelope_db + (1.0 - attack) * instant_db;
            } else {
                self.envelope_db = release * self.envelope_db + (1.0 - release) * instant_db;
            }

            // Compute desired gain
            let desired_gain_db = if self.envelope_db < gate {
                0.0
            } else {
                (target - self.envelope_db).clamp(-max_cut, max_boost)
            };

            // Smooth gain changes
            self.smooth_gain_db = gain_smooth * self.smooth_gain_db
                + (1.0 - gain_smooth) * desired_gain_db;

            // Apply same gain to both channels
            let gain_linear = 10.0_f64.powf(self.smooth_gain_db / 20.0);
            left[i] = (left[i] as f64 * gain_linear) as f32;
            right[i] = (right[i] as f64 * gain_linear) as f32;

            // Hard limiter on both
            left[i] = left[i].clamp(-ceiling, ceiling);
            right[i] = right[i].clamp(-ceiling, ceiling);
        }
    }
}
