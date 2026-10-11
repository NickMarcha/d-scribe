//! Download Whisper models from Hugging Face.

use std::path::{Path, PathBuf};

/// Model source: (model_id, hf_repo, hf_filename, local_filename)
pub const MODEL_SOURCES: &[(&str, &str, &str, &str)] = &[
    // ggerganov
    (
        "tiny.en",
        "ggerganov/whisper.cpp",
        "ggml-tiny.en.bin",
        "ggml-tiny.en.bin",
    ),
    (
        "tiny",
        "ggerganov/whisper.cpp",
        "ggml-tiny.bin",
        "ggml-tiny.bin",
    ),
    (
        "base.en",
        "ggerganov/whisper.cpp",
        "ggml-base.en.bin",
        "ggml-base.en.bin",
    ),
    (
        "base",
        "ggerganov/whisper.cpp",
        "ggml-base.bin",
        "ggml-base.bin",
    ),
    (
        "small.en",
        "ggerganov/whisper.cpp",
        "ggml-small.en.bin",
        "ggml-small.bin",
    ),
    (
        "small",
        "ggerganov/whisper.cpp",
        "ggml-small.bin",
        "ggml-small.bin",
    ),
    (
        "medium.en",
        "ggerganov/whisper.cpp",
        "ggml-medium.en.bin",
        "ggml-medium.en.bin",
    ),
    (
        "medium",
        "ggerganov/whisper.cpp",
        "ggml-medium.bin",
        "ggml-medium.bin",
    ),
    (
        "large-v3",
        "ggerganov/whisper.cpp",
        "ggml-large-v3.bin",
        "ggml-large-v3.bin",
    ),
    (
        "large-v3-turbo",
        "ggerganov/whisper.cpp",
        "ggml-large-v3-turbo.bin",
        "ggml-large-v3-turbo.bin",
    ),
    // NbAiLab Norwegian
    (
        "nb-whisper-tiny",
        "NbAiLab/nb-whisper-tiny",
        "ggml-model.bin",
        "nb-whisper-tiny.bin",
    ),
    (
        "nb-whisper-base",
        "NbAiLab/nb-whisper-base",
        "ggml-model.bin",
        "nb-whisper-base.bin",
    ),
    (
        "nb-whisper-small",
        "NbAiLab/nb-whisper-small",
        "ggml-model.bin",
        "nb-whisper-small.bin",
    ),
    (
        "nb-whisper-medium",
        "NbAiLab/nb-whisper-medium",
        "ggml-model.bin",
        "nb-whisper-medium.bin",
    ),
    (
        "nb-whisper-large",
        "NbAiLab/nb-whisper-large",
        "ggml-model.bin",
        "nb-whisper-large.bin",
    ),
];

/// Download with progress callback. Callback receives (bytes_downloaded, total_bytes).
/// total_bytes is None if Content-Length header is missing.
pub async fn download_model_with_progress<F>(
    models_dir: &Path,
    model_name: &str,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let (_, hf_repo, hf_filename, local_filename) = MODEL_SOURCES
        .iter()
        .find(|(id, _, _, _)| *id == model_name)
        .ok_or_else(|| {
            format!(
                "Unknown model: {}. Available: {:?}",
                model_name,
                MODEL_SOURCES
                    .iter()
                    .map(|(id, _, _, _)| *id)
                    .collect::<Vec<_>>()
            )
        })?;

    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        hf_repo, hf_filename
    );
    let output_path = models_dir.join(local_filename);

    if output_path.exists() {
        return Ok(output_path.to_string_lossy().into_owned());
    }

    super::download::download_to_file(&url, &output_path, &mut on_progress).await?;
    Ok(output_path.to_string_lossy().into_owned())
}

/// Resolve model name (e.g. "base.en", "tiny", "nb-whisper-base") to full path if the model file exists.
pub fn resolve_model_path(models_dir: &Path, model_name: &str) -> Option<PathBuf> {
    let (_, _, _, local_filename) = MODEL_SOURCES
        .iter()
        .find(|(id, _, _, _)| *id == model_name)?;
    let path = models_dir.join(*local_filename);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// List model names for .bin files that exist in models_dir and match known MODEL_SOURCES.
pub fn list_installed_model_names(models_dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if !models_dir.exists() {
        return names;
    }
    let Ok(entries) = std::fs::read_dir(models_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "bin") {
            if let Some(name) = path.file_name().and_then(|f| f.to_str()) {
                if let Some((model_id, _, _, _)) =
                    MODEL_SOURCES.iter().find(|(_, _, _, local)| *local == name)
                {
                    names.push((*model_id).to_string());
                }
            }
        }
    }
    names
}
