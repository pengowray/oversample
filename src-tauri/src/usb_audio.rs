// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Portions of this file are derived from batgizmo (nativeusb.cpp)
// Copyright (c) 2025 John Mears — MIT License
// https://github.com/jmears63/batgizmo-app-public
//
//! Raw USB isochronous audio streaming for Android.
//!
//! Uses Linux's usbdevice_fs ioctl interface to perform isochronous transfers
//! directly on a USB file descriptor obtained from Android's UsbDeviceConnection.
//! This bypasses Android's audio framework (Oboe/AAudio) which caps at ~192 kHz,
//! enabling sample rates up to 384 kHz (or 500 kHz if hardware supports it).
//!
//! Architecture modeled after batgizmo's nativeusb.cpp.

use crate::recording::{NativeSampleFormat, RecordingBuffer};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ── Shared types (all platforms) ────────────────────────────────────────

#[allow(dead_code)]
pub struct UsbStreamState {
    pub cancel_flag: Arc<AtomicBool>,
    pub is_recording: Arc<AtomicBool>,
    pub is_streaming: Arc<AtomicBool>,
    /// Samples in the stream's own format (see [`usb_sample_format`]), with the
    /// same pre-roll ring, recovery flushing and WAV encoding as the cpal mic.
    pub buffer: Arc<Mutex<RecordingBuffer>>,
    pub format: NativeSampleFormat,
    pub sample_rate: u32,
    pub device_name: String,
    pub uac_version: u32, // 1 or 2, from USB Audio Class descriptor
    /// Crash-recovery writer. Active between `usb_start_recording` and
    /// `usb_stop_recording`; shared with the emitter thread for streaming
    /// flushes.
    pub recovery: crate::recovery::RecoveryHandle,
}

#[derive(Serialize)]
pub struct UsbStreamInfo {
    pub sample_rate: u32,
    pub device_name: String,
}

#[derive(Serialize)]
pub struct UsbStreamStatus {
    pub is_open: bool,
    pub is_streaming: bool,
    pub samples_recorded: usize,
    pub sample_rate: u32,
}

// ── Cross-platform functions ────────────────────────────────────────────

pub fn stop_usb_stream(state: &UsbStreamState) {
    state.cancel_flag.store(true, Ordering::Relaxed);
}

/// The format a USB stream is stored and recorded in, from the bytes per sample
/// in a packet (UAC1 bSubframeSize, UAC2 bSubSlotSize) and whether the
/// descriptor says IEEE float. Any size other than 3 or 4 is taken as 2 bytes,
/// as before. Float counts only with 4-byte samples.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn usb_sample_format(sample_bytes: u32, is_float: bool) -> NativeSampleFormat {
    match sample_bytes {
        4 if is_float => NativeSampleFormat::F32,
        4 => NativeSampleFormat::I32,
        3 => NativeSampleFormat::I24,
        _ => NativeSampleFormat::I16,
    }
}

/// All recorded samples as f32, interleaved L/R when the stream is stereo.
pub fn get_usb_samples_f32(state: &UsbStreamState) -> Vec<f32> {
    crate::recording::get_samples_f32(&state.buffer.lock().unwrap())
}

pub fn get_usb_status(state: &UsbStreamState) -> UsbStreamStatus {
    let buf = state.buffer.lock().unwrap();
    UsbStreamStatus {
        is_open: true,
        is_streaming: state.is_streaming.load(Ordering::Relaxed),
        samples_recorded: buf.total_samples,
        sample_rate: state.sample_rate,
    }
}

/// The recording as a WAV file in the stream's own format.
pub fn encode_usb_wav(state: &UsbStreamState) -> Result<Vec<u8>, String> {
    crate::recording::encode_native_wav(&state.buffer.lock().unwrap())
}

/// The mean of one frame's samples, for a mono mix.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
trait MixSample: Copy {
    fn mean(samples: impl Iterator<Item = Self>, count: usize) -> Self;
}

impl MixSample for i16 {
    // Rounds down, so a 16-bit stereo device mixed to mono gives the same
    // samples as before full-resolution decoding.
    fn mean(samples: impl Iterator<Item = Self>, count: usize) -> Self {
        samples.map(i64::from).sum::<i64>().div_euclid(count as i64) as i16
    }
}

