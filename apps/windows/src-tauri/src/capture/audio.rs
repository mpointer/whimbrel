//! Meeting audio capture — docs/12 §4.4.
//!
//! Three sources, each written to its own 16 kHz mono i16 WAV (the contract
//! `transcribe::Transcriber::transcribe_wav` and `diarize::diarize` both take):
//!
//! - [`Source::ProcessLoopback`]: only the audio a process tree renders
//!   (`ActivateAudioInterfaceAsync` on `VAD\Process_Loopback`). The default for a
//!   meeting whose app is known.
//! - [`Source::EndpointLoopback`]: everything the default render device plays.
//! - [`Source::Microphone`]: the default capture device.
//!
//! The microphone and process loopback ask the audio engine for 16 kHz mono PCM directly
//! (`AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`). Endpoint loopback does not: see [`Converter`]
//! for why it captures the device's own mix format and converts in the shell.
//!
//! Loopback delivers no packets while nothing is playing, so writing packets back to
//! back would make the system WAV shorter than the mic WAV and drift the two apart —
//! which breaks `transcribe::postprocess::filter_bleed_through`'s timestamp matching.
//! Each packet is placed by its QPC timestamp relative to one shared start instant, and
//! gaps are filled with silence, so every WAV from one recording shares a timeline.
//!
//! For meetings (M3.3 Task 3) [`record_with`] adds three things the M3.1 spike didn't
//! need: a live "last audible moment" for the end detector, stream health (a failing
//! stream is reopened into the same WAV, whose timeline padding keeps it aligned), and a
//! degraded reason when that doesn't help or a Bluetooth headset drops to call quality
//! (docs/12 W4).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use windows::core::{implement, Interface, IUnknown, Ref, HRESULT};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Variant::VT_BLOB;

pub const SAMPLE_RATE: u32 = 16_000;
/// Gaps shorter than this are jitter, not silence, and are not padded.
const PAD_TOLERANCE_FRAMES: i64 = (SAMPLE_RATE as i64) * 30 / 1000;
/// 200 ms engine buffer, in 100 ns units.
const BUFFER_100NS: i64 = 2_000_000;

/// A sample above this counts as audible: the Mac's 0.1 of full scale
/// (`AudioCaptureService`'s silence threshold).
pub const AUDIBLE_SAMPLE: i16 = 3_277;
/// A microphone that has delivered no packet this long after opening has failed. M3.1 saw
/// a cold microphone take 4.46 s to deliver its first packet and then work normally, so
/// the plan's 3 s would reopen healthy cold starts; 8 s leaves room for them.
pub const MIC_FIRST_PACKET_DEADLINE_MS: f64 = 8_000.0;
/// A loopback stream whose samples have stayed within ±1 this long, while the audio
/// engine's meter shows its source playing, has failed (M3.1 A2).
pub const FLAT_WINDOW_MS: f64 = 10_000.0;
/// A meter peak (0..=1) at or above this counts as "playing".
pub const METER_SIGNAL: f32 = 0.01;
/// Reopens per stream before it gives up. The plan's rule is one; a device change (a
/// headset connecting, a Bluetooth profile switch) needs a reopen of its own, so two.
pub const MAX_REOPENS: u32 = 2;
/// How often a supervised stream logs that it's still running, so a stall shows in the log.
const HEARTBEAT_MS: f64 = 30_000.0;
/// A render device mixing at this rate or lower is a headset in its call profile.
const CALL_QUALITY_MAX_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    ProcessLoopback { pid: u32 },
    EndpointLoopback,
    Microphone,
}

impl Source {
    fn is_loopback(&self) -> bool {
        !matches!(self, Source::Microphone)
    }

    fn file_stem(&self) -> &'static str {
        match self {
            Source::Microphone => "mic-audio",
            _ => "system-audio",
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StreamStats {
    pub wav_path: PathBuf,
    pub packets: u64,
    /// Packets flagged `AUDCLNT_BUFFERFLAGS_SILENT` by the engine.
    pub silent_packets: u64,
    pub frames_from_device: u64,
    pub frames_padded: u64,
    pub longest_gap_ms: f64,
    /// Time from the shared start instant to the first packet with a timestamp.
    pub first_packet_ms: Option<f64>,
    /// Peak absolute sample (0..=32767) — a quick "was anything captured" check.
    pub peak: i16,
    pub error: Option<String>,
    /// How many times the stream was reopened after a health failure or device change.
    pub reopens: u32,
}

/// What the recorder can read while a recording runs.
#[derive(Debug, Default)]
pub struct Live {
    /// Unix ms of the last packet with an audible sample on any stream; 0 before one.
    pub last_audible_unix_ms: AtomicI64,
    degraded: Mutex<Option<String>>,
}

impl Live {
    pub fn degraded(&self) -> Option<String> {
        self.degraded.lock().unwrap().clone()
    }

    /// Keeps the first reason: it's the one that explains the rest.
    pub fn set_degraded(&self, reason: String) {
        let mut slot = self.degraded.lock().unwrap();
        if slot.is_none() {
            tracing::warn!(reason = %reason, "meeting recording degraded");
            *slot = Some(reason);
        }
    }
}

/// How [`record_with`] records.
#[derive(Debug, Clone)]
pub struct RecordOptions {
    /// The shared part of every file name (`system-audio-<stamp>.wav`).
    pub stamp: String,
    /// Reopen failing streams and report to `live` (meetings). Off for the spike's `record`.
    pub supervise: bool,
    pub live: Arc<Live>,
}

/// Why a stream is failing, or `None` while it's healthy. Pure: the stream loop feeds
/// it what it has measured. `since_signal_ms` is the time since the later of the stream
/// opening and its last sample outside ±1; `meter_peak` is the audio engine's own level
/// for the source, read only when needed.
pub fn stream_failure(
    loopback: bool,
    since_open_ms: f64,
    got_packet: bool,
    since_signal_ms: f64,
    meter_peak: Option<f32>,
) -> Option<&'static str> {
    if loopback {
        let playing = meter_peak.is_some_and(|p| p >= METER_SIGNAL);
        (since_signal_ms >= FLAT_WINDOW_MS && playing).then_some("system audio is playing but not reaching Whimbrel")
    } else {
        (!got_packet && since_open_ms >= MIC_FIRST_PACKET_DEADLINE_MS).then_some("the microphone isn't delivering audio")
    }
}

/// The degraded reason when a stream gives up.
pub fn gave_up_reason(source: Source) -> String {
    if source.is_loopback() {
        "System audio isn't reaching Whimbrel, so this meeting records your microphone only.".into()
    } else {
        "The microphone isn't delivering audio, so this meeting records system audio only.".into()
    }
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A capture stream's wire format: what the engine hands `GetBuffer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// 16 kHz mono i16, straight from the engine's own converter (the microphone).
    Mono16k,
    /// The device's mix format, converted in the shell (loopback — see [`Converter`]).
    Mix(MixFormat),
}

/// The part of a `WAVEFORMATEX` the shell's converter needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub float: bool,
}

