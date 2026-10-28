//! A Windows Job Object that kills what is assigned to it when the app goes away (M3.4
//! Task 5).
//!
//! The MCP sidecar must never outlive Whimbrel, even when Whimbrel is killed from Task
//! Manager. The job's only handle lives in this process, so when the process ends for any
//! reason the handle closes and Windows terminates the job's processes.
//!
//! The app itself is deliberately **not** in the job. A process in a kill-on-close job
//! passes membership to everything it starts, so the installer the updater launches
//! just before "Restart now" exits the app would be killed with it. Only the processes
//! assigned here (the sidecar) are covered.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub struct KillOnCloseJob(HANDLE);

// A job handle is just a kernel object reference; it may be used and closed from any thread.
unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    pub fn new() -> io::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(None, None).map_err(io::Error::other)?;
            let job = KillOnCloseJob(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(io::Error::other)?;
            Ok(job)
        }
    }

    /// Puts a child we started into the job. From then on it is killed when the job's handle
    /// closes, and so are any processes it starts afterwards.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())).map_err(io::Error::other) }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;

    /// A process that would run for a minute if nothing stopped it.
    fn long_running() -> Child {
        Command::new("ping").args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
    }

    fn exits_within(child: &mut Child, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_child_in_the_job_dies_when_the_job_closes() {
        let job = KillOnCloseJob::new().unwrap();
        let mut child = long_running();
        job.assign(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "running while the job is open");
        drop(job);
        assert!(exits_within(&mut child, Duration::from_secs(5)), "killed when the job handle closed");
    }

    #[test]
    fn a_process_that_was_never_assigned_survives_the_job_closing() {
        // The installer case: started by the app, never put in the job.
        let job = KillOnCloseJob::new().unwrap();
        let mut bystander = long_running();
        drop(job);
        let died = exits_within(&mut bystander, Duration::from_millis(800));
        let _ = bystander.kill();
        assert!(!died, "closing the job must not touch a process outside it");
    }

    #[test]
    fn a_process_the_assigned_child_starts_is_killed_with_it() {
        let job = KillOnCloseJob::new().unwrap();
        // cmd starts ping as its own child; both must go.
        let mut parent = Command::new("cmd")
            .args(["/c", "ping -n 60 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        job.assign(&parent).unwrap();
        drop(job);
        assert!(exits_within(&mut parent, Duration::from_secs(5)));
    }
}
