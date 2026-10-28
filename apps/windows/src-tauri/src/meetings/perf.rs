//! CPU for the post-meeting pipeline (M3.3 Decision 4).
//!
//! Transcription is the one place Whimbrel may burst CPU (NFR-PRF-2), and it has a budget
//! to meet (NFR-PRF-5: a summary within 2 minutes of a meeting ending). Two things get in
//! the way on Windows: whisper.cpp and sherpa-onnx default to far fewer threads than a
//! laptop has, and a tray-resident process with no window is a power-throttling candidate
//! — M3.1 measured whisper at 74 s against 53 s for the same audio with throttling off.

use windows::Win32::System::Threading::{
    GetCurrentProcess, SetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling,
};

/// The most threads the pipeline asks for. M3.1's diarization bench gained little past 8
/// (1 → 8 threads is 3.1×), and leaving cores free keeps the rest of the machine usable.
pub const MAX_PIPELINE_THREADS: usize = 8;

/// How many threads to give whisper and sherpa-onnx: the performance cores, capped.
pub fn pipeline_threads() -> i32 {
    let cores = performance_cores().unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    cores.clamp(1, MAX_PIPELINE_THREADS) as i32
}

/// Physical performance cores (efficiency class above the lowest), or `None` if Windows
/// won't say. A machine with one efficiency class reports all of its cores.
fn performance_cores() -> Option<usize> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    unsafe {
        let mut bytes = 0u32;
        let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut bytes);
        if bytes == 0 {
            return None;
        }
        let mut buffer = vec![0u8; bytes as usize];
        GetLogicalProcessorInformationEx(RelationProcessorCore, Some(buffer.as_mut_ptr().cast()), &mut bytes).ok()?;

        let mut classes: Vec<u8> = Vec::new();
        let mut offset = 0usize;
        while offset + std::mem::size_of::<u32>() * 2 <= bytes as usize {
            let info = &*(buffer.as_ptr().add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX);
            let size = info.Size as usize;
            if size == 0 {
                break;
            }
            if info.Relationship == RelationProcessorCore {
                classes.push(info.Anonymous.Processor.EfficiencyClass);
            }
            offset += size;
        }
        (!classes.is_empty()).then(|| {
            let top = classes.iter().copied().max().unwrap_or(0);
            classes.iter().filter(|&&c| c == top).count()
        })
    }
}

/// Opts this process out of execution-speed throttling while it lives, and restores the
/// default on drop. Held only for the pipeline: ambient capture belongs on the E-cores
/// (M3.1 §4), so the rest of the app stays throttled.
pub struct FullSpeed(());

impl FullSpeed {
    pub fn acquire() -> Self {
        set_throttling(PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
        FullSpeed(())
    }
}

impl Drop for FullSpeed {
    fn drop(&mut self) {
        // ControlMask 0: back to whatever Windows decides, the default.
        set_throttling(0, 0);
    }
}

fn set_throttling(control_mask: u32, state_mask: u32) {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control_mask,
        StateMask: state_mask,
    };
    unsafe {
        let result = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not change power throttling");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_count_is_within_the_cap_on_this_machine() {
        let threads = pipeline_threads();
        assert!((1..=MAX_PIPELINE_THREADS as i32).contains(&threads), "got {threads}");
    }

    #[test]
    fn throttling_can_be_turned_off_and_back_on() {
        // Both calls must succeed on a real Windows session; a failure only logs.
        let full_speed = FullSpeed::acquire();
        drop(full_speed);
    }
}