impl MixFormat {
    fn frame_bytes(&self) -> usize {
        self.channels as usize * (self.bits as usize / 8)
    }

    /// Whether the shell can decode this format's samples.
    fn supported(&self) -> bool {
        self.channels > 0 && matches!((self.bits, self.float), (32, true) | (32, false) | (16, false))
    }
}

/// Downmixes a mix-format packet to mono `f32` in `-1.0..=1.0`. Pure, so the sample
/// decoding is unit-testable without an audio device.
pub fn decode_mono(format: MixFormat, bytes: &[u8], out: &mut Vec<f32>) {
    let channels = format.channels as usize;
    let frames = bytes.len() / format.frame_bytes().max(1);
    out.clear();
    out.reserve(frames);
    for frame in 0..frames {
        let base = frame * format.frame_bytes();
        let mut sum = 0f32;
        for channel in 0..channels {
            let at = base + channel * (format.bits as usize / 8);
            let sample = match (format.bits, format.float) {
                (32, true) => f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                (32, false) => i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as f32 / 2_147_483_648.0,
                (16, false) => i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as f32 / 32_768.0,
                _ => 0.0,
            };
            sum += sample;
        }
        out.push(sum / channels as f32);
    }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16
}

/// Mix format in, 16 kHz mono i16 out: the loopback path (M3.3 Task 3's manual check).
///
/// Windows' own loopback conversion is unreliable: on the author's PC, asking the engine
/// for 16 kHz mono on the default render endpoint delivered flat silence for three
/// recordings in a row while its meter showed the same device playing at 0.97, and the
/// same device opened in its mix format carried audio every time. docs/12 §4 always
/// specified a shell-side resampler; M3.3's Decision 3 dropped it because the engine's
/// conversion had worked in M3.1, and this restores it for loopback only. The microphone
/// keeps the engine's conversion, which has never failed.
pub struct Converter {
    format: MixFormat,
    resampler: Option<rubato::FftFixedIn<f32>>,
    chunk_in: usize,
    /// Input frames not yet resampled (a chunk boundary rarely lands on a packet).
    pending: Vec<f32>,
    decoded: Vec<f32>,
    out_buffers: Vec<Vec<f32>>,
}

impl Converter {
    /// `None` if the format's samples can't be decoded, so the caller can fall back to
    /// the engine's conversion rather than record nothing.
    pub fn new(format: MixFormat) -> Option<Self> {
        if !format.supported() {
            return None;
        }
        let (resampler, chunk_in) = if format.sample_rate == SAMPLE_RATE {
            (None, 0)
        } else {
            // 20 ms of input per chunk: under `PAD_TOLERANCE_FRAMES`, so buffering a
            // partial chunk across packets never shows up as a gap.
            let chunk_in = (format.sample_rate as usize / 50).max(1);
            let resampler = rubato::FftFixedIn::<f32>::new(format.sample_rate as usize, SAMPLE_RATE as usize, chunk_in, 1, 1)
                .map_err(|e| tracing::warn!(error = %e, ?format, "no resampler for this mix format"))
                .ok()?;
            (Some(resampler), chunk_in)
        };
        Some(Converter { format, resampler, chunk_in, pending: Vec::new(), decoded: Vec::new(), out_buffers: vec![Vec::new()] })
    }

    pub fn format(&self) -> MixFormat {
        self.format
    }

    /// Input frames per output frame, for placing a packet on the output timeline.
    pub fn input_rate(&self) -> u32 {
        self.format.sample_rate
    }

