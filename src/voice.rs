use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use whisper_rs::whisper_rs_sys::ggml_log_level;
use std::ffi::{c_char, c_void};

pub struct VoiceRecording {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct VadConfig {
    pub frame_ms: u32,
    pub start_threshold: f32,
    pub start_frames: usize,
    pub end_silence_ms: u32,
    pub pre_roll_ms: u32,
    pub max_record_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            frame_ms: 20,
            start_threshold: 0.02,
            start_frames: 3,
            end_silence_ms: 800,
            pre_roll_ms: 200,
            max_record_ms: 10_000,
        }
    }
}

pub fn transcribe_from_mic(duration: Duration, model_path: &str) -> Result<String, String> {
    silence_whisper_logs();
    let recorder = start_recording()?;
    let start = Instant::now();
    while start.elapsed() < duration {
        std::thread::sleep(Duration::from_millis(50));
    }
    let (audio, input_rate, channels) = recorder.stop();
    transcribe_audio(model_path, audio, input_rate, channels)
}

pub fn transcribe_from_mic_vad(model_path: &str, config: VadConfig) -> Result<String, String> {
    silence_whisper_logs();
    let recorder = start_recording()?;
    let start = Instant::now();

    let frame_samples_per_channel =
        (recorder.sample_rate as u64 * config.frame_ms as u64 / 1000) as usize;
    let channels = recorder.channels as usize;
    if frame_samples_per_channel == 0 || channels == 0 {
        return Err("입력 샘플레이트가 너무 낮습니다".to_string());
    }
    let frame_samples = frame_samples_per_channel * channels;
    let pre_roll_samples =
        (recorder.sample_rate as u64 * config.pre_roll_ms as u64 / 1000) as usize * channels;

    let mut processed = 0usize;
    let mut speaking = false;
    let mut voiced_frames = 0usize;
    let mut silence_ms = 0u32;
    let mut speech_start = 0usize;
    let mut speech_end = 0usize;

    loop {
        if start.elapsed() > Duration::from_millis(config.max_record_ms as u64) {
            if speaking {
                let data = recorder.buffer.lock().unwrap();
                speech_end = data.len();
            } else {
                drop(recorder);
                return Err("음성이 감지되지 않았습니다".to_string());
            }
            break;
        }

        let available = {
            let data = recorder.buffer.lock().unwrap();
            data.len()
        };

        if available < processed + frame_samples {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        let (rms, frame_end) = {
            let data = recorder.buffer.lock().unwrap();
            let frame = &data[processed..processed + frame_samples];
            (rms_energy(frame, recorder.channels), processed + frame_samples)
        };

        let voiced = rms >= config.start_threshold;
        if !speaking {
            if voiced {
                voiced_frames += 1;
            } else {
                voiced_frames = 0;
            }
            if voiced_frames >= config.start_frames {
                speaking = true;
                speech_start = processed.saturating_sub(pre_roll_samples);
                silence_ms = 0;
            }
        } else if voiced {
            silence_ms = 0;
        } else {
            silence_ms += config.frame_ms;
            if silence_ms >= config.end_silence_ms {
                speech_end = frame_end;
                break;
            }
        }

        processed = frame_end;
    }

    let mut data = recorder.buffer.lock().unwrap().clone();
    let sample_rate = recorder.sample_rate;
    let channels = recorder.channels;
    drop(recorder);

    if speech_end <= speech_start || speech_end > data.len() {
        return Err("유효한 음성 구간을 찾지 못했습니다".to_string());
    }

    let audio = data.drain(speech_start..speech_end).collect::<Vec<_>>();
    transcribe_audio(model_path, audio, sample_rate, channels)
}

pub fn start_recording() -> Result<VoiceRecording, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "마이크 장치가 없습니다".to_string())?;

    let supported = device
        .default_input_config()
        .map_err(|e| format!("입력 디바이스 설정 실패: {}", e))?;

    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let sample_format = supported.sample_format();

    let config: StreamConfig = supported.into();
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_clone = Arc::clone(&buffer);

    let err_fn = |_err| {};

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                &config,
                move |data: &[f32], _| {
                    buffer_clone.lock().unwrap().extend_from_slice(data);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("스트림 생성 실패: {}", e))?,
        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let mut b = buffer_clone.lock().unwrap();
                    b.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("스트림 생성 실패: {}", e))?,
        SampleFormat::U16 => device
            .build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let mut b = buffer_clone.lock().unwrap();
                    b.extend(data.iter().map(|&s| {
                        (s as f32 / u16::MAX as f32) * 2.0 - 1.0
                    }));
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("스트림 생성 실패: {}", e))?,
        _ => return Err("지원하지 않는 샘플 포맷".to_string()),
    };

    stream
        .play()
        .map_err(|e| format!("스트림 재생 실패: {}", e))?;

    Ok(VoiceRecording {
        stream,
        buffer,
        sample_rate,
        channels,
    })
}

