//! Helpers for moving between interleaved stereo and separate channels.

/// Split interleaved L/R samples into separate channels. A trailing half frame
/// is dropped.
pub fn split_stereo(interleaved: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let frames = interleaved.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for frame in interleaved.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    (left, right)
}

/// Average two channels into one. The result is as long as the shorter channel.
pub fn mix_stereo(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter()
        .zip(right)
        .map(|(&l, &r)| (l + r) * 0.5)
        .collect()
}

/// Append two channels to `out` as interleaved L/R. Stops at the end of the
/// shorter channel.
pub fn interleave_stereo(left: &[f32], right: &[f32], out: &mut Vec<f32>) {
    out.reserve(left.len().min(right.len()) * 2);
    for (&l, &r) in left.iter().zip(right) {
        out.push(l);
        out.push(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_stereo_separates_channels() {
        let (l, r) = split_stereo(&[1.0, -1.0, 2.0, -2.0]);
        assert_eq!(l, vec![1.0, 2.0]);
        assert_eq!(r, vec![-1.0, -2.0]);
    }

    #[test]
    fn split_stereo_drops_a_trailing_half_frame() {
        let (l, r) = split_stereo(&[1.0, -1.0, 2.0]);
        assert_eq!(l, vec![1.0]);
        assert_eq!(r, vec![-1.0]);
    }

    #[test]
    fn interleave_stereo_appends_left_then_right() {
        let mut out = vec![9.0];
        interleave_stereo(&[1.0, 2.0], &[-1.0, -2.0, -3.0], &mut out);
        assert_eq!(out, vec![9.0, 1.0, -1.0, 2.0, -2.0]);
    }

    #[test]
    fn mix_stereo_averages_each_frame() {
        assert_eq!(mix_stereo(&[1.0, 0.0], &[0.0, -1.0]), vec![0.5, -0.5]);
    }
}
