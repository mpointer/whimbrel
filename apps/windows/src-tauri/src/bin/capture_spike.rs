//! M3.1 capture spike CLI — docs/12-windows-app-design.md §9. No UI; each subcommand
//! is one measurement. Nothing here persists captured *text*: `uia`, `ocr`, `measure`
//! and `ambient` report counts and timings only (`--show N` prints a prefix to stdout
//! for eyeballing). `record` writes WAVs, as the product does (§4.4).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand, ValueEnum};
use whimbrel_windows::capture::{audio, ocr, policy, proc_stats, uia, window};
use windows::Win32::Foundation::HWND;

#[derive(Parser)]
#[command(name = "capture-spike", about = "Whimbrel M3.1 Windows capture spike")]
struct Cli {
    /// Windows 11 power throttling (EcoQoS) for this process: leave the OS default,
    /// opt out (`high`), or opt in (`eco`). A/B knob for transcription speed and for
    /// what ambient capture costs on E-cores vs P-cores.
    #[arg(long, value_enum, global = true, default_value = "default")]
    qos: Qos,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Qos {
    Default,
    High,
    Eco,
}

fn apply_qos(q: Qos) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };
    let state_mask = match q {
        Qos::Default => return,
        Qos::High => 0,
        Qos::Eco => PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: state_mask,
    };
    let r = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    eprintln!("qos {q:?}: {}", r.map_or_else(|e| e.to_string(), |_| "applied".into()));
}

