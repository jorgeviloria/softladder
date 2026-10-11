//! Roles, bearer tokens, audit and the request session — **M6 slice one (pure)**.
//!
//! The crate docs describe the planned transports (TCP, WebSocket, TLS); this
//! module is the transport-free core every one of them will share: *who* may
//! call *what* ([`Role`]), *which* connection acts ([`Token`]), *what* a
//! connection may touch ([`MonitorTarget`]), and *who did what* ([`AuditEntry`]).
//! [`Session::handle`] enforces the role before touching the target, so a
//! refused request can never reach the runtime, and records every mutating
//! call with the token, the role, the variable and its previous value.

use serde::{Deserialize, Serialize};

use crate::{Alarm, Request, Response, Value, VarRef};

/// What a bearer token may do.
///
/// * `viewer` reads status, variables and alarms.
/// * `operator` additionally writes (`SetVar`) and releases forces (`ReleaseVar`).
/// * `engineer` additionally pins forces (`ForceVar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read-only monitoring.
    Viewer,
    /// Monitoring plus writes the program may overwrite.
    Operator,
    /// Everything, including forces the program cannot overwrite.
    Engineer,
}

impl Role {
    /// `true` when this role may issue `request`.
    pub fn permits(self, request: &Request) -> bool {
        match request {
            Request::GetStatus | Request::GetVars { .. } | Request::GetAlarms => true,
            Request::SetVar { .. } | Request::ReleaseVar { .. } => self != Role::Viewer,
            Request::ForceVar { .. } => self == Role::Engineer,
        }
    }

    /// Stable lowercase name used in tokens and audit entries.
    pub fn name(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Engineer => "engineer",
        }
    }

    /// Parses [`Role::name`] output; anything else is `None`.
    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "viewer" => Some(Role::Viewer),
            "operator" => Some(Role::Operator),
            "engineer" => Some(Role::Engineer),
            _ => None,
        }
    }
}

/// An authenticated connection: an opaque bearer id plus its [`Role`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// Opaque bearer string presented by the client.
    pub id: String,
    /// What the bearer may do.
    pub role: Role,
}

impl Token {
    /// Creates a token with `id` and `role`.
    pub fn new(id: String, role: Role) -> Self {
        Self { id, role }
    }
}

/// Why a [`MonitorTarget`] refused a mutating call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetError {
    /// Stable machine readable code, forwarded as [`Response::Error::code`].
    pub code: String,
    /// Human readable explanation, forwarded as [`Response::Error::message`].
    pub message: String,
}

impl TargetError {
    /// Creates an error from a code and a message.
    pub fn new(code: String, message: String) -> Self {
        Self { code, message }
    }
}

impl From<TargetError> for Response {
    fn from(error: TargetError) -> Response {
        Response::Error {
            code: error.code,
            message: error.message,
        }
    }
}

/// What a monitoring session may observe and mutate.
///
/// A future slice implements this for the runtime (status, variable store,
/// force map, alarms); the session logic and its tests only need this trait.
pub trait MonitorTarget {
    /// `(state name, completed scans, start timestamp)` for [`Request::GetStatus`].
    fn status(&self) -> (String, u64, Option<u64>);
    /// Current value of `var`, or `None` when unknown.
    fn read(&self, var: &VarRef) -> Option<Value>;
    /// Writes a value the program may overwrite; returns the previous value.
    fn write(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError>;
    /// Pins a force the program cannot overwrite; returns the previous value.
    fn force(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError>;
    /// Removes a force; returns `true` when one was installed.
    fn release(&mut self, var: &VarRef) -> bool;
    /// Active alarms, most severe first.
    fn alarms(&self) -> Vec<Alarm>;
}

/// Which mutating call an [`AuditEntry`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// A [`Request::SetVar`] call.
    Set,
    /// A [`Request::ForceVar`] call.
    Force,
    /// A [`Request::ReleaseVar`] call.
    Release,
}

/// One audited mutation: who did what to which variable, and what was there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Bearer id that issued the call.
    pub token: String,
    /// Role that bearer carried.
    pub role: Role,
    /// Which call it was.
    pub action: AuditAction,
    /// Variable it touched.
    pub var: VarRef,
    /// Value there before the call, when the target knew one.
    pub previous: Option<Value>,
}

/// An authenticated conversation with a [`MonitorTarget`].
///
/// [`Session::handle`] checks the token role first: refused calls answer
/// `unauthorized` and never reach the target. Served mutations append one
/// [`AuditEntry`] each, in call order.
#[derive(Debug)]
pub struct Session<T> {
    token: Token,
    target: T,
    audit: Vec<AuditEntry>,
}