    /// Converts one packet, dropping its first `skip_in` input frames, and appends the
    /// 16 kHz mono result to `out`. Whatever doesn't fill a resampler chunk stays
    /// buffered for the next packet.
    pub fn convert(&mut self, bytes: &[u8], skip_in: usize, out: &mut Vec<i16>) {
        let mut decoded = std::mem::take(&mut self.decoded);
        decode_mono(self.format, bytes, &mut decoded);
        let start = skip_in.min(decoded.len());
        self.pending.extend_from_slice(&decoded[start..]);
        decoded.clear();
        self.decoded = decoded;

        let Some(resampler) = &mut self.resampler else {
            out.extend(self.pending.drain(..).map(to_i16));
            return;
        };
        use rubato::Resampler;
        while self.pending.len() >= self.chunk_in {
            let chunk: Vec<f32> = self.pending.drain(..self.chunk_in).collect();
            match resampler.process(&[chunk], None) {
                Ok(frames) => out.extend(frames[0].iter().copied().map(to_i16)),
                Err(e) => {
                    tracing::warn!(error = %e, "resampling a packet failed");
                    return;
                }
            }
        }
        self.out_buffers[0].clear();
    }

    /// Converts the last partial chunk, zero-padded, when the stream stops.
    pub fn flush(&mut self, out: &mut Vec<i16>) {
        if self.pending.is_empty() {
            return;
        }
        let Some(resampler) = &mut self.resampler else {
            out.extend(self.pending.drain(..).map(to_i16));
            return;
        };
        use rubato::Resampler;
        let mut chunk: Vec<f32> = self.pending.drain(..).collect();
        chunk.resize(self.chunk_in, 0.0);
        if let Ok(frames) = resampler.process(&[chunk], None) {
            out.extend(frames[0].iter().copied().map(to_i16));
        }
    }
}

/// 100 ns ticks on the QPC clock — the unit `IAudioCaptureClient::GetBuffer` reports
/// packet positions in.
fn now_100ns() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    ((c as u128) * 10_000_000 / (f.max(1) as u128)) as u64
}

fn pcm16_mono_16k() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    }
}

// ActivateAudioInterfaceAsync requires an agile completion handler; `#[implement]`
// provides IAgileObject + the free-threaded marshaler by default (`Agile = true`).
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationDone {
    tx: Mutex<Option<mpsc::Sender<()>>>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationDone_Impl {
    fn ActivateCompleted(&self, _op: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

fn activate_process_loopback(pid: u32) -> windows::core::Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // A VT_BLOB PROPVARIANT pointing at `params` on this stack frame. windows' PROPVARIANT
    // has a Drop that calls PropVariantClear, which CoTaskMemFree's a VT_BLOB's data — here
    // a stack pointer, so dropping it corrupted the heap (STATUS_HEAP_CORRUPTION, found in
    // M3.1). ManuallyDrop: this PROPVARIANT owns nothing and is never cleared.
    let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
    unsafe {
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: &mut params as *mut _ as *mut u8,
        };
    }
    let (tx, rx) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationDone { tx: Mutex::new(Some(tx)) }.into();
    unsafe {
        let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*pv as *const PROPVARIANT), &handler)?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "process-loopback activation timed out")
        })?;
        let mut hr = HRESULT(0);
        let mut unk: Option<IUnknown> = None;
        op.GetActivateResult(&mut hr, &mut unk)?;
        hr.ok()?;
        unk.expect("activation succeeded without an interface").cast()
    }
}

fn default_device(flow: EDataFlow) -> windows::core::Result<IMMDevice> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        en.GetDefaultAudioEndpoint(flow, eConsole)
    }
}

/// The endpoint's mix format, or `None` if it can't be read or the shell can't decode it.
fn endpoint_mix_format(device: &IMMDevice) -> Option<MixFormat> {
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
        let fmt = client.GetMixFormat().ok()?;
        let wave = std::ptr::read_unaligned(fmt);
        let float = if wave.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE {
            let ext = fmt as *const WAVEFORMATEXTENSIBLE;
            std::ptr::addr_of!((*ext).SubFormat).read_unaligned() == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        } else {
            wave.wFormatTag as u32 == WAVE_FORMAT_IEEE_FLOAT
        };
        windows::Win32::System::Com::CoTaskMemFree(Some(fmt as *const core::ffi::c_void));
        let format = MixFormat { sample_rate: wave.nSamplesPerSec, channels: wave.nChannels, bits: wave.wBitsPerSample, float };
        format.supported().then_some(format)
    }
}

/// Opens a capture stream and says what its packets carry. The microphone and process
/// loopback use the engine's own conversion to 16 kHz mono; endpoint loopback asks for the
/// device's mix format and converts in the shell ([`Converter`]).
fn open_client(source: Source) -> windows::core::Result<(IAudioClient, Wire)> {
    let base = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    let converting = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    unsafe {
        match source {
            Source::ProcessLoopback { pid } => {
                let client = activate_process_loopback(pid)?;
                let fmt = pcm16_mono_16k();
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, base | converting | AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_100NS, 0, &fmt, None)?;
                Ok((client, Wire::Mono16k))
            }
            Source::Microphone => {
                let client: IAudioClient = default_device(eCapture)?.Activate(CLSCTX_ALL, None)?;
                let fmt = pcm16_mono_16k();
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, base | converting, BUFFER_100NS, 0, &fmt, None)?;
                Ok((client, Wire::Mono16k))
            }
            Source::EndpointLoopback => {
                let device = default_device(eRender)?;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                match endpoint_mix_format(&device) {
                    Some(format) => {
                        let fmt = wave_format(format);
                        let as_ex = &fmt as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;
                        client.Initialize(AUDCLNT_SHAREMODE_SHARED, base | AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_100NS, 0, as_ex, None)?;
                        Ok((client, Wire::Mix(format)))
                    }
                    None => {
                        // No usable mix format: the engine's conversion is all there is.
                        tracing::warn!("no supported mix format on the default output; using the engine's conversion");
                        let fmt = pcm16_mono_16k();
                        client.Initialize(AUDCLNT_SHAREMODE_SHARED, base | converting | AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_100NS, 0, &fmt, None)?;
                        Ok((client, Wire::Mono16k))
                    }
                }
            }
        }
    }
}

