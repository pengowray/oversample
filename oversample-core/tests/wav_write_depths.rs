//! Writing f32 samples at each recording bit depth.

use oversample_core::audio::wav::{
    decode_pcm, encode_f32_samples, header_bytes, parse_wav_header, SampleEncoding, WavWriteFormat,
};

fn format(bits: u16, is_float: bool) -> WavWriteFormat {
    WavWriteFormat::for_f32_samples(384_000, 2, bits, is_float)
}

#[test]
fn unsupported_depths_are_written_as_16_bit() {
    for bits in [0, 8, 12, 16, 20, 64] {
        let f = format(bits, false);
        assert_eq!((f.bits_per_sample, f.is_float), (16, false), "{bits}");
    }
    assert_eq!(format(16, true).is_float, false);
    assert_eq!(format(24, true).is_float, false);
    assert_eq!(format(32, true).is_float, true);
    assert_eq!(format(32, false).is_float, false);
}

#[test]
fn sixteen_bit_output_rounds_and_clamps() {
    let data = encode_f32_samples(&[0.5, -0.5, 1.5, -1.5], &format(16, false));
    let samples: Vec<i16> = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(samples, vec![16384, -16384, 32767, -32768]);
}

#[test]
fn every_16_bit_sample_is_written_back_exactly() {
    // An export of a 16-bit file must keep its low bits: a 12-bit recorder's
    // zero-padded LSBs, or a 1-step noise floor, are how its source is told.
    let original: Vec<u8> = (i16::MIN..=i16::MAX).flat_map(i16::to_le_bytes).collect();
    let samples = decode_pcm(&original, SampleEncoding::I16);
    assert!(encode_f32_samples(&samples, &format(16, false)) == original);
}

#[test]
fn samples_read_from_24_bit_are_written_back_exactly() {
    let original: Vec<u8> = [0x7f_ffff, -0x80_0000, 1, -1, 0x12_3456]
        .iter()
        .flat_map(|s: &i32| s.to_le_bytes()[..3].to_vec())
        .collect();
    let samples = decode_pcm(&original, SampleEncoding::I24);
    assert_eq!(encode_f32_samples(&samples, &format(24, false)), original);
    // Past full scale is clamped.
    let clamped = encode_f32_samples(&[2.0, -2.0], &format(24, false));
    assert_eq!(clamped, vec![0xff, 0xff, 0x7f, 0x00, 0x00, 0x80]);
}

#[test]
fn thirty_two_bit_pcm_and_float_round_trip() {
    let samples = [0.5f32, -0.25, -1.0, 0.0];
    let pcm = encode_f32_samples(&samples, &format(32, false));
    assert_eq!(decode_pcm(&pcm, SampleEncoding::I32), samples);
    assert_eq!(
        i32::from_le_bytes(
            encode_f32_samples(&[1.0], &format(32, false))[..]
                .try_into()
                .unwrap()
        ),
        i32::MAX
    );
    let float = encode_f32_samples(&[0.5, -1.5], &format(32, true));
    assert_eq!(decode_pcm(&float, SampleEncoding::F32), vec![0.5, -1.5]);
}

#[test]
fn header_matches_the_data_for_each_depth() {
    for (bits, is_float, encoding) in [
        (16, false, SampleEncoding::I16),
        (24, false, SampleEncoding::I24),
        (32, false, SampleEncoding::I32),
        (32, true, SampleEncoding::F32),
    ] {
        let f = format(bits, is_float);
        let data = encode_f32_samples(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3], &f);
        let mut wav = header_bytes(&f, data.len() as u64, 0);
        wav.extend_from_slice(&data);
        let h = parse_wav_header(&wav).unwrap();
        assert_eq!(h.encoding, encoding);
        assert_eq!((h.channels, h.total_frames), (2, 3));
        assert_eq!(h.data_size as usize, data.len());
    }
}