#[derive(Args, Clone, Debug)]
struct Target {
    /// Window handle in hex, as printed by `windows`.
    #[arg(long)]
    hwnd: Option<String>,
    /// Executable basename, e.g. msedge.exe, ms-teams.exe, slack.exe, code.exe.
    #[arg(long)]
    exe: Option<String>,
    /// Case-insensitive substring of the window title (with --exe).
    #[arg(long)]
    title: Option<String>,
    /// Use whatever window is in the foreground after --delay seconds.
    #[arg(long)]
    foreground: bool,
    #[arg(long, default_value_t = 0)]
    delay: u64,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum StrategyArg {
    TextPattern,
    CachedWalk,
    NaiveWalk,
}

impl From<StrategyArg> for uia::Strategy {
    fn from(s: StrategyArg) -> Self {
        match s {
            StrategyArg::TextPattern => uia::Strategy::TextPattern,
            StrategyArg::CachedWalk => uia::Strategy::CachedWalk,
            StrategyArg::NaiveWalk => uia::Strategy::NaiveWalk,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Proves the whole core graph (memory's bundled SQLite, whisper.cpp, ONNX Runtime)
    /// links into one MSVC executable alongside the `windows` crate.
    LinkCheck,
    /// Lists visible top-level windows with their §4.3 app identity.
    Windows {
        #[arg(long)]
        exe: Option<String>,
    },
    /// UIA extraction on one window, one or more strategies.
    Uia {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_enum)]
        strategy: Vec<StrategyArg>,
        #[arg(long, default_value_t = 1)]
        repeat: usize,
        /// Print the first N characters of visible text per strategy.
        #[arg(long, default_value_t = 0)]
        show: usize,
    },
    /// OCR one window via Windows.Graphics.Capture, in memory.
    Ocr {
        #[command(flatten)]
        target: Target,
        /// Call GraphicsCaptureAccess.RequestAccessAsync(Borderless) first.
        #[arg(long)]
        request_access: bool,
        #[arg(long, default_value_t = 0)]
        show: usize,
    },
    /// One §4.1 table row: baseline CPU → OCR → UIA attach → post-attach CPU (idle,
    /// then with 5 s polling). Appends a JSON line to --out.
    Measure {
        #[command(flatten)]
        target: Target,
        /// Row label for the report, e.g. "Edge: Gmail thread".
        #[arg(long)]
        label: String,
        /// Seconds per CPU window.
        #[arg(long, default_value_t = 30)]
        cpu_secs: u64,
        /// Skip the naive-walk baseline (slow on big Chromium trees).
        #[arg(long)]
        skip_naive: bool,
        #[arg(long, default_value = "m31-measurements.jsonl")]
        out: PathBuf,
    },
    /// OCR + every UIA strategy on one window, scored by OCR-word recall. No CPU windows,
    /// so it is valid on an app that has already been attached.
    Yield {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        label: String,
        #[arg(long, default_value = "m31-yield.jsonl")]
        out: PathBuf,
    },
    /// CPU of a process tree over a window.
    Cpu {
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        exe: Option<String>,
        #[arg(long, default_value_t = 30)]
        secs: u64,
    },
    /// Per-process audio sessions on every active endpoint (§4.5).
    Sessions,
    /// M3.3 Task 6: logs, once a second, whether each meeting app's process tree holds an
    /// active output and input audio session — the signal a detected meeting ends on.
    WatchSessions {
        #[arg(long, default_value_t = 600)]
        secs: u64,
    },
    /// Endpoint loopback of the default render device in its native mix format (no
    /// engine conversion): format, packets, and peak. Play something while it runs.
    LoopbackProbe {
        #[arg(long, default_value_t = 5)]
        secs: u64,
    },
    /// Records loopback + mic to 16 kHz mono WAVs until --secs or Ctrl+C.
    Record {
        /// Process-loopback target: the tree rooted at this PID.
        #[arg(long)]
        pid: Option<u32>,
        /// Process-loopback target by exe; uses the tree root (e.g. ms-teams.exe).
        #[arg(long)]
        exe: Option<String>,
        /// Endpoint loopback instead of process loopback.
        #[arg(long)]
        endpoint: bool,
        /// Also record endpoint loopback alongside process loopback, for comparison.
        #[arg(long)]
        also_endpoint: bool,
        #[arg(long)]
        no_mic: bool,
        #[arg(long, default_value_t = 60)]
        secs: u64,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// mic + system WAVs → transcribe + diarize + postprocess → labeled transcript.
    Transcribe {
        #[arg(long)]
        mic: Option<PathBuf>,
        #[arg(long)]
        sys: PathBuf,
        #[arg(long)]
        models: Option<PathBuf>,
        /// Whisper model filename inside --models.
        #[arg(long, default_value = "ggml-tiny.en.bin")]
        whisper_model: String,
        /// Exact speaker count for the system channel, if known.
        #[arg(long)]
        speakers: Option<i32>,
    },
    /// The core/diarize pipeline at explicit ONNX thread counts (core/diarize itself
    /// takes sherpa-onnx's default of 1).
    DiarizeBench {
        #[arg(long)]
        wav: PathBuf,
        #[arg(long)]
        models: Option<PathBuf>,
        /// Comma-separated thread counts.
        #[arg(long, default_value = "1,2,4,8")]
        threads: String,
        #[arg(long)]
        speakers: Option<i32>,
    },
    /// Level stats for 16-bit WAVs: duration, peak, RMS, clipping, and a per-second peak
    /// profile (to see whether a stretch whisper skipped is silent, clipped, or intact).
    WavStats {
        files: Vec<PathBuf>,
    },
    /// Downloads the whisper + diarization models through core/api::models.
    Models {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Idle-capture loop at the §4 cadence; reports this process's CPU and RSS.
    Ambient {
        #[arg(long, default_value_t = 600)]
        secs: u64,
        #[arg(long, value_enum, default_value = "cached-walk")]
        strategy: StrategyArg,
        /// Capture this app's window instead of the foreground window — simulates the
        /// user sitting in that app without stealing focus from whoever is driving.
        #[arg(long)]
        pin_exe: Option<String>,
        #[arg(long)]
        pin_title: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    apply_qos(cli.qos);
    let result = match cli.cmd {
        Cmd::LinkCheck => {
            link_check();
            Ok(())
        }
        Cmd::Windows { exe } => list_windows(exe),
        Cmd::Uia { target, strategy, repeat, show } => cmd_uia(&target, &strategy, repeat, show),
        Cmd::Ocr { target, request_access, show } => cmd_ocr(&target, request_access, show),
        Cmd::Measure { target, label, cpu_secs, skip_naive, out } => cmd_measure(&target, &label, cpu_secs, skip_naive, &out),
        Cmd::Yield { target, label, out } => cmd_yield(&target, &label, &out),
        Cmd::Cpu { pid, exe, secs } => cmd_cpu(pid, exe, secs),
        Cmd::Sessions => cmd_sessions(),
        Cmd::WatchSessions { secs } => cmd_watch_sessions(secs),
        Cmd::LoopbackProbe { secs } => audio::probe_endpoint_loopback(secs)
            .map(|p| println!("{}", serde_json::to_string(&p).unwrap()))
            .map_err(|e| e.to_string()),
        Cmd::Record { pid, exe, endpoint, also_endpoint, no_mic, secs, out } => {
            cmd_record(pid, exe, endpoint, also_endpoint, no_mic, secs, out)
        }
        Cmd::Transcribe { mic, sys, models, whisper_model, speakers } => cmd_transcribe(mic, &sys, models, &whisper_model, speakers),
        Cmd::DiarizeBench { wav, models, threads, speakers } => cmd_diarize_bench(&wav, models, &threads, speakers),
        Cmd::WavStats { files } => cmd_wav_stats(&files),
        Cmd::Models { dir } => cmd_models(dir),
        Cmd::Ambient { secs, strategy, pin_exe, pin_title } => cmd_ambient(secs, strategy.into(), pin_exe, pin_title),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

// ---------- shared helpers ----------

fn whimbrel_dir() -> PathBuf {
    whimbrel_windows::paths::Paths::from_env().map_or_else(|| PathBuf::from("."), |p| p.root)
}

fn resolve(t: &Target) -> Result<window::WindowInfo, String> {
    if t.delay > 0 {
        eprintln!("waiting {} s — bring the target window to the front…", t.delay);
        std::thread::sleep(Duration::from_secs(t.delay));
    }
    if let Some(h) = &t.hwnd {
        let hwnd = window::hwnd_from_str(h).ok_or("bad --hwnd")?;
        return Ok(window::info(hwnd));
    }
    if t.foreground {
        return window::foreground().ok_or_else(|| "no foreground window".into());
    }
    let exe = t.exe.as_deref().ok_or("give --hwnd, --exe, or --foreground")?.to_lowercase();
    let title = t.title.as_deref().map(str::to_lowercase);
    window::top_level_windows()
        .into_iter()
        .filter(|w| w.exe == exe && !w.minimized)
        .find(|w| title.as_ref().map_or(true, |t| w.title.to_lowercase().contains(t)))
        .ok_or_else(|| format!("no visible {exe} window{}", t.title.as_ref().map(|t| format!(" titled *{t}*")).unwrap_or_default()))
}

/// The root of the process tree that owns `pid` and shares its exe (the browser
/// process for a renderer's window, `ms-teams.exe` for Teams). Falls back to `pid`.
fn tree_root_for(pid: u32, exe: &str) -> u32 {
    let all = proc_stats::list_processes();
    proc_stats::tree_roots_named(exe, &all)
        .into_iter()
        .find(|&r| proc_stats::process_tree(r, &all).contains(&pid))
        .unwrap_or(pid)
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn prefix(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

fn stop_flag() -> Arc<AtomicBool> {
    STOP.get_or_init(|| {
        unsafe extern "system" fn on_ctrl(_: u32) -> windows::core::BOOL {
            if let Some(s) = STOP.get() {
                s.store(true, Ordering::SeqCst);
            }
            true.into()
        }
        unsafe {
            let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), true);
        }
        Arc::new(AtomicBool::new(false))
    })
    .clone()
}

// ---------- subcommands ----------

fn link_check() {
    let store = memory::Store::open_in_memory().expect("in-memory store");
    drop(store);
    println!("memory:     rusqlite bundled SQLite + sqlite-vec OK");

    let specs = api::models::manifest(&api::models::ModelDestinations {
        whisper_model_path: PathBuf::from("w"),
        segmentation_model_path: PathBuf::from("s"),
        embedding_model_path: PathBuf::from("e"),
    });
    println!("api:        {} model specs", specs.len());

    // Nonexistent paths: these fail at runtime, but reaching the call proves whisper.cpp
    // and sherpa-onnx/ONNX Runtime linked.
    let missing = Path::new("does-not-exist.bin");
    match transcribe::Transcriber::load(missing) {
        Ok(_) => println!("transcribe: unexpectedly loaded"),
        Err(e) => println!("transcribe: linked (expected load error: {e})"),
    }
    let paths = diarize::DiarizationModelPaths {
        segmentation_model_path: missing.into(),
        embedding_model_path: missing.into(),
    };
    match diarize::diarize(missing, &paths, None) {
        Ok(_) => println!("diarize:    unexpectedly ran"),
        Err(e) => println!("diarize:    linked (expected error: {e})"),
    }

    let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    println!("windows:    foreground HWND {:?}", hwnd.0);
}

fn list_windows(exe: Option<String>) -> Result<(), String> {
    let exe = exe.map(|e| e.to_lowercase());
    for w in window::top_level_windows() {
        if exe.as_ref().map_or(true, |e| &w.exe == e) {
            println!("{}", serde_json::to_string(&w).unwrap());
        }
    }
    Ok(())
}

fn strategies(requested: &[StrategyArg]) -> Vec<uia::Strategy> {
    if requested.is_empty() {
        uia::Strategy::ALL.to_vec()
    } else {
        requested.iter().map(|&s| s.into()).collect()
    }
}

fn cmd_uia(t: &Target, requested: &[StrategyArg], repeat: usize, show: usize) -> Result<(), String> {
    let w = resolve(t)?;
    eprintln!("target: {} ({}) pid {}", w.app_id, w.exe, w.pid);
    let u = uia::Uia::new().map_err(|e| e.to_string())?;
    for s in strategies(requested) {
        for i in 0..repeat.max(1) {
            match u.extract(w.hwnd, s) {
                Ok(e) => {
                    println!("{}", serde_json::json!({"run": i, "extraction": e}));
                    if show > 0 && i == 0 {
                        println!("--- {:?} visible text ---\n{}\n---", s, prefix(&e.visible_text, show));
                    }
                }
                Err(err) => println!("{}", serde_json::json!({"run": i, "strategy": s, "error": err.to_string()})),
            }
        }
    }
    Ok(())
}

fn cmd_ocr(t: &Target, request_access: bool, show: usize) -> Result<(), String> {
    if request_access {
        match ocr::request_borderless_access() {
            Ok(status) => println!("{}", serde_json::json!({"borderless_access": format!("{status:?}")})),
            Err(e) => println!("{}", serde_json::json!({"borderless_access_error": e.to_string()})),
        }
    }
    let w = resolve(t)?;
    let engine = ocr::Ocr::new().map_err(|e| e.to_string())?;
    let out = engine.recognize_window(w.hwnd).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string(&out).unwrap());
    if show > 0 {
        println!("--- OCR text ---\n{}\n---", prefix(&out.text, show));
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct StrategyResult {
    strategy: uia::Strategy,
    /// Median over steady-state runs (after the attach has settled).
    median_ms: Option<f64>,
    runs: usize,
    chars_total: usize,
    chars_visible: usize,
    units: usize,
    truncated: bool,
    document_found: Option<bool>,
    /// UIA visible chars / OCR chars. Kept for the record; misleading on its own (see
    /// `ocr_word_recall`).
    visible_yield_vs_ocr: Option<f64>,
    /// Share of OCR's on-screen words present in this strategy's text — the §4.1
    /// "yield" column.
    ocr_word_recall: Option<f64>,
    error: Option<String>,
}

#[derive(serde::Serialize)]
struct MeasureRow {
    label: String,
    measured_at: String,
    app_id: String,
    exe: String,
    tree_root: u32,
    tree_processes: usize,
    tree_memory_before_mb: f64,
    tree_memory_after_mb: f64,
    cpu_before: proc_stats::CpuWindow,
    cpu_after_attach_idle: proc_stats::CpuWindow,
    cpu_during_5s_polling: proc_stats::CpuWindow,
    spike_cpu_during_polling: proc_stats::CpuWindow,
    polling_strategy: Option<uia::Strategy>,
    /// The first extraction after attach: what a capture sees the moment it touches a
    /// fresh Chromium process (the accessibility tree is built asynchronously after).
    first_contact_chars: usize,
    first_contact_ms: Option<f64>,
    /// From first contact until the extracted text stopped growing.
    settle_ms: Option<f64>,
    settled_chars: usize,
    ocr: Option<ocr::OcrOutput>,
    ocr_error: Option<String>,
    strategies: Vec<StrategyResult>,
    ocr_fires_mac_rule: bool,
    /// `policy::should_ocr` on the production pick — the Windows rule, whatever it is
    /// at build time.
    ocr_fires_windows_rule: bool,
    /// OCR-word recall of TextPattern ∪ CachedWalk — what production would capture.
    union_recall: Option<f64>,
}

fn cmd_measure(t: &Target, label: &str, cpu_secs: u64, skip_naive: bool, out: &Path) -> Result<(), String> {
    let w = resolve(t)?;
    let root = tree_root_for(w.pid, &w.exe);
    let win = Duration::from_secs(cpu_secs);
    eprintln!("[{label}] {} pid {} tree root {root}", w.app_id, w.pid);

    // 1. Baseline: nothing from this process has touched the target's UIA tree yet.
    eprintln!("  baseline CPU ({cpu_secs} s)…");
    let mem_before = proc_stats::tree_memory(root);
    let cpu_before = proc_stats::measure_tree_cpu(root, win);

    // 2. OCR — the "visible characters" reference. Doesn't touch accessibility.
    eprintln!("  OCR…");
    let (ocr_out, ocr_error) = match ocr::Ocr::new().and_then(|o| o.recognize_window(w.hwnd)) {
        Ok(o) => (Some(o), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let ocr_chars = ocr_out.as_ref().map(|o| o.chars).filter(|&c| c > 0);

    // 3. UIA attach. The first query is what switches Chromium's accessibility on, and
    // the tree is built asynchronously after it, so the first extraction sees a partial
    // tree. Record that first contact, then poll until the text stops growing.
    let u = uia::Uia::new().map_err(|e| e.to_string())?;
    eprintln!("  UIA attach + settle…");
    let attach = Instant::now();
    let first = u.extract(w.hwnd, uia::Strategy::CachedWalk).ok();
    let (first_contact_chars, first_contact_ms) = (first.as_ref().map_or(0, |e| e.chars_total), first.as_ref().map(|e| e.latency_ms));
    let (mut last, mut last_change, mut stable) = (first_contact_chars, Duration::ZERO, 0);
    let mut settle_ms = None;
    while attach.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(500));
        let now = u.extract(w.hwnd, uia::Strategy::CachedWalk).map_or(0, |e| e.chars_total);
        if now == last && now > 0 {
            stable += 1;
            if stable >= 2 {
                settle_ms = Some(last_change.as_secs_f64() * 1000.0);
                break;
            }
        } else {
            (last, last_change, stable) = (now, attach.elapsed(), 0);
        }
    }

    // 4. Steady state, per strategy.
    let ocr_text = ocr_out.as_ref().map_or("", |o| o.text.as_str());
    let (results, union_recall) = run_strategies(&u, w.hwnd, ocr_text, ocr_chars, skip_naive);
    let best = production_pick(&results);
    let best_total = best.and_then(|b| results.iter().find(|r| r.strategy == b)).map_or(0, |r| r.chars_total);

    // 4. Accessibility is now on inside the target. Idle cost, then polling cost.
    eprintln!("  post-attach idle CPU ({cpu_secs} s)…");
    let cpu_after = proc_stats::measure_tree_cpu(root, win);

    eprintln!("  CPU during 5 s polling ({cpu_secs} s)…");
    let me = [std::process::id()];
    let (t_before, me_before) = (
        proc_stats::CpuSnapshot::take(&proc_stats::process_tree(root, &proc_stats::list_processes())),
        proc_stats::CpuSnapshot::take(&me),
    );
    let deadline = Instant::now() + win;
    while Instant::now() < deadline {
        if let Some(b) = best {
            let _ = u.extract(w.hwnd, b);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(left.min(Duration::from_secs(5)));
    }
    let cpu_polling = proc_stats::CpuSnapshot::take(&proc_stats::process_tree(root, &proc_stats::list_processes())).since(&t_before);
    let spike_polling = proc_stats::CpuSnapshot::take(&me).since(&me_before);
    let mem_after = proc_stats::tree_memory(root);

    // Before the literal: `strategies: results` moves the vector.
    let ocr_fires_windows_rule = policy::should_ocr(&policy::FallbackInput {
        app_id: &w.app_id,
        exe: &w.exe,
        uia_chars: best_total,
        document_found: results.iter().any(|r| r.document_found == Some(true)),
    });
    let row = MeasureRow {
        label: label.to_string(),
        measured_at: chrono::Local::now().to_rfc3339(),
        app_id: w.app_id.clone(),
        exe: w.exe.clone(),
        tree_root: root,
        tree_processes: proc_stats::process_tree(root, &proc_stats::list_processes()).len(),
        tree_memory_before_mb: mb(mem_before.private_bytes),
        tree_memory_after_mb: mb(mem_after.private_bytes),
        cpu_before,
        cpu_after_attach_idle: cpu_after,
        cpu_during_5s_polling: cpu_polling,
        spike_cpu_during_polling: spike_polling,
        polling_strategy: best,
        first_contact_chars,
        first_contact_ms,
        settle_ms,
        settled_chars: last,
        ocr: ocr_out,
        ocr_error,
        strategies: results,
        union_recall,
        ocr_fires_mac_rule: policy::mac_parity_fires(best_total),
        ocr_fires_windows_rule,
    };
    let line = serde_json::to_string(&row).unwrap();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .and_then(|mut f| writeln!(f, "{line}"))
        .map_err(|e| e.to_string())?;
    print_row(&row);
    Ok(())
}

/// Each strategy at steady state: 3 runs (1 for the naive walk), median latency, the
/// last run's sizes, and its OCR-word recall.
/// Also returns the OCR-word recall of TextPattern ∪ CachedWalk text — what production,
/// which runs both and keeps the union, would capture.
fn run_strategies(u: &uia::Uia, hwnd: HWND, ocr_text: &str, ocr_chars: Option<usize>, skip_naive: bool) -> (Vec<StrategyResult>, Option<f64>) {
    let mut results = Vec::new();
    let mut texts = Vec::new();
    for s in uia::Strategy::ALL {
        if skip_naive && s == uia::Strategy::NaiveWalk {
            continue;
        }
        eprintln!("  UIA {s:?}…");
        let runs = if s == uia::Strategy::NaiveWalk { 1 } else { 3 };
        let (mut lat, mut last_ok, mut err) = (Vec::new(), None, None);
        for _ in 0..runs {
            match u.extract(hwnd, s) {
                Ok(e) => {
                    lat.push(e.latency_ms);
                    last_ok = Some(e);
                }
                Err(e) => err = Some(e.to_string()),
            }
        }
        let e = last_ok.as_ref();
        if s != uia::Strategy::NaiveWalk {
            if let Some(e) = e {
                texts.push(e.text.clone());
            }
        }
        results.push(StrategyResult {
            strategy: s,
            median_ms: (!lat.is_empty()).then(|| median(lat.clone())),
            runs: lat.len(),
            chars_total: e.map_or(0, |e| e.chars_total),
            chars_visible: e.map_or(0, |e| e.chars_visible),
            units: e.map_or(0, |e| e.units),
            truncated: e.map_or(false, |e| e.truncated),
            document_found: e.map(|e| e.document_found),
            visible_yield_vs_ocr: e.and_then(|e| ocr_chars.map(|c| e.chars_visible as f64 / c as f64)),
            ocr_word_recall: e.and_then(|e| uia::ocr_word_recall(ocr_text, &e.text)),
            error: if e.is_some() { None } else { err },
        });
    }
    let union = uia::ocr_word_recall(ocr_text, &texts.join("\n"));
    (results, union)
}

/// What production would poll with: the fastest strategy recovering at least 80% of the
/// words OCR reads on screen; failing that, the one with the best recall.
fn production_pick(results: &[StrategyResult]) -> Option<uia::Strategy> {
    let candidates: Vec<&StrategyResult> =
        results.iter().filter(|r| r.error.is_none() && r.strategy != uia::Strategy::NaiveWalk).collect();
    let recall = |r: &StrategyResult| r.ocr_word_recall.unwrap_or(0.0);
    candidates
        .iter()
        .copied()
        .filter(|r| recall(r) >= 0.8)
        .min_by(|a, b| a.median_ms.unwrap_or(f64::MAX).partial_cmp(&b.median_ms.unwrap_or(f64::MAX)).unwrap())
        .or_else(|| candidates.iter().copied().max_by(|a, b| recall(a).partial_cmp(&recall(b)).unwrap()))
        .map(|r| r.strategy)
}

fn cmd_yield(t: &Target, label: &str, out: &Path) -> Result<(), String> {
    let w = resolve(t)?;
    let o = ocr::Ocr::new().and_then(|o| o.recognize_window(w.hwnd)).map_err(|e| e.to_string())?;
    let u = uia::Uia::new().map_err(|e| e.to_string())?;
    let (results, union_recall) = run_strategies(&u, w.hwnd, &o.text, Some(o.chars).filter(|&c| c > 0), false);
    let best = production_pick(&results);
    let line = serde_json::json!({
        "label": label, "measured_at": chrono::Local::now().to_rfc3339(), "app_id": w.app_id,
        "exe": w.exe, "ocr_chars": o.chars, "polling_strategy": best, "union_recall": union_recall, "strategies": results,
    });
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .and_then(|mut f| writeln!(f, "{line}"))
        .map_err(|e| e.to_string())?;
    println!(
        "== {label} ({}) OCR {} chars; production pick {:?}; union recall {}",
        w.app_id,
        o.chars,
        best,
        union_recall.map_or("-".into(), |x| format!("{x:.2}"))
    );
    for s in &results {
        println!(
            "  {:<12} recall {:>5} total {:>7} median {:>8} ms{}",
            format!("{:?}", s.strategy),
            s.ocr_word_recall.map_or("-".into(), |r| format!("{r:.2}")),
            s.chars_total,
            s.median_ms.map_or("-".into(), |m| format!("{m:.1}")),
            s.error.as_ref().map(|e| format!(" ERROR {e}")).unwrap_or_default()
        );
    }
    Ok(())
}

fn print_row(r: &MeasureRow) {
    println!("== {} ({})", r.label, r.app_id);
    if let Some(o) = &r.ocr {
        println!("  OCR: {} chars, {} lines, capture {:.0} ms + ocr {:.0} ms, frame {}x{} → {}x{}, border-off {:?}",
            o.chars, o.lines, o.capture_ms, o.ocr_ms, o.frame_width, o.frame_height, o.ocr_width, o.ocr_height, o.border_disable);
    } else {
        println!("  OCR failed: {:?}", r.ocr_error);
    }
    println!(
        "  attach: first contact {} chars in {} ms; settled at {} chars after {} ms",
        r.first_contact_chars,
        r.first_contact_ms.map_or("-".into(), |m| format!("{m:.0}")),
        r.settled_chars,
        r.settle_ms.map_or("(not within 20 s)".into(), |m| format!("{m:.0}"))
    );
    println!("  union (TextPattern ∪ CachedWalk) recall: {}", r.union_recall.map_or("-".into(), |x| format!("{x:.2}")));
    for s in &r.strategies {
        println!(
            "  {:<12} recall {:>5} total {:>7} visible {:>7} median {:>8} ms ({} runs) units {:>6}{}{}",
            format!("{:?}", s.strategy),
            s.ocr_word_recall.map_or("-".into(), |r| format!("{r:.2}")),
            s.chars_total,
            s.chars_visible,
            s.median_ms.map_or("-".into(), |m| format!("{m:.1}")),
            s.runs,
            s.units,
            if s.truncated { " TRUNCATED" } else { "" },
            s.error.as_ref().map(|e| format!(" ERROR {e}")).unwrap_or_default()
        );
    }
    let c = |w: &proc_stats::CpuWindow| format!("{:.2}% core / {:.2}% machine", w.pct_of_one_core, w.pct_of_machine);
    println!("  target tree CPU: before {} | after attach (idle) {} | 5 s polling {}", c(&r.cpu_before), c(&r.cpu_after_attach_idle), c(&r.cpu_during_5s_polling));
    println!("  spike CPU while polling: {}", c(&r.spike_cpu_during_polling));
    println!(
        "  tree private MB: {:.0} → {:.0}; OCR fallback fires — Mac rule: {}, Windows rule: {}",
        r.tree_memory_before_mb, r.tree_memory_after_mb, r.ocr_fires_mac_rule, r.ocr_fires_windows_rule
    );
}

fn cmd_cpu(pid: Option<u32>, exe: Option<String>, secs: u64) -> Result<(), String> {
    let all = proc_stats::list_processes();
    let roots = match (pid, exe) {
        (Some(p), _) => vec![p],
        (None, Some(e)) => proc_stats::tree_roots_named(&e, &all),
        _ => return Err("give --pid or --exe".into()),
    };
    for r in roots {
        let n = proc_stats::process_tree(r, &all).len();
        let w = proc_stats::measure_tree_cpu(r, Duration::from_secs(secs));
        let m = proc_stats::tree_memory(r);
        println!("{}", serde_json::json!({"root": r, "processes": n, "cpu": w, "private_mb": mb(m.private_bytes), "working_set_mb": mb(m.working_set_bytes)}));
    }
    Ok(())
}

fn cmd_sessions() -> Result<(), String> {
    for s in audio::audio_sessions().map_err(|e| e.to_string())? {
        println!("{}", serde_json::to_string(&s).unwrap());
    }
    Ok(())
}

/// Prints one line a second per meeting app in a call: whether its tree is rendering and
/// capturing. Run it across mute, unmute, a device switch and leaving to the lobby to see
/// what the capture session does (M3.3 Decision 7).
fn cmd_watch_sessions(secs: u64) -> Result<(), String> {
    use whimbrel_windows::meetings::detect;
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut last: std::collections::HashMap<String, (bool, bool)> = std::collections::HashMap::new();
    eprintln!("watching audio sessions for {secs}s; Ctrl+C to stop");
    while std::time::Instant::now() < deadline {
        let started = std::time::Instant::now();
        let processes = proc_stats::list_processes();
        let sessions = detect::Sessions::from(&audio::audio_sessions().map_err(|e| e.to_string())?);
        for exe in detect::MEETING_APPS.iter().chain(detect::BROWSERS.iter()) {
            for root in proc_stats::tree_roots_named(exe, &processes) {
                let tree = proc_stats::process_tree(root, &processes);
                let state = (sessions.tree_renders(&tree), sessions.tree_captures(&tree));
                if state == (false, false) {
                    continue;
                }
                let key = format!("{exe}:{root}");
                let changed = last.insert(key.clone(), state) != Some(state);
                println!(
                    "{}",
                    serde_json::json!({
                        "at": chrono::Local::now().format("%H:%M:%S").to_string(),
                        "app": exe, "pid": root, "renders": state.0, "captures": state.1, "changed": changed,
                    })
                );
            }
        }
        std::thread::sleep(Duration::from_secs(1).saturating_sub(started.elapsed()));
    }
    Ok(())
}

fn cmd_record(pid: Option<u32>, exe: Option<String>, endpoint: bool, also_endpoint: bool, no_mic: bool, secs: u64, out: Option<PathBuf>) -> Result<(), String> {
    let mut sources = Vec::new();
    if endpoint {
        sources.push(audio::Source::EndpointLoopback);
    } else {
        let target = match (pid, &exe) {
            (Some(p), _) => p,
            (None, Some(e)) => {
                let roots = proc_stats::tree_roots_named(e, &proc_stats::list_processes());
                *roots.first().ok_or_else(|| format!("{e} is not running"))?
            }
            _ => return Err("give --pid/--exe, or --endpoint".into()),
        };
        eprintln!("process loopback on tree root pid {target}");
        sources.push(audio::Source::ProcessLoopback { pid: target });
        if also_endpoint {
            sources.push(audio::Source::EndpointLoopback);
        }
    }
    if !no_mic {
        sources.push(audio::Source::Microphone);
    }
    let out = out.unwrap_or_else(|| whimbrel_dir().join("recordings"));
    let stop = stop_flag();
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            stop.store(true, Ordering::SeqCst);
        });
    }
    eprintln!("recording {secs} s to {} (Ctrl+C stops early)…", out.display());
    let rec = {
        let (sources, out, stop) = (sources.clone(), out.clone(), stop.clone());
        std::thread::spawn(move || audio::record(&sources, &out, stop))
    };

    // Which processes actually render/capture audio during the call (open question 1).
    let mut heard: std::collections::BTreeMap<(String, String, u32, &'static str), f32> = Default::default();
    while !stop.load(Ordering::SeqCst) {
        if let Ok(sessions) = audio::audio_sessions() {
            for s in sessions.into_iter().filter(|s| s.state == "active") {
                let device = if s.default_device { format!("{} (default)", s.device) } else { s.device };
                let e = heard.entry((device, s.exe, s.pid, s.flow)).or_insert(0.0);
                *e = e.max(s.peak);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let results = rec.join().map_err(|_| "recorder thread panicked")?;
    for (src, st) in &results {
        println!("{}", serde_json::json!({"source": src, "stats": st}));
    }
    println!("active audio sessions seen (max peak):");
    for ((device, exe, pid, flow), peak) in heard {
        println!("  {flow:<7} {pid:>6} {exe:<28} peak {peak:.3}  on {device}");
    }
    Ok(())
}

fn cmd_transcribe(mic: Option<PathBuf>, sys: &Path, models: Option<PathBuf>, whisper_model: &str, speakers: Option<i32>) -> Result<(), String> {
    use transcribe::postprocess::Line;
    let models = models.unwrap_or_else(|| whimbrel_dir().join("models"));
    let t0 = Instant::now();
    let tr = transcribe::Transcriber::load(&models.join(whisper_model)).map_err(|e| e.to_string())?;
    let load_s = t0.elapsed().as_secs_f64();

    let audio_secs = |p: &Path| hound::WavReader::open(p).map(|r| r.duration() as f64 / r.spec().sample_rate as f64).unwrap_or(0.0);

    let t1 = Instant::now();
    let sys_segs = tr.transcribe_wav(sys).map_err(|e| e.to_string())?;
    let sys_s = t1.elapsed().as_secs_f64();

    let t2 = Instant::now();
    let dm = diarize::DiarizationModelPaths {
        segmentation_model_path: models.join("pyannote-segmentation-3-0.onnx"),
        embedding_model_path: models.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
    };
    let spk = diarize::diarize(sys, &dm, speakers).map_err(|e| e.to_string())?;
    let diar_s = t2.elapsed().as_secs_f64();
    let labeled = diarize::label_speakers(&sys_segs, &spk);

    let (mic_segs, mic_s) = match &mic {
        Some(m) => {
            let t3 = Instant::now();
            (tr.transcribe_wav(m).map_err(|e| e.to_string())?, t3.elapsed().as_secs_f64())
        }
        None => (Vec::new(), 0.0),
    };

    let keep = |t: &str| !t.trim().is_empty() && !t.contains("[BLANK_AUDIO]");
    let sys_lines: Vec<Line> = labeled
        .iter()
        .filter(|l| keep(&l.text))
        .map(|l| Line {
            speaker_label: l.speaker_id.map(|i| format!("Speaker {}", i + 1)).unwrap_or_else(|| "Others".into()),
            start_cs: l.start_cs,
            end_cs: l.end_cs,
            text: l.text.clone(),
        })
        .collect();
    let mic_lines: Vec<Line> = mic_segs
        .iter()
        .filter(|s| keep(&s.text))
        .map(|s| Line { speaker_label: "Me".into(), start_cs: s.start_cs, end_cs: s.end_cs, text: s.text.clone() })
        .collect();
    let (lines, mic_kept) = transcribe::postprocess::postprocess(&mic_lines, &sys_lines);

    for l in &lines {
        let s = l.start_cs / 100;
        println!("[{:02}:{:02}:{:02}] {}: {}", s / 3600, (s / 60) % 60, s % 60, l.speaker_label, l.text);
    }
    let distinct: std::collections::BTreeSet<i32> = spk.iter().map(|s| s.speaker_id).collect();
    let sys_audio = audio_secs(sys);
    eprintln!(
        "{}",
        serde_json::json!({
            "model_load_s": load_s,
            "system_audio_s": sys_audio,
            "system_transcribe_s": sys_s,
            "system_rtf": sys_s / sys_audio.max(1e-9),
            "diarize_s": diar_s,
            "speakers_found": distinct.len(),
            "system_segments": sys_lines.len(),
            "mic_audio_s": mic.as_deref().map(audio_secs),
            "mic_transcribe_s": mic_s,
            "mic_segments": mic_lines.len(),
            "mic_kept_after_bleed_filter": mic_kept,
            "final_lines": lines.len(),
        })
    );
    Ok(())
}

/// Mirrors `diarize::diarize` exactly (same models, same 0.9 threshold / exact-count
/// clustering, same mono f32 input) except for `num_threads` on the two ONNX models.
fn cmd_diarize_bench(wav: &Path, models: Option<PathBuf>, threads: &str, speakers: Option<i32>) -> Result<(), String> {
    use sherpa_onnx::{
        FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
        OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
        SpeakerEmbeddingExtractorConfig,
    };
    let models = models.unwrap_or_else(|| whimbrel_dir().join("models"));
    let seg = models.join("pyannote-segmentation-3-0.onnx").to_string_lossy().into_owned();
    let emb = models.join("wespeaker_en_voxceleb_resnet34_LM.onnx").to_string_lossy().into_owned();

    let reader = hound::WavReader::open(wav).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let samples: Vec<i16> = reader.into_samples::<i16>().collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    let mut audio: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
    if spec.channels == 2 {
        audio = audio.chunks_exact(2).map(|c| (c[0] + c[1]) / 2.0).collect();
    }
    let audio_s = audio.len() as f64 / spec.sample_rate as f64;

    for n in threads.split(',').filter_map(|t| t.trim().parse::<i32>().ok()) {
        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig { model: Some(seg.clone()) },
                num_threads: n,
                ..Default::default()
            },
            embedding: SpeakerEmbeddingExtractorConfig { model: Some(emb.clone()), num_threads: n, ..Default::default() },
            clustering: FastClusteringConfig { num_clusters: speakers.filter(|&s| s > 0).unwrap_or(-1), threshold: 0.9 },
            ..Default::default()
        };
        let t = Instant::now();
        let sd = OfflineSpeakerDiarization::create(&config).ok_or("failed to create diarization pipeline")?;
        let create_s = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let result = sd.process(&audio).ok_or("diarization failed")?;
        let process_s = t.elapsed().as_secs_f64();
        let segs = result.sort_by_start_time();
        let found: std::collections::BTreeSet<i32> = segs.iter().map(|s| s.speaker).collect();
        println!(
            "{}",
            serde_json::json!({"threads": n, "audio_s": audio_s, "create_s": create_s, "process_s": process_s,
                "rtf": process_s / audio_s, "segments": segs.len(), "speakers_found": found.len()})
        );
    }
    Ok(())
}

fn cmd_wav_stats(files: &[PathBuf]) -> Result<(), String> {
    for f in files {
        let r = hound::WavReader::open(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let spec = r.spec();
        let s: Vec<i16> = r.into_samples::<i16>().collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let n = s.len().max(1);
        let peak = s.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let rms = (s.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
        let clipped = s.iter().filter(|v| v.unsigned_abs() >= 32_000).count();
        let non_silent = s.iter().filter(|v| v.unsigned_abs() > 50).count();
        let per_second = (spec.sample_rate * spec.channels as u32) as usize;
        let profile: Vec<u16> = s.chunks(per_second.max(1)).map(|c| c.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0)).collect();
        println!(
            "{}",
            serde_json::json!({
                "file": f.file_name().map(|x| x.to_string_lossy().into_owned()),
                "sample_rate": spec.sample_rate, "channels": spec.channels, "bits": spec.bits_per_sample,
                "secs": s.len() as f64 / per_second.max(1) as f64,
                "peak": peak, "rms": rms.round(), "clipped": clipped,
                "non_silent_share": (non_silent as f64 / n as f64 * 1000.0).round() / 1000.0,
                "peak_per_second": profile,
            })
        );
    }
    Ok(())
}

fn cmd_models(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir.unwrap_or_else(|| whimbrel_dir().join("models"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = api::models::ModelDestinations {
        whisper_model_path: dir.join("ggml-tiny.en.bin"),
        segmentation_model_path: dir.join("pyannote-segmentation-3-0.onnx"),
        embedding_model_path: dir.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
    };
    let job = api::models::download_all_start(&dest);
    let mut last = String::new();
    loop {
        let Some(p) = api::models::download_poll(&job) else { return Err("unknown job".into()) };
        let line = format!("{:>5.1}% {}", p.fraction * 100.0, p.status);
        if line != last {
            eprintln!("{line}");
            last = line;
        }
        if p.done {
            api::models::download_release(&job);
            return match p.error {
                Some(e) => Err(e),
                None => {
                    println!("models in {}", dir.display());
                    Ok(())
                }
            };
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ---------- ambient ----------

/// Exe basenames never captured (docs/12 §4 exclusions row).
const ALWAYS_EXCLUDE: &[&str] = &["lockapp.exe", "logonui.exe", "credentialuibroker.exe", "consent.exe"];
const DEFAULT_EXCLUDE: &[&str] = &["1password.exe", "bitwarden.exe"];

fn excluded(w: &window::WindowInfo) -> bool {
    ALWAYS_EXCLUDE.contains(&w.exe.as_str())
        || DEFAULT_EXCLUDE.contains(&w.exe.as_str())
        || w.exe.starts_with("keepass")
        || w.title.ends_with("InPrivate") || w.title.contains("[InPrivate]") || w.title.ends_with("Incognito")
}

static LAST_EVENT_MS: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();

fn ms_since_epoch() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

unsafe extern "system" fn win_event(
    _: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    LAST_EVENT_MS.store(ms_since_epoch(), Ordering::Relaxed);
}

fn cmd_ambient(secs: u64, strategy: uia::Strategy, pin_exe: Option<String>, pin_title: Option<String>) -> Result<(), String> {
    let pinned = match pin_exe {
        Some(exe) => Some(resolve(&Target { hwnd: None, exe: Some(exe), title: pin_title, foreground: false, delay: 0 })?),
        None => None,
    };
    if let Some(w) = &pinned {
        eprintln!("pinned to {} ({}) instead of the foreground window", w.app_id, w.exe);
    }
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, EVENT_OBJECT_FOCUS,
        EVENT_SYSTEM_FOREGROUND, MSG, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
    };
    const DEBOUNCE_MS: u64 = 800;
    const REPOLL_MS: u64 = 5_000;
    ms_since_epoch();

    // Trigger thread: WinEvent hooks need a message pump on the hooking thread.
    let (tid_tx, tid_rx) = std::sync::mpsc::channel();
    let hook_thread = std::thread::spawn(move || unsafe {
        tid_tx.send(windows::Win32::System::Threading::GetCurrentThreadId()).unwrap();
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let h1 = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event), 0, 0, flags);
        let h2 = SetWinEventHook(EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS, None, Some(win_event), 0, 0, flags);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWinEvent(h1);
        let _ = UnhookWinEvent(h2);
    });
    let hook_tid = tid_rx.recv().unwrap();

    let u = uia::Uia::new().map_err(|e| e.to_string())?;
    let mut ocr_engine: Option<ocr::Ocr> = None;
    let me = [std::process::id()];
    let cpu_start = proc_stats::CpuSnapshot::take(&me);
    let stop = stop_flag();
    let deadline = Instant::now() + Duration::from_secs(secs);

    let (mut captures, mut ocr_runs, mut changed, mut event_triggered) = (0u64, 0u64, 0u64, 0u64);
    let mut latencies = Vec::new();
    let mut last_hash = 0u64;
    let mut last_capture_ms = 0u64;
    let mut handled_event_ms = 0u64;
    let (mut ws_peak, mut priv_peak, mut ws_sum, mut samples) = (0u64, 0u64, 0u64, 0u64);
    let mut next_mem = Instant::now();
    let mut minute_cpu = Vec::new();
    let mut minute_snap = proc_stats::CpuSnapshot::take(&me);
    let mut next_minute = Instant::now() + Duration::from_secs(60);

    eprintln!("ambient capture for {secs} s with {strategy:?} (Ctrl+C stops)…");
    while Instant::now() < deadline && !stop.load(Ordering::SeqCst) {
        let now = ms_since_epoch();
        let ev = LAST_EVENT_MS.load(Ordering::Relaxed);
        let event_due = ev > handled_event_ms && now.saturating_sub(ev) >= DEBOUNCE_MS;
        let repoll_due = now.saturating_sub(last_capture_ms) >= REPOLL_MS;
        if event_due || repoll_due {
            if event_due {
                handled_event_ms = ev;
                event_triggered += 1;
            }
            last_capture_ms = now;
            let target = match &pinned {
                Some(w) => Some(w.clone()),
                None => window::foreground().filter(|w| !excluded(w) && w.pid != std::process::id()),
            };
            if let Some(w) = target {
                let t = Instant::now();
                if let Ok(e) = u.extract(w.hwnd, strategy) {
                    let mut text_len = e.chars_total;
                    let mut text_hash = hash(&e.text);
                    if policy::should_ocr(&policy::FallbackInput {
                        app_id: &w.app_id,
                        exe: &w.exe,
                        uia_chars: e.chars_total,
                        document_found: e.document_found,
                    }) {
                        let o = ocr_engine.get_or_insert_with(|| ocr::Ocr::new().expect("ocr"));
                        if let Ok(r) = o.recognize_window(w.hwnd) {
                            ocr_runs += 1;
                            text_len = r.chars;
                            text_hash = hash(&r.text);
                        }
                    }
                    captures += 1;
                    if text_len > 0 && text_hash != last_hash {
                        changed += 1;
                        last_hash = text_hash;
                    }
                }
                latencies.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if Instant::now() >= next_mem {
            if let Some(m) = proc_stats::memory(me[0]) {
                ws_peak = ws_peak.max(m.working_set_bytes);
                priv_peak = priv_peak.max(m.private_bytes);
                ws_sum += m.working_set_bytes;
                samples += 1;
            }
            next_mem = Instant::now() + Duration::from_secs(5);
        }
        if Instant::now() >= next_minute {
            let s = proc_stats::CpuSnapshot::take(&me);
            minute_cpu.push(s.since(&minute_snap).pct_of_one_core);
            minute_snap = s;
            next_minute = Instant::now() + Duration::from_secs(60);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe {
        let _ = PostThreadMessageW(hook_tid, WM_QUIT, Default::default(), Default::default());
    }
    let _ = hook_thread.join();

    let cpu = proc_stats::CpuSnapshot::take(&me).since(&cpu_start);
    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| sorted.get(((sorted.len() as f64 - 1.0) * p).round() as usize).copied();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "strategy": strategy,
            "pinned_to": pinned.as_ref().map(|w| w.app_id.clone()),
            "duration_s": cpu.wall_secs,
            "cpu": cpu,
            "cpu_per_minute_pct_of_one_core": minute_cpu,
            "working_set_mb_avg": mb(ws_sum / samples.max(1)),
            "working_set_mb_peak": mb(ws_peak),
            "private_mb_peak": mb(priv_peak),
            "captures": captures,
            "event_triggered": event_triggered,
            "text_changed": changed,
            "ocr_runs": ocr_runs,
            "capture_ms_p50": pct(0.5),
            "capture_ms_p95": pct(0.95),
            "capture_ms_max": sorted.last(),
        }))
        .unwrap()
    );
    Ok(())
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