impl VoiceRecording {
    pub fn stop(self) -> (Vec<f32>, u32, u16) {
        drop(self.stream);
        let data = self.buffer.lock().unwrap().clone();
        (data, self.sample_rate, self.channels)
    }
}

pub fn transcribe_audio(
    model_path: &str,
    audio: Vec<f32>,
    input_rate: u32,
    channels: u16,
) -> Result<String, String> {
    silence_whisper_logs();
    if audio.is_empty() {
        return Err("녹음된 오디오가 비어 있음".to_string());
    }
    let audio_16k = to_16k_mono(audio, input_rate, channels);
    transcribe_whisper(model_path, &audio_16k)
}

/// interleaved f32 → mono + 16kHz
fn to_16k_mono(interleaved: Vec<f32>, input_rate: u32, channels: u16) -> Vec<f32> {
    let channels = channels as usize;
    let mono = if channels <= 1 {
        interleaved
    } else {
        let frames = interleaved.len() / channels;
        let mut out = Vec::with_capacity(frames);
        for frame in 0..frames {
            let base = frame * channels;
            let mut sum = 0.0f32;
            for ch in 0..channels {
                sum += interleaved[base + ch];
            }
            out.push(sum / channels as f32);
        }
        out
    };

    if input_rate == 16_000 {
        return mono;
    }

    linear_resample(&mono, input_rate, 16_000)
}

/// 간단 선형 리샘플링
fn linear_resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if input.is_empty() {
        return vec![];
    }

    let ratio = output_rate as f64 / input_rate as f64;
    let out_len = (input.len() as f64 * ratio) as usize;

    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let pos = n as f64 / ratio;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let t = (pos - i0 as f64) as f32;

        out.push(input[i0] * (1.0 - t) + input[i1] * t);
    }

    out
}

fn rms_energy(frame: &[f32], channels: u16) -> f32 {
    let channels = channels as usize;
    if channels == 0 || frame.is_empty() {
        return 0.0;
    }
    let frames = frame.len() / channels;
    if frames == 0 {
        return 0.0;
    }
    let mut sum_sq = 0.0f64;
    for i in 0..frames {
        let base = i * channels;
        let mut sum = 0.0f32;
        for ch in 0..channels {
            sum += frame[base + ch];
        }
        let mono = sum / channels as f32;
        sum_sq += (mono as f64) * (mono as f64);
    }
    ((sum_sq / frames as f64) as f32).sqrt()
}

fn transcribe_whisper(model_path: &str, audio_16k: &[f32]) -> Result<String, String> {
    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("Whisper 모델 로드 실패: {}", e))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("Whisper state 생성 실패: {}", e))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("ko"));
    params.set_translate(false);

    state
        .full(params, audio_16k)
        .map_err(|e| format!("Whisper 추론 실패: {}", e))?;

    let n = state
        .full_n_segments()
        .map_err(|e| format!("세그먼트 읽기 실패: {}", e))?;
    let mut result = String::new();

    for i in 0..n {
        let seg = state
            .full_get_segment_text(i)
            .map_err(|e| format!("세그먼트 텍스트 읽기 실패: {}", e))?;
        result.push_str(&seg);
    }

    Ok(result)
}

pub fn silence_whisper_logs() {
    static INIT: Once = Once::new();
    unsafe {
        INIT.call_once(|| {
            whisper_rs::set_log_callback(Some(whisper_log_callback), std::ptr::null_mut());
        });
    }
}

unsafe extern "C" fn whisper_log_callback(
    _level: ggml_log_level,
    _text: *const c_char,
    _user_data: *mut c_void,
) {
}
