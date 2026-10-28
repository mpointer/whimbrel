//! Model download for a fresh install (M3.0 part 2, Task 3), moved from apps/macos
//! `ModelDownloadService.swift`: the three files transcription and diarization need
//! (whisper `ggml-tiny.en.bin`, the pyannote segmentation model inside a `.tar.bz2`,
//! the wespeaker embedding model), their size floors, and the download-verify-move
//! sequence. Driven the same way as [`crate::inference`]: [`download_all_start`]
//! returns a job id and runs on its own thread; the shell polls
//! [`download_poll`] from its own timer and calls [`download_cancel`] /
//! [`download_release`]. No `Store` anywhere. Destination paths are the shell's
//! (they are user-overridable in Settings on macOS); nothing here computes a path.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Where the three files go. Serialized field names are what the FFI accepts.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelDestinations {
    pub whisper_model_path: PathBuf,
    pub segmentation_model_path: PathBuf,
    pub embedding_model_path: PathBuf,
}

/// One step of the download. `archive_member` is set when the URL is a `.tar.bz2` and
/// only that member is wanted (extracted to `destination`).
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub name: &'static str,
    pub url: String,
    pub destination: PathBuf,
    /// Below this size a download is treated as truncated or an HTML error page, not
    /// a model. Deliberately loose (well under the real ~75 MB / ~7 MB / ~26 MB).
    pub min_bytes: u64,
    /// Share of the overall progress fraction this step represents.
    pub weight: f64,
    pub archive_member: Option<&'static str>,
    /// Whole-request timeout, the same role `URLSessionConfiguration.timeoutIntervalForResource` had.
    pub timeout: Duration,
}

const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin";
const SEGMENTATION_ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2";
const EMBEDDING_MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx";
const SEGMENTATION_ARCHIVE_MEMBER: &str = "sherpa-onnx-pyannote-segmentation-3-0/model.onnx";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The production manifest for `destinations`.
pub fn manifest(destinations: &ModelDestinations) -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            name: "Transcription model",
            url: WHISPER_MODEL_URL.to_string(),
            destination: destinations.whisper_model_path.clone(),
            min_bytes: 10_000_000,
            weight: 0.6,
            archive_member: None,
            timeout: DOWNLOAD_TIMEOUT,
        },
        ModelSpec {
            name: "Speaker segmentation model",
            url: SEGMENTATION_ARCHIVE_URL.to_string(),
            destination: destinations.segmentation_model_path.clone(),
            min_bytes: 1_000_000,
            weight: 0.1,
            archive_member: Some(SEGMENTATION_ARCHIVE_MEMBER),
            timeout: DOWNLOAD_TIMEOUT,
        },
        ModelSpec {
            name: "Speaker embedding model",
            url: EMBEDDING_MODEL_URL.to_string(),
            destination: destinations.embedding_model_path.clone(),
            min_bytes: 5_000_000,
            weight: 0.3,
            archive_member: None,
            timeout: DOWNLOAD_TIMEOUT,
        },
    ]
}

const MINILM_BASE_URL: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main";

/// The semantic-search embedding model `core/memory` loads (all-MiniLM-L6-v2's
/// `config.json`, `tokenizer.json` and `model.safetensors`), for a shell that downloads
/// it instead of bundling it (Windows, M3.2 Task 7). Same files and source as
/// `apps/macos/scripts/release-publish.sh` and CI, so every device embeds identically.
/// `dir` is the store's `models_dir`: the loader joins the file names directly onto it.
/// The weights go last, so a failed weights download leaves the two small files in
/// place and a retry fetches only what is missing.
pub fn embedding_manifest(dir: &Path) -> Vec<ModelSpec> {
    [
        ("Search model configuration", "config.json", 200, 0.01),
        ("Search model tokenizer", "tokenizer.json", 100_000, 0.04),
        ("Search model weights", "model.safetensors", 50_000_000, 0.95),
    ]
    .into_iter()
    .map(|(name, file, min_bytes, weight)| ModelSpec {
        name,
        url: format!("{MINILM_BASE_URL}/{file}"),
        destination: dir.join(file),
        min_bytes,
        weight,
        archive_member: None,
        timeout: DOWNLOAD_TIMEOUT,
    })
    .collect()
}