impl MixSample for i32 {
    fn mean(samples: impl Iterator<Item = Self>, count: usize) -> Self {
        samples.map(i64::from).sum::<i64>().div_euclid(count as i64) as i32
    }
}

impl MixSample for f32 {
    fn mean(samples: impl Iterator<Item = Self>, count: usize) -> Self {
        samples.sum::<f32>() / count as f32
    }
}

/// Appends the whole frames in `bytes` to `out`: every sample when `keep_channels`
/// is true, otherwise one mono mix per frame. A partial frame at the end is
/// dropped.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn decode_frames<T: MixSample>(
    bytes: &[u8],
    sample_bytes: usize,
    channels: usize,
    keep_channels: bool,
    read: fn(&[u8]) -> T,
    out: &mut Vec<T>,
) {
    for frame in bytes.chunks_exact(sample_bytes * channels) {
        let samples = frame.chunks_exact(sample_bytes).map(read);
        if keep_channels {
            out.extend(samples);
        } else {
            out.push(T::mean(samples, channels));
        }
    }
}

/// Turns the packets of one USB stream into samples in its own format, without
/// losing resolution. 3-byte samples go in the top 24 bits of an i32, as cpal
/// gives 24-bit audio, so they are written as 24-bit WAV and shown at the right
/// level.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) struct UsbDecoder {
    format: NativeSampleFormat,
    channels: usize,
    /// Keep every channel: the device has one, or two with `stereo` asked for.
    keep_channels: bool,
    i16s: Vec<i16>,
    i32s: Vec<i32>,
    f32s: Vec<f32>,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl UsbDecoder {
    pub(crate) fn new(format: NativeSampleFormat, channels: usize, stereo: bool) -> Self {
        let channels = channels.max(1);
        Self {
            format,
            channels,
            keep_channels: channels == 1 || (stereo && channels == 2),
            i16s: Vec::new(),
            i32s: Vec::new(),
            f32s: Vec::new(),
        }
    }

    /// Channels stored per frame: 1, or 2 for a stereo stream.
    pub(crate) fn stored_channels(&self) -> u16 {
        if self.keep_channels {
            self.channels as u16
        } else {
            1
        }
    }

    pub(crate) fn clear(&mut self) {
        self.i16s.clear();
        self.i32s.clear();
        self.f32s.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.i16s.is_empty() && self.i32s.is_empty() && self.f32s.is_empty()
    }

    /// Decode one packet (little-endian samples, channels interleaved).
    pub(crate) fn decode(&mut self, bytes: &[u8]) {
        let (ch, keep) = (self.channels, self.keep_channels);
        match self.format {
            NativeSampleFormat::I16 => decode_frames(
                bytes,
                2,
                ch,
                keep,
                |b| i16::from_le_bytes([b[0], b[1]]),
                &mut self.i16s,
            ),
            NativeSampleFormat::I24 => decode_frames(
                bytes,
                3,
                ch,
                keep,
                |b| i32::from_le_bytes([0, b[0], b[1], b[2]]),
                &mut self.i32s,
            ),
            NativeSampleFormat::I32 => decode_frames(
                bytes,
                4,
                ch,
                keep,
                |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                &mut self.i32s,
            ),
            NativeSampleFormat::F32 => decode_frames(
                bytes,
                4,
                ch,
                keep,
                |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                &mut self.f32s,
            ),
        }
    }

    /// Hand the decoded samples to the recording (or the pre-roll ring) and the
    /// live display.
    pub(crate) fn push_to(&self, buf: &mut RecordingBuffer, recording: bool) {
        match self.format {
            NativeSampleFormat::I16 => buf.push_i16(&self.i16s, recording),
            NativeSampleFormat::I24 => buf.push_i32(&self.i32s, recording),
            NativeSampleFormat::I32 => {
                // A 4-byte slot often carries 24-bit audio; detection lets the
                // recording be written at 24 bits. (A mono mix can add one low
                // bit, which only makes detection keep more bits.)
                buf.note_i32_bits(&self.i32s);
                buf.push_i32(&self.i32s, recording)
            }
            NativeSampleFormat::F32 => buf.push_f32(&self.f32s, recording),
        }
    }
}

// ── Android isochronous streaming ───────────────────────────────────────

