//! Minimal web dashboard — **M6 slice six (plain HTTP, `std` only)**.
//!
//! `GET /` serves an offline-friendly page that polls three JSON endpoints;
//! reads and mutations share the monitor [`Session`](crate::Session), so roles
//! and audit behave exactly like the TCP protocol. The `/api/vars` shape is
//! dashboard-local (variable names as text); everything else reuses the
//! protocol [`Response`](crate::Response) JSON.
//!
//! Endpoints (all but `/` require `?token=…`):
//!
//! * `GET /api/status` → status.
//! * `GET /api/vars?var=%Q0&var=%MW3` → `{"values":[{"var":"%I0","value":…}]}`.
//! * `GET /api/alarms` → alarms.
//! * `POST /api/set`, `/api/force` with `{"var":"…","value":…}` → `Ok`.
//! * `POST /api/release` with `{"var":"…"}` → `Ok`.
//! * `POST /api/ack` with `{"id":…}` → `Ok`.
//!
//! Bodies use the friendly scalar form (`true`, `17`, `1.5`); unknown bearers
//! get `403`, unknown paths `404`. WebSocket and TLS stay for later slices.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{MonitorTarget, Request, Response, Session, Token, Value, VarRef};

/// Dashboard page served at `GET /`.
const PAGE: &str = include_str!("dashboard.html");

/// How [`serve_http`] behaves.
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    /// Known bearers, by id.
    pub tokens: HashMap<String, Token>,
    /// Read timeout per connection; a silent client is dropped.
    pub read_timeout: Duration,
    /// Requests served before the listener closes.
    pub max_requests: usize,
}

impl DashboardConfig {
    /// Creates a config with `tokens`, a 30 s read timeout and 1024 requests.
    pub fn new(tokens: HashMap<String, Token>) -> Self {
        Self {
            tokens,
            read_timeout: Duration::from_secs(30),
            max_requests: 1024,
        }
    }
}

/// One dashboard variable reading, with the name as text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VarReading {
    /// Variable name, e.g. `"%Q0"`.
    var: String,
    /// Current value, if known.
    value: Option<Value>,
}

/// Serves up to `max_requests` HTTP requests sequentially on `listener`.
pub fn serve_http<T: MonitorTarget>(
    listener: TcpListener,
    target: &mut T,
    config: &DashboardConfig,
) -> std::io::Result<usize> {
    let mut served = 0;
    while served < config.max_requests {
        let (stream, _) = listener.accept()?;
        serve_http_stream(stream, target, config).map_err(http_other)?;
        served += 1;
    }
    Ok(served)
}

/// Serves one HTTP request on `stream`.
pub fn serve_http_stream<T: MonitorTarget>(
    stream: TcpStream,
    target: &mut T,
    config: &DashboardConfig,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut writer = stream;

    let mut headline = String::new();
    if reader
        .read_line(&mut headline)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Ok(());
    }
    let mut parts = headline.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target_path = parts.next().unwrap_or("/").to_owned();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .map_err(|error| error.to_string())?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length.min(1 << 20)];
    if !body.is_empty() {
        reader
            .read_exact(&mut body)
            .map_err(|error| error.to_string())?;
    }

    let (status, content_type, payload) = route(&method, &target_path, &body, target, config);
    let head = format!(
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    writer
        .write_all(head.as_bytes())
        .and_then(|()| writer.write_all(&payload))
        .map_err(|error| error.to_string())
}

