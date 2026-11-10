//! `setup_protocol` and `poll_protocol` against a stand-in for the `manivelle` tool.
//!
//! The protocol is read from the process environment, so this binary holds one test.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use manivelle::protocol::{
    Command, FromProgram, PACKAGES_ENV, PROTOCOL_ENV, Reply, Request, TOKEN_ENV,
};
use manivelle::{Engine, EngineOptions};

fn read(reader: &mut impl BufRead) -> FromProgram {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn the_program_follows_the_tool() {
    let packages = tempfile::tempdir().unwrap();
    let package_dir = packages.path().join("hello");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("manivelle.kdl"),
        "package \"hello\" {\n version \"0.3.0\"\n entry \"main.ts\"\n permissions {\n  world.read\n }\n}\n",
    )
    .unwrap();
    fs::write(package_dir.join("main.ts"), "export const v = 1;").unwrap();

    let tool = TcpListener::bind("127.0.0.1:0").unwrap();
    // SAFETY: this binary has a single test and spawns no thread before these calls.
    unsafe {
        std::env::set_var(PROTOCOL_ENV, tool.local_addr().unwrap().to_string());
        std::env::set_var(TOKEN_ENV, "secret");
        std::env::set_var(PACKAGES_ENV, packages.path());
    }

    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    let setup = engine.setup_protocol().unwrap();
    assert!(setup.attached && !setup.exported);
    assert_eq!(setup.packages.loaded[0].name, "hello");

    let (stream, _) = tool.accept().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    let FromProgram::Hello { token, pid, .. } = read(&mut reader) else {
        panic!("the program starts with hello");
    };
    assert_eq!((token.as_str(), pid), ("secret", std::process::id()));
    let FromProgram::Packages { loaded, failed } = read(&mut reader) else {
        panic!("then reports its packages");
    };
    assert_eq!(
        (loaded[0].version.as_str(), loaded[0].permissions.as_slice()),
        ("0.3.0", ["world.read".to_owned()].as_slice())
    );
    assert!(failed.is_empty());

    assert_eq!(engine.poll_protocol(), 0, "nothing was asked yet");
    for (id, request) in [
        (1, Request::ListPackages),
        (2, Request::Reload),
        (3, Request::ListContracts),
    ] {
        let line = serde_json::to_string(&Command { id, request }).unwrap();
        writeln!(writer, "{line}").unwrap();
    }
    let started = Instant::now();
    let mut handled = 0;
    while handled < 3 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "requests never reached poll_protocol"
        );
        handled += engine.poll_protocol();
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut replies = (0..3).map(|_| match read(&mut reader) {
        FromProgram::Reply { id, reply } => (id, reply),
        other => panic!("expected a reply, got {other:?}"),
    });
    let Some((1, Reply::Packages { packages })) = replies.next() else {
        panic!("list_packages")
    };
    assert_eq!(packages[0].name, "hello");
    let Some((2, Reply::Reloaded { reloaded, failed })) = replies.next() else {
        panic!("reload")
    };
    assert!(reloaded.is_empty() && failed.is_empty());
    let Some((3, Reply::Contracts { names })) = replies.next() else {
        panic!("contracts")
    };
    assert!(names.is_empty());
}