#[cfg(target_os = "android")]
pub fn start_usb_stream(
    fd: i32,
    endpoint_address: u32,
    max_packet_size: u32,
    sample_rate: u32,
    num_channels: u32,
    stereo: bool,
    device_name: String,
    app: tauri::AppHandle,
    interface_number: u32,
    alternate_setting: u32,
    uac_version: u32,
    sample_bytes: u32,
    is_float: bool,
) -> Result<UsbStreamState, String> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let is_streaming = Arc::new(AtomicBool::new(false));
    let is_recording = Arc::new(AtomicBool::new(false));
    let format = usb_sample_format(sample_bytes, is_float);
    let decoder = UsbDecoder::new(format, num_channels as usize, stereo);
    let buffer = Arc::new(Mutex::new(RecordingBuffer::new(
        format,
        sample_rate,
        decoder.stored_channels(),
    )));
    eprintln!(
        "USB stream: {} ch, {} bytes per sample, stored as {:?}",
        num_channels, sample_bytes, format
    );

    let cancel = cancel_flag.clone();
    let streaming = is_streaming.clone();
    let recording = is_recording.clone();
    let buf = buffer.clone();

    // Channel to wait for the isochronous thread to confirm streaming has started
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);

    let app_for_thread = app.clone();
    let cancel_for_emit = cancel_flag.clone();
    std::thread::spawn(move || {
        let result = isochronous::run_isochronous_loop(
            fd,
            endpoint_address,
            max_packet_size,
            decoder,
            interface_number,
            alternate_setting,
            &cancel,
            &streaming,
            &recording,
            &buf,
            startup_tx,
        );

        streaming.store(false, Ordering::Relaxed);
        // Also signal the emitter thread to stop
        cancel_for_emit.store(true, Ordering::Relaxed);

        // Emit frontend event if stream ended unexpectedly (not by explicit cancel)
        use tauri::Emitter;
        match result {
            Ok(()) => {
                eprintln!("USB stream ended normally");
                // Normal end without cancel means disconnect
                let _ = app_for_thread.emit("usb-stream-error", "USB device disconnected");
            }
            Err(e) => {
                eprintln!("USB stream error: {}", e);
                let _ =
                    app_for_thread.emit("usb-stream-error", &format!("USB stream error: {}", e));
            }
        }
    });

    // Wait for the isochronous thread to confirm data is flowing
    match startup_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => { /* streaming started successfully */ }
        Ok(Err(e)) => return Err(format!("USB stream failed to start: {}", e)),
        Err(_) => {
            return Err("USB stream startup timeout (5s) — device may not be sending data".into())
        }
    }

    // Start emitter for streaming audio chunks to the frontend (also does
    // best-effort crash-recovery flushes when a writer is installed).
    let recovery = crate::recovery::RecoveryHandle::default();
    crate::recording::start_emitter(buffer.clone(), cancel_flag.clone(), recovery.clone());

    Ok(UsbStreamState {
        cancel_flag,
        is_recording,
        buffer,
        format,
        sample_rate,
        is_streaming,
        device_name,
        uac_version,
        recovery,
    })
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub fn start_usb_stream(
    _fd: i32,
    _endpoint_address: u32,
    _max_packet_size: u32,
    _sample_rate: u32,
    _num_channels: u32,
    _stereo: bool,
    _device_name: String,
    _app: tauri::AppHandle,
    _interface_number: u32,
    _alternate_setting: u32,
    _uac_version: u32,
    _sample_bytes: u32,
    _is_float: bool,
) -> Result<UsbStreamState, String> {
    Err("USB audio streaming is only supported on Android".into())
}

// ── Linux USB isochronous implementation (Android only) ─────────────────

#[cfg(target_os = "android")]
mod isochronous {
    use super::{RecordingBuffer, UsbDecoder};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    // ioctl direction bits
    const _IOC_WRITE: u32 = 1;
    const _IOC_READ: u32 = 2;
    const _IOC_NRBITS: u32 = 8;
    const _IOC_TYPEBITS: u32 = 8;
    const _IOC_SIZEBITS: u32 = 14;
    const _IOC_NRSHIFT: u32 = 0;
    const _IOC_TYPESHIFT: u32 = _IOC_NRSHIFT + _IOC_NRBITS;
    const _IOC_SIZESHIFT: u32 = _IOC_TYPESHIFT + _IOC_TYPEBITS;
    const _IOC_DIRSHIFT: u32 = _IOC_SIZESHIFT + _IOC_SIZEBITS;

