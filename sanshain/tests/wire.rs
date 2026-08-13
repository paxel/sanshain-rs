//! Wire-level tests against a minimal in-process HTTP server: what the client
//! actually sends (stability, version, no branch-era fields) and how failures
//! (409, 404, 410, pre-2.0 server) are surfaced.

use sanshain::{ApiType, Client, Endpoint, Error, ProvideEntry, RequireEntry, Stability};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread::JoinHandle;

/// Serves the canned responses to sequential connections (one request per
/// connection) and returns the base URL plus a handle yielding the raw requests.
fn mock_server(responses: Vec<String>) -> (String, JoinHandle<Vec<String>>) {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) => panic!("failed to bind mock server: {err}"),
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(err) => panic!("failed to read mock server address: {err}"),
    };
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(err) => panic!("mock server accept failed: {err}"),
            };
            requests.push(read_request(&mut stream));
            if let Err(err) = stream.write_all(response.as_bytes()) {
                panic!("mock server write failed: {err}");
            }
        }
        requests
    });
    (format!("http://{addr}"), handle)
}

/// Reads one HTTP request (headers plus a Content-Length body) as a string.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            Err(err) => panic!("mock server read failed: {err}"),
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(header_end) = find(&buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            let total = header_end + 4 + content_length;
            while buf.len() < total {
                let n = match stream.read(&mut chunk) {
                    Ok(n) => n,
                    Err(err) => panic!("mock server read failed: {err}"),
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            return String::from_utf8_lossy(&buf[..buf.len().min(total)]).to_string();
        }
        if n == 0 {
            return String::from_utf8_lossy(&buf).to_string();
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn http_response(status_line: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn body_of(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn request_line(request: &str) -> &str {
    request.lines().next().unwrap_or("")
}

fn accepted_response() -> String {
    http_response(
        "202 Accepted",
        "application/json",
        r#"{"version":"1.4.0","stability":"snapshot","content_hash":"sha256:abc","changes":{"inserts":3,"updates":0,"deletes":0}}"#,
    )
}

fn version_response(version: &str) -> String {
    http_response(
        "200 OK",
        "application/json",
        &format!(r#"{{"version":"{version}","instance_id":"test"}}"#),
    )
}

fn test_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        panic!("failed to create test dir: {err}");
    }
    dir
}

fn spec_entry(name: &str, api_type: ApiType) -> ProvideEntry {
    let file = test_dir(name).join("openapi.yaml");
    let content = "openapi: 3.0.3\ninfo:\n  title: Demo\n  version: 1.4.0\npaths: {}\n";
    if let Err(err) = std::fs::write(&file, content) {
        panic!("failed to write spec file: {err}");
    }
    ProvideEntry {
        file,
        api_type,
        retired: false,
    }
}

fn require_entry(name: &str, endpoints: Vec<Endpoint>) -> RequireEntry {
    RequireEntry {
        service_name: "user-service".to_string(),
        api_type: ApiType::OpenApi,
        version: "1.2.0".to_string(),
        output_directory: test_dir(name).join("out"),
        endpoints,
    }
}

fn joined_requests(handle: JoinHandle<Vec<String>>) -> Vec<String> {
    match handle.join() {
        Ok(requests) => requests,
        Err(_) => panic!("mock server thread panicked"),
    }
}

#[test]
fn provide_sends_stability_and_no_branch_era_fields() {
    let (url, handle) = mock_server(vec![accepted_response()]);
    let client = Client::new(url.as_str(), None);
    let entry = spec_entry("provide_snapshot", ApiType::OpenApi);
    let outcome = match client.provide("demo-service", &entry, Stability::Snapshot) {
        Ok(outcome) => outcome,
        Err(err) => panic!("provide failed: {err}"),
    };
    assert_eq!(outcome.version, "1.4.0");
    assert_eq!(outcome.stability, "snapshot");
    assert_eq!(outcome.changes.inserts, 3);

    let requests = joined_requests(handle);
    assert_eq!(requests.len(), 1, "no preflight requests expected");
    assert!(request_line(&requests[0]).starts_with("POST /provide "));
    let payload: serde_json::Value = match serde_json::from_str(body_of(&requests[0])) {
        Ok(payload) => payload,
        Err(err) => panic!("provide body is not JSON: {err}"),
    };
    assert_eq!(payload["producername"], "demo-service");
    assert_eq!(payload["stability"], "snapshot");
    assert!(
        payload["openapi_yaml"]
            .as_str()
            .is_some_and(|yaml| yaml.contains("version: 1.4.0"))
    );
    for gone in ["branch", "base_version", "force", "author"] {
        assert!(
            payload.get(gone).is_none(),
            "payload must not carry '{gone}'"
        );
    }
}

#[test]
fn provide_sends_ga_when_switched() {
    let (url, handle) = mock_server(vec![accepted_response()]);
    let client = Client::new(url.as_str(), None);
    let entry = spec_entry("provide_ga", ApiType::OpenApi);
    if let Err(err) = client.provide("demo-service", &entry, Stability::Ga) {
        panic!("provide failed: {err}");
    }
    let requests = joined_requests(handle);
    let payload: serde_json::Value = match serde_json::from_str(body_of(&requests[0])) {
        Ok(payload) => payload,
        Err(err) => panic!("provide body is not JSON: {err}"),
    };
    assert_eq!(payload["stability"], "ga");
}

#[test]
fn require_sends_version_and_no_branch_era_params() {
    let (url, handle) = mock_server(vec![http_response(
        "200 OK",
        "application/yaml",
        "openapi: 3.0.3\n",
    )]);
    let client = Client::new(url.as_str(), None);
    let entry = require_entry(
        "require_single",
        vec![Endpoint {
            method: "GET".to_string(),
            path: "/api/v1/users/{id}".to_string(),
        }],
    );
    let out = match client.require("gateway-service", &entry) {
        Ok(out) => out,
        Err(err) => panic!("require failed: {err}"),
    };
    let written = match std::fs::read_to_string(&out) {
        Ok(written) => written,
        Err(err) => panic!("output file missing: {err}"),
    };
    assert_eq!(written, "openapi: 3.0.3\n");

    let requests = joined_requests(handle);
    assert_eq!(requests.len(), 1);
    let line = request_line(&requests[0]).to_string();
    assert!(line.starts_with("GET /require?"), "unexpected: {line}");
    assert!(line.contains("version=1.2.0"), "version missing: {line}");
    assert!(
        line.contains("consumername=gateway-service"),
        "consumername missing: {line}"
    );
    assert!(
        line.contains("producername=user-service"),
        "producername missing: {line}"
    );
    for gone in [
        "branch",
        "timeout",
        "pull_from_branch",
        "source_protected_branch",
    ] {
        assert!(
            !line.contains(gone),
            "request must not carry '{gone}': {line}"
        );
    }
}

#[test]
fn require_bundle_sends_version_and_endpoints() {
    let (url, handle) = mock_server(vec![http_response(
        "200 OK",
        "application/yaml",
        "openapi: 3.0.3\n",
    )]);
    let client = Client::new(url.as_str(), None);
    let entry = require_entry(
        "require_bundle",
        vec![
            Endpoint {
                method: "GET".to_string(),
                path: "/api/v1/users/{id}".to_string(),
            },
            Endpoint {
                method: "POST".to_string(),
                path: "/api/v1/users".to_string(),
            },
        ],
    );
    if let Err(err) = client.require("gateway-service", &entry) {
        panic!("require failed: {err}");
    }
    let requests = joined_requests(handle);
    assert!(request_line(&requests[0]).starts_with("POST /require-bundle "));
    let payload: serde_json::Value = match serde_json::from_str(body_of(&requests[0])) {
        Ok(payload) => payload,
        Err(err) => panic!("bundle body is not JSON: {err}"),
    };
    assert_eq!(payload["version"], "1.2.0");
    assert_eq!(payload["api_type"], "openapi");
    assert_eq!(payload["endpoints"].as_array().map(Vec::len), Some(2));
}

#[test]
fn conflict_surfaces_server_message_and_proposed_version() {
    let (url, handle) = mock_server(vec![
        http_response(
            "409 Conflict",
            "application/json",
            r#"{"error":"GA 1.4.0 already exists with different content","proposed_version":"1.5.0"}"#,
        ),
        version_response("2.0.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let entry = spec_entry("provide_conflict", ApiType::OpenApi);
    let err = match client.provide("demo-service", &entry, Stability::Ga) {
        Err(err) => err,
        Ok(_) => panic!("expected the provide to be rejected"),
    };
    let message = err.to_string();
    assert!(
        message.contains("GA 1.4.0 already exists with different content"),
        "server message missing: {message}"
    );
    assert!(
        message.contains("Publish as 1.5.0 — update info.version in"),
        "proposed_version missing: {message}"
    );

    let requests = joined_requests(handle);
    assert_eq!(requests.len(), 2);
    assert!(
        request_line(&requests[1]).starts_with("GET /version "),
        "expected the lazy version check: {}",
        request_line(&requests[1])
    );
}

#[test]
fn pre_2_server_is_diagnosed_lazily() {
    let (url, handle) = mock_server(vec![
        http_response("422 Unprocessable Entity", "text/plain", "unknown field"),
        version_response("1.7.3"),
    ]);
    let client = Client::new(url.as_str(), None);
    let entry = spec_entry("provide_old_server", ApiType::OpenApi);
    let err = match client.provide("demo-service", &entry, Stability::Snapshot) {
        Err(err) => err,
        Ok(_) => panic!("expected the provide to fail"),
    };
    match &err {
        Error::WrongServer(message) => assert_eq!(
            message,
            &format!(
                "Sanshain server at {url} is 1.7.3; this client requires Sanshain 2.x — upgrade the server."
            )
        ),
        other => panic!("expected a wrong-server diagnosis, got {other:?}"),
    }
    let requests = joined_requests(handle);
    assert_eq!(
        requests.len(),
        2,
        "exactly one lazy /version check expected"
    );
}

#[test]
fn require_unknown_pin_fails_with_fix_the_pin_message() {
    let (url, handle) = mock_server(vec![
        http_response(
            "404 Not Found",
            "application/json",
            r#"{"error":"producer 'user-service' has no version 1.2.0"}"#,
        ),
        version_response("2.0.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let entry = require_entry(
        "require_unknown",
        vec![Endpoint {
            method: "GET".to_string(),
            path: "/x".to_string(),
        }],
    );
    let err = match client.require("gateway-service", &entry) {
        Err(err) => err,
        Ok(_) => panic!("expected the require to fail"),
    };
    let message = err.to_string();
    assert!(message.contains("404, unknown"), "unexpected: {message}");
    assert!(
        message.contains("configuration error")
            && message.contains("/producers/user-service/versions"),
        "fix-the-pin hint missing: {message}"
    );
    drop(joined_requests(handle));
}

#[test]
fn require_absent_endpoint_fails_with_deliberate_absence_message() {
    let (url, handle) = mock_server(vec![
        http_response(
            "410 Gone",
            "application/json",
            r#"{"error":"version 1.2.0 does not provide GET /x"}"#,
        ),
        version_response("2.0.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let entry = require_entry(
        "require_absent",
        vec![Endpoint {
            method: "GET".to_string(),
            path: "/x".to_string(),
        }],
    );
    let err = match client.require("gateway-service", &entry) {
        Err(err) => err,
        Ok(_) => panic!("expected the require to fail"),
    };
    let message = err.to_string();
    assert!(message.contains("410, absent"), "unexpected: {message}");
    assert!(message.contains("deliberately"), "unexpected: {message}");
    drop(joined_requests(handle));
}
