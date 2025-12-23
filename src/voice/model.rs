use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
const MODEL_FILENAME: &str = "ggml-base.bin";

pub struct PreparedModel {
    pub path: String,
    pub downloaded: bool,
}

pub fn prepare_model_path_with_status<F>(mut on_status: F) -> Result<PreparedModel, String>
where
    F: FnMut(&str),
{
    let path = resolve_model_path()?;
    if path.exists() {
        if path.is_file() {
            return Ok(PreparedModel {
                path: path_to_string(path)?,
                downloaded: false,
            });
        }
        return Err("모델 경로가 파일이 아닙니다".to_string());
    }

    on_status("모델이 없어 다운로드합니다...");
    download_model(&path)?;
    on_status("모델 다운로드 완료");
    Ok(PreparedModel {
        path: path_to_string(path)?,
        downloaded: true,
    })
}

fn resolve_model_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("WHISPER_MODEL") {
        return Ok(PathBuf::from(path));
    }

    default_model_path()
}

fn default_model_path() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .map_err(|e| format!("현재 디렉토리를 가져올 수 없습니다: {}", e))?;
    Ok(cwd.join("models").join(MODEL_FILENAME))
}

fn download_model(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("모델 디렉토리 생성 실패: {}", e))?;
    }

    let tmp_path = path.with_extension("part");
    let result = (|| {
        let mut response =
            reqwest::blocking::get(MODEL_URL).map_err(|e| format!("다운로드 요청 실패: {}", e))?;
        response = response
            .error_for_status()
            .map_err(|e| format!("다운로드 응답 오류: {}", e))?;

        let mut file = File::create(&tmp_path)
            .map_err(|e| format!("임시 파일 생성 실패: {}", e))?;
        std::io::copy(&mut response, &mut file)
            .map_err(|e| format!("다운로드 저장 실패: {}", e))?;
        file.flush()
            .map_err(|e| format!("다운로드 파일 플러시 실패: {}", e))?;

        fs::rename(&tmp_path, path).map_err(|e| format!("모델 파일 저장 실패: {}", e))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    result
}

fn path_to_string(path: PathBuf) -> Result<String, String> {
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "모델 경로가 UTF-8이 아닙니다".to_string())
}
