//! Line-delimited JSON monitor server — **M6 slice three (`std` only)**.
//!
//! One [`Request`](crate::Request) per line, one [`Response`](crate::Response) back, in order.
//! The first line of every connection authenticates it (`{"token":"…"}`) and is
//! answered `Ok` on success; an unknown bearer gets a single `unknown_token`
//! error and the connection closes. Each connection runs its own
//! [`Session`](crate::Session), so its audit log attributes every mutation to
//! the bearer that issued it.
//!
//! Connections are served sequentially on one thread (the ClassicLadder poll
//! pattern, and deterministic under test). A connection ends on a clean client
//! close, on a read timeout, or after `max_requests` requests. Malformed lines
//! answer `bad_request` without closing: one bad line is not an attack.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use serde::Deserialize;

use crate::{MonitorTarget, Request, Response, Session, Token};

/// Line the client sends first to authenticate a connection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AuthLine {
    token: String,
}

/// How a [`serve`] loop and [`serve_stream`] behave.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Known bearers, by id.
    pub tokens: HashMap<String, Token>,
    /// Read timeout per line; a silent client is dropped.
    pub read_timeout: Duration,
    /// Requests served per connection before closing it.
    pub max_requests: usize,
}

impl ServerConfig {
    /// Creates a config with `tokens`, a 30 s read timeout and 1024 requests
    /// per connection.
    pub fn new(tokens: HashMap<String, Token>) -> Self {
        Self {
            tokens,
            read_timeout: Duration::from_secs(30),
            max_requests: 1024,
        }
    }
}

/// Serves up to `max_connections` connections sequentially on `listener`.
///
/// Each connection gets a fresh [`Session`] over `target`, so audit logs stay
/// per bearer. Returns how many connections were served.
pub fn serve<T: MonitorTarget>(
    listener: TcpListener,
    target: &mut T,
    config: &ServerConfig,
    max_connections: usize,
) -> std::io::Result<usize> {
    let mut served = 0;
    for _ in 0..max_connections {
        let (stream, _) = listener.accept()?;
        serve_stream(stream, target, config).map_err(io_other)?;
        served += 1;
    }
    Ok(served)
}

/// Serves one connection: auth line, then requests until close, timeout or
/// `max_requests`. Returns how many requests ran.
pub fn serve_stream<T: MonitorTarget>(
    stream: TcpStream,
    target: &mut T,
    config: &ServerConfig,
) -> Result<usize, String> {
    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut writer = stream;
    let mut line = String::new();

    // Authentication first.
    line.clear();
    if read_line(&mut reader, &mut line)? == 0 {
        return Ok(0);
    }
    let token = match serde_json::from_str::<AuthLine>(&line) {
        Ok(auth) => match config.tokens.get(&auth.token) {
            Some(token) => token.clone(),
            None => {
                write_response(&mut writer, &unknown_token())?;
                return Ok(0);
            }
        },
        Err(_) => {
            write_response(
                &mut writer,
                &bad_request("first line must be {\"token\":\"…\"}"),
            )?;
            return Ok(0);
        }
    };

    let mut session = Session::new(token, target);
    write_response(&mut writer, &Response::Ok)?;
    let mut served = 0;
    while served < config.max_requests {
        line.clear();
        let bytes = read_line(&mut reader, &mut line)?;
        if bytes == 0 {
            break;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => session.handle(request),
            Err(_) => bad_request("unparseable request line"),
        };
        write_response(&mut writer, &response)?;
        served += 1;
    }
    Ok(served)
}

/// Reads one line; maps timeouts and errors to strings, EOF to `Ok(0)`.
fn read_line(reader: &mut BufReader<TcpStream>, line: &mut String) -> Result<usize, String> {
    match reader.read_line(line) {
        Ok(0) => Ok(0),
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut
            {
                Err("read timeout".to_owned())
            } else {
                Err(error.to_string())
            }
        }
    }
}

fn write_response(writer: &mut TcpStream, response: &Response) -> Result<(), String> {
    let mut json = serde_json::to_string(response).map_err(|error| error.to_string())?;
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .map_err(|error| error.to_string())
}

fn unknown_token() -> Response {
    Response::Error {
        code: "unknown_token".to_owned(),
        message: "bearer token is not known".to_owned(),
    }
}

fn bad_request(message: &str) -> Response {
    Response::Error {
        code: "bad_request".to_owned(),
        message: message.to_owned(),
    }
}

