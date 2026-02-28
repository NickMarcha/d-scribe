//! Download Whisper models from Hugging Face.

use std::path::Path;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Known model files and approximate sizes (for progress display).
pub const MODELS: &[(&str, &str)] = &[
    ("ggml-tiny.en.bin", "tiny.en"),
    ("ggml-tiny.bin", "tiny"),
    ("ggml-base.en.bin", "base.en"),
    ("ggml-base.bin", "base"),
    ("ggml-small.en.bin", "small.en"),
    ("ggml-small.bin", "small"),
    ("ggml-medium.en.bin", "medium.en"),
    ("ggml-medium.bin", "medium"),
    ("ggml-large-v3.bin", "large-v3"),
];

/// Download a model file to the models directory.
/// model_name: e.g. "base.en", "tiny", "small"
pub async fn download_model(models_dir: &Path, model_name: &str) -> Result<String, String> {
    let (filename, _) = MODELS
        .iter()
        .find(|(_, name)| *name == model_name)
        .ok_or_else(|| format!("Unknown model: {}. Available: {:?}", model_name, MODELS.iter().map(|(_, n)| *n).collect::<Vec<_>>()))?;

    let url = format!("{}/{}", HF_BASE, filename);
    let output_path = models_dir.join(filename);

    if output_path.exists() {
        return Ok(output_path.to_string_lossy().into_owned());
    }

    let response = reqwest::get(&url)
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed: {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&output_path, &bytes).map_err(|e| e.to_string())?;

    Ok(output_path.to_string_lossy().into_owned())
}
