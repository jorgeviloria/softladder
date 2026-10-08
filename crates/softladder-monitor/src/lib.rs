//! Monitoring protocol types for SoftLadder — **server lands in M6**.
//!
//! This crate defines *what* a monitoring client and a SoftLadder runtime say
//! to each other, not *how* they say it. The types are plain `serde` data so
//! that one definition serves every transport.
//!
//! # Planned transport (M6)
//!
//! * **Framing** — one [`Request`] per message, one [`Response`] back, matched
//!   by order on a connection. JSON is the default encoding; CBOR is offered
//!   for bandwidth-constrained links.
//! * **TCP** — a line-delimited JSON stream on a configurable port, disabled
//!   unless a project or a command line flag enables it.
//! * **WebSocket** — the same messages over a WebSocket endpoint for browser
//!   dashboards and the future web UI.
//! * **TLS** — optional; when enabled, certificates and keys are configured by
//!   path and the plain TCP listener is not started.
//! * **Authentication** — every connection presents a bearer token. Tokens
//!   carry a role: `viewer` may call `GetStatus`, `GetVars` and `GetAlarms`,
//!   `operator` may additionally call `SetVar` and `ReleaseVar`, and `engineer`
//!   may additionally call `ForceVar`. Requests above the role are refused with
//!   [`Response::Error`] and never reach the runtime.
//! * **Audit** — every mutating request is logged with the token role, the
//!   variable and the previous value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use serde::{Deserialize, Serialize};

pub use softladder_core::{Value, VarRef};

/// Request sent by a monitoring client to a SoftLadder runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Ask for the runtime status.
    GetStatus,
    /// Read the listed variables.
    GetVars {
        /// Variables to read.
        vars: Vec<VarRef>,
    },
    /// Write values that the control program may overwrite on the next scan.
    SetVar {
        /// Variable to write.
        var: VarRef,
        /// Value to write.
        value: Value,
    },
    /// Pin a variable to a value that the control program cannot overwrite.
    ForceVar {
        /// Variable to force.
        var: VarRef,
        /// Value to force it to.
        value: Value,
    },
    /// Remove a force installed by [`Request::ForceVar`].
    ReleaseVar {
        /// Variable to release.
        var: VarRef,
    },
    /// Ask for the currently active alarms.
    GetAlarms,
}

/// Response sent by the SoftLadder runtime back to a monitoring client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Reply to [`Request::GetStatus`].
    Status {
        /// Runtime state name, e.g. `run` or `stop`.
        state: String,
        /// Number of completed scans.
        cycles: u64,
        /// Simulated timestamp of the first scan after the last start.
        started_at_ms: Option<u64>,
    },
    /// Reply to [`Request::GetVars`]; unknown variables are reported as `null`.
    Vars {
        /// One entry per requested variable, in request order.
        values: Vec<(VarRef, Option<Value>)>,
    },
    /// Reply to [`Request::GetAlarms`].
    Alarms {
        /// Active alarms, most severe first.
        alarms: Vec<Alarm>,
    },
    /// Acknowledgement of a mutating request.
    Ok,
    /// A request could not be served.
    Error {
        /// Stable machine readable error code.
        code: String,
        /// Human readable explanation.
        message: String,
    },
}

/// One active alarm reported by [`Response::Alarms`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
    /// Severity name, e.g. `warning` or `error`.
    pub severity: String,
    /// Stable alarm code.
    pub code: String,
    /// Human readable message.
    pub message: String,
    /// Variable the alarm refers to, when it is variable specific.
    pub var: Option<VarRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    #[test]
    fn requests_round_trip_through_json() {
        let requests = vec![
            Request::GetStatus,
            Request::GetVars {
                vars: vec![var("%Q0"), var("%MW3")],
            },
            Request::SetVar {
                var: var("%MW3"),
                value: Value::Word(17),
            },
            Request::ForceVar {
                var: var("%Q0"),
                value: Value::Bit(true),
            },
            Request::ReleaseVar { var: var("%Q0") },
            Request::GetAlarms,
        ];
        for request in requests {
            let json = serde_json::to_string(&request).expect("serializes");
            assert!(json.contains("\"type\""), "tagged representation: {json}");
            let decoded: Request = serde_json::from_str(&json).expect("deserializes");
            assert_eq!(decoded, request);
        }
    }

    #[test]
    fn responses_round_trip_through_json() {
        let responses = vec![
            Response::Status {
                state: "run".to_owned(),
                cycles: 42,
                started_at_ms: Some(1000),
            },
            Response::Vars {
                values: vec![(var("%Q0"), Some(Value::Bit(true))), (var("%M9"), None)],
            },
            Response::Alarms {
                alarms: vec![Alarm {
                    severity: "error".to_owned(),
                    code: "SL-E002".to_owned(),
                    message: "division by zero".to_owned(),
                    var: Some(var("%MW0")),
                }],
            },
            Response::Ok,
            Response::Error {
                code: "unauthorized".to_owned(),
                message: "token role `viewer` may not force variables".to_owned(),
            },
        ];
        for response in responses {
            let json = serde_json::to_string(&response).expect("serializes");
            let decoded: Response = serde_json::from_str(&json).expect("deserializes");
            assert_eq!(decoded, response);
        }
    }
}
