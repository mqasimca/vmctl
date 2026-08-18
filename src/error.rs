use std::fmt::Display;
use std::io;
use std::path::Path;

use serde_json::{Value, json};
use thiserror::Error;

use crate::AGENT_SCHEMA_VERSION;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("invalid argument `{argument}`: {reason}")]
    InvalidArgument { argument: String, reason: String },

    #[error("I/O error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("invalid configuration {path}: {message}")]
    Config { path: String, message: String },

    #[error("command `{command}` is unavailable: {source}")]
    CommandUnavailable {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("command `{command}` failed{status}")]
    CommandFailed { command: String, status: String },

    #[error("VM '{name}' did not stop within {timeout_seconds} seconds (pid {pid}{forced_suffix})")]
    StopTimeout {
        name: String,
        pid: i32,
        timeout_seconds: u64,
        forced_suffix: String,
    },

    #[error("QMP error: {0}")]
    Qmp(String),

    #[error("guest command `{program}` failed: {reason}")]
    GuestCommandFailed {
        program: String,
        reason: String,
        exit_code: Option<i64>,
        signal: Option<i64>,
        result: Box<Value>,
    },

    #[error("guest agent unavailable while running {command}: {detail}")]
    GuestAgentUnavailable { command: String, detail: String },

    #[error("guest-agent protocol error for {command}: {detail}")]
    GuestAgentProtocol { command: String, detail: String },

    #[error(
        "guest command `{program}` timed out after {timeout_seconds} seconds (pid {pid} may still be running)"
    )]
    GuestCommandTimeout {
        program: String,
        pid: u64,
        timeout_seconds: u64,
    },

    #[error(
        "guest shutdown was requested for `{name}`, but QEMU did not stop within {timeout_seconds} seconds (pid {pid})"
    )]
    GuestShutdownTimeout {
        name: String,
        pid: i32,
        timeout_seconds: u64,
    },

    #[error("doctor found {errors} error(s) and {warnings} warning(s)")]
    DoctorFailed {
        errors: usize,
        warnings: usize,
        report: Box<Value>,
    },

    #[error("disk check found integrity problems in {path}")]
    DiskCheckFailed { path: String, report: Box<Value> },

    #[error("image URL is unavailable for {os} {release} ({architecture}): {cause}")]
    ImageUnavailable {
        os: String,
        release: String,
        architecture: String,
        cause: String,
    },

    #[error("VM '{name}' was not found in {root}")]
    VmNotFound { name: String, root: String },

    #[error(
        "cannot determine the home directory; set HOME on Unix/macOS or USERPROFILE, HOMEDRIVE, and HOMEPATH on Windows"
    )]
    HomeDirectoryUnavailable,
}

impl Error {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub fn invalid_argument(argument: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidArgument {
            argument: argument.into(),
            reason: reason.into(),
        }
    }

    pub fn io(path: impl Display, source: io::Error) -> Self {
        Self::Io {
            path: path.to_string(),
            source,
        }
    }

    pub fn command_unavailable(command: &str, source: io::Error) -> Self {
        Self::CommandUnavailable {
            command: command.to_string(),
            source,
        }
    }

    pub fn command_failed(command: &str) -> Self {
        Self::CommandFailed {
            command: command.to_string(),
            status: String::new(),
        }
    }

    pub fn command_failed_status(command: &str, status: impl Display) -> Self {
        Self::CommandFailed {
            command: command.to_string(),
            status: format!(" with status {status}"),
        }
    }

    pub fn config(path: impl AsRef<Path>, message: impl Into<String>) -> Self {
        Self::Config {
            path: path.as_ref().display().to_string(),
            message: message.into(),
        }
    }

