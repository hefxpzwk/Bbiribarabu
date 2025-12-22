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