fn io_other(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Alarm, Value, VarRef};

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
            ("run".to_owned(), 3, Some(500))
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
                Token::new("view-1".to_owned(), crate::Role::Viewer),
            ),
            (
                "eng-1".to_owned(),
                Token::new("eng-1".to_owned(), crate::Role::Engineer),
            ),
        ])
    }

    fn exchange(lines: &[String]) -> Vec<String> {
        use std::io::{BufRead, BufReader};

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let mut target = MockTarget::default();
        target.store.insert(var("%Q0"), Value::Bit(false));
        let mut config = ServerConfig::new(tokens());
        config.read_timeout = Duration::from_secs(5);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("client connects");
            serve_stream(stream, &mut target, &config).expect("server serves");
        });

        let stream = TcpStream::connect(addr).expect("client connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout sets");
        let mut reader = BufReader::new(stream.try_clone().expect("clone connects"));
        let mut writer = stream;
        // Every line is answered: the auth line with Ok (or an error that
        // closes the connection), each request with its response.
        let mut answers = Vec::new();
        for line in lines {
            writer
                .write_all(format!("{line}\n").as_bytes())
                .expect("client writes");
            let mut answer = String::new();
            reader.read_line(&mut answer).expect("client reads");
            answers.push(answer.trim_end().to_owned());
        }
        // Closing first lets the server see EOF instead of idling to its
        // read timeout.
        drop(reader);
        drop(writer);
        server.join().expect("server joins");
        answers
    }

    fn request_line(request: &Request) -> String {
        serde_json::to_string(request).expect("serializes")
    }

    #[test]
    fn status_read_and_write_round_trip_over_loopback() {
        let answers = exchange(&[
            r#"{"token":"eng-1"}"#.to_owned(),
            request_line(&Request::GetStatus),
            request_line(&Request::GetVars {
                vars: vec![var("%Q0")],
            }),
            request_line(&Request::SetVar {
                var: var("%Q0"),
                value: Value::Bit(true),
            }),
            request_line(&Request::GetVars {
                vars: vec![var("%Q0")],
            }),
        ]);
        assert_eq!(answers.len(), 5);
        assert_eq!(
            serde_json::from_str::<Response>(&answers[0]).expect("decodes"),
            Response::Ok
        );
        let status: Response = serde_json::from_str(&answers[1]).expect("decodes");
        assert!(matches!(status, Response::Status { .. }), "{status:?}");
        let vars: Response = serde_json::from_str(&answers[2]).expect("decodes");
        assert_eq!(
            vars,
            Response::Vars {
                values: vec![(var("%Q0"), Some(Value::Bit(false)))]
            }
        );
        assert_eq!(
            serde_json::from_str::<Response>(&answers[3]).expect("decodes"),
            Response::Ok
        );
        let vars: Response = serde_json::from_str(&answers[4]).expect("decodes");
        assert_eq!(
            vars,
            Response::Vars {
                values: vec![(var("%Q0"), Some(Value::Bit(true)))]
            }
        );
    }

    #[test]
    fn serve_handles_sequential_connections() {
        use std::io::{BufRead, BufReader};

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let addr = listener.local_addr().expect("listener has an address");
        let mut config = ServerConfig::new(tokens());
        config.read_timeout = Duration::from_secs(5);
        let server = std::thread::spawn(move || {
            let mut target = MockTarget::default();
            serve(listener, &mut target, &config, 2).expect("server serves")
        });

        for _ in 0..2 {
            let stream = TcpStream::connect(addr).expect("client connects");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout sets");
            let mut reader = BufReader::new(stream.try_clone().expect("clone connects"));
            let mut writer = stream;
            writer
                .write_all(b"{\"token\":\"view-1\"}\n")
                .expect("client writes");
            writer
                .write_all(format!("{}\n", request_line(&Request::GetStatus)).as_bytes())
                .expect("client writes");
            // Auth ack first, then the status answer.
            for _ in 0..2 {
                let mut answer = String::new();
                reader.read_line(&mut answer).expect("client reads");
                let response: Response = serde_json::from_str(answer.trim_end()).expect("decodes");
                assert!(
                    matches!(response, Response::Ok | Response::Status { .. }),
                    "{response:?}"
                );
            }
        }
        assert_eq!(server.join().expect("server joins"), 2);
    }

    #[test]
    fn unknown_bearers_get_one_error_and_no_session() {
        let answers = exchange(&[r#"{"token":"nope"}"#.to_owned()]);
        assert_eq!(answers.len(), 1);
        let response: Response = serde_json::from_str(&answers[0]).expect("decodes");
        match response {
            Response::Error { code, .. } => assert_eq!(code, "unknown_token"),
            other => panic!("expected unknown_token, got {other:?}"),
        }
    }

    #[test]
    fn viewers_cannot_force_through_the_server() {
        let answers = exchange(&[
            r#"{"token":"view-1"}"#.to_owned(),
            request_line(&Request::ForceVar {
                var: var("%Q0"),
                value: Value::Bit(true),
            }),
            request_line(&Request::GetVars {
                vars: vec![var("%Q0")],
            }),
        ]);
        assert_eq!(answers.len(), 3);
        assert_eq!(
            serde_json::from_str::<Response>(&answers[0]).expect("decodes"),
            Response::Ok
        );
        let denied: Response = serde_json::from_str(&answers[1]).expect("decodes");
        match denied {
            Response::Error { code, .. } => assert_eq!(code, "unauthorized"),
            other => panic!("expected unauthorized, got {other:?}"),
        }
        let vars: Response = serde_json::from_str(&answers[2]).expect("decodes");
        assert_eq!(
            vars,
            Response::Vars {
                values: vec![(var("%Q0"), Some(Value::Bit(false)))]
            }
        );
    }

    #[test]
    fn bad_lines_answer_bad_request_without_closing() {
        let answers = exchange(&[
            r#"{"token":"view-1"}"#.to_owned(),
            "not json at all".to_owned(),
            request_line(&Request::GetStatus),
        ]);
        assert_eq!(answers.len(), 3);
        assert_eq!(
            serde_json::from_str::<Response>(&answers[0]).expect("decodes"),
            Response::Ok
        );
        let bad: Response = serde_json::from_str(&answers[1]).expect("decodes");
        match bad {
            Response::Error { code, .. } => assert_eq!(code, "bad_request"),
            other => panic!("expected bad_request, got {other:?}"),
        }
        let status: Response = serde_json::from_str(&answers[2]).expect("decodes");
        assert!(matches!(status, Response::Status { .. }), "{status:?}");
    }
}
