//! Windows WASAPI Loopback + SMTC (GlobalSystemMediaTransportControls) music rhythm & style sensor.

use crate::win32_sensors::AppSensorState;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

#[derive(Clone, Serialize)]
pub struct MusicPayload {
    pub playing: bool,
    pub style: String, // "lofi" | "pop" | "edm" | "vocal"
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub beat: bool,
    pub title: String,
    pub artist: String,
}

#[cfg(windows)]
fn query_smtc_now_playing() -> (bool, String, String) {
    let Ok(op) = GlobalSystemMediaTransportControlsSessionManager::RequestAsync() else {
        return (false, String::new(), String::new());
    };
    let Ok(manager) = op.get() else {
        return (false, String::new(), String::new());
    };
    let Ok(session) = manager.GetCurrentSession() else {
        return (false, String::new(), String::new());
    };
    let playing = session
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .map(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);

    if !playing {
        return (false, String::new(), String::new());
    }

    let (mut title, mut artist) = (String::new(), String::new());
    if let Ok(props_op) = session.TryGetMediaPropertiesAsync() {
        if let Ok(props) = props_op.get() {
            if let Ok(t) = props.Title() {
                title = t.to_string_lossy();
            }
            if let Ok(a) = props.Artist() {
                artist = a.to_string_lossy();
            }
        }
    }
    (true, title, artist)
}