/// A `WAVEFORMATEXTENSIBLE` for `format`, which is what a mix format needs (a bare
/// `WAVEFORMATEX` is rejected for multichannel float).
fn wave_format(format: MixFormat) -> WAVEFORMATEXTENSIBLE {
    use windows::Win32::Media::KernelStreaming::KSDATAFORMAT_SUBTYPE_PCM;
    let block_align = format.channels * format.bits / 8;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: format.channels,
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: format.bits,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: format.bits },
        dwChannelMask: if format.channels == 1 { 0x4 } else { 0x3 },
        SubFormat: if format.float { KSDATAFORMAT_SUBTYPE_IEEE_FLOAT } else { KSDATAFORMAT_SUBTYPE_PCM },
    }
}

/// Where a packet's frames belong in the output, given its QPC position. Pure, so the
/// alignment rule is unit-testable: returns `(output frames to pad, input frames to skip)`.
/// `written` counts output frames (16 kHz); `packet_frames` and the skip count input
/// frames, which differ when the shell resamples ([`Converter`]).
fn place_packet_at(t0_100ns: u64, qpc_100ns: u64, written: i64, packet_frames: i64, input_rate: u32) -> (i64, i64) {
    if qpc_100ns == 0 {
        return (0, 0); // no timestamp: append
    }
    let expected = (qpc_100ns as i128 - t0_100ns as i128) * SAMPLE_RATE as i128 / 10_000_000;
    let expected = expected as i64;
    if expected < 0 {
        // Packet began before the recording did: drop its leading part.
        let skip_in = (-expected) * input_rate as i64 / SAMPLE_RATE as i64;
        return (0, skip_in.min(packet_frames));
    }
    let gap = expected - written;
    if gap > PAD_TOLERANCE_FRAMES {
        (gap, 0)
    } else {
        (0, 0)
    }
}

/// [`place_packet_at`] for a stream the engine already delivers at 16 kHz.
fn place_packet(t0_100ns: u64, qpc_100ns: u64, written: i64, packet_frames: i64) -> (i64, i64) {
    place_packet_at(t0_100ns, qpc_100ns, written, packet_frames, SAMPLE_RATE)
}

fn run_stream(source: Source, wav_path: PathBuf, t0: u64, stop: Arc<AtomicBool>, options: &RecordOptions) -> StreamStats {
    let mut stats = StreamStats { wav_path: wav_path.clone(), ..Default::default() };
    if let Err(e) = capture_into(source, &wav_path, t0, &stop, options, &mut stats) {
        stats.error = Some(e);
    }
    stats
}

/// How one opened client's run ended.
enum ClientEnd {
    Stopped,
    Failed(String),
}

/// The engine's level for what `source` records, when there is a meter for it.
fn source_meter_peak(source: Source) -> Option<f32> {
    match source {
        Source::EndpointLoopback => unsafe {
            let meter: IAudioMeterInformation = default_device(eRender).ok()?.Activate(CLSCTX_ALL, None).ok()?;
            meter.GetPeakValue().ok()
        },
        Source::ProcessLoopback { pid } => audio_sessions()
            .ok()?
            .into_iter()
            .filter(|s| s.flow == "render" && s.pid == pid)
            .map(|s| s.peak)
            .reduce(f32::max),
        Source::Microphone => None,
    }
}

/// Whether the default render device now mixes at call quality: a Bluetooth headset in
/// its hands-free profile (docs/12 W4). Untested on real hardware; no Bluetooth headset
/// was available when this was written.
fn render_at_call_quality() -> bool {
    unsafe {
        let Ok(client) = default_device(eRender).and_then(|d| d.Activate::<IAudioClient>(CLSCTX_ALL, None)) else {
            return false;
        };
        let Ok(fmt) = client.GetMixFormat() else { return false };
        let rate = std::ptr::read_unaligned(fmt).nSamplesPerSec;
        windows::Win32::System::Com::CoTaskMemFree(Some(fmt as *const core::ffi::c_void));
        rate <= CALL_QUALITY_MAX_RATE
    }
}