/// A snapshot of a job, as [`download_poll`] returns it. `status` is display-ready.
/// Exactly one of `done && error.is_none() && !cancelled` (success), `error.is_some()`
/// (failure) or `cancelled` (stopped by [`download_cancel`]) holds once `done`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DownloadProgress {
    pub fraction: f64,
    pub status: String,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

struct Job {
    progress: Mutex<DownloadProgress>,
    cancel: AtomicBool,
}

static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();
static JOB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn jobs() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Downloads and installs whichever of the three files are missing (already-present
/// files are skipped, so re-running after a partial failure only fetches what is still
/// missing). Returns immediately with a job id; the work runs on its own thread.
pub fn download_all_start(destinations: &ModelDestinations) -> String {
    start_with_manifest(manifest(destinations))
}

/// [`download_all_start`] over an explicit manifest (tests point it at local servers).
pub fn start_with_manifest(specs: Vec<ModelSpec>) -> String {
    let job_id = format!(
        "models-{}-{}",
        std::process::id(),
        JOB_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let job = Arc::new(Job {
        progress: Mutex::new(DownloadProgress {
            fraction: 0.0,
            status: String::new(),
            done: false,
            cancelled: false,
            error: None,
        }),
        cancel: AtomicBool::new(false),
    });
    jobs().lock().unwrap().insert(job_id.clone(), Arc::clone(&job));
    std::thread::spawn(move || run_job(&job, &specs));
    job_id
}

/// The latest progress for `job_id`, or `None` if it is unknown (never started, or
/// already released).
pub fn download_poll(job_id: &str) -> Option<DownloadProgress> {
    let job = Arc::clone(jobs().lock().unwrap().get(job_id)?);
    let progress = job.progress.lock().unwrap().clone();
    Some(progress)
}

/// Requests cancellation. Returns `true` if the job was known; the job reports
/// `cancelled` through [`download_poll`] once its thread notices (between steps, or as
/// soon as the in-flight request's next chunk arrives or times out).
pub fn download_cancel(job_id: &str) -> bool {
    match jobs().lock().unwrap().get(job_id) {
        Some(job) => {
            job.cancel.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Frees the registry entry. Safe to call twice or while the thread is still running
/// (it holds its own `Arc` and finishes independently).
pub fn download_release(job_id: &str) {
    jobs().lock().unwrap().remove(job_id);
}

fn set_progress(job: &Job, fraction: f64, status: &str) {
    let mut p = job.progress.lock().unwrap();
    p.fraction = fraction;
    p.status = status.to_string();
}

fn run_job(job: &Job, specs: &[ModelSpec]) {
    let total_weight: f64 = specs.iter().map(|s| s.weight).sum();
    let mut completed_weight = 0.0;
    let mut outcome: Result<(), String> = Ok(());
    for spec in specs {
        if job.cancel.load(Ordering::Relaxed) {
            break;
        }
        set_progress(job, completed_weight / total_weight, &format!("Downloading {}…", spec.name));
        if let Err(e) = install_if_missing(spec, &job.cancel) {
            outcome = Err(e);
            break;
        }
        completed_weight += spec.weight;
        set_progress(job, completed_weight / total_weight, &format!("{} ready.", spec.name));
    }
    let mut p = job.progress.lock().unwrap();
    p.done = true;
    if job.cancel.load(Ordering::Relaxed) {
        p.cancelled = true;
    } else {
        match outcome {
            Ok(()) => {
                p.fraction = 1.0;
                p.status = "All models ready.".to_string();
            }
            Err(e) => p.error = Some(e),
        }
    }
}

fn install_if_missing(spec: &ModelSpec, cancel: &AtomicBool) -> Result<(), String> {
    if spec.destination.exists() {
        return Ok(());
    }
    match spec.archive_member {
        None => download_file(spec, &spec.destination, cancel),
        Some(member) => {
            let scratch = std::env::temp_dir().join(format!(
                "whimbrel-models-{}-{}",
                std::process::id(),
                JOB_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&scratch).map_err(|e| format!("Failed to download {}: {e}", spec.name))?;
            let result = (|| {
                let archive = scratch.join("archive.tar.bz2");
                download_file(spec, &archive, cancel)?;
                // Extract only the one member we want, into scratch (never straight
                // into `destination`'s directory, which is user-overridable), then
                // move that file into place.
                let extracted = extract_member(&archive, member, &scratch)
                    .ok_or_else(|| format!("Failed to unpack {}.", spec.name))?;
                move_into_place(&extracted, &spec.destination).map_err(|e| format!("Failed to unpack {}: {e}", spec.name))
            })();
            let _ = std::fs::remove_dir_all(&scratch);
            result
        }
    }
}

/// Downloads to a `.partial` sibling first, verifies the size floor, then moves it
/// into place, so a killed or interrupted download never leaves a file at the real
/// destination that an existence check would treat as complete.
fn download_file(spec: &ModelSpec, destination: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let name = spec.name;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to download {name}: {e}"))?;
    }
    let partial = partial_path(destination);
    let _ = std::fs::remove_file(&partial);

    let result = (|| {
        let agent = ureq::AgentBuilder::new().timeout(spec.timeout).build();
        let response = agent
            .get(&spec.url)
            .call()
            .map_err(|e| format!("Failed to download {name}: {e}"))?;
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&partial).map_err(|e| format!("Failed to download {name}: {e}"))?;
        let mut buf = [0u8; 64 * 1024];
        let mut size: u64 = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".to_string());
            }
            let n = reader.read(&mut buf).map_err(|e| format!("Failed to download {name}: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| format!("Failed to download {name}: {e}"))?;
            size += n as u64;
        }
        file.flush().map_err(|e| format!("Failed to download {name}: {e}"))?;
        drop(file);
        if size < spec.min_bytes {
            return Err(format!(
                "{name} downloaded but is unexpectedly small ({size} bytes) — likely a truncated download or an error page instead of the real file."
            ));
        }
        move_into_place(&partial, destination).map_err(|e| format!("Failed to download {name}: {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".partial");
    destination.with_file_name(name)
}

fn move_into_place(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(to);
    std::fs::rename(from, to)
}

/// Unpacks just `member` from the `.tar.bz2` at `archive` into `scratch`; `None` if
/// the archive is unreadable or has no such member.
fn extract_member(archive: &Path, member: &str, scratch: &Path) -> Option<PathBuf> {
    let file = std::fs::File::open(archive).ok()?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));
    for entry in tar.entries().ok()? {
        let mut entry = entry.ok()?;
        let path = entry.path().ok()?;
        if path == Path::new(member) && entry.header().entry_type().is_file() {
            let out = scratch.join("model.onnx");
            entry.unpack(&out).ok()?;
            return Some(out);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    /// One-shot HTTP server: serves `body` with a Content-Length of `declared_len`
    /// (defaults to the body length), then closes. A shorter body than declared
    /// simulates a truncated download. `hang` never responds at all.
    fn serve(body: Vec<u8>, declared_len: Option<usize>, hang: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            if hang {
                std::thread::sleep(Duration::from_secs(5));
                return;
            }
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let len = declared_len.unwrap_or(body.len());
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n").as_bytes(),
            );
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        });
        url
    }

    fn spec(name: &'static str, url: String, destination: PathBuf, min_bytes: u64, weight: f64) -> ModelSpec {
        ModelSpec { name, url, destination, min_bytes, weight, archive_member: None, timeout: Duration::from_secs(2) }
    }

    fn wait_done(job_id: &str) -> DownloadProgress {
        let start = Instant::now();
        loop {
            let p = download_poll(job_id).expect("job known");
            if p.done {
                download_release(job_id);
                return p;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "job did not finish: {p:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn tar_bz2_with(member: &str, contents: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let encoder = bzip2::write::BzEncoder::new(&mut out, bzip2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, member, contents).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    #[test]
    fn downloads_all_three_unpacks_the_archive_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let whisper = dir.path().join("models").join("ggml-tiny.en.bin");
        let seg = dir.path().join("diarize").join("segmentation.onnx");
        let emb = dir.path().join("diarize").join("embedding.onnx");
        let archive = tar_bz2_with(SEGMENTATION_ARCHIVE_MEMBER, b"onnx-seg");
        let mut seg_spec = spec("Speaker segmentation model", serve(archive, None, false), seg.clone(), 10, 0.1);
        seg_spec.archive_member = Some(SEGMENTATION_ARCHIVE_MEMBER);
        let job = start_with_manifest(vec![
            spec("Transcription model", serve(vec![1u8; 100], None, false), whisper.clone(), 50, 0.6),
            seg_spec,
            spec("Speaker embedding model", serve(vec![2u8; 100], None, false), emb.clone(), 50, 0.3),
        ]);
        let p = wait_done(&job);
        assert_eq!(p, DownloadProgress { fraction: 1.0, status: "All models ready.".into(), done: true, cancelled: false, error: None });
        assert_eq!(std::fs::read(&whisper).unwrap().len(), 100);
        assert_eq!(std::fs::read(&seg).unwrap(), b"onnx-seg");
        assert_eq!(std::fs::read(&emb).unwrap().len(), 100);
        assert!(!partial_path(&whisper).exists());
        assert!(download_poll(&job).is_none(), "released");
    }

    #[test]
    fn size_floor_rejects_a_small_file_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ggml-tiny.en.bin");
        let job = start_with_manifest(vec![spec("Transcription model", serve(b"<html>error</html>".to_vec(), None, false), dest.clone(), 1_000, 1.0)]);
        let p = wait_done(&job);
        assert!(p.done && !p.cancelled);
        let err = p.error.unwrap();
        assert!(err.starts_with("Transcription model downloaded but is unexpectedly small (18 bytes)"), "{err}");
        assert!(!dest.exists());
        assert!(!partial_path(&dest).exists());
        assert_eq!(p.status, "Downloading Transcription model…");
    }

    #[test]
    fn truncated_download_leaves_no_file_at_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ggml-tiny.en.bin");
        // Declares 10 000 bytes, sends 100, closes.
        let job = start_with_manifest(vec![spec("Transcription model", serve(vec![1u8; 100], Some(10_000), false), dest.clone(), 1_000, 1.0)]);
        let p = wait_done(&job);
        assert!(p.error.is_some(), "{p:?}");
        assert!(!dest.exists());
        assert!(!partial_path(&dest).exists());
    }

    #[test]
    fn already_present_files_are_skipped_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("present.bin");
        std::fs::write(&dest, b"keep me").unwrap();
        // Nothing listens on this port: a request would fail.
        let job = start_with_manifest(vec![spec("Transcription model", "http://127.0.0.1:1/file".into(), dest.clone(), 1_000, 1.0)]);
        let p = wait_done(&job);
        assert_eq!(p.error, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
    }

    #[test]
    fn archive_without_the_expected_member_fails_to_unpack() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("segmentation.onnx");
        let archive = tar_bz2_with("something-else/model.onnx", b"nope");
        let mut s = spec("Speaker segmentation model", serve(archive, None, false), dest.clone(), 10, 1.0);
        s.archive_member = Some(SEGMENTATION_ARCHIVE_MEMBER);
        let p = wait_done(&start_with_manifest(vec![s]));
        assert_eq!(p.error.as_deref(), Some("Failed to unpack Speaker segmentation model."));
        assert!(!dest.exists());
    }

    #[test]
    fn cancel_reports_cancelled_not_error_and_skips_later_steps() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.bin");
        let second = dir.path().join("second.bin");
        let job = start_with_manifest(vec![
            spec("Transcription model", serve(Vec::new(), None, true), first.clone(), 10, 0.5),
            spec("Speaker embedding model", serve(vec![2u8; 100], None, false), second.clone(), 10, 0.5),
        ]);
        std::thread::sleep(Duration::from_millis(50));
        assert!(download_cancel(&job));
        let p = wait_done(&job);
        assert!(p.cancelled && p.done && p.error.is_none(), "{p:?}");
        assert!(!first.exists() && !partial_path(&first).exists());
        assert!(!second.exists(), "later steps are skipped after a cancel");
        assert!(!download_cancel(&job), "released jobs are unknown");
    }

    #[test]
    fn embedding_manifest_is_the_three_minilm_files_the_mac_bundles() {
        let dir = Path::new("/k/models/minilm");
        let m = embedding_manifest(dir);
        let base = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main";
        assert_eq!(
            m.iter().map(|s| (s.url.as_str(), s.destination.clone(), s.min_bytes)).collect::<Vec<_>>(),
            [
                (format!("{base}/config.json").as_str(), dir.join("config.json"), 200),
                (format!("{base}/tokenizer.json").as_str(), dir.join("tokenizer.json"), 100_000),
                (format!("{base}/model.safetensors").as_str(), dir.join("model.safetensors"), 50_000_000),
            ]
        );
        assert!((m.iter().map(|s| s.weight).sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(m.iter().all(|s| s.archive_member.is_none()));
    }

    /// The embedding manifest with each URL pointed at a local server (`None`: a closed
    /// port, so fetching that file fails) and the size floors lowered to fit test bodies.
    fn local_embedding_manifest(dir: &Path, bodies: [Option<Vec<u8>>; 3]) -> Vec<ModelSpec> {
        embedding_manifest(dir)
            .into_iter()
            .zip(bodies)
            .map(|(spec, body)| ModelSpec {
                url: match body {
                    Some(body) => serve(body, None, false),
                    None => "http://127.0.0.1:9/closed".to_string(),
                },
                min_bytes: 4,
                timeout: Duration::from_secs(2),
                ..spec
            })
            .collect()
    }

    #[test]
    fn embedding_manifest_installs_the_files_where_the_loader_reads_them() {
        let dir = tempfile::tempdir().unwrap();
        let minilm = dir.path().join("models").join("minilm");
        let job = start_with_manifest(local_embedding_manifest(
            &minilm,
            [Some(b"{\"hidden_size\":384}".to_vec()), Some(b"{\"model\":{}}".to_vec()), Some(vec![7u8; 64])],
        ));
        let p = wait_done(&job);
        assert_eq!(p.error, None);
        assert_eq!(p.fraction, 1.0);
        assert_eq!(std::fs::read(minilm.join("config.json")).unwrap(), b"{\"hidden_size\":384}");
        assert_eq!(std::fs::read(minilm.join("tokenizer.json")).unwrap(), b"{\"model\":{}}");
        assert_eq!(std::fs::read(minilm.join("model.safetensors")).unwrap().len(), 64);
    }

    #[test]
    fn an_embedding_retry_fetches_only_the_missing_weights() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), b"kept-config").unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"kept-tokenizer").unwrap();
        // The two small files would fail to fetch, so success proves they were skipped.
        let job = start_with_manifest(local_embedding_manifest(dir.path(), [None, None, Some(vec![7u8; 64])]));
        let p = wait_done(&job);
        assert_eq!(p.error, None);
        assert_eq!(std::fs::read(dir.path().join("config.json")).unwrap(), b"kept-config");
        assert_eq!(std::fs::read(dir.path().join("model.safetensors")).unwrap().len(), 64);
    }

    #[test]
    fn production_manifest_matches_the_swift_service() {
        let d = ModelDestinations {
            whisper_model_path: "/m/w.bin".into(),
            segmentation_model_path: "/m/s.onnx".into(),
            embedding_model_path: "/m/e.onnx".into(),
        };
        let m = manifest(&d);
        assert_eq!(m.iter().map(|s| (s.name, s.min_bytes, s.weight)).collect::<Vec<_>>(), [
            ("Transcription model", 10_000_000, 0.6),
            ("Speaker segmentation model", 1_000_000, 0.1),
            ("Speaker embedding model", 5_000_000, 0.3),
        ]);
        assert_eq!(m[1].archive_member, Some(SEGMENTATION_ARCHIVE_MEMBER));
        assert_eq!(m[1].destination, PathBuf::from("/m/s.onnx"));
    }
}
