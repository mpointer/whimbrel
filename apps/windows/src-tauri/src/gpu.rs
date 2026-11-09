//! Vulkan drivers Whimbrel must not load.
//!
//! Whisper runs on the GPU through Vulkan, inside this process. On Windows on Arm, Microsoft's
//! "OpenCL, OpenGL, and Vulkan Compatibility Pack" (the `Microsoft.D3DMappingLayers` package)
//! installs Dozen, a Vulkan driver over Direct3D 12 (`vulkan_dzn.dll`). The Vulkan loader finds it
//! on its own, and in our x64 build running under emulation it crashes the process with an access
//! violation as soon as whisper starts Vulkan. On 0.9.60 that was the engine benchmark, a few
//! seconds after every launch (2026-11-08, a Windows 11 Arm laptop).
//!
//! The loader we bundle (1.4.363) skips drivers whose manifest matches `VK_LOADER_DRIVERS_DISABLE`.
//! Dozen is never the right driver here: a real GPU has its own Vulkan driver, and with none,
//! whisper runs on the CPU. The variable is set before anything starts, so the supervisor, the app
//! and the helpers it launches (the local model's llama-server) all inherit it.

/// The Vulkan loader's driver filter (loader 1.3.234 and later).
pub const DRIVERS_DISABLE_ENV: &str = "VK_LOADER_DRIVERS_DISABLE";
/// Dozen's manifests are `dzn_icd.*.json`.
pub const DISABLED_DRIVERS: &str = "*dzn*";

/// The filter to use, given what the environment already has: someone who set it keeps theirs.
pub fn drivers_disable(existing: Option<&str>) -> Option<&'static str> {
    match existing {
        Some(_) => None,
        None => Some(DISABLED_DRIVERS),
    }
}

/// Call first in `app::run`, while the process has one thread (`set_var` is not thread-safe).
pub fn avoid_broken_vulkan_drivers() {
    let existing = std::env::var(DRIVERS_DISABLE_ENV).ok();
    if let Some(value) = drivers_disable(existing.as_deref()) {
        std::env::set_var(DRIVERS_DISABLE_ENV, value);
    }
}

/// The PC's own processor, which `std::env::consts::ARCH` can't tell: this build is x64, and on
/// Windows on Arm it runs under emulation. Logged at launch so an Arm PC shows in the log.
pub fn native_machine() -> &'static str {
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
    use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let (mut process, mut native) = (IMAGE_FILE_MACHINE::default(), IMAGE_FILE_MACHINE::default());
    match unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) } {
        Ok(()) => machine_name(native.0),
        Err(_) => "unknown",
    }
}

/// `IMAGE_FILE_MACHINE_*` values.
pub fn machine_name(machine: u16) -> &'static str {
    match machine {
        0xAA64 => "arm64",
        0x8664 => "x64",
        0x014C => "x86",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_are_named_and_this_pc_is_one_of_them() {
        assert_eq!((machine_name(0xAA64), machine_name(0x8664), machine_name(0x014C), machine_name(1)), ("arm64", "x64", "x86", "other"));
        assert_ne!(native_machine(), "unknown");
    }

    #[test]
    fn dozen_is_disabled_unless_the_user_set_the_filter() {
        assert_eq!(drivers_disable(None), Some("*dzn*"));
        assert_eq!(drivers_disable(Some("")), None);
        assert_eq!(drivers_disable(Some("*nvidia*")), None);
    }
}