fn capture_into(
    source: Source,
    wav_path: &PathBuf,
    t0: u64,
    stop: &AtomicBool,
    options: &RecordOptions,
    stats: &mut StreamStats,
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let spec = hound::WavSpec { channels: 1, sample_rate: SAMPLE_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut wav = hound::WavWriter::create(wav_path, spec).map_err(|e| e.to_string())?;
    let mut written: i64 = 0;

    loop {
        // Step logs while supervised: a WASAPI call that never returns has no other trace.
        if options.supervise {
            tracing::info!(?source, attempt = stats.reopens, "opening an audio stream");
        }
        let end = match open_client(source) {
            Ok((client, wire)) => {
                if options.supervise && source.is_loopback() && render_at_call_quality() {
                    options.live.set_degraded("Your headset switched to call quality, so the meeting's audio may sound muffled.".into());
                }
                if options.supervise {
                    tracing::info!(?source, ?wire, "audio stream opened");
                }
                run_client(&client, wire, source, t0, stop, options, &mut wav, &mut written, stats)
            }
            Err(e) => ClientEnd::Failed(format!("open {source:?}: {e}")),
        };
        match end {
            ClientEnd::Stopped => break,
            ClientEnd::Failed(reason) if !options.supervise => {
                stats.error = Some(reason);
                break;
            }
            ClientEnd::Failed(reason) => {
                if stats.reopens < MAX_REOPENS && !stop.load(Ordering::Relaxed) {
                    stats.reopens += 1;
                    tracing::warn!(?source, reason = %reason, attempt = stats.reopens, "reopening an audio stream");
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
                tracing::warn!(?source, reason = %reason, "an audio stream gave up");
                options.live.set_degraded(gave_up_reason(source));
                stats.error = Some(reason);
                // Keep the file's timeline: wait for the stop, then pad to it below.
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                }
                break;
            }
        }
    }

    // Pad to the stop instant so every stream from one recording ends together.
    let end = ((now_100ns() - t0) as i128 * SAMPLE_RATE as i128 / 10_000_000) as i64;
    if end > written {
        pad_silence(end - written, &mut wav, &mut written, stats);
    }
    if stats.error.is_none() && stats.reopens > 0 {
        tracing::info!(?source, reopens = stats.reopens, "an audio stream recovered after reopening");
    }
    wav.finalize().map_err(|e| e.to_string())?;
    Ok(())
}

fn pad_silence<W: std::io::Write + std::io::Seek>(n: i64, wav: &mut hound::WavWriter<W>, written: &mut i64, stats: &mut StreamStats) {
    for _ in 0..n {
        let _ = wav.write_sample(0i16);
    }
    *written += n;
    stats.frames_padded += n as u64;
    stats.longest_gap_ms = stats.longest_gap_ms.max(n as f64 * 1000.0 / SAMPLE_RATE as f64);
}

/// Captures from one opened client until the stop flag, a device error, or (when
/// supervised) a health failure.
#[allow(clippy::too_many_arguments)]
fn run_client<W: std::io::Write + std::io::Seek>(
    client: &IAudioClient,
    wire: Wire,
    source: Source,
    t0: u64,
    stop: &AtomicBool,
    options: &RecordOptions,
    wav: &mut hound::WavWriter<W>,
    written: &mut i64,
    stats: &mut StreamStats,
) -> ClientEnd {
    let opened = now_100ns();
    let ms_since = |t: u64| now_100ns().saturating_sub(t) as f64 / 10_000.0;
    let mut got_packet = false;
    let mut converter = match wire {
        Wire::Mix(format) => Converter::new(format),
        Wire::Mono16k => None,
    };
    let frame_bytes = match wire {
        Wire::Mix(format) => format.frame_bytes(),
        Wire::Mono16k => 2,
    };
    let input_rate = match wire {
        Wire::Mix(format) => format.sample_rate,
        Wire::Mono16k => SAMPLE_RATE,
    };
    let mut samples: Vec<i16> = Vec::new();
    let mut last_signal = opened;
    let mut last_health_check = opened;
    let mut last_heartbeat = opened;

    unsafe {
        let event = match CreateEventW(None, false, false, None) {
            Ok(e) => e,
            Err(e) => return ClientEnd::Failed(e.to_string()),
        };
        let setup = client
            .SetEventHandle(event)
            .and_then(|_| client.GetService::<IAudioCaptureClient>())
            .and_then(|capture| client.Start().map(|_| capture));
        let capture = match setup {
            Ok(c) => c,
            Err(e) => {
                let _ = CloseHandle(event);
                return ClientEnd::Failed(e.to_string());
            }
        };

        let end = loop {
            let stopping = stop.load(Ordering::Relaxed);
            let signalled = stopping || WaitForSingleObject(event, 100) == WAIT_OBJECT_0;
            if signalled {
                let mut failed = None;
                loop {
                    match capture.GetNextPacketSize() {
                        Ok(0) => break,
                        Ok(_) => {}
                        // A removed or reconfigured device (AUDCLNT_E_DEVICE_INVALIDATED).
                        Err(e) => {
                            failed = Some(format!("audio device changed: {e}"));
                            break;
                        }
                    }
                    let mut data = std::ptr::null_mut();
                    let (mut frames, mut flags, mut qpc) = (0u32, 0u32, 0u64);
                    if let Err(e) = capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc)) {
                        failed = Some(format!("audio device changed: {e}"));
                        break;
                    }
                    got_packet = true;
                    stats.packets += 1;
                    if stats.first_packet_ms.is_none() && qpc != 0 {
                        stats.first_packet_ms = Some((qpc as f64 - t0 as f64) / 10_000.0);
                    }
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    if silent {
                        stats.silent_packets += 1;
                    }
                    let (gap, skip) = place_packet_at(t0, qpc, *written, frames as i64, input_rate);
                    pad_silence(gap, wav, written, stats);
                    samples.clear();
                    match (&mut converter, silent) {
                        // A silent packet's bytes may be undefined; the engine only
                        // promises the frame count.
                        (Some(converter), true) => {
                            let quiet = vec![0u8; (frames as usize).saturating_sub(skip as usize) * frame_bytes];
                            converter.convert(&quiet, 0, &mut samples);
                        }
                        (Some(converter), false) => {
                            let bytes = std::slice::from_raw_parts(data, frames as usize * frame_bytes);
                            converter.convert(bytes, skip as usize, &mut samples);
                        }
                        (None, _) => {
                            let frames16 = std::slice::from_raw_parts(data as *const i16, frames as usize);
                            samples.extend(frames16[skip as usize..].iter().map(|&s| if silent { 0 } else { s }));
                        }
                    }
                    let mut packet_peak: i16 = 0;
                    for &s in &samples {
                        packet_peak = packet_peak.max(s.saturating_abs());
                        let _ = wav.write_sample(s);
                    }
                    stats.peak = stats.peak.max(packet_peak);
                    if packet_peak > 1 {
                        last_signal = now_100ns();
                    }
                    if options.supervise && packet_peak > AUDIBLE_SAMPLE {
                        options.live.last_audible_unix_ms.store(unix_ms(), Ordering::Relaxed);
                    }
                    let kept = samples.len() as i64;
                    *written += kept;
                    stats.frames_from_device += kept as u64;
                    let _ = capture.ReleaseBuffer(frames);
                }
                if let Some(reason) = failed {
                    break ClientEnd::Failed(reason);
                }
                if stopping {
                    break ClientEnd::Stopped;
                }
            }

            if options.supervise && ms_since(last_heartbeat) >= HEARTBEAT_MS {
                last_heartbeat = now_100ns();
                tracing::info!(?source, packets = stats.packets, frames = stats.frames_from_device, peak = stats.peak, "audio stream running");
            }

            if options.supervise && ms_since(last_health_check) >= 1_000.0 {
                last_health_check = now_100ns();
                let since_signal = ms_since(last_signal);
                // The meter is only worth reading once the stream has been flat long enough.
                let meter = if source.is_loopback() && since_signal >= FLAT_WINDOW_MS { source_meter_peak(source) } else { None };
                if let Some(reason) = stream_failure(source.is_loopback(), ms_since(opened), got_packet, since_signal, meter) {
                    tracing::warn!(?source, meter_peak = ?meter, stream_peak = stats.peak, packets = stats.packets, "audio stream health check failed");
                    break ClientEnd::Failed(reason.to_string());
                }
            }
        };
        let _ = client.Stop();
        let _ = CloseHandle(event);
        // Whatever the resampler still holds belongs in this file.
        if let Some(converter) = &mut converter {
            samples.clear();
            converter.flush(&mut samples);
            for &s in &samples {
                let _ = wav.write_sample(s);
            }
            *written += samples.len() as i64;
            stats.frames_from_device += samples.len() as u64;
        }
        end
    }
}

