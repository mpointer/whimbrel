//! `whimbrel-cloud-runner`: the cloud routine runner, one account per process.
//!
//! - `whimbrel-cloud-runner init --secret-key-file <path>` makes the runner's device keypair, writes
//!   the secret key (base64) to `<path>` (a new file, readable by its owner only) for the host's
//!   secret store, and prints only the public key, which the approving device registers.
//! - `whimbrel-cloud-runner backfill [--max-minutes N]` is the one-time history backfill (`kind_tag`, then
//!   `slice_tag`, on every `items` row on the relay), with the same environment as a run. It works for at most N
//!   minutes over both passes (default 50, 1 to 1380: the runner's token lasts a day), prints the report as JSON
//!   (`finished:false` when the time ran out with progress; `slice_done`, `slice_total` and `slice_stopped` for
//!   the second pass) and exits 0, or prints a short reason and exits 1. One stderr line a minute shows progress.
//!   Safe to run again. The job's task timeout must be above N minutes.
//! - `whimbrel-cloud-runner --version` prints the version and exits 0; it reads no setting.
//! - `whimbrel-cloud-runner` (or `run`) runs once with `RUNNER_DEVICE_SECRET_KEY`, `RUNNER_PROVISION`,
//!   `RUNNER_APPROVER_PUBLIC_KEY` and `RUNNER_WORKER_URL` from the environment, prints the run report
//!   as JSON and exits 0, or prints a short reason and exits 1. Retrying is the host's job.
//!
//! Local time: the run sets `TZ` to the user's synced zone (`cloud_runner::set_process_zone`) before
//! it builds any local time; nothing here uses local time before that.

use std::process::ExitCode;

use base64::{engine::general_purpose::STANDARD, Engine as _};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["run"] => run(),
        ["backfill", rest @ ..] => match parse_backfill_args(rest) {
            Some((minutes, slice_only)) => backfill(minutes, slice_only),
            None => usage(),
        },
        ["init", "--secret-key-file", path] => init(path),
        ["--version"] | ["-V"] => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: whimbrel-cloud-runner [run] | backfill [--slice-only] [--max-minutes N] | init --secret-key-file <path> | --version");
    ExitCode::from(2)
}

/// Default and bounds of `backfill --max-minutes`: the runner's token expires after a day.
const BACKFILL_DEFAULT_MINUTES: u64 = 50;
const BACKFILL_MAX_MINUTES: u64 = 1380;

/// The minutes from the arguments after `backfill`: none gives the default; `--max-minutes N` needs a whole
/// number from 1 to 1380. Anything else is `None` (a usage error).
fn parse_backfill_minutes(args: &[&str]) -> Option<u64> {
    match args {
        [] => Some(BACKFILL_DEFAULT_MINUTES),
        ["--max-minutes", n] => n.parse::<u64>().ok().filter(|m| (1..=BACKFILL_MAX_MINUTES).contains(m)),
        _ => None,
    }
}

/// The arguments after `backfill`: `--slice-only` at most once, before or after the minutes
/// ([`parse_backfill_minutes`]), and nothing else. Returns the minutes and whether only the slice pass runs.
fn parse_backfill_args(args: &[&str]) -> Option<(u64, bool)> {
    let slice_only = args.iter().filter(|a| **a == "--slice-only").count();
    if slice_only > 1 {
        return None;
    }
    let rest: Vec<&str> = args.iter().copied().filter(|a| *a != "--slice-only").collect();
    // `--slice-only` only at either end, never between `--max-minutes` and its number.
    if slice_only == 1 && !(args.first() == Some(&"--slice-only") || args.last() == Some(&"--slice-only")) {
        return None;
    }
    parse_backfill_minutes(&rest).map(|m| (m, slice_only == 1))
}