    const fn _ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u32 {
        (dir << _IOC_DIRSHIFT)
            | (ty << _IOC_TYPESHIFT)
            | (nr << _IOC_NRSHIFT)
            | (size << _IOC_SIZESHIFT)
    }

    const fn _ior(ty: u32, nr: u32, size: u32) -> u32 {
        _ioc(_IOC_READ, ty, nr, size)
    }

    const fn _iow(ty: u32, nr: u32, size: u32) -> u32 {
        _ioc(_IOC_WRITE, ty, nr, size)
    }

    const USBDEVFS_MAGIC: u32 = b'U' as u32;

    const USBDEVFS_SUBMITURB: u32 = _ior(
        USBDEVFS_MAGIC,
        10,
        std::mem::size_of::<UsbdevfsUrb>() as u32,
    );
    const USBDEVFS_DISCARDURB: u32 = _ioc(0, USBDEVFS_MAGIC, 11, 0);
    const USBDEVFS_REAPURB: u32 = _iow(
        USBDEVFS_MAGIC,
        12,
        std::mem::size_of::<*mut std::ffi::c_void>() as u32,
    );
    const USBDEVFS_REAPURBNDELAY: u32 = _iow(
        USBDEVFS_MAGIC,
        13,
        std::mem::size_of::<*mut std::ffi::c_void>() as u32,
    );
    const USBDEVFS_CLAIMINTERFACE: u32 =
        _ior(USBDEVFS_MAGIC, 15, std::mem::size_of::<u32>() as u32);
    const USBDEVFS_SETINTERFACE: u32 = _ior(
        USBDEVFS_MAGIC,
        4,
        std::mem::size_of::<UsbdevfsSetinterface>() as u32,
    );

    const USBDEVFS_URB_TYPE_ISO: u8 = 0;
    const USBDEVFS_URB_ISO_ASAP: u32 = 0x02;

    #[repr(C)]
    struct UsbdevfsSetinterface {
        interface: u32,
        altsetting: u32,
    }

    #[repr(C)]
    #[derive(Clone)]
    struct UsbdevfsIsoPacketDesc {
        length: u32,
        actual_length: u32,
        status: u32,
    }

    #[repr(C)]
    struct UsbdevfsUrb {
        urb_type: u8,
        endpoint: u8,
        status: i32,
        flags: u32,
        buffer: *mut u8,
        buffer_length: i32,
        actual_length: i32,
        start_frame: i32,
        number_of_packets: i32,
        error_count: i32,
        signr: u32,
        usercontext: *mut std::ffi::c_void,
    }

    const MAX_CHANNELS: usize = 2;
    const MAX_SAMPLES_PER_FRAME: usize = 501; // 500 kHz = 500 samples/ms + slack
    const URBS_TO_JUGGLE: usize = 10;
    const PACKETS_PER_URB: usize = 25; // 25ms worth of USB frames
    const MAX_DATA_POINTS_PER_URB: usize = MAX_SAMPLES_PER_FRAME * MAX_CHANNELS * PACKETS_PER_URB;

    #[repr(C)]
    struct UrbWithPackets {
        urb: UsbdevfsUrb,
        packet_desc: [UsbdevfsIsoPacketDesc; PACKETS_PER_URB],
    }

    pub fn run_isochronous_loop(
        fd: i32,
        endpoint_address: u32,
        max_packet_size: u32,
        mut decoder: UsbDecoder,
        interface_number: u32,
        alternate_setting: u32,
        cancel: &AtomicBool,
        is_streaming: &AtomicBool,
        is_recording: &AtomicBool,
        buffer: &Arc<Mutex<RecordingBuffer>>,
        startup_tx: std::sync::mpsc::SyncSender<Result<(), String>>,
    ) -> Result<(), String> {
        // Ensure the interface is claimed and alternate setting is active from the
        // kernel's perspective. The Kotlin side should have already done this via
        // connection.setInterface(), but we do it again here as a safety net.
        // CLAIMINTERFACE: claim the audio streaming interface (may already be claimed)
        let mut iface_num = interface_number;
        let ret = unsafe {
            libc::ioctl(
                fd,
                USBDEVFS_CLAIMINTERFACE as libc::c_int,
                &mut iface_num as *mut u32,
            )
        };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            // EBUSY means already claimed — that's fine
            if err.raw_os_error() != Some(libc::EBUSY) {
                eprintln!(
                    "USBDEVFS_CLAIMINTERFACE({}) warning: {}",
                    interface_number, err
                );
            }
        }