    pub fn vm_not_found(name: &str, root: impl AsRef<Path>) -> Self {
        Self::VmNotFound {
            name: name.to_string(),
            root: root.as_ref().display().to_string(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Message(_) => "error",
            Self::InvalidArgument { .. } => "invalid_argument",
            Self::Io { .. } => "io_error",
            Self::Config { .. } => "config_invalid",
            Self::CommandUnavailable { .. } => "command_unavailable",
            Self::CommandFailed { .. } => "command_failed",
            Self::StopTimeout { .. } => "stop_timeout",
            Self::Qmp(_) => "qmp_error",
            Self::GuestCommandFailed { .. } => "guest_command_failed",
            Self::GuestAgentUnavailable { .. } => "guest_agent_unavailable",
            Self::GuestAgentProtocol { .. } => "guest_agent_protocol",
            Self::GuestCommandTimeout { .. } => "guest_command_timeout",
            Self::GuestShutdownTimeout { .. } => "guest_shutdown_timeout",
            Self::DoctorFailed { .. } => "doctor_failed",
            Self::DiskCheckFailed { .. } => "disk_check_failed",
            Self::ImageUnavailable { .. } => "image_unavailable",
            Self::VmNotFound { .. } => "vm_not_found",
            Self::HomeDirectoryUnavailable => "home_directory_unavailable",
        }
    }

    pub fn json_value(&self) -> Value {
        let mut error = json!({
            "code": self.code(),
            "message": self.to_string(),
        });
        let details = error.as_object_mut().expect("error object");
        match self {
            Self::InvalidArgument { argument, reason } => {
                details.insert("argument".to_string(), json!(argument));
                details.insert("cause".to_string(), json!(reason));
                details.insert(
                    "hint".to_string(),
                    json!("Correct the argument and retry the command."),
                );
            }
            Self::Io { path, source } => {
                details.insert("path".to_string(), json!(path));
                details.insert("cause".to_string(), json!(source.to_string()));
            }
            Self::Config { path, message } => {
                details.insert("path".to_string(), json!(path));
                details.insert("cause".to_string(), json!(message));
                details.insert(
                    "hint".to_string(),
                    json!("Fix the configuration field, then retry `vmctl doctor VM`."),
                );
            }
            Self::CommandUnavailable { command, source } => {
                details.insert("command".to_string(), json!(command));
                details.insert("cause".to_string(), json!(source.to_string()));
                details.insert(
                    "hint".to_string(),
                    json!("Install the missing dependency or make it available on PATH."),
                );
            }
            Self::CommandFailed { command, status } => {
                details.insert("command".to_string(), json!(command));
                if !status.is_empty() {
                    details.insert("status".to_string(), json!(status.trim()));
                }
            }
            Self::StopTimeout {
                name,
                pid,
                timeout_seconds,
                forced_suffix,
            } => {
                details.insert("vm".to_string(), json!(name));
                details.insert("pid".to_string(), json!(pid));
                details.insert("timeout_seconds".to_string(), json!(timeout_seconds));
                details.insert("forced".to_string(), json!(!forced_suffix.is_empty()));
                details.insert(
                    "hint".to_string(),
                    json!("Inspect `vmctl status VM`; use `vmctl kill VM` only after confirming the PID."),
                );
            }
            Self::GuestCommandFailed {
                program,
                reason,
                exit_code,
                signal,
                result,
            } => {
                details.insert("program".to_string(), json!(program));
                details.insert("reason".to_string(), json!(reason));
                if let Some(exit_code) = exit_code {
                    details.insert("exit_code".to_string(), json!(exit_code));
                }
                if let Some(signal) = signal {
                    details.insert("signal".to_string(), json!(signal));
                }
                details.insert("result".to_string(), (**result).clone());
                details.insert(
                    "hint".to_string(),
                    json!("Inspect result.stderr and result.stdout, then check the guest command and its permissions."),
                );
            }
            Self::GuestAgentUnavailable { command, detail } => {
                details.insert("command".to_string(), json!(command));
                details.insert("cause".to_string(), json!(detail));
                details.insert(
                    "hint".to_string(),
                    json!(
                        "Install qemu-guest-agent in the guest and enable its service, then retry."
                    ),
                );
            }
            Self::GuestAgentProtocol { command, detail } => {
                details.insert("command".to_string(), json!(command));
                details.insert("cause".to_string(), json!(detail));
                details.insert(
                    "hint".to_string(),
                    json!("Verify the guest agent version and retry the command."),
                );
            }
            Self::GuestCommandTimeout {
                program,
                pid,
                timeout_seconds,
            } => {
                details.insert("program".to_string(), json!(program));
                details.insert("pid".to_string(), json!(pid));
                details.insert("timeout_seconds".to_string(), json!(timeout_seconds));
                details.insert(
                    "hint".to_string(),
                    json!("The guest process may still be running; inspect it in the guest or use a shorter-lived command."),
                );
            }
            Self::GuestShutdownTimeout {
                name,
                pid,
                timeout_seconds,
            } => {
                details.insert("vm".to_string(), json!(name));
                details.insert("pid".to_string(), json!(pid));
                details.insert("timeout_seconds".to_string(), json!(timeout_seconds));
                details.insert(
                    "hint".to_string(),
                    json!("Inspect `vmctl status VM`; use `vmctl stop VM --force` if the VM is not responding."),
                );
            }
            Self::DoctorFailed {
                errors,
                warnings,
                report,
            } => {
                details.insert("errors".to_string(), json!(errors));
                details.insert("warnings".to_string(), json!(warnings));
                details.insert("report".to_string(), (**report).clone());
            }
            Self::DiskCheckFailed { path, report } => {
                details.insert("path".to_string(), json!(path));
                details.insert("report".to_string(), (**report).clone());
                details.insert(
                    "hint".to_string(),
                    json!("Restore a known-good backup before attempting repair."),
                );
            }
            Self::ImageUnavailable {
                os,
                release,
                architecture,
                cause,
            } => {
                details.insert("os".to_string(), json!(os));
                details.insert("release".to_string(), json!(release));
                details.insert("architecture".to_string(), json!(architecture));
                details.insert("cause".to_string(), json!(cause));
                details.insert(
                    "hint".to_string(),
                    json!("Choose a supported release or verify the provider URL."),
                );
            }
            Self::VmNotFound { name, root } => {
                details.insert("vm".to_string(), json!(name));
                details.insert("directory".to_string(), json!(root));
                details.insert(
                    "hint".to_string(),
                    json!("Use `vmctl list` or pass the VM configuration path."),
                );
            }
            Self::HomeDirectoryUnavailable => {
                details.insert(
                    "hint".to_string(),
                    json!("Set HOME on Unix/macOS or USERPROFILE, HOMEDRIVE, and HOMEPATH on Windows, then retry."),
                );
            }
            Self::Message(_) | Self::Qmp(_) => {}
        }
        json!({
            "schema_version": AGENT_SCHEMA_VERSION,
            "ok": false,
            "error": error,
        })
    }

    pub fn doctor_failed(errors: usize, warnings: usize, report: Value) -> Self {
        Self::DoctorFailed {
            errors,
            warnings,
            report: Box::new(report),
        }
    }

    pub fn disk_check_failed(path: impl AsRef<Path>, report: Value) -> Self {
        Self::DiskCheckFailed {
            path: path.as_ref().display().to_string(),
            report: Box::new(report),
        }
    }

    pub fn guest_command_failed(program: &str, result: Value) -> Self {
        let exit_code = result.get("exitcode").and_then(Value::as_i64);
        let signal = result.get("signal").and_then(Value::as_i64);
        let reason = signal.map_or_else(
            || {
                exit_code.map_or_else(
                    || "the guest agent reported an unsuccessful command".to_string(),
                    |exit_code| format!("exited with code {exit_code}"),
                )
            },
            |signal| format!("terminated by signal {signal}"),
        );
        Self::GuestCommandFailed {
            program: program.to_string(),
            reason,
            exit_code,
            signal,
            result: Box::new(result),
        }
    }

    pub fn guest_agent_unavailable(command: &str, detail: impl Into<String>) -> Self {
        Self::GuestAgentUnavailable {
            command: command.to_string(),
            detail: detail.into(),
        }
    }

    pub fn guest_agent_protocol(command: &str, detail: impl Into<String>) -> Self {
        Self::GuestAgentProtocol {
            command: command.to_string(),
            detail: detail.into(),
        }
    }

    pub fn guest_command_timeout(program: &str, pid: u64, timeout_seconds: u64) -> Self {
        Self::GuestCommandTimeout {
            program: program.to_string(),
            pid,
            timeout_seconds,
        }
    }

    pub fn guest_shutdown_timeout(name: &str, pid: i32, timeout_seconds: u64) -> Self {
        Self::GuestShutdownTimeout {
            name: name.to_string(),
            pid,
            timeout_seconds,
        }
    }

    pub fn stop_timeout(name: &str, pid: i32, timeout_seconds: u64, forced: bool) -> Self {
        Self::StopTimeout {
            name: name.to_string(),
            pid,
            timeout_seconds,
            forced_suffix: if forced {
                ", forced termination also failed".to_string()
            } else {
                String::new()
            },
        }
    }

    pub fn image_unavailable(
        os: &str,
        release: &str,
        architecture: &str,
        cause: impl Into<String>,
    ) -> Self {
        Self::ImageUnavailable {
            os: os.to_string(),
            release: release.to_string(),
            architecture: architecture.to_string(),
            cause: cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_error_has_stable_code_and_context() {
        let error = Error::config("vm.conf", "display is invalid");
        let value = error.json_value();
        assert_eq!(value["schema_version"], crate::AGENT_SCHEMA_VERSION);
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "config_invalid");
        assert_eq!(value["error"]["path"], "vm.conf");

        let error = Error::disk_check_failed("disk.qcow2", json!({"corruptions": 1}));
        assert_eq!(error.json_value()["error"]["code"], "disk_check_failed");
        assert_eq!(error.json_value()["error"]["report"]["corruptions"], 1);

        let error = Error::guest_command_failed(
            "/bin/false",
            json!({"exitcode": 7, "stderr": "permission denied"}),
        );
        assert_eq!(error.json_value()["error"]["code"], "guest_command_failed");
        assert_eq!(error.json_value()["error"]["exit_code"], 7);
        assert_eq!(
            error.json_value()["error"]["result"]["stderr"],
            "permission denied"
        );

        let error = Error::guest_agent_unavailable("guest-ping", "did not respond");
        assert_eq!(
            error.json_value()["error"]["code"],
            "guest_agent_unavailable"
        );
        assert_eq!(error.json_value()["error"]["command"], "guest-ping");

        let error = Error::guest_command_failed("/bin/kill", json!({"signal": 9}));
        assert_eq!(error.json_value()["error"]["signal"], 9);

        let error = Error::guest_command_timeout("/bin/sleep", 42, 1);
        assert_eq!(error.json_value()["error"]["code"], "guest_command_timeout");
        assert_eq!(error.json_value()["error"]["pid"], 42);

        let error = Error::guest_shutdown_timeout("ubuntu", 42, 10);
        assert_eq!(
            error.json_value()["error"]["code"],
            "guest_shutdown_timeout"
        );
        assert_eq!(error.json_value()["error"]["timeout_seconds"], 10);
    }
}
