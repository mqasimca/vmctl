use clap::{CommandFactory, Parser, error::ErrorKind};
use clap_complete::CompleteEnv;

use vmctl::cli::OutputFormat;

#[cfg(unix)]
fn configure_sigpipe() {
    // Let the kernel terminate cleanly when a downstream command closes stdout.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    configure_sigpipe();

    CompleteEnv::with_factory(vmctl::cli::Cli::command)
        .var("VMCTL_COMPLETE")
        .complete();

    let cli = match vmctl::cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit();
        }
        Err(error) if json_requested() => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "schema_version": 1,
                    "ok": false,
                    "error": {
                        "code": "cli_parse",
                        "message": error.to_string(),
                    },
                })
            );
            return std::process::ExitCode::FAILURE;
        }
        Err(error) => error.exit(),
    };
    let output = cli.output;
    match vmctl::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if output == OutputFormat::Json {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&error.json_value()).unwrap()
                );
            } else {
                eprintln!("vmctl: {error}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn json_requested() -> bool {
    json_requested_from(std::env::args().skip(1))
}

fn json_requested_from(mut args: impl Iterator<Item = String>) -> bool {
    while let Some(argument) = args.next() {
        if argument == "--" {
            break;
        }
        if argument == "--output=json" {
            return true;
        }
        if argument == "--output" && args.next().as_deref() == Some("json") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_detection_stops_at_guest_argument_delimiter() {
        assert!(!json_requested_from(
            [
                "guest".to_string(),
                "vm".to_string(),
                "exec".to_string(),
                "program".to_string(),
                "--".to_string(),
                "--output".to_string(),
                "json".to_string(),
            ]
            .into_iter()
        ));
        assert!(json_requested_from(
            ["--output=json".to_string()].into_iter()
        ));
    }
}
