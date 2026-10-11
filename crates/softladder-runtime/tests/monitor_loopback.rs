//! End to end: the JSON monitor server driving a real runtime over loopback.
//!
//! A lamp rung (`%I0` → `%Q0`) plus a jump to nowhere (which raises `SL-E004`
//! every scan) run behind [`serve`](softladder_monitor::serve); the client
//! authenticates, forces the lamp, reads it back, lists alarms, acknowledges
//! the alarm and confirms the force still holds after more scans.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section, Value, VarRef};
use softladder_monitor::{serve, Request, Response, Role, ServerConfig, Token};

fn var(text: &str) -> VarRef {
    text.parse::<VarRef>().expect("test variable parses")
}

fn project() -> Project {
    let mut project = Project::new("monitor loopback");
    let mut section = Section::new(1, "Main");
    section.rungs.push(1);
    section.rungs.push(2);
    project.sections.push(section);
    project.rungs.push(Rung {
        elements: vec![
            PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
            PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
        ],
        ..Rung::new(1)
    });
    project.rungs.push(Rung {
        elements: vec![PlacedElement::with_params(ElementKind::CoilJump, 0, 0, &[])],
        ..Rung::new(2)
    });
    project
}

fn request_line(request: &Request) -> String {
    serde_json::to_string(request).expect("serializes")
}

#[test]
fn monitor_server_drives_a_real_runtime() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let addr = listener.local_addr().expect("listener has an address");
    let mut config = ServerConfig::new(HashMap::from([(
        "eng-1".to_owned(),
        Token::new("eng-1".to_owned(), Role::Engineer),
    )]));
    config.read_timeout = Duration::from_secs(5);
    config.max_requests = 64;

    let server = std::thread::spawn(move || {
        let mut target = softladder_runtime::MonitoredRuntime::new(project());
        target.start();
        target.tick(0);
        target.tick(10);
        serve(listener, &mut target, &config, 1).expect("server serves");
        // Scans after the client left: the force must still hold, and the
        // still-present fault re-raises the alarm, clearing the ack.
        target.tick(20);
        target.tick(30);
        let lamp = softladder_monitor::MonitorTarget::read(&target, &var("%Q0"));
        let realarmed = target.journal().active().iter().all(|entry| !entry.acked());
        (lamp, realarmed, target.forces().len())
    });

    let stream = TcpStream::connect(addr).expect("client connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout sets");
    let mut reader = BufReader::new(stream.try_clone().expect("clone connects"));
    let mut writer = stream;
    let mut ask = |line: String| {
        writer
            .write_all(format!("{line}\n").as_bytes())
            .expect("client writes");
        let mut answer = String::new();
        reader.read_line(&mut answer).expect("client reads");
        serde_json::from_str::<Response>(answer.trim_end()).expect("decodes")
    };

    assert_eq!(ask(r#"{"token":"eng-1"}"#.to_owned()), Response::Ok);
    match ask(request_line(&Request::GetStatus)) {
        Response::Status { state, cycles, .. } => {
            assert_eq!(state, "run");
            assert_eq!(cycles, 2);
        }
        other => panic!("expected status, got {other:?}"),
    }
    assert_eq!(
        ask(request_line(&Request::ForceVar {
            var: var("%Q0"),
            value: Value::Bit(true),
        })),
        Response::Ok
    );
    assert_eq!(
        ask(request_line(&Request::GetVars {
            vars: vec![var("%Q0")],
        })),
        Response::Vars {
            values: vec![(var("%Q0"), Some(Value::Bit(true)))]
        }
    );
    let id = match ask(request_line(&Request::GetAlarms)) {
        Response::Alarms { alarms } => {
            assert_eq!(alarms.len(), 1);
            assert_eq!(alarms[0].code, "SL-E004");
            assert!(!alarms[0].acked);
            0
        }
        other => panic!("expected alarms, got {other:?}"),
    };
    assert_eq!(ask(request_line(&Request::AckAlarm { id })), Response::Ok);
    match ask(request_line(&Request::GetAlarms)) {
        Response::Alarms { alarms } => {
            assert_eq!(alarms.len(), 1);
            assert!(alarms[0].acked);
        }
        other => panic!("expected alarms, got {other:?}"),
    }
    drop(reader);
    drop(writer);

    let (lamp, realarmed, forces) = server.join().expect("server joins");
    assert_eq!(lamp, Some(Value::Bit(true)));
    assert!(realarmed);
    assert_eq!(forces, 1);
}