#[cfg(windows)]
pub fn start_music_sensor(app: AppHandle, state: Arc<AppSensorState>) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            CoUninitialize();
            return;
        };

        let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
            CoUninitialize();
            return;
        };

        let Ok(audio_client): Result<IAudioClient, _> = device.Activate(CLSCTX_ALL, None) else {
            CoUninitialize();
            return;
        };

        let Ok(pwfx) = audio_client.GetMixFormat() else {
            CoUninitialize();
            return;
        };
        let wfx = *pwfx;
        let channels = (wfx.nChannels as usize).max(1);
        let sample_rate = (wfx.nSamplesPerSec as f32).max(8000.0);
        let bits_per_sample = wfx.wBitsPerSample;
        let is_float = wfx.wFormatTag == 3 || (wfx.wFormatTag == 0xFFFE && bits_per_sample == 32);

        // 100ms buffer in 100-nanosecond units
        let hns_buffer: i64 = 1_000_000;
        let init_res = audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            hns_buffer,
            0,
            pwfx as *const WAVEFORMATEX,
            None,
        );
        CoTaskMemFree(Some(pwfx as *const _));

        if init_res.is_err() {
            CoUninitialize();
            return;
        }

        let Ok(capture_client): Result<IAudioCaptureClient, _> = audio_client.GetService() else {
            CoUninitialize();
            return;
        };

        let _ = audio_client.Start();

        const FFT_SIZE: usize = 1024;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let hann: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / (FFT_SIZE as f32 - 1.0)).cos())
            })
            .collect();

        let mut ring = Vec::with_capacity(FFT_SIZE * 2);
        let mut smtc_check = Instant::now() - Duration::from_secs(5);
        let mut smtc_playing = false;
        let mut smtc_title = String::new();
        let mut smtc_artist = String::new();
        let mut was_emitting_playing = false;

        let mut bass_avg = 0.01f32;
        let mut smooth_bass = 0.0f32;
        let mut smooth_mid = 0.0f32;
        let mut smooth_high = 0.0f32;
        let mut last_beat = Instant::now() - Duration::from_secs(1);

        loop {
            // Pause audio processing when disabled or in Stealth mode
            if !state.music_reactive_enabled.load(Ordering::Relaxed)
                || state.is_stealth.load(Ordering::Relaxed)
            {
                if was_emitting_playing {
                    was_emitting_playing = false;
                    let _ = app.emit(
                        "buddy://music",
                        MusicPayload {
                            playing: false,
                            style: "lofi".to_string(),
                            bass: 0.0,
                            mid: 0.0,
                            high: 0.0,
                            beat: false,
                            title: String::new(),
                            artist: String::new(),
                        },
                    );
                }
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }

            if smtc_check.elapsed() >= Duration::from_millis(900) {
                smtc_check = Instant::now();
                let (p, t, a) = query_smtc_now_playing();
                smtc_playing = p;
                smtc_title = t;
                smtc_artist = a;
            }

            // Drain available WASAPI loopback packets
            while let Ok(pkt_len) = capture_client.GetNextPacketSize() {
                if pkt_len == 0 {
                    break;
                }
                let mut data_ptr = std::ptr::null_mut();
                let mut num_frames = 0u32;
                let mut flags = 0u32;
                if capture_client
                    .GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)
                    .is_ok()
                {
                    if !data_ptr.is_null() && num_frames > 0 {
                        let is_silent = (flags & 0x2) != 0; // AUDCLNT_BUFFERFLAGS_SILENT
                        if is_silent {
                            ring.resize(ring.len() + num_frames as usize, 0.0);
                        } else if is_float && bits_per_sample == 32 {
                            let samples = std::slice::from_raw_parts(
                                data_ptr as *const f32,
                                (num_frames as usize) * channels,
                            );
                            for frame in samples.chunks_exact(channels) {
                                let mono = frame.iter().sum::<f32>() / (channels as f32);
                                ring.push(mono);
                            }
                        } else if bits_per_sample == 16 {
                            let samples = std::slice::from_raw_parts(
                                data_ptr as *const i16,
                                (num_frames as usize) * channels,
                            );
                            for frame in samples.chunks_exact(channels) {
                                let mono = frame.iter().map(|&s| (s as f32) / 32768.0).sum::<f32>()
                                    / (channels as f32);
                                ring.push(mono);
                            }
                        }
                    }
                    let _ = capture_client.ReleaseBuffer(num_frames);
                } else {
                    break;
                }
            }

            if ring.len() > FFT_SIZE * 2 {
                let drain = ring.len() - FFT_SIZE;
                ring.drain(..drain);
            }

            if smtc_playing && ring.len() >= FFT_SIZE {
                let start = ring.len() - FFT_SIZE;
                let mut buf: Vec<Complex<f32>> = ring[start..]
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| Complex::new(s * hann[i], 0.0))
                    .collect();
                fft.process(&mut buf);

                let hz_per_bin = sample_rate / (FFT_SIZE as f32);
                let mut e_bass = 0.0f32;
                let mut c_bass = 0.0f32;
                let mut e_mid = 0.0f32;
                let mut c_mid = 0.0f32;
                let mut e_high = 0.0f32;
                let mut c_high = 0.0f32;

                for (i, c) in buf.iter().take(FFT_SIZE / 2).enumerate().skip(1) {
                    let freq = (i as f32) * hz_per_bin;
                    let mag = c.norm() / (FFT_SIZE as f32);
                    if freq < 170.0 {
                        e_bass += mag;
                        c_bass += 1.0;
                    } else if freq < 2000.0 {
                        e_mid += mag;
                        c_mid += 1.0;
                    } else if freq < 11000.0 {
                        e_high += mag;
                        c_high += 1.0;
                    }
                }

                let raw_bass = ((e_bass / c_bass.max(1.0)) * 28.0).clamp(0.0, 1.0);
                let raw_mid = ((e_mid / c_mid.max(1.0)) * 45.0).clamp(0.0, 1.0);
                let raw_high = ((e_high / c_high.max(1.0)) * 75.0).clamp(0.0, 1.0);

                smooth_bass = smooth_bass * 0.6 + raw_bass * 0.4;
                smooth_mid = smooth_mid * 0.65 + raw_mid * 0.35;
                smooth_high = smooth_high * 0.65 + raw_high * 0.35;
                bass_avg = bass_avg * 0.92 + smooth_bass * 0.08;

                let beat = smooth_bass > (bass_avg * 1.42 + 0.06)
                    && last_beat.elapsed() >= Duration::from_millis(220);
                if beat {
                    last_beat = Instant::now();
                }

                // Heuristic acoustic + metadata style classification
                let lower_title = format!(
                    "{} {}",
                    smtc_title.to_ascii_lowercase(),
                    smtc_artist.to_ascii_lowercase()
                );
                let style = if lower_title.contains("podcast") || lower_title.contains("interview") {
                    "vocal"
                } else if lower_title.contains("lofi")
                    || lower_title.contains("lo-fi")
                    || lower_title.contains("chill")
                    || lower_title.contains("piano")
                    || lower_title.contains("ambient")
                {
                    "lofi"
                } else if smooth_bass > 0.36 && (smooth_high > 0.16 || smooth_bass > smooth_mid * 1.35) {
                    "edm"
                } else if smooth_bass > 0.16 || smooth_mid > 0.15 {
                    "pop"
                } else if smooth_mid > smooth_bass * 2.2 && smooth_high < 0.08 {
                    "vocal"
                } else {
                    "lofi"
                };

                let audible = smooth_bass + smooth_mid + smooth_high > 0.015;
                if audible {
                    was_emitting_playing = true;
                    let _ = app.emit(
                        "buddy://music",
                        MusicPayload {
                            playing: true,
                            style: style.to_string(),
                            bass: smooth_bass,
                            mid: smooth_mid,
                            high: smooth_high,
                            beat,
                            title: smtc_title.clone(),
                            artist: smtc_artist.clone(),
                        },
                    );
                }
            } else if was_emitting_playing {
                was_emitting_playing = false;
                let _ = app.emit(
                    "buddy://music",
                    MusicPayload {
                        playing: false,
                        style: "lofi".to_string(),
                        bass: 0.0,
                        mid: 0.0,
                        high: 0.0,
                        beat: false,
                        title: String::new(),
                        artist: String::new(),
                    },
                );
            }

            std::thread::sleep(Duration::from_millis(42)); // ~24Hz smooth envelope push
        }
    });
}

#[cfg(not(windows))]
pub fn start_music_sensor(_app: AppHandle, _state: Arc<AppSensorState>) {}
