//! Wire-level tests against a minimal in-process HTTP server: what the client
//! actually sends (stability, version, no branch-era fields) and how failures
//! (409, 404, 410, pre-2.0 server) are surfaced.

use sanshain::{
    ApiType, Client, Config, Endpoint, Error, ProvideEntry, RequireEntry, RequireFormat, Stability,
    Stream,
};
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
        format: None,
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

/// A config in another folder than the working directory: its relative paths
/// resolve next to it, absolute ones stay absolute.
#[test]
fn relative_paths_resolve_against_the_config_directory() {
    let dir = test_dir("config_elsewhere/project");
    let absolute_out = test_dir("config_elsewhere/absolute-out");
    if let Err(err) = std::fs::create_dir_all(dir.join("specs")) {
        panic!("failed to create spec dir: {err}");
    }
    let spec = "openapi: 3.0.3\ninfo:\n  title: Elsewhere\n  version: 1.4.0\npaths: {}\n";
    if let Err(err) = std::fs::write(dir.join("specs/openapi.yaml"), spec) {
        panic!("failed to write spec file: {err}");
    }
    let yaml = format!(
        "serviceName: demo-service\nprovides:\n  - file: specs/openapi.yaml\nrequires:\n  - serviceName: user-service\n    version: 1.2.0\n    outputDirectory: generated/user-service\n    endpoints:\n      - method: GET\n        path: /x\n  - serviceName: order-service\n    version: 1.0.0\n    outputDirectory: {}\n    endpoints:\n      - method: GET\n        path: /y\n",
        absolute_out.display()
    );
    let config_path = dir.join("sanshain.yaml");
    if let Err(err) = std::fs::write(&config_path, yaml) {
        panic!("failed to write config: {err}");
    }
    let config = match Config::from_yaml_file(&config_path) {
        Ok(config) => config,
        Err(err) => panic!("config should load: {err}"),
    };
    assert_eq!(config.provides[0].file, dir.join("specs/openapi.yaml"));
    assert_eq!(
        config.requires[0].output_directory,
        dir.join("generated/user-service")
    );
    assert_eq!(config.requires[1].output_directory, absolute_out);

    // The provide reads the spec next to the config, and the require writes
    // next to it — neither depends on where the test runs.
    let (url, handle) = mock_server(vec![
        accepted_response(),
        http_response("200 OK", "application/yaml", "openapi: 3.0.3\n"),
    ]);
    let client = Client::new(url.as_str(), None);
    if let Err(err) = client.provide("demo-service", &config.provides[0], Stability::Snapshot) {
        panic!("provide failed: {err}");
    }
    let written = match client.require("demo-service", &config.requires[0]) {
        Ok(written) => written,
        Err(err) => panic!("require failed: {err}"),
    };
    assert_eq!(
        written,
        dir.join("generated/user-service").join("user-service.yaml")
    );
    let requests = joined_requests(handle);
    let payload: serde_json::Value = match serde_json::from_str(body_of(&requests[0])) {
        Ok(payload) => payload,
        Err(err) => panic!("provide body is not JSON: {err}"),
    };
    assert!(
        payload["openapi_yaml"]
            .as_str()
            .is_some_and(|yaml| yaml.contains("title: Elsewhere"))
    );
}

// --- catalog ---

fn catalog_ok_response(trunk: bool) -> String {
    http_response(
        "200 OK",
        "application/json",
        &format!(
            r#"{{"system":"orders-service","trunk":{trunk},"resources":["resource:default/orders-db","resource:default/orders-bucket"]}}"#
        ),
    )
}

fn catalog_file(name: &str) -> PathBuf {
    let file = test_dir(name).join("catalog-info.yaml");
    let content =
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders-service\n";
    if let Err(err) = std::fs::write(&file, content) {
        panic!("failed to write catalog file: {err}");
    }
    file
}

fn json_body(request: &str) -> serde_json::Value {
    match serde_json::from_str(body_of(request)) {
        Ok(payload) => payload,
        Err(err) => panic!("body is not JSON: {err}"),
    }
}

