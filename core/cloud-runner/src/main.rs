//! `whimbrel-cloud-runner`: the cloud routine runner, one account per process.
//!
//! - `whimbrel-cloud-runner init --secret-key-file <path>` makes the runner's device keypair, writes
//!   the secret key (base64) to `<path>` (a new file, readable by its owner only) for the host's
//!   secret store, and prints only the public key, which the approving device registers.
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
        ["init", "--secret-key-file", path] => init(path),
        ["--version"] | ["-V"] => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: whimbrel-cloud-runner [run] | init --secret-key-file <path> | --version");
            ExitCode::from(2)
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
    use super::version_line;

    #[test]
    fn version_line_names_the_binary_and_the_crate_version() {
        let line = version_line();
        assert_eq!(line, format!("whimbrel-cloud-runner {}", env!("CARGO_PKG_VERSION")));
        assert!(!line.contains('\n'));
    }
}