impl<T: MonitorTarget> Session<T> {
    /// Opens a session for `token` against `target` with an empty audit log.
    pub fn new(token: Token, target: T) -> Self {
        Self {
            token,
            target,
            audit: Vec::new(),
        }
    }

    /// Token the session acts as.
    pub fn token(&self) -> &Token {
        &self.token
    }

    /// Target the session drives.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Mutable access to the target (tests, handover).
    pub fn target_mut(&mut self) -> &mut T {
        &mut self.target
    }

    /// Audit log, in call order.
    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    /// Serves one request: authorize, dispatch, audit.
    pub fn handle(&mut self, request: Request) -> Response {
        if !self.token.role.permits(&request) {
            return Response::Error {
                code: "unauthorized".to_owned(),
                message: format!(
                    "token role `{}` may not issue `{}`",
                    self.token.role.name(),
                    request_name(&request),
                ),
            };
        }
        match request {
            Request::GetStatus => {
                let (state, cycles, started_at_ms) = self.target.status();
                Response::Status {
                    state,
                    cycles,
                    started_at_ms,
                }
            }
            Request::GetVars { vars } => {
                let values = vars
                    .into_iter()
                    .map(|var| {
                        let value = self.target.read(&var);
                        (var, value)
                    })
                    .collect();
                Response::Vars { values }
            }
            Request::SetVar { var, value } => match self.target.write(&var, &value) {
                Ok(previous) => {
                    self.push_audit(AuditAction::Set, var, previous);
                    Response::Ok
                }
                Err(error) => Response::from(error),
            },
            Request::ForceVar { var, value } => match self.target.force(&var, &value) {
                Ok(previous) => {
                    self.push_audit(AuditAction::Force, var, previous);
                    Response::Ok
                }
                Err(error) => Response::from(error),
            },
            Request::ReleaseVar { var } => {
                let previous = self.target.read(&var);
                self.target.release(&var);
                self.push_audit(AuditAction::Release, var, previous);
                Response::Ok
            }
            Request::GetAlarms => Response::Alarms {
                alarms: self.target.alarms(),
            },
        }
    }

    fn push_audit(&mut self, action: AuditAction, var: VarRef, previous: Option<Value>) {
        self.audit.push(AuditEntry {
            token: self.token.id.clone(),
            role: self.token.role,
            action,
            var,
            previous,
        });
    }
}