/// Records `sources` concurrently until `stop` is set. All WAVs share one start instant
/// (stream-open latency becomes leading silence), so they line up sample-for-sample.
/// The M3.1 spike's entry point: no supervision, and a compact timestamp in the names.
pub fn record(sources: &[Source], out_dir: &std::path::Path, stop: Arc<AtomicBool>) -> Vec<(Source, StreamStats)> {
    let options = RecordOptions {
        stamp: chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
        supervise: false,
        live: Arc::new(Live::default()),
    };
    record_with(sources, out_dir, stop, &options)
}

/// [`record`] with the file stamp, supervision and live levels the meeting recorder
/// needs. Blocks until `stop` is set and every stream has finalized its WAV.
pub fn record_with(sources: &[Source], out_dir: &std::path::Path, stop: Arc<AtomicBool>, options: &RecordOptions) -> Vec<(Source, StreamStats)> {
    let _ = std::fs::create_dir_all(out_dir);
    let t0 = now_100ns();
    let handles: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(i, &src)| {
            // Two system sources in one recording (e.g. process + endpoint for a
            // comparison) must not collide on a filename.
            let suffix = if sources.iter().filter(|s| s.file_stem() == src.file_stem()).count() > 1 {
                format!("-{i}")
            } else {
                String::new()
            };
            let path = out_dir.join(format!("{}-{}{suffix}.wav", src.file_stem(), options.stamp));
            let (stop, options) = (stop.clone(), options.clone());
            (src, std::thread::spawn(move || run_stream(src, path, t0, stop, &options)))
        })
        .collect();
    handles.into_iter().map(|(s, h)| (s, h.join().unwrap_or_default())).collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioSessionInfo {
    pub flow: &'static str,
    /// Endpoint friendly name — virtual devices (NVIDIA Broadcast, headsets' comms
    /// endpoints) mean "the default device" is not always where a call's audio goes.
    pub device: String,
    /// Whether that endpoint is the console default for its direction (what `record`'s
    /// endpoint loopback and mic legs open).
    pub default_device: bool,
    pub pid: u32,
    pub exe: String,
    pub state: &'static str,
    pub peak: f32,
}