fn route<T: MonitorTarget>(
    method: &str,
    path: &str,
    body: &[u8],
    target: &mut T,
    config: &DashboardConfig,
) -> (&'static str, &'static str, Vec<u8>) {
    if method == "GET" && (path == "/" || path.starts_with("/?")) {
        return (
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes().to_vec(),
        );
    }
    let (route, query) = match path.split_once('?') {
        Some((route, query)) => (route, query),
        None => (path, ""),
    };
    let params = parse_query(query);
    let token = match params.get("token").and_then(|id| config.tokens.get(id)) {
        Some(token) => token.clone(),
        None => {
            return json(
                403,
                &Response::Error {
                    code: "unknown_token".to_owned(),
                    message: "a known ?token=… bearer is required".to_owned(),
                },
            );
        }
    };
    let mut session = Session::new(token, target);
    let response = match (method, route) {
        ("GET", "/api/status") => session.handle(Request::GetStatus),
        ("GET", "/api/vars") => {
            let mut vars = Vec::new();
            for name in params.get_all("var") {
                match name.parse::<VarRef>() {
                    Ok(var) => vars.push(var),
                    Err(error) => {
                        return json(
                            400,
                            &Response::Error {
                                code: "bad_var".to_owned(),
                                message: error.to_string(),
                            },
                        );
                    }
                }
            }
            let readings = match session.handle(Request::GetVars { vars }) {
                Response::Vars { values } => values
                    .into_iter()
                    .map(|(var, value)| VarReading {
                        var: var.to_string(),
                        value,
                    })
                    .collect::<Vec<_>>(),
                other => {
                    return json(500, &other);
                }
            };
            return json(200, &serde_json::json!({ "values": readings }));
        }
        ("GET", "/api/alarms") => session.handle(Request::GetAlarms),
        ("POST", "/api/set" | "/api/force") => {
            let write = match serde_json::from_slice::<VarWrite>(body) {
                Ok(write) => write,
                Err(_) => {
                    return json(
                        400,
                        &Response::Error {
                            code: "bad_request".to_owned(),
                            message: "body must be {\"var\":\"…\",\"value\":…}".to_owned(),
                        },
                    );
                }
            };
            let var = match write.var.parse::<VarRef>() {
                Ok(var) => var,
                Err(error) => {
                    return json(
                        400,
                        &Response::Error {
                            code: "bad_var".to_owned(),
                            message: error.to_string(),
                        },
                    );
                }
            };
            let request = if route == "/api/set" {
                Request::SetVar {
                    var,
                    value: write.value.into(),
                }
            } else {
                Request::ForceVar {
                    var,
                    value: write.value.into(),
                }
            };
            session.handle(request)
        }
        ("POST", "/api/release") => {
            let release = match serde_json::from_slice::<VarOnly>(body) {
                Ok(release) => release,
                Err(_) => {
                    return json(400, &bad_request("body must be {\"var\":\"…\"}"));
                }
            };
            let var = match release.var.parse::<VarRef>() {
                Ok(var) => var,
                Err(error) => {
                    return json(400, &bad_var(&error.to_string()));
                }
            };
            session.handle(Request::ReleaseVar { var })
        }
        ("POST", "/api/ack") => {
            let ack = match serde_json::from_slice::<AckBody>(body) {
                Ok(ack) => ack,
                Err(_) => {
                    return json(400, &bad_request("body must be {\"id\":…}"));
                }
            };
            session.handle(Request::AckAlarm { id: ack.id })
        }
        _ => {
            return json(
                404,
                &Response::Error {
                    code: "not_found".to_owned(),
                    message: format!("no such endpoint: {route}"),
                },
            );
        }
    };
    let status = match &response {
        Response::Error { code, .. } if code == "unauthorized" => 403,
        Response::Error { .. } => 400,
        _ => 200,
    };
    json(status, &response)
}

fn json(status: u16, payload: &impl Serialize) -> (&'static str, &'static str, Vec<u8>) {
    let status_text: &'static str = match status {
        200 => "200 OK",
        400 => "400 Bad Request",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        _ => "500 Internal Server Error",
    };
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    (status_text, "application/json", body)
}

fn bad_request(message: &str) -> Response {
    Response::Error {
        code: "bad_request".to_owned(),
        message: message.to_owned(),
    }
}

fn bad_var(message: &str) -> Response {
    Response::Error {
        code: "bad_var".to_owned(),
        message: message.to_owned(),
    }
}

/// Minimal query string: `&`-separated, percent-decoded, multi-valued.
#[derive(Debug, Default)]
struct QueryParams {
    values: Vec<(String, String)>,
}

impl QueryParams {
    fn get(&self, key: &str) -> Option<&String> {
        self.values
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    fn get_all(&self, key: &str) -> Vec<String> {
        self.values
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
            .collect()
    }
}

fn parse_query(query: &str) -> QueryParams {
    let mut params = QueryParams::default();
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        params
            .values
            .push((percent_decode(name), percent_decode(value)));
    }
    params
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let (Some(high), Some(low)) = (hex(index + 1, bytes), hex(index + 2, bytes)) {
                out.push(high << 4 | low);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(index: usize, bytes: &[u8]) -> Option<u8> {
    bytes.get(index).and_then(|byte| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    })
}

/// `{"var":"…","value":…}` with the friendly scalar form.
#[derive(Debug, Deserialize)]
struct VarWrite {
    var: String,
    value: FriendlyScalar,
}

/// `{"var":"…"}`.
#[derive(Debug, Deserialize)]
struct VarOnly {
    var: String,
}

/// `{"id":…}`.
#[derive(Debug, Deserialize)]
struct AckBody {
    id: u64,
}

/// Friendly scalar: boolean, integer or real.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FriendlyScalar {
    /// A bit.
    Bool(bool),
    /// A word when it fits `i32`, a double word otherwise.
    Int(i64),
    /// A real.
    Float(f64),
}