fn request_name(request: &Request) -> &'static str {
    match request {
        Request::GetStatus => "get_status",
        Request::GetVars { .. } => "get_vars",
        Request::SetVar { .. } => "set_var",
        Request::ForceVar { .. } => "force_var",
        Request::ReleaseVar { .. } => "release_var",
        Request::GetAlarms => "get_alarms",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    /// In-memory target that counts mutating calls.
    #[derive(Debug, Default)]
    struct MockTarget {
        store: HashMap<VarRef, Value>,
        forces: HashMap<VarRef, Value>,
        writes: usize,
        forces_set: usize,
        releases: usize,
    }

    impl MonitorTarget for MockTarget {
        fn status(&self) -> (String, u64, Option<u64>) {
            ("run".to_owned(), 7, Some(1000))
        }

        fn read(&self, var: &VarRef) -> Option<Value> {
            self.forces
                .get(var)
                .or_else(|| self.store.get(var))
                .copied()
        }

        fn write(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError> {
            self.writes += 1;
            if self.forces.contains_key(var) {
                return Err(TargetError::new(
                    "forced".to_owned(),
                    format!("{var} is forced; release it first"),
                ));
            }
            Ok(self.store.insert(var.clone(), *value))
        }

        fn force(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError> {
            self.forces_set += 1;
            Ok(self.forces.insert(var.clone(), *value))
        }

        fn release(&mut self, var: &VarRef) -> bool {
            self.releases += 1;
            self.forces.remove(var).is_some()
        }

        fn alarms(&self) -> Vec<Alarm> {
            Vec::new()
        }
    }

    fn request_for(role: Role) -> Vec<(Request, bool)> {
        let var = var("%Q0");
        let value = Value::Bit(true);
        // (request, whether `role` may issue it)
        vec![
            (Request::GetStatus, true),
            (
                Request::GetVars {
                    vars: vec![var.clone()],
                },
                true,
            ),
            (Request::GetAlarms, true),
            (
                Request::SetVar {
                    var: var.clone(),
                    value,
                },
                role != Role::Viewer,
            ),
            (
                Request::ReleaseVar { var: var.clone() },
                role != Role::Viewer,
            ),
            (Request::ForceVar { var, value }, role == Role::Engineer),
        ]
    }

    #[test]
    fn roles_permit_exactly_the_documented_calls() {
        for role in [Role::Viewer, Role::Operator, Role::Engineer] {
            for (request, allowed) in request_for(role) {
                assert_eq!(role.permits(&request), allowed, "{role:?} on {request:?}");
            }
        }
    }

    #[test]
    fn role_names_round_trip() {
        for role in [Role::Viewer, Role::Operator, Role::Engineer] {
            assert_eq!(Role::parse(role.name()), Some(role));
        }
        assert_eq!(Role::parse("admin"), None);
        assert_eq!(Role::parse("Viewer"), None);
    }

    #[test]
    fn refused_calls_answer_unauthorized_and_never_touch_the_target() {
        let token = Token::new("v-1".to_owned(), Role::Viewer);
        let mut session = Session::new(token, MockTarget::default());
        let response = session.handle(Request::ForceVar {
            var: var("%Q0"),
            value: Value::Bit(true),
        });
        match response {
            Response::Error { code, message } => {
                assert_eq!(code, "unauthorized");
                assert!(message.contains("viewer"), "{message}");
                assert!(message.contains("force_var"), "{message}");
            }
            other => panic!("expected unauthorized, got {other:?}"),
        }
        assert_eq!(session.target().forces_set, 0);
        assert!(session.audit().is_empty());
    }

    #[test]
    fn mutations_audit_token_role_var_and_previous_value() {
        let token = Token::new("e-1".to_owned(), Role::Engineer);
        let mut target = MockTarget::default();
        target.store.insert(var("%MW3"), Value::Word(17));
        let mut session = Session::new(token, target);

        assert_eq!(
            session.handle(Request::SetVar {
                var: var("%MW3"),
                value: Value::Word(18),
            }),
            Response::Ok
        );
        assert_eq!(
            session.handle(Request::ForceVar {
                var: var("%Q0"),
                value: Value::Bit(true),
            }),
            Response::Ok
        );
        // Forcing then writing reports the force as the conflict.
        assert!(matches!(
            session.handle(Request::SetVar {
                var: var("%Q0"),
                value: Value::Bit(false),
            }),
            Response::Error { .. }
        ));
        assert_eq!(
            session.handle(Request::ReleaseVar { var: var("%Q0") }),
            Response::Ok
        );

        let audit = session.audit();
        assert_eq!(audit.len(), 3);
        assert_eq!(
            audit[0],
            AuditEntry {
                token: "e-1".to_owned(),
                role: Role::Engineer,
                action: AuditAction::Set,
                var: var("%MW3"),
                previous: Some(Value::Word(17)),
            }
        );
        assert_eq!(audit[1].action, AuditAction::Force);
        assert_eq!(audit[1].previous, None);
        assert_eq!(audit[2].action, AuditAction::Release);
        assert_eq!(audit[2].previous, Some(Value::Bit(true)));
    }

    #[test]
    fn reads_and_status_serve_without_auditing() {
        let token = Token::new("v-1".to_owned(), Role::Viewer);
        let mut target = MockTarget::default();
        target.store.insert(var("%Q0"), Value::Bit(true));
        let mut session = Session::new(token, target);

        assert!(matches!(
            session.handle(Request::GetStatus),
            Response::Status { .. }
        ));
        match session.handle(Request::GetVars {
            vars: vec![var("%Q0"), var("%M9")],
        }) {
            Response::Vars { values } => {
                assert_eq!(
                    values,
                    vec![(var("%Q0"), Some(Value::Bit(true))), (var("%M9"), None)]
                );
            }
            other => panic!("expected vars, got {other:?}"),
        }
        assert!(session.audit().is_empty());
    }

    #[test]
    fn roles_tokens_and_audit_round_trip_through_json() {
        let token = Token::new("op-2".to_owned(), Role::Operator);
        let json = serde_json::to_string(&token).expect("serializes");
        let decoded: Token = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(decoded, token);

        let entry = AuditEntry {
            token: "op-2".to_owned(),
            role: Role::Operator,
            action: AuditAction::Set,
            var: var("%MW3"),
            previous: Some(Value::Word(17)),
        };
        let json = serde_json::to_string(&entry).expect("serializes");
        assert!(json.contains("\"set\""), "{json}");
        let decoded: AuditEntry = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(decoded, entry);
    }
}