/// Every audio session on every active endpoint, with its process and current peak
/// level. This is §4.5's meeting-detection signal ("app has an active render *and*
/// capture session") and the instrument for open question 1 (which Teams process
/// actually renders the call audio).
pub fn audio_sessions() -> windows::core::Result<Vec<AudioSessionInfo>> {
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let default_render = en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| device_name(&d)).unwrap_or_default();
        let default_capture = en.GetDefaultAudioEndpoint(eCapture, eConsole).map(|d| device_name(&d)).unwrap_or_default();
        let devices = en.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let dev = devices.Item(i)?;
            let flow = match dev.cast::<IMMEndpoint>().and_then(|e| e.GetDataFlow()) {
                Ok(f) if f == eCapture => "capture",
                _ => "render",
            };
            let device = device_name(&dev);
            let default_device = device == if flow == "capture" { default_capture.as_str() } else { default_render.as_str() };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let sessions = mgr.GetSessionEnumerator()?;
            for j in 0..sessions.GetCount()? {
                let ctl = sessions.GetSession(j)?;
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                let pid = ctl2.GetProcessId().unwrap_or(0);
                let state = match ctl.GetState() {
                    Ok(s) if s == AudioSessionStateActive => "active",
                    Ok(s) if s == AudioSessionStateExpired => "expired",
                    _ => "inactive",
                };
                let peak = ctl.cast::<IAudioMeterInformation>().and_then(|m| m.GetPeakValue()).unwrap_or(0.0);
                let exe = crate::capture::window::exe_of(pid).unwrap_or_else(|| if pid == 0 { "(system)".into() } else { "?".into() });
                out.push(AudioSessionInfo { flow, device: device.clone(), default_device, pid, exe, state, peak });
            }
        }
    }
    Ok(out)
}

/// What endpoint loopback delivers when opened in the device's *own* mix format, with no
/// `AUTOCONVERTPCM`: the M3.1 diagnostic for A2 (auto-converted endpoint loopback
/// intermittently recorded all-zero samples while the device was playing).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoopbackProbe {
    pub device: String,
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits: u16,
    pub float: bool,
    pub packets: u64,
    pub frames: u64,
    pub silent_packets: u64,
    /// Peak absolute sample, normalized to 0.0..=1.0.
    pub peak: f32,
}

pub fn probe_endpoint_loopback(secs: u64) -> windows::core::Result<LoopbackProbe> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let dev = default_device(eRender)?;
        let device = device_name(&dev);
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = client.GetMixFormat()?;
        let f = std::ptr::read_unaligned(fmt);
        let float = if f.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE {
            let ext = fmt as *const WAVEFORMATEXTENSIBLE;
            std::ptr::addr_of!((*ext).SubFormat).read_unaligned() == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        } else {
            f.wFormatTag as u32 == WAVE_FORMAT_IEEE_FLOAT
        };
        let event = CreateEventW(None, false, false, None)?;
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            BUFFER_100NS,
            0,
            fmt,
            None,
        )?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let (mut packets, mut frames, mut silent_packets, mut peak) = (0u64, 0u64, 0u64, 0f32);
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        while std::time::Instant::now() < deadline {
            if WaitForSingleObject(event, 200) != WAIT_OBJECT_0 {
                continue;
            }
            while capture.GetNextPacketSize().unwrap_or(0) > 0 {
                let mut data = std::ptr::null_mut();
                let (mut n, mut flags) = (0u32, 0u32);
                if capture.GetBuffer(&mut data, &mut n, &mut flags, None, None).is_err() {
                    break;
                }
                packets += 1;
                frames += n as u64;
                let samples = n as usize * f.nChannels as usize;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    silent_packets += 1;
                } else if float && f.wBitsPerSample == 32 {
                    for &v in std::slice::from_raw_parts(data as *const f32, samples) {
                        peak = peak.max(v.abs());
                    }
                } else if f.wBitsPerSample == 16 {
                    for &v in std::slice::from_raw_parts(data as *const i16, samples) {
                        peak = peak.max(v.unsigned_abs() as f32 / 32768.0);
                    }
                }
                let _ = capture.ReleaseBuffer(n);
            }
        }
        let _ = client.Stop();
        let _ = CloseHandle(event);
        windows::Win32::System::Com::CoTaskMemFree(Some(fmt as *const core::ffi::c_void));
        Ok(LoopbackProbe {
            device,
            format_tag: f.wFormatTag,
            channels: f.nChannels,
            sample_rate: f.nSamplesPerSec,
            bits: f.wBitsPerSample,
            float,
            packets,
            frames,
            silent_packets,
            peak,
        })
    }
}