        // SETINTERFACE: activate the correct alternate setting so the
        // isochronous endpoint becomes visible to the kernel
        if alternate_setting > 0 {
            let mut setif = UsbdevfsSetinterface {
                interface: interface_number,
                altsetting: alternate_setting,
            };
            let ret = unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_SETINTERFACE as libc::c_int,
                    &mut setif as *mut UsbdevfsSetinterface,
                )
            };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                let msg = format!(
                    "USBDEVFS_SETINTERFACE(iface={}, alt={}) failed: {}",
                    interface_number, alternate_setting, err
                );
                let _ = startup_tx.send(Err(msg.clone()));
                return Err(msg);
            }
            eprintln!(
                "USBDEVFS_SETINTERFACE iface={} alt={} OK",
                interface_number, alternate_setting
            );
        }

        let requested_bytes_per_frame = max_packet_size as usize;
        // Room for every packet at its requested size: 3- and 4-byte samples
        // need more than the 2 bytes per data point the minimum allows for.
        let urb_buffer_bytes =
            (MAX_DATA_POINTS_PER_URB * 2).max(PACKETS_PER_URB * requested_bytes_per_frame);

        let mut audio_buffers: Vec<Vec<u8>> = (0..URBS_TO_JUGGLE)
            .map(|_| vec![0u8; urb_buffer_bytes])
            .collect();

        let mut urbs: Vec<Box<UrbWithPackets>> = (0..URBS_TO_JUGGLE)
            .map(|i| {
                let mut urb_box = Box::new(UrbWithPackets {
                    urb: UsbdevfsUrb {
                        urb_type: USBDEVFS_URB_TYPE_ISO,
                        endpoint: (endpoint_address | 0x80) as u8,
                        status: 0,
                        flags: USBDEVFS_URB_ISO_ASAP,
                        buffer: audio_buffers[i].as_mut_ptr(),
                        buffer_length: urb_buffer_bytes as i32,
                        actual_length: 0,
                        start_frame: 0,
                        number_of_packets: PACKETS_PER_URB as i32,
                        error_count: 0,
                        signr: 0,
                        usercontext: std::ptr::null_mut(),
                    },
                    packet_desc: std::array::from_fn(|_| UsbdevfsIsoPacketDesc {
                        length: requested_bytes_per_frame as u32,
                        actual_length: 0,
                        status: 0,
                    }),
                });
                urb_box.urb.usercontext =
                    &*urb_box as *const UrbWithPackets as *mut std::ffi::c_void;
                urb_box
            })
            .collect();

        for (i, urb) in urbs.iter_mut().enumerate() {
            urb.urb.buffer = audio_buffers[i].as_mut_ptr();
        }

        // Submit all URBs
        for urb in &urbs {
            let ret = unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_SUBMITURB as libc::c_int,
                    &urb.urb as *const UsbdevfsUrb,
                )
            };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                let msg = format!("USBDEVFS_SUBMITURB failed: {}", err);
                let _ = startup_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        }

        let mut balls_in_air = URBS_TO_JUGGLE;
        let mut startup_signaled = false;

        // EMT2 workaround: retry on poll timeout instead of failing immediately.
        // Wildlife Acoustics EMT2 can send oversized USB packets that cause the
        // isochronous transfer to hang. Discarding URBs and resubmitting recovers.
        // See batgizmo nativeusb.cpp lines 337-367 and UsbService.kt lines 1152-1173.
        const MAX_TIMEOUT_RETRIES: u32 = 3;
        let mut timeout_retries: u32 = 0;

        // Main juggling loop
        while !cancel.load(Ordering::Relaxed) || balls_in_air > 0 {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN | libc::POLLOUT,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if poll_ret <= 0 {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                timeout_retries += 1;
                if timeout_retries > MAX_TIMEOUT_RETRIES {
                    discard_urbs(fd, &urbs);
                    let msg = format!(
                        "USB stream poll timeout after {} retries — device may have disconnected",
                        MAX_TIMEOUT_RETRIES
                    );
                    if !startup_signaled {
                        let _ = startup_tx.send(Err(msg.clone()));
                    }
                    return Err(msg);
                }
                eprintln!(
                    "USB poll timeout ({}/{}), discarding URBs and retrying (EMT2-style recovery)...",
                    timeout_retries, MAX_TIMEOUT_RETRIES
                );
                discard_urbs(fd, &urbs);
                // Resubmit all URBs with fresh packet descriptors
                balls_in_air = 0;
                for urb in &mut urbs {
                    // Reset packet descriptors to requested sizes
                    for packet in &mut urb.packet_desc {
                        packet.length = requested_bytes_per_frame as u32;
                        packet.actual_length = 0;
                        packet.status = 0;
                    }
                    urb.urb.status = 0;
                    urb.urb.actual_length = 0;
                    urb.urb.error_count = 0;
                    let ret = unsafe {
                        libc::ioctl(
                            fd,
                            USBDEVFS_SUBMITURB as libc::c_int,
                            &urb.urb as *const UsbdevfsUrb,
                        )
                    };
                    if ret == 0 {
                        balls_in_air += 1;
                    } else {
                        let err = std::io::Error::last_os_error();
                        if err.raw_os_error() == Some(libc::ENODEV) {
                            let msg = "USB device disconnected during timeout recovery".to_string();
                            if !startup_signaled {
                                let _ = startup_tx.send(Err(msg.clone()));
                            }
                            return Err(msg);
                        }
                        eprintln!("USBDEVFS_SUBMITURB resubmit during retry error: {}", err);
                    }
                }
                if balls_in_air == 0 {
                    let msg = "Failed to resubmit any URBs after timeout recovery".to_string();
                    if !startup_signaled {
                        let _ = startup_tx.send(Err(msg.clone()));
                    }
                    return Err(msg);
                }
                continue;
            }
            // Reset timeout counter on successful poll
            if timeout_retries > 0 {
                eprintln!(
                    "USB stream recovered after {} timeout retries",
                    timeout_retries
                );
                timeout_retries = 0;
            }

            let mut urb_reaped: *mut UsbdevfsUrb = std::ptr::null_mut();
            let ret = unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_REAPURB as libc::c_int,
                    &mut urb_reaped as *mut *mut UsbdevfsUrb,
                )
            };

            if ret != 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if err.raw_os_error() == Some(libc::ENODEV) {
                    eprintln!("USB device disconnected");
                    if !startup_signaled {
                        let _ = startup_tx.send(Err("USB device disconnected".into()));
                    }
                    break;
                }
                eprintln!("USBDEVFS_REAPURB error: {}", err);
                continue;
            }

            balls_in_air -= 1;

            // Process the reaped URB's audio data
            if !cancel.load(Ordering::Relaxed) {
                let reaped = unsafe { &*urb_reaped };
                let urb_with_packets = unsafe { &*(reaped.usercontext as *const UrbWithPackets) };

                decoder.clear();

                let mut src_byte_offset: usize = 0;
                for packet in &urb_with_packets.packet_desc {
                    let actual_bytes = packet.actual_length as usize;

                    if actual_bytes > 0 {
                        let src = unsafe {
                            std::slice::from_raw_parts(
                                (reaped.buffer as *const u8).add(src_byte_offset),
                                actual_bytes,
                            )
                        };
                        decoder.decode(src);
                    }

                    src_byte_offset += packet.length as usize;
                }

                if !decoder.is_empty() {
                    // Signal startup success on first URB with actual audio data
                    if !startup_signaled {
                        is_streaming.store(true, Ordering::Relaxed);
                        let _ = startup_tx.send(Ok(()));
                        startup_signaled = true;
                    }

                    let recording = is_recording.load(Ordering::Relaxed);
                    if let Ok(mut buf) = buffer.lock() {
                        decoder.push_to(&mut buf, recording);
                    }
                }
            }

            // Resubmit unless cancelling
            if !cancel.load(Ordering::Relaxed) {
                let ret = unsafe { libc::ioctl(fd, USBDEVFS_SUBMITURB as libc::c_int, urb_reaped) };
                if ret == 0 {
                    balls_in_air += 1;
                } else {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::ENODEV) {
                        break;
                    }
                    eprintln!("USBDEVFS_SUBMITURB resubmit error: {}", err);
                }
            }
        }

        Ok(())
    }

    fn discard_urbs(fd: i32, urbs: &[Box<UrbWithPackets>]) {
        // Retry on EINTR — a signal can interrupt the ioctl mid-call.
        // Matches batgizmo nativeusb.cpp discard_URBs().
        for urb in urbs {
            loop {
                let ret = unsafe {
                    libc::ioctl(
                        fd,
                        USBDEVFS_DISCARDURB as libc::c_int,
                        &urb.urb as *const UsbdevfsUrb,
                    )
                };
                if ret == 0 {
                    break;
                }
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    break; // non-retriable error — expected during cleanup
                }
            }
        }
        for _ in urbs {
            let mut urb_reaped: *mut UsbdevfsUrb = std::ptr::null_mut();
            loop {
                let ret = unsafe {
                    libc::ioctl(
                        fd,
                        USBDEVFS_REAPURBNDELAY as libc::c_int,
                        &mut urb_reaped as *mut *mut UsbdevfsUrb,
                    )
                };
                if ret == 0 {
                    break;
                }
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    break; // non-retriable error — expected during cleanup
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oversample_core::audio::wav::parse_wav_header;

    fn le_bytes(samples: &[i32], bytes: usize) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|s| s.to_le_bytes()[..bytes].to_vec())
            .collect()
    }

    /// Decode `packet` and return what reaches the recording.
    fn decode(
        format: NativeSampleFormat,
        channels: usize,
        stereo: bool,
        packet: &[u8],
    ) -> RecordingBuffer {
        let mut decoder = UsbDecoder::new(format, channels, stereo);
        let mut buf = RecordingBuffer::new(format, 48_000, decoder.stored_channels());
        decoder.decode(packet);
        decoder.push_to(&mut buf, true);
        buf
    }

    /// The header and data of a WAV file.
    fn wav_data(wav: &[u8]) -> (oversample_core::audio::wav::WavHeader, &[u8]) {
        let h = parse_wav_header(wav).unwrap();
        let start = h.data_offset as usize;
        let data = &wav[start..start + h.data_size as usize];
        (h, data)
    }

    #[test]
    fn picks_the_format_from_the_sample_size() {
        use NativeSampleFormat::*;
        assert_eq!(usb_sample_format(2, false), I16);
        assert_eq!(usb_sample_format(0, false), I16);
        assert_eq!(usb_sample_format(2, true), I16);
        assert_eq!(usb_sample_format(3, false), I24);
        assert_eq!(usb_sample_format(4, false), I32);
        assert_eq!(usb_sample_format(4, true), F32);
    }

    #[test]
    fn decodes_16_bit_packets_as_before() {
        let bytes = le_bytes(&[100, -200, 300, -400], 2);
        let stereo = decode(NativeSampleFormat::I16, 2, true, &bytes);
        assert_eq!(stereo.samples_i16, vec![100, -200, 300, -400]);
        assert_eq!(stereo.total_samples, 2);
        let mixed = decode(NativeSampleFormat::I16, 2, false, &bytes);
        assert_eq!(mixed.samples_i16, vec![-50, -50]);
        let mono = decode(NativeSampleFormat::I16, 1, false, &bytes);
        assert_eq!(mono.samples_i16, vec![100, -200, 300, -400]);
        // A mono mix rounds down, as the 16-bit decoder always did.
        let odd = decode(NativeSampleFormat::I16, 2, false, &le_bytes(&[-2, -1], 2));
        assert_eq!(odd.samples_i16, vec![-2]);
    }

    /// The Portacapture X8's 2-channel alternate setting: 3-byte samples, left then right.
    #[test]
    fn keeps_all_24_bits_of_24_bit_stereo() {
        let (left, right) = (0x12_3456, -0x12_3457);
        let (max, min) = (0x7f_ffff, -0x80_0000);
        let bytes = le_bytes(&[left, right, max, min], 3);
        let buf = decode(NativeSampleFormat::I24, 2, true, &bytes);
        assert_eq!(buf.total_samples, 2);
        assert_eq!(
            buf.samples_i32,
            vec![left << 8, right << 8, max << 8, min << 8]
        );
        // Full scale reads as full scale.
        let mut buf = buf;
        let live = buf.drain_pending();
        assert!((live[2] - 1.0).abs() < 1e-6 && live[3] == -1.0);
        assert!((live[0] - left as f32 / 8_388_608.0).abs() < 1e-7);

        // The WAV holds the same 3 bytes per sample the device sent.
        let wav = crate::recording::encode_native_wav(&buf).unwrap();
        let (h, data) = wav_data(&wav);
        assert_eq!((h.channels, h.bits_per_sample, h.total_frames), (2, 24, 2));
        assert_eq!(data, &bytes[..]);
    }

    #[test]
    fn mixes_24_bit_stereo_to_mono_at_full_resolution() {
        let bytes = le_bytes(&[0x00_0001, 0x00_0002, -3, -4], 3);
        let buf = decode(NativeSampleFormat::I24, 2, false, &bytes);
        // The means, 1.5 and -3.5, keep their half step in the spare low byte.
        assert_eq!(buf.samples_i32, vec![0x180, -0x380]);
        assert_eq!(buf.channels, 1);
    }

    #[test]
    fn keeps_all_32_bits_and_drops_a_partial_frame() {
        let samples = [0x7fff_0001, i32::MIN, 5];
        let buf = decode(NativeSampleFormat::I32, 2, true, &le_bytes(&samples, 4));
        assert_eq!(buf.samples_i32, vec![0x7fff_0001, i32::MIN]);
        let wav = crate::recording::encode_native_wav(&buf).unwrap();
        let (h, data) = wav_data(&wav);
        assert_eq!(
            (h.bits_per_sample, h.is_float, h.total_frames),
            (32, false, 1)
        );
        assert_eq!(data, &le_bytes(&samples[..2], 4)[..]);
    }

    #[test]
    fn writes_32_bit_float_as_float_wav() {
        let samples = [0.5f32, -0.25, 1.0, -1.0];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut buf = decode(NativeSampleFormat::F32, 1, false, &bytes);
        assert_eq!(buf.samples_f32, samples);
        assert_eq!(buf.drain_pending(), samples);
        let wav = crate::recording::encode_native_wav(&buf).unwrap();
        let (h, data) = wav_data(&wav);
        assert_eq!((h.bits_per_sample, h.is_float, h.channels), (32, true, 1));
        assert_eq!(data, &bytes[..]);

        let stereo = decode(NativeSampleFormat::F32, 2, false, &bytes);
        assert_eq!(stereo.samples_f32, vec![0.125, 0.0]);
    }

    #[test]
    fn stereo_preroll_ring_keeps_whole_recent_frames_and_seeds_them() {
        // Cap: 1 Hz x 15 s = 15 frames.
        let mut decoder = UsbDecoder::new(NativeSampleFormat::I24, 2, true);
        let mut buf = RecordingBuffer::new(NativeSampleFormat::I24, 1, 2);
        let frames: Vec<i32> = (0..20).flat_map(|i| [i, 100 + i]).collect();
        decoder.decode(&le_bytes(&frames, 3));
        decoder.push_to(&mut buf, false);
        assert_eq!(buf.total_samples, 0);
        assert_eq!(buf.preroll_i32.len(), 30);
        assert_eq!(buf.preroll_i32.front(), Some(&(5 << 8)));

        buf.clear();
        assert_eq!(buf.seed_preroll(2), 2);
        assert_eq!(buf.total_samples, 2);
        assert_eq!(buf.samples_i32, vec![18 << 8, 118 << 8, 19 << 8, 119 << 8]);
    }

    #[test]
    fn decoder_clears_between_urbs() {
        let mut decoder = UsbDecoder::new(NativeSampleFormat::I16, 1, false);
        assert!(decoder.is_empty());
        decoder.decode(&le_bytes(&[1, 2], 2));
        assert!(!decoder.is_empty());
        decoder.clear();
        assert!(decoder.is_empty());
    }
}