impl From<FriendlyScalar> for Value {
    fn from(value: FriendlyScalar) -> Value {
        match value {
            FriendlyScalar::Bool(bit) => Value::Bit(bit),
            FriendlyScalar::Int(int) => i32::try_from(int)
                .map(Value::Word)
                .unwrap_or(Value::DWord(int)),
            FriendlyScalar::Float(real) => Value::Real(real),
        }
    }
}

fn http_other(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Alarm, Role};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    #[derive(Debug, Default)]
    struct MockTarget {
        store: HashMap<VarRef, Value>,
        forces: HashMap<VarRef, Value>,
    }

    impl MonitorTarget for MockTarget {
        fn status(&self) -> (String, u64, Option<u64>) {
            ("run".to_owned(), 9, Some(100))
        }

        fn read(&self, var: &VarRef) -> Option<Value> {
            self.forces
                .get(var)
                .or_else(|| self.store.get(var))
                .copied()
        }

        fn write(
            &mut self,
            var: &VarRef,
            value: &Value,
        ) -> Result<Option<Value>, crate::TargetError> {
            Ok(self.store.insert(var.clone(), *value))
        }

        fn force(
            &mut self,
            var: &VarRef,
            value: &Value,
        ) -> Result<Option<Value>, crate::TargetError> {
            Ok(self.forces.insert(var.clone(), *value))
        }

        fn release(&mut self, var: &VarRef) -> bool {
            self.forces.remove(var).is_some()
        }

        fn alarms(&self) -> Vec<Alarm> {
            Vec::new()
        }

        fn acknowledge(&mut self, _id: u64, _by: &str) -> bool {
            false
        }
    }

    fn tokens() -> HashMap<String, Token> {
        HashMap::from([
            (
                "view-1".to_owned(),
                Token::new("view-1".to_owned(), Role::Viewer),
            ),
            (
                "eng-1".to_owned(),
                Token::new("eng-1".to_owned(), Role::Engineer),
            ),
        ])
    }

    /// Sends one raw HTTP request, returns (status line, body).
    fn exchange(request: &str) -> (String, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let mut config = DashboardConfig::new(tokens());
        config.read_timeout = Duration::from_secs(5);
        config.max_requests = 1;
        let server = std::thread::spawn(move || {
            let mut target = MockTarget::default();
            target.store.insert(var("%Q0"), Value::Bit(false));
            serve_http(listener, &mut target, &config).expect("dashboard serves");
            target
        });

        let mut stream = TcpStream::connect(addr).expect("client connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout sets");
        stream.write_all(request.as_bytes()).expect("client writes");
        let mut response = String::new();
        BufReader::new(stream)
            .read_to_string(&mut response)
            .expect("client reads");
        server.join().expect("server joins");

        let (head, body) = response.split_once("\r\n\r\n").expect("head and body");
        let status = head.lines().next().unwrap_or("").to_owned();
        (status, body.to_owned())
    }

    fn get(path: &str) -> (String, String) {
        exchange(&format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"))
    }

    fn post(path: &str, body: &str) -> (String, String) {
        exchange(&format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
    }

    #[test]
    fn page_status_vars_and_alarms_serve() {
        let (status, body) = get("/");
        assert!(status.contains("200"), "{status}");
        assert!(body.contains("SoftLadder dashboard"), "serves the page");

        let (status, body) = get("/api/status?token=view-1");
        assert!(status.contains("200"), "{status} {body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("decodes");
        assert_eq!(json["cycles"], 9);

        let (status, body) = get("/api/vars?token=view-1&var=%25Q0&var=%25M9");
        assert!(status.contains("200"), "{status} {body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("decodes");
        assert_eq!(json["values"][0]["var"], "%Q0");
        assert_eq!(
            json["values"][0]["value"],
            serde_json::json!({"Bit": false})
        );
        assert_eq!(json["values"][1]["var"], "%M9");
        assert!(json["values"][1]["value"].is_null());

        let (status, body) = get("/api/alarms?token=view-1");
        assert!(status.contains("200"), "{status} {body}");
        assert_eq!(body.trim(), r#"{"type":"alarms","alarms":[]}"#);
    }

    #[test]
    fn mutations_go_through_roles() {
        let (status, body) = post("/api/force?token=eng-1", r#"{"var":"%Q0","value":true}"#);
        assert!(status.contains("200"), "{status} {body}");

        let (status, _) = post("/api/force?token=view-1", r#"{"var":"%Q0","value":true}"#);
        assert!(status.contains("403"), "{status}");

        let (status, _) = post("/api/set?token=view-1", r#"{"var":"%Q0"}"#);
        assert!(status.contains("400"), "{status}");

        let (status, _) = get("/api/status");
        assert!(status.contains("403"), "{status}");

        let (status, _) = get("/nope?token=view-1");
        assert!(status.contains("404"), "{status}");
    }
}