fn device_name(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "?".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = 10_000_000;

    #[test]
    fn contiguous_packets_are_appended() {
        // 1 s after t0 with 16000 frames already written: no gap.
        assert_eq!(place_packet(SEC, 2 * SEC, 16_000, 160), (0, 0));
    }

    #[test]
    fn jitter_under_tolerance_is_not_padded() {
        // 10 ms late — under the 30 ms tolerance.
        assert_eq!(place_packet(0, SEC + SEC / 100, 16_000, 160), (0, 0));
    }

    #[test]
    fn silence_gap_is_padded_to_the_packet_timestamp() {
        // Loopback went quiet for 2 s: packet stamped at 3 s, only 1 s written.
        assert_eq!(place_packet(0, 3 * SEC, 16_000, 160), (32_000, 0));
    }

    #[test]
    fn packet_straddling_the_start_is_trimmed() {
        // Packet begins 5 ms before t0: its first 80 frames predate the recording.
        assert_eq!(place_packet(SEC, SEC - SEC / 200, 0, 160), (0, 80));
    }

    #[test]
    fn untimestamped_packet_is_appended() {
        assert_eq!(place_packet(SEC, 0, 500, 160), (0, 0));
    }

    // ---- stream_failure ----

    const MIX_48K_STEREO_F32: MixFormat = MixFormat { sample_rate: 48_000, channels: 2, bits: 32, float: true };

    #[test]
    fn a_48k_packet_before_the_start_skips_input_frames_not_output_frames() {
        // 5 ms before t0 at 48 kHz: 240 input frames, which are 80 output frames.
        assert_eq!(place_packet_at(SEC, SEC - SEC / 200, 0, 480, 48_000), (0, 240));
    }

    #[test]
    fn a_48k_packets_gap_is_padded_in_output_frames() {
        assert_eq!(place_packet_at(0, 3 * SEC, 16_000, 480, 48_000), (32_000, 0));
    }

    #[test]
    fn stereo_float_frames_downmix_to_mono() {
        let mut out = Vec::new();
        let frame = |l: f32, r: f32| [l.to_le_bytes(), r.to_le_bytes()].concat();
        let bytes: Vec<u8> = [frame(1.0, -1.0), frame(0.5, 0.5), frame(-1.0, -1.0)].concat();
        decode_mono(MIX_48K_STEREO_F32, &bytes, &mut out);
        assert_eq!(out, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn sixteen_bit_and_thirty_two_bit_integer_frames_decode_to_full_scale() {
        let mut out = Vec::new();
        let i16_mono = MixFormat { sample_rate: 16_000, channels: 1, bits: 16, float: false };
        decode_mono(i16_mono, &i16::MAX.to_le_bytes(), &mut out);
        assert!((out[0] - 1.0).abs() < 0.001);
        let i32_mono = MixFormat { sample_rate: 16_000, channels: 1, bits: 32, float: false };
        decode_mono(i32_mono, &i32::MIN.to_le_bytes(), &mut out);
        assert_eq!(out, vec![-1.0]);
    }

    #[test]
    fn an_unsupported_mix_format_has_no_converter() {
        // 24-bit packed audio: the shell doesn't decode it, so the caller falls back.
        assert!(Converter::new(MixFormat { sample_rate: 48_000, channels: 2, bits: 24, float: false }).is_none());
    }

    #[test]
    fn a_16k_mono_mix_format_needs_no_resampling() {
        let format = MixFormat { sample_rate: 16_000, channels: 1, bits: 16, float: false };
        let mut converter = Converter::new(format).unwrap();
        let bytes: Vec<u8> = (0..100i16).flat_map(|i| (i * 300).to_le_bytes()).collect();
        let mut out = Vec::new();
        converter.convert(&bytes, 0, &mut out);
        assert_eq!(out.len(), 100, "every frame passes straight through");
        assert_eq!(out[10], 3_000);
    }

    #[test]
    fn a_48k_stereo_tone_resamples_to_16k_mono_at_a_third_of_the_frames() {
        let mut converter = Converter::new(MIX_48K_STEREO_F32).unwrap();
        // 1 s of a 440 Hz tone, in 10 ms packets, the same in both channels.
        let mut out = Vec::new();
        for packet in 0..100 {
            let bytes: Vec<u8> = (0..480)
                .flat_map(|i| {
                    let t = (packet * 480 + i) as f32 / 48_000.0;
                    let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                    [v.to_le_bytes(), v.to_le_bytes()].concat()
                })
                .collect();
            converter.convert(&bytes, 0, &mut out);
        }
        converter.flush(&mut out);
        assert!((out.len() as i64 - 16_000).abs() < 500, "about one second at 16 kHz, got {}", out.len());
        let peak = out.iter().map(|s| s.saturating_abs()).max().unwrap();
        assert!(peak > 12_000 && peak < 20_000, "a half-scale tone survives resampling, got {peak}");
    }

    #[test]
    fn a_microphone_fails_only_with_no_packet_past_its_deadline() {
        assert_eq!(stream_failure(false, 5_000.0, false, 5_000.0, None), None, "a cold start");
        assert!(stream_failure(false, MIC_FIRST_PACKET_DEADLINE_MS, false, 8_000.0, None).is_some());
        assert_eq!(stream_failure(false, 60_000.0, true, 60_000.0, None), None, "a quiet microphone is fine");
    }

    #[test]
    fn a_flat_loopback_fails_only_while_its_source_is_playing() {
        assert_eq!(stream_failure(true, 30_000.0, false, 30_000.0, Some(0.0)), None, "nothing is playing");
        assert_eq!(stream_failure(true, 30_000.0, false, 30_000.0, None), None, "no meter to confirm");
        assert_eq!(stream_failure(true, 9_000.0, true, 9_000.0, Some(0.8)), None, "inside the window");
        assert!(stream_failure(true, 30_000.0, true, FLAT_WINDOW_MS, Some(0.8)).is_some());
    }

    #[test]
    fn a_loopback_never_fails_for_arriving_late() {
        // Loopback delivers no packets while nothing plays (A2's first-packet delay).
        assert_eq!(stream_failure(true, 60_000.0, false, 0.0, None), None);
    }

    #[test]
    fn giving_up_names_the_half_that_still_records() {
        assert!(gave_up_reason(Source::EndpointLoopback).contains("microphone only"));
        assert!(gave_up_reason(Source::Microphone).contains("system audio only"));
    }

    #[test]
    fn the_first_degraded_reason_is_kept() {
        let live = Live::default();
        live.set_degraded("first".into());
        live.set_degraded("second".into());
        assert_eq!(live.degraded().as_deref(), Some("first"));
    }
}
