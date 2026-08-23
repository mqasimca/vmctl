use serde_json::{Value, json};

use crate::AGENT_SCHEMA_VERSION;

pub(crate) fn success(result: Value) -> Value {
    json!({
        "schema_version": AGENT_SCHEMA_VERSION,
        "ok": true,
        "result": result,
    })
}

pub(crate) fn print_json_success(result: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&success(result)).expect("JSON values are serializable")
    );
}

pub(crate) fn schema() -> Value {
    json!({
        "schema_version": AGENT_SCHEMA_VERSION,
        "vmctl_version": env!("CARGO_PKG_VERSION"),
        "json": {
            "success": {
                "schema_version": "integer",
                "ok": true,
                "result": "command-specific JSON value"
            },
            "error": {
                "schema_version": "integer",
                "ok": false,
                "error": {
                    "code": "stable machine-readable error code",
                    "message": "human-readable error",
                    "hint": "optional recovery guidance",
                    "…": "optional command-specific fields"
                }
            },
            "transport": {
                "success": "stdout",
                "error": "stderr"
            }
        },
        "commands": {
            "read_only": ["list", "status", "plan", "logs", "report", "doctor", "schema"],
            "state_changing": ["create", "set", "start", "stop", "kill", "restart", "snapshot", "disk", "delete-disk", "delete-vm", "shortcut", "host"],
            "conditional": ["get", "guest", "monitor"],
            "native_output": ["ssh", "completion"]
        },
        "notes": [
            "Pass --output json for structured command results.",
            "The schema command always writes JSON; ssh and completion do not use the JSON contract.",
            "Unknown fields may be added; automation should rely only on documented fields."
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_is_versioned() {
        let value = success(json!({"name": "demo"}));
        assert_eq!(value["schema_version"], AGENT_SCHEMA_VERSION);
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"]["name"], "demo");
    }

    #[test]
    fn schema_describes_safe_command_groups() {
        let value = schema();
        assert_eq!(value["schema_version"], AGENT_SCHEMA_VERSION);
        assert!(
            value["commands"]["read_only"]
                .as_array()
                .is_some_and(|commands| commands.iter().any(|command| command == "doctor"))
        );
        assert!(
            value["commands"]["state_changing"]
                .as_array()
                .is_some_and(|commands| commands.iter().any(|command| command == "delete-vm"))
        );
    }
}