#[test]
fn catalog_sends_trunk_only_on_a_trunk_build_and_never_a_tag() {
    let file = catalog_file("catalog_streams");
    let cases = [
        (Stream::Trunk, true),
        (Stream::None, false),
        (Stream::Tag("Release-Maribou".to_string()), false),
    ];
    for (stream, trunk) in cases {
        let (url, handle) = mock_server(vec![catalog_ok_response(trunk)]);
        let client = Client::new(url.as_str(), None).with_stream(stream.clone());
        let recorded = match client.catalog("orders-service", &file) {
            Ok(recorded) => recorded,
            Err(err) => panic!("catalog failed for {stream}: {err}"),
        };
        assert_eq!(recorded.trunk, trunk);
        let requests = joined_requests(handle);
        assert_eq!(requests.len(), 1, "no preflight requests expected");
        assert!(
            request_line(&requests[0]).starts_with("PUT /catalog/orders-service "),
            "unexpected: {}",
            request_line(&requests[0])
        );
        let payload = json_body(&requests[0]);
        assert!(
            payload["catalog_info_yaml"]
                .as_str()
                .is_some_and(|yaml| yaml.contains("name: orders-service"))
        );
        assert_eq!(payload.get("trunk").is_some(), trunk, "{stream}: {payload}");
        if trunk {
            assert_eq!(payload["trunk"], true);
        }
        assert!(payload.get("tag").is_none(), "{stream}: {payload}");
    }
}

#[test]
fn catalog_returns_the_recorded_resources() {
    let file = catalog_file("catalog_ok");
    let (url, handle) = mock_server(vec![catalog_ok_response(true)]);
    let client = Client::new(url.as_str(), Some("tok".to_string())).with_stream(Stream::Trunk);
    let recorded = match client.catalog("orders-service", &file) {
        Ok(recorded) => recorded,
        Err(err) => panic!("catalog failed: {err}"),
    };
    assert_eq!(recorded.system, "orders-service");
    assert_eq!(
        recorded.resources,
        vec![
            "resource:default/orders-db",
            "resource:default/orders-bucket"
        ]
    );
    let requests = joined_requests(handle);
    assert!(requests[0].contains("Authorization: Bearer tok"));
}

#[test]
fn catalog_url_encodes_the_service_name() {
    let file = catalog_file("catalog_encoded");
    let (url, handle) = mock_server(vec![catalog_ok_response(false)]);
    let client = Client::new(url.as_str(), None);
    if let Err(err) = client.catalog("my service/x", &file) {
        panic!("catalog failed: {err}");
    }
    let requests = joined_requests(handle);
    assert!(
        request_line(&requests[0]).starts_with("PUT /catalog/my%20service%2Fx "),
        "unexpected: {}",
        request_line(&requests[0])
    );
}

