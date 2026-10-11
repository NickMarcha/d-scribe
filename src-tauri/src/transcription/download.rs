//! Large file downloads that survive stalled connections.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// No data for this long counts as a stalled connection.
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Reconnect attempts after a stall or dropped connection; each one resumes where it stopped.
const MAX_RETRIES: u32 = 8;

/// Downloads `url` to `dest`. Writes to `dest` + ".part" and renames when complete, so an
/// interrupted download never looks finished. Resumes with a Range request after a stall.
pub async fn download_to_file<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;

    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")
    ));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut total: Option<u64> = None;
    let mut attempt = 0;

    loop {
        let mut req = client.get(url);
        if downloaded > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", downloaded));
        }
        let result: Result<(), String> = async {
            let resp = req.send().await.map_err(|e| e.to_string())?;
            let status = resp.status();
            if downloaded > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(format!("Server did not resume the download ({})", status));
            }
            if !status.is_success() {
                return Err(format!("Download failed ({}): {}", status, url));
            }
            if total.is_none() {
                total = resp.content_length();
            }
            let mut stream = resp.bytes_stream();
            loop {
                match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
                    Err(_) => return Err("Download stalled".into()),
                    Ok(None) => return Ok(()),
                    Ok(Some(chunk)) => {
                        let bytes = chunk.map_err(|e| e.to_string())?;
                        file.write_all(&bytes).map_err(|e| e.to_string())?;
                        downloaded += bytes.len() as u64;
                        on_progress(downloaded, total);
                    }
                }
            }
        }
        .await;

        let complete = total.map_or(true, |t| downloaded >= t);
        match result {
            Ok(()) if complete => break,
            Ok(()) => log::warn!("[download] Connection closed early at {} bytes, resuming", downloaded),
            Err(e) if e.starts_with("Download failed") || e.starts_with("Server did not") => {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
            Err(e) => log::warn!("[download] {} at {} bytes, resuming", e, downloaded),
        }
        attempt += 1;
        if attempt > MAX_RETRIES {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(format!("Download kept stalling. Check your connection and try again. ({})", url));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}