fn backfill(minutes: u64, slice_only: bool) -> ExitCode {
    let budget = std::time::Duration::from_secs(minutes * 60);
    match cloud_runner::RunnerConfig::from_env().and_then(|cfg| cloud_runner::backfill_once(&cfg, budget, slice_only)) {
        Ok(report) => {
            println!("{}", report.to_json());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // A fixed sentence: no key, token, URL or setting value.
            eprintln!("whimbrel-cloud-runner: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The one line `--version` prints: the binary name and the workspace version. It reads no setting.
fn version_line() -> String {
    format!("whimbrel-cloud-runner {}", env!("CARGO_PKG_VERSION"))
}

fn run() -> ExitCode {
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let result = cloud_runner::RunnerConfig::from_env().and_then(|cfg| cloud_runner::run_once(&cfg, now_ms));
    match result {
        Ok(report) => {
            println!("{}", report.to_json());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // A fixed sentence: no key, token, URL or setting value.
            eprintln!("whimbrel-cloud-runner: {e}");
            ExitCode::FAILURE
        }
    }
}

fn init(path: &str) -> ExitCode {
    let keypair = sync::DeviceKeypair::generate();
    match write_secret(path, &STANDARD.encode(keypair.to_secret_bytes())) {
        Ok(()) => {
            println!("{}", STANDARD.encode(keypair.public_key_bytes()));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("whimbrel-cloud-runner: could not write the secret key file: {}", e.kind());
            ExitCode::FAILURE
        }
    }
}

/// Writes `secret` to a new file at `path` that only its owner can read; never replaces a file.
fn write_secret(path: &str, secret: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(secret.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{parse_backfill_args, parse_backfill_minutes, version_line};

    #[test]
    fn version_line_names_the_binary_and_the_crate_version() {
        let line = version_line();
        assert_eq!(line, format!("whimbrel-cloud-runner {}", env!("CARGO_PKG_VERSION")));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn backfill_minutes_default_to_50_and_accept_1_to_1380() {
        assert_eq!(parse_backfill_minutes(&[]), Some(50));
        assert_eq!(parse_backfill_minutes(&["--max-minutes", "5"]), Some(5));
        assert_eq!(parse_backfill_minutes(&["--max-minutes", "1"]), Some(1));
        assert_eq!(parse_backfill_minutes(&["--max-minutes", "1380"]), Some(1380));
    }

    #[test]
    fn backfill_minutes_reject_zero_too_many_non_numbers_and_stray_arguments() {
        for bad in [&["--max-minutes", "0"][..], &["--max-minutes", "1381"], &["--max-minutes", "abc"], &["--max-minutes", "-3"], &["--max-minutes", ""], &["--max-minutes"], &["5"], &["--max-minutes", "5", "x"], &["--other", "5"]] {
            assert_eq!(parse_backfill_minutes(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn backfill_slice_only_combines_with_max_minutes_in_either_order() {
        assert_eq!(parse_backfill_args(&[]), Some((50, false)));
        assert_eq!(parse_backfill_args(&["--max-minutes", "5"]), Some((5, false)));
        assert_eq!(parse_backfill_args(&["--slice-only"]), Some((50, true)));
        assert_eq!(parse_backfill_args(&["--slice-only", "--max-minutes", "7"]), Some((7, true)));
        assert_eq!(parse_backfill_args(&["--max-minutes", "7", "--slice-only"]), Some((7, true)));
    }

    #[test]
    fn backfill_slice_only_rejects_repeats_stray_arguments_and_bad_minutes() {
        for bad in [
            &["--slice-only", "--slice-only"][..],
            &["--max-minutes", "--slice-only", "5"],
            &["--slice-only", "x"],
            &["x", "--slice-only"],
            &["--slice-only", "--max-minutes", "0"],
            &["--slice-only", "--max-minutes", "1381"],
            &["--slice-only", "--max-minutes"],
            &["--max-minutes", "5", "--slice-only", "x"],
            &["--slice", "--max-minutes", "5"],
            &["--slice-only=1"],
        ] {
            assert_eq!(parse_backfill_args(bad), None, "{bad:?}");
        }
    }
}
