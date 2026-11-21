//! Large file downloads that survive stalled connections.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

/// No data for this long counts as a stalled connection.
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Reconnect attempts after a stall or dropped connection; each one resumes where it stopped.
const MAX_RETRIES: u32 = 8;

static PATH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

/// Waits until nothing else holds `path`, then holds it until the guard is dropped. Used so
/// two downloads or installs of the same file (Settings and a transcription, say) never
/// write it at the same time: the second one waits and then finds the work done.
pub async fn lock_path(path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = PATH_LOCKS.lock().unwrap().entry(path.to_path_buf()).or_default().clone();
    lock.lock_owned().await
}

/// Downloads `url` to `dest`. If another download to `dest` is running, waits for it and
/// returns without downloading again when it left the file in place. Writes to `dest` + ".part" and renames when complete, so an
/// interrupted download never looks finished. Resumes with a Range request after a stall.
pub async fn download_to_file<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;

    let _lock = lock_path(dest).await;
    if dest.exists() {
        return Ok(());
    }

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

#[cfg(test)]
mod tests {
    use super::download_to_file;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const BODY_LEN: usize = 4 * 1024 * 1024;
    const CHUNK: usize = 64 * 1024;
    /// 64 chunks with this pause between them: one download takes about a second.
    const CHUNK_PAUSE: Duration = Duration::from_millis(15);

    fn body() -> Vec<u8> {
        (0..BODY_LEN).map(|i| ((i * 31 + 7) % 251) as u8).collect()
    }

    /// A tiny HTTP/1.1 server that sends `body` slowly to every GET, one connection per request.
    async fn serve_slowly(body: Arc<Vec<u8>>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    if socket.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    for chunk in body.chunks(CHUNK) {
                        if socket.write_all(chunk).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(CHUNK_PAUSE).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        addr
    }

    /// When a download first and last reported progress, i.e. while it streamed into the file.
    type Span = Arc<Mutex<Option<(Instant, Instant)>>>;

    fn overlap(a: (Instant, Instant), b: (Instant, Instant)) -> bool {
        a.0 < b.1 && b.0 < a.1
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_downloads_to_the_same_file_do_not_both_stream_into_it() {
        let body = Arc::new(body());
        let addr = serve_slowly(body.clone()).await;
        let url = format!("http://{}/ggml-test.bin", addr);
        let dir = std::env::temp_dir().join(format!("d-scribe-download-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("ggml-test.bin");

        let start = |span: Span| {
            let url = url.clone();
            let dest = dest.clone();
            tokio::spawn(async move {
                download_to_file(&url, &dest, move |_done, _total| {
                    let now = Instant::now();
                    let mut s = span.lock().unwrap();
                    *s = Some(match *s {
                        Some((first, _)) => (first, now),
                        None => (now, now),
                    });
                })
                .await
            })
        };
        let span_a: Span = Arc::new(Mutex::new(None));
        let span_b: Span = Arc::new(Mutex::new(None));
        let a = start(span_a.clone());
        let b = start(span_b.clone());
        let (ra, rb) = tokio::time::timeout(Duration::from_secs(60), async { (a.await.unwrap(), b.await.unwrap()) })
            .await
            .expect("downloads did not finish within 60 s");

        let written = std::fs::read(&dest);
        let _ = std::fs::remove_dir_all(&dir);

        // Correct content, whichever download wrote it.
        assert!(ra.is_ok() || rb.is_ok(), "both downloads failed: {:?} / {:?}", ra, rb);
        let written = written.expect("the destination file is missing");
        assert_eq!(written.len(), body.len(), "the file has the wrong length");
        assert!(written == *body, "the file differs from what the server sent");

        // Only one of them streams into the file at a time: the other errors, or waits.
        let (sa, sb) = (*span_a.lock().unwrap(), *span_b.lock().unwrap());
        if let (Some(sa), Some(sb)) = (sa, sb) {
            assert!(
                !overlap(sa, sb),
                "both downloads streamed into the file at the same time (results: {:?} / {:?})",
                ra,
                rb
            );
        }
    }
}