#[test]
fn a_catalog_400_from_an_old_pre_release_service_is_a_plain_failure() {
    let file = catalog_file("catalog_400");
    let (url, handle) = mock_server(vec![
        http_response(
            "400 Bad Request",
            "application/json",
            r#"{"error":"no Component named 'orders-service'"}"#,
        ),
        version_response("2.4.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.catalog("orders-service", &file) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the catalog push to be rejected"),
    };
    assert_eq!(
        message,
        "catalog failed (400): no Component named 'orders-service'"
    );
    drop(joined_requests(handle));
}

#[test]
fn catalog_404_reports_the_service_is_older_than_2_4() {
    let file = catalog_file("catalog_404");
    let (url, handle) = mock_server(vec![
        http_response("404 Not Found", "text/plain", ""),
        version_response("2.3.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.catalog("orders-service", &file) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the catalog push to fail"),
    };
    assert_eq!(
        message,
        "sanshain catalog needs Sanshain Service 2.4.0 or later (server is 2.3.0)"
    );
    let requests = joined_requests(handle);
    assert!(request_line(&requests[1]).starts_with("GET /version "));

    let (url, handle) = mock_server(vec![
        http_response("404 Not Found", "text/plain", ""),
        http_response("404 Not Found", "text/plain", ""),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.catalog("orders-service", &file) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the catalog push to fail"),
    };
    assert_eq!(
        message,
        "sanshain catalog needs Sanshain Service 2.4.0 or later (server version unknown)"
    );
    drop(joined_requests(handle));
}

#[test]
fn catalog_without_the_file_names_the_resolved_path() {
    let missing = test_dir("catalog_missing").join("nope.yaml");
    let client = Client::new("http://127.0.0.1:9", None);
    let message = match client.catalog("orders-service", &missing) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the catalog push to fail"),
    };
    assert!(
        message.contains(&missing.display().to_string()),
        "unexpected: {message}"
    );
}

#[test]
fn a_relative_catalog_info_resolves_next_to_the_config() {
    let dir = test_dir("catalog_elsewhere/project");
    let file = catalog_file("catalog_elsewhere/project");
    let config_path = dir.join("sanshain.yaml");
    if let Err(err) = std::fs::write(
        &config_path,
        "serviceName: orders-service\ncatalogInfo: catalog-info.yaml\n",
    ) {
        panic!("failed to write config: {err}");
    }
    let config = match Config::from_yaml_file(&config_path) {
        Ok(config) => config,
        Err(err) => panic!("config should load: {err}"),
    };
    assert_eq!(config.catalog_info.as_deref(), Some(file.as_path()));

    let (url, handle) = mock_server(vec![catalog_ok_response(false)]);
    let client = Client::new(url.as_str(), None);
    let Some(catalog_info) = config.catalog_info.as_deref() else {
        panic!("catalogInfo should be set");
    };
    if let Err(err) = client.catalog("orders-service", catalog_info) {
        panic!("catalog failed: {err}");
    }
    let requests = joined_requests(handle);
    assert!(
        json_body(&requests[0])["catalog_info_yaml"]
            .as_str()
            .is_some_and(|yaml| yaml.contains("kind: Component"))
    );
}

// --- format: openapi ---

fn asyncapi_as_openapi(name: &str, endpoints: usize) -> RequireEntry {
    let mut entry = require_entry(
        name,
        (0..endpoints)
            .map(|i| Endpoint {
                method: "SUB".to_string(),
                path: format!("user/event{i}"),
            })
            .collect(),
    );
    entry.api_type = ApiType::AsyncApi;
    entry.format = Some(RequireFormat::OpenApi);
    entry
}

#[test]
fn format_openapi_is_sent_on_the_single_and_the_bundle_call() {
    for (name, endpoints, expected) in [
        ("format_single", 1, "GET /require/asyncapi?"),
        ("format_bundle", 2, "POST /require-bundle?"),
    ] {
        let (url, handle) = mock_server(vec![http_response(
            "200 OK",
            "application/yaml",
            "openapi: 3.0.3\ninfo:\n  title: events\n",
        )]);
        let client = Client::new(url.as_str(), None);
        let entry = asyncapi_as_openapi(name, endpoints);
        let written = match client.require("gateway-service", &entry) {
            Ok(written) => written,
            Err(err) => panic!("{name}: require failed: {err}"),
        };
        // Same file name as without a format.
        assert_eq!(written, entry.output_directory.join("user-service.yaml"));
        let requests = joined_requests(handle);
        let line = request_line(&requests[0]);
        assert!(line.starts_with(expected), "{name}: {line}");
        assert!(line.contains("format=openapi"), "{name}: {line}");
    }
}

#[test]
fn without_a_format_no_format_parameter_is_sent() {
    for (name, endpoints) in [("no_format_single", 1), ("no_format_bundle", 2)] {
        let (url, handle) = mock_server(vec![http_response(
            "200 OK",
            "application/yaml",
            "asyncapi: 2.6.0\n",
        )]);
        let client = Client::new(url.as_str(), None);
        let mut entry = asyncapi_as_openapi(name, endpoints);
        entry.format = None;
        if let Err(err) = client.require("gateway-service", &entry) {
            panic!("{name}: require failed: {err}");
        }
        let requests = joined_requests(handle);
        assert!(!requests[0].contains("format"), "{name}: {}", requests[0]);
    }
}

#[test]
fn an_asyncapi_answer_to_format_openapi_needs_2_4_and_writes_nothing() {
    let entry = asyncapi_as_openapi("format_old_server", 1);
    let (url, handle) = mock_server(vec![
        http_response("200 OK", "application/yaml", "asyncapi: 2.6.0\n"),
        version_response("2.3.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.require("gateway-service", &entry) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the require to fail"),
    };
    assert_eq!(
        message,
        "format: openapi needs Sanshain Service 2.4.0 or later (server is 2.3.0)"
    );
    assert!(!entry.output_directory.join("user-service.yaml").exists());
    let requests = joined_requests(handle);
    assert!(request_line(&requests[1]).starts_with("GET /version "));

    let (url, handle) = mock_server(vec![
        http_response("200 OK", "application/yaml", "asyncapi: 2.6.0\n"),
        http_response("404 Not Found", "text/plain", ""),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.require("gateway-service", &entry) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the require to fail"),
    };
    assert_eq!(
        message,
        "format: openapi needs Sanshain Service 2.4.0 or later (server version unknown)"
    );
    drop(joined_requests(handle));
}

fn unknown_format_400() -> String {
    http_response(
        "400 Bad Request",
        "application/json",
        r#"{"error":"Failed to deserialize query string: unknown field `format`"}"#,
    )
}

#[test]
fn a_pre_2_4_service_rejecting_format_on_the_single_route_needs_2_4() {
    let entry = asyncapi_as_openapi("format_400_old", 1);
    let (url, handle) = mock_server(vec![unknown_format_400(), version_response("2.3.0")]);
    let client = Client::new(url.as_str(), None);
    let message = match client.require("gateway-service", &entry) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the require to fail"),
    };
    assert_eq!(
        message,
        "format: openapi needs Sanshain Service 2.4.0 or later (server is 2.3.0)"
    );
    assert!(!entry.output_directory.join("user-service.yaml").exists());
    drop(joined_requests(handle));

    let (url, handle) = mock_server(vec![
        unknown_format_400(),
        http_response("404 Not Found", "text/plain", ""),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.require("gateway-service", &entry) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the require to fail"),
    };
    assert_eq!(
        message,
        "format: openapi needs Sanshain Service 2.4.0 or later (server version unknown)"
    );
    drop(joined_requests(handle));
}

#[test]
fn a_2_4_service_s_own_format_400_passes_through() {
    let entry = asyncapi_as_openapi("format_400_new", 1);
    let (url, handle) = mock_server(vec![
        http_response(
            "400 Bad Request",
            "application/json",
            r#"{"error":"cannot convert to format openapi: message 'UserSignedUp' uses an Avro payload"}"#,
        ),
        version_response("2.4.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    let message = match client.require("gateway-service", &entry) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected the require to fail"),
    };
    assert!(
        message.contains(
            "cannot convert to format openapi: message 'UserSignedUp' uses an Avro payload"
        ),
        "unexpected: {message}"
    );
    assert!(
        !message.contains("needs Sanshain Service 2.4.0"),
        "unexpected: {message}"
    );
    drop(joined_requests(handle));
}

// --- Sanshain-Warning ---

/// `response` with one `Sanshain-Warning` header line per warning.
fn with_warnings(response: String, warnings: &[&str]) -> String {
    let headers: String = warnings
        .iter()
        .map(|w| format!("Sanshain-Warning: {w}\r\n"))
        .collect();
    response.replacen("\r\n", &format!("\r\n{headers}"), 1)
}

const NO_BRANCH: &str =
    "no sanshain-branch 'Release-X' - this build was not recorded in a release graph";

#[test]
fn every_call_hands_over_its_warnings_once_in_order() {
    let snippet = http_response("200 OK", "application/yaml", "openapi: 3.0.3\n");
    let retired = http_response(
        "200 OK",
        "application/json",
        r#"{"trunk_pins_closed":0,"contracts_released":0}"#,
    );
    let (url, handle) = mock_server(vec![
        with_warnings(accepted_response(), &[NO_BRANCH]),
        with_warnings(retired, &["retired, a, b"]),
        with_warnings(snippet.clone(), &[NO_BRANCH, "second, with a comma"]),
        with_warnings(snippet, &["bundle"]),
        snippet_without_warning(),
    ]);
    let client = Client::new(url.as_str(), None).with_stream(Stream::Tag("Release-X".to_string()));
    assert!(client.take_warnings().is_empty());

    let entry = spec_entry("warn_provide", ApiType::OpenApi);
    if let Err(err) = client.provide("svc", &entry, Stability::Snapshot) {
        panic!("provide failed: {err}");
    }
    assert_eq!(client.take_warnings(), vec![NO_BRANCH]);
    assert!(client.take_warnings().is_empty(), "taken means gone");

    if let Err(err) = client.retire("svc", ApiType::OpenApi) {
        panic!("retire failed: {err}");
    }
    assert_eq!(client.take_warnings(), vec!["retired, a, b"]);

    let one = require_entry(
        "warn_require",
        vec![Endpoint {
            method: "GET".to_string(),
            path: "/x".to_string(),
        }],
    );
    if let Err(err) = client.require("svc", &one) {
        panic!("require failed: {err}");
    }
    assert_eq!(
        client.take_warnings(),
        vec![NO_BRANCH, "second, with a comma"]
    );

    let two = require_entry(
        "warn_bundle",
        vec![
            Endpoint {
                method: "GET".to_string(),
                path: "/x".to_string(),
            },
            Endpoint {
                method: "GET".to_string(),
                path: "/y".to_string(),
            },
        ],
    );
    if let Err(err) = client.require("svc", &two) {
        panic!("bundle failed: {err}");
    }
    assert_eq!(client.take_warnings(), vec!["bundle"]);

    if let Err(err) = client.require("svc", &one) {
        panic!("require failed: {err}");
    }
    assert!(client.take_warnings().is_empty());
    drop(joined_requests(handle));
}

fn snippet_without_warning() -> String {
    http_response("200 OK", "application/yaml", "openapi: 3.0.3\n")
}

#[test]
fn an_unread_catalog_is_recorded_false_with_the_reason_as_a_warning() {
    let file = catalog_file("catalog_unread");
    let reason = "no Component named 'orders-service' in catalog-info.yaml";
    let (url, handle) = mock_server(vec![
        with_warnings(
            http_response(
                "200 OK",
                "application/json",
                r#"{"system":"orders-service","trunk":false,"resources":[],"recorded":false}"#,
            ),
            &[reason],
        ),
        catalog_ok_response(false),
    ]);
    let client = Client::new(url.as_str(), None);
    let unread = match client.catalog("orders-service", &file) {
        Ok(response) => response,
        Err(err) => panic!("an unread catalog is no failure: {err}"),
    };
    assert!(!unread.recorded);
    assert!(unread.resources.is_empty());
    assert_eq!(client.take_warnings(), vec![reason]);

    // A service answering without the field recorded what it answered.
    match client.catalog("orders-service", &file) {
        Ok(response) => assert!(response.recorded),
        Err(err) => panic!("catalog failed: {err}"),
    }
    drop(joined_requests(handle));
}

#[test]
fn a_warning_on_a_failure_is_kept_too() {
    let entry = spec_entry("warn_failure", ApiType::OpenApi);
    let (url, handle) = mock_server(vec![
        with_warnings(
            http_response(
                "409 Conflict",
                "application/json",
                r#"{"error":"GA 1.4.0 already exists"}"#,
            ),
            &[NO_BRANCH],
        ),
        version_response("2.4.0"),
    ]);
    let client = Client::new(url.as_str(), None);
    assert!(client.provide("svc", &entry, Stability::Ga).is_err());
    assert_eq!(client.take_warnings(), vec![NO_BRANCH]);
    drop(joined_requests(handle));
}
