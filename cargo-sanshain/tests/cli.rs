//! End-to-end tests of the `cargo sanshain` binary: a project folder with a
//! `sanshain.yaml`, the binary run inside it, and a minimal in-process HTTP
//! server standing in for Sanshain. They check what users see — exit status,
//! stdout and stderr — for strict, best-effort and the other run modes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
        if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
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

fn http_response(status_line: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn version_response(version: &str) -> String {
    http_response(
        "200 OK",
        &format!(r#"{{"version":"{version}","instance_id":"test"}}"#),
    )
}

fn conflict_response() -> String {
    http_response(
        "409 Conflict",
        r#"{"error":"GA 1.4.0 already exists with different content","proposed_version":"1.5.0"}"#,
    )
}

fn joined_requests(handle: JoinHandle<Vec<String>>) -> Vec<String> {
    match handle.join() {
        Ok(requests) => requests,
        Err(_) => panic!("mock server thread panicked"),
    }
}

/// A fresh, empty project folder for one test.
fn project(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(name);
    if dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            panic!("failed to clear {}: {err}", dir.display());
        }
    }
    if let Err(err) = std::fs::create_dir_all(&dir) {
        panic!("failed to create {}: {err}", dir.display());
    }
    dir
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            panic!("failed to create {}: {err}", parent.display());
        }
    }
    if let Err(err) = std::fs::write(path, content) {
        panic!("failed to write {}: {err}", path.display());
    }
}

fn write_spec(dir: &Path) {
    write(
        &dir.join("specs/openapi.yaml"),
        "openapi: 3.0.3\ninfo:\n  title: Demo\n  version: 1.4.0\npaths: {}\n",
    );
}

/// Runs `cargo sanshain <args>` in `dir` with only the given `SANSHAIN_*`
/// variables set — the developer's own environment must not leak in.
fn run(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-sanshain"));
    command.arg("sanshain").args(args).current_dir(dir);
    // The runner's own variables feed the origin of a provide: a test run
    // inside GitHub Actions must not send its build URL either.
    const RUNNER: [&str; 9] = [
        "GITHUB_SERVER_URL",
        "GITHUB_REPOSITORY",
        "GITHUB_RUN_ID",
        "GITHUB_REF",
        "CI_JOB_URL",
        "CI_MERGE_REQUEST_PROJECT_URL",
        "CI_MERGE_REQUEST_IID",
        "BUILD_URL",
        "CHANGE_URL",
    ];
    for (key, _) in std::env::vars() {
        if key.starts_with("SANSHAIN_") || RUNNER.contains(&key.as_str()) {
            command.env_remove(key);
        }
    }
    command.envs(envs.iter().copied());
    match command.output() {
        Ok(output) => output,
        Err(err) => panic!("failed to run cargo-sanshain: {err}"),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure\nstdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

// --- strict ---

#[test]
fn missing_service_name_warns_and_skips() {
    let dir = project("no_service_name");
    write(
        &dir.join("sanshain.yaml"),
        "provides:\n  - file: specs/openapi.yaml\n",
    );
    for command in ["provide", "require"] {
        let output = run(&dir, &[command], &[]);
        assert_success(&output);
        assert!(
            stderr(&output).contains(&format!(
                "No serviceName in sanshain.yaml - skipping {command}."
            )),
            "unexpected stderr: {}",
            stderr(&output)
        );
    }
}

#[test]
fn missing_service_name_fails_under_strict_from_yaml_or_environment() {
    let dir = project("no_service_name_strict");
    write(
        &dir.join("sanshain.yaml"),
        "strict: true\nprovides:\n  - file: specs/openapi.yaml\n",
    );
    let output = run(&dir, &["provide"], &[]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("strict: sanshain.yaml has no serviceName"),
        "unexpected stderr: {}",
        stderr(&output)
    );

    write(
        &dir.join("sanshain.yaml"),
        "provides:\n  - file: specs/openapi.yaml\n",
    );
    assert_failure(&run(&dir, &["require"], &[("SANSHAIN_STRICT", "1")]));
    // A set environment variable decides either way.
    write(
        &dir.join("sanshain.yaml"),
        "strict: true\nprovides:\n  - file: specs/openapi.yaml\n",
    );
    assert_success(&run(&dir, &["provide"], &[("SANSHAIN_STRICT", "false")]));
}

#[test]
fn nothing_to_provide_or_require_warns_and_skips_unless_strict() {
    let dir = project("nothing_configured");
    write(&dir.join("sanshain.yaml"), "serviceName: demo-service\n");

    let provide = run(&dir, &["provide"], &[]);
    assert_success(&provide);
    assert!(
        stderr(&provide)
            .contains("No provide/provides entries in sanshain.yaml - skipping provide."),
        "unexpected stderr: {}",
        stderr(&provide)
    );
    let require = run(&dir, &["require"], &[]);
    assert_success(&require);
    assert!(
        stderr(&require).contains("No requires entries in sanshain.yaml - skipping require."),
        "unexpected stderr: {}",
        stderr(&require)
    );

    let strict = [("SANSHAIN_STRICT", " TRUE ")];
    let provide = run(&dir, &["provide"], &strict);
    assert_failure(&provide);
    assert!(stderr(&provide).contains("strict: sanshain.yaml has no provide/provides entries"));
    let require = run(&dir, &["require"], &strict);
    assert_failure(&require);
    assert!(stderr(&require).contains("strict: sanshain.yaml has no requires entries"));
}

// --- best effort ---

/// Test name, arguments, environment, extra `sanshain.yaml` lines.
type BestEffortCase = (
    &'static str,
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
    &'static str,
);

#[test]
fn a_failed_provide_fails_the_build_by_default() {
    let dir = project("provide_fails");
    write_spec(&dir);
    let (url, handle) = mock_server(vec![conflict_response(), version_response("2.3.0")]);
    write(
        &dir.join("sanshain.yaml"),
        &format!(
            "serviceName: demo-service\nsanshainUrl: {url}\nprovides:\n  - file: specs/openapi.yaml\n"
        ),
    );
    let output = run(&dir, &["provide"], &[]);
    assert_failure(&output);
    assert!(stderr(&output).contains("Publish as 1.5.0"));
    drop(joined_requests(handle));
}

#[test]
fn best_effort_turns_a_failed_provide_into_a_warning_from_flag_env_or_yaml() {
    let cases: [BestEffortCase; 4] = [
        ("be_flag_before", &["--best-effort", "provide"], &[], ""),
        ("be_flag_after", &["provide", "--best-effort"], &[], ""),
        ("be_env", &["provide"], &[("SANSHAIN_BEST_EFFORT", "1")], ""),
        ("be_yaml", &["provide"], &[], "bestEffort: true\n"),
    ];
    for (name, args, envs, extra_yaml) in cases {
        let dir = project(name);
        write_spec(&dir);
        let (url, handle) = mock_server(vec![conflict_response(), version_response("2.3.0")]);
        write(
            &dir.join("sanshain.yaml"),
            &format!(
                "serviceName: demo-service\nsanshainUrl: {url}\n{extra_yaml}provides:\n  - file: specs/openapi.yaml\n"
            ),
        );
        let output = run(&dir, args, envs);
        assert_success(&output);
        let err = stderr(&output);
        assert!(err.contains("Publish as 1.5.0"), "{name}: {err}");
        assert!(err.contains("Continuing (best effort)"), "{name}: {err}");
        drop(joined_requests(handle));
    }
}

#[test]
fn best_effort_skips_a_missing_spec_file_and_provides_the_rest() {
    let dir = project("be_missing_spec");
    write_spec(&dir);
    let (url, handle) = mock_server(vec![http_response(
        "202 Accepted",
        r#"{"version":"1.4.0","stability":"snapshot","content_hash":"sha256:abc","changes":{"inserts":1,"updates":0,"deletes":0}}"#,
    )]);
    write(
        &dir.join("sanshain.yaml"),
        &format!(
            "serviceName: demo-service\nsanshainUrl: {url}\nprovides:\n  - file: specs/missing.yaml\n  - file: specs/openapi.yaml\n"
        ),
    );
    let output = run(&dir, &["provide", "--best-effort"], &[]);
    assert_success(&output);
    assert!(stderr(&output).contains("Spec file not found"));
    assert!(stdout(&output).contains("Stored version 1.4.0"));
    assert_eq!(joined_requests(handle).len(), 1);

    let output = run(&dir, &["provide"], &[]);
    assert_failure(&output);
    assert!(stderr(&output).contains("Spec file not found"));
}

#[test]
fn best_effort_require_carries_on_with_the_next_entry() {
    let dir = project("be_require");
    let (url, handle) = mock_server(vec![
        http_response("404 Not Found", r#"{"error":"no version 9.9.9"}"#),
        version_response("2.3.0"),
        http_response("200 OK", "openapi: 3.0.3\n"),
    ]);
    write(
        &dir.join("sanshain.yaml"),
        &format!(
            "serviceName: demo-service\nsanshainUrl: {url}\nrequires:\n  - serviceName: ghost\n    version: 9.9.9\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /x\n  - serviceName: user-service\n    version: 1.2.0\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /y\n"
        ),
    );
    let output = run(&dir, &["require"], &[("SANSHAIN_BEST_EFFORT", "true")]);
    assert_success(&output);
    assert!(stderr(&output).contains("Error requiring ghost 9.9.9"));
    assert!(dir.join("out/user-service.yaml").exists());
    drop(joined_requests(handle));
}

#[test]
fn best_effort_does_not_soften_strict() {
    let dir = project("be_and_strict");
    write(
        &dir.join("sanshain.yaml"),
        "serviceName: demo-service\nstrict: true\n",
    );
    assert_failure(&run(&dir, &["provide", "--best-effort"], &[]));
}

#[test]
fn an_unloadable_config_is_a_warning_only_with_best_effort_from_flag_or_env() {
    let dir = project("no_config");
    assert_failure(&run(&dir, &["provide"], &[]));
    let output = run(&dir, &["provide"], &[("SANSHAIN_BEST_EFFORT", "1")]);
    assert_success(&output);
    assert!(stderr(&output).contains("Warning loading config"));
}

// --- the puzzled cat ---

const CAT_IGNORED: &str = "   /\\_/\\  ?\n  ( O_o )   sanshain.yaml: unknown fields 'catalogInf', 'requires[0].verison' - ignored\n ---U-U---\n";

const UNKNOWN_FIELDS_YAML: &str = "serviceName: demo-service\ncatalogInf: catalog-info.yaml\nrequires:\n  - serviceName: user-service\n    version: 1.2.0\n    verison: 1.2.0\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /x\n";

#[test]
fn unknown_fields_print_the_cat_once_and_carry_on() {
    let dir = project("cat_ignored");
    let (url, handle) = mock_server(vec![http_response("200 OK", "openapi: 3.0.3\n")]);
    write(
        &dir.join("sanshain.yaml"),
        &format!("sanshainUrl: {url}\n{UNKNOWN_FIELDS_YAML}"),
    );
    let output = run(&dir, &["require"], &[]);
    assert_success(&output);
    let err = stderr(&output);
    assert!(err.starts_with(CAT_IGNORED), "unexpected stderr: {err}");
    assert_eq!(err.matches("O_o").count(), 1, "the cat appears once: {err}");
    assert!(dir.join("out/user-service.yaml").exists());
    drop(joined_requests(handle));
}

#[test]
fn unknown_fields_fail_under_strict_even_with_best_effort() {
    let dir = project("cat_strict");
    write(&dir.join("sanshain.yaml"), UNKNOWN_FIELDS_YAML);
    let output = run(
        &dir,
        &["require", "--best-effort"],
        &[("SANSHAIN_STRICT", "true")],
    );
    assert_failure(&output);
    let err = stderr(&output);
    assert!(
        err.contains("'catalogInf', 'requires[0].verison' - rejected (strict)\n ---U-U---"),
        "unexpected stderr: {err}"
    );
}

// --- catalog ---

fn catalog_project(name: &str, url: &str, extra_yaml: &str) -> PathBuf {
    let dir = project(name);
    write(
        &dir.join("catalog-info.yaml"),
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders-service\n",
    );
    write(
        &dir.join("sanshain.yaml"),
        &format!("serviceName: orders-service\nsanshainUrl: {url}\n{extra_yaml}"),
    );
    dir
}

#[test]
fn catalog_prints_the_recorded_resources() {
    let (url, handle) = mock_server(vec![http_response(
        "200 OK",
        r#"{"system":"orders-service","trunk":true,"resources":["resource:default/orders-db","resource:default/orders-bucket"]}"#,
    )]);
    let dir = catalog_project("catalog_ok", &url, "catalogInfo: catalog-info.yaml\n");
    let output = run(&dir, &["catalog", "--trunk"], &[]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "Catalog recorded for orders-service (main+dev): 2 resources\n  resource:default/orders-db\n  resource:default/orders-bucket\n"
    );
    let requests = joined_requests(handle);
    assert!(requests[0].starts_with("PUT /catalog/orders-service "));
    assert!(requests[0].contains(r#""trunk":true"#), "{}", requests[0]);
}

#[test]
fn catalog_on_a_dev_build_reports_the_dev_graph() {
    let (url, handle) = mock_server(vec![http_response(
        "200 OK",
        r#"{"system":"orders-service","trunk":false,"resources":[]}"#,
    )]);
    let dir = catalog_project("catalog_dev", &url, "catalogInfo: catalog-info.yaml\n");
    let output = run(&dir, &["catalog"], &[]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "Catalog recorded for orders-service (dev): 0 resources\n"
    );
    let requests = joined_requests(handle);
    assert!(!requests[0].contains("trunk"), "{}", requests[0]);
}

#[test]
fn catalog_without_catalog_info_warns_and_skips_unless_strict() {
    let dir = catalog_project("catalog_none", "http://127.0.0.1:9", "");
    let output = run(&dir, &["catalog"], &[]);
    assert_success(&output);
    assert_eq!(
        stderr(&output),
        "No catalogInfo in sanshain.yaml - skipping catalog.\n"
    );

    let output = run(
        &dir,
        &["catalog", "--best-effort"],
        &[("SANSHAIN_STRICT", "true")],
    );
    assert_failure(&output);
    assert!(
        stderr(&output).contains("strict: sanshain.yaml has no catalogInfo"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

#[test]
fn catalog_strict_does_not_ask_for_provides_or_requires() {
    let (url, handle) = mock_server(vec![http_response(
        "200 OK",
        r#"{"system":"orders-service","trunk":false,"resources":[]}"#,
    )]);
    let dir = catalog_project(
        "catalog_strict_ok",
        &url,
        "strict: true\ncatalogInfo: catalog-info.yaml\n",
    );
    assert_success(&run(&dir, &["catalog"], &[]));
    drop(joined_requests(handle));
}

#[test]
fn a_catalog_400_fails_and_best_effort_turns_it_into_a_warning() {
    let rejected = || {
        mock_server(vec![
            http_response(
                "400 Bad Request",
                r#"{"error":"no Component named 'orders-service'"}"#,
            ),
            version_response("2.4.0"),
        ])
    };
    let (url, handle) = rejected();
    let dir = catalog_project("catalog_400", &url, "catalogInfo: catalog-info.yaml\n");
    let output = run(&dir, &["catalog"], &[]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("catalog failed (400): no Component named 'orders-service'"),
        "unexpected stderr: {}",
        stderr(&output)
    );
    drop(joined_requests(handle));

    let (url, handle) = rejected();
    let dir = catalog_project(
        "catalog_400_best_effort",
        &url,
        "catalogInfo: catalog-info.yaml\n",
    );
    let output = run(&dir, &["catalog", "--best-effort"], &[]);
    assert_success(&output);
    let err = stderr(&output);
    assert!(
        err.contains("catalog failed (400)"),
        "unexpected stderr: {err}"
    );
    assert!(
        err.contains("Continuing (best effort)"),
        "unexpected stderr: {err}"
    );
    drop(joined_requests(handle));
}

#[test]
fn catalog_on_a_pre_2_4_service_says_so() {
    let (url, handle) = mock_server(vec![
        http_response("404 Not Found", ""),
        version_response("2.3.0"),
    ]);
    let dir = catalog_project("catalog_404", &url, "catalogInfo: catalog-info.yaml\n");
    let output = run(&dir, &["catalog"], &[]);
    assert_failure(&output);
    assert!(
        stderr(&output)
            .contains("sanshain catalog needs Sanshain Service 2.4.0 or later (server is 2.3.0)"),
        "unexpected stderr: {}",
        stderr(&output)
    );
    drop(joined_requests(handle));
}

#[test]
fn catalog_with_a_missing_file_names_the_resolved_path() {
    let dir = catalog_project(
        "catalog_missing",
        "http://127.0.0.1:9",
        "catalogInfo: docs/catalog.yaml\n",
    );
    let output = run(&dir, &["catalog"], &[]);
    assert_failure(&output);
    let resolved = dir.join("docs/catalog.yaml");
    assert!(
        stderr(&output).contains(&format!("cannot read catalogInfo {}", resolved.display())),
        "unexpected stderr: {}",
        stderr(&output)
    );
    let output = run(&dir, &["catalog"], &[("SANSHAIN_BEST_EFFORT", "true")]);
    assert_success(&output);
    assert!(stderr(&output).contains("docs/catalog.yaml"));
}

// --- the cat comes before every other validation ---

/// `verison` instead of `version`: the typo is the cause of the next error, so
/// the cat must show before it.
const TYPO_THEN_MISSING_VERSION: &str = "serviceName: demo-service\nrequires:\n  - serviceName: user-service\n    verison: 1.2.0\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /x\n";

const CAT_VERISON: &str = "   /\\_/\\  ?\n  ( O_o )   sanshain.yaml: unknown field 'requires[0].verison' - ignored\n ---U-U---\n";

#[test]
fn the_cat_shows_before_a_later_validation_error() {
    let dir = project("cat_then_missing_version");
    write(&dir.join("sanshain.yaml"), TYPO_THEN_MISSING_VERSION);
    let output = run(&dir, &["require"], &[]);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(err.starts_with(CAT_VERISON), "unexpected stderr: {err}");
    assert!(
        err.contains("missing 'version'"),
        "unexpected stderr: {err}"
    );
}

#[test]
fn the_cat_shows_on_the_best_effort_fallback() {
    let dir = project("cat_then_fallback");
    write(&dir.join("sanshain.yaml"), TYPO_THEN_MISSING_VERSION);
    let output = run(&dir, &["require", "--best-effort"], &[]);
    assert_success(&output);
    let err = stderr(&output);
    assert!(err.starts_with(CAT_VERISON), "unexpected stderr: {err}");
    assert!(
        err.contains("Warning loading config") && err.contains("missing 'version'"),
        "unexpected stderr: {err}"
    );
}

#[test]
fn under_strict_unknown_fields_fail_before_other_validation() {
    let dir = project("cat_strict_first");
    write(
        &dir.join("sanshain.yaml"),
        &format!("strict: true\n{TYPO_THEN_MISSING_VERSION}"),
    );
    let output = run(&dir, &["require", "--best-effort"], &[]);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(
        err.contains("'requires[0].verison' - rejected (strict)"),
        "unexpected stderr: {err}"
    );
    assert!(
        !err.contains("missing 'version'"),
        "unexpected stderr: {err}"
    );
}

#[test]
fn a_legacy_field_errors_before_any_cat() {
    let dir = project("legacy_before_cat");
    write(
        &dir.join("sanshain.yaml"),
        "serviceName: demo-service\ncatalogInf: x\nrequires:\n  - serviceName: user-service\n    branch: main\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /x\n",
    );
    let output = run(&dir, &["require"], &[]);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(
        err.contains("'branch' is no longer supported"),
        "unexpected stderr: {err}"
    );
    assert!(!err.contains("O_o"), "unexpected stderr: {err}");
}

// --- format: openapi ---

fn format_yaml(url: &str, api_type: &str) -> String {
    format!(
        "serviceName: demo-service\nsanshainUrl: {url}\nrequires:\n  - serviceName: events\n    apiType: {api_type}\n    format: openapi\n    version: 1.0.0\n    outputDirectory: out\n    endpoints:\n      - method: SUB\n        path: user/signup\n"
    )
}

#[test]
fn format_with_the_wrong_api_type_fails_before_any_call() {
    let dir = project("format_wrong_api_type");
    // Nothing listens here: reaching the network would be a different error.
    write(
        &dir.join("sanshain.yaml"),
        &format_yaml("http://127.0.0.1:9", "openapi"),
    );
    let output = run(&dir, &["require"], &[]);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(
        err.contains("requires[0] (events): 'format: openapi' is not valid with apiType 'openapi'"),
        "unexpected stderr: {err}"
    );
    assert!(!err.contains("O_o"), "format is a known field: {err}");
}

#[test]
fn best_effort_warns_on_a_non_openapi_answer_and_writes_nothing() {
    let dir = project("format_best_effort");
    let (url, handle) = mock_server(vec![
        http_response("200 OK", "asyncapi: 2.6.0\n"),
        version_response("2.3.0"),
    ]);
    write(&dir.join("sanshain.yaml"), &format_yaml(&url, "asyncapi"));
    let output = run(&dir, &["require", "--best-effort"], &[]);
    assert_success(&output);
    let err = stderr(&output);
    assert!(
        err.contains("format: openapi needs Sanshain Service 2.4.0 or later (server is 2.3.0)"),
        "unexpected stderr: {err}"
    );
    assert!(
        err.contains("Continuing (best effort)"),
        "unexpected stderr: {err}"
    );
    assert!(!dir.join("out/events.yaml").exists());
    let requests = joined_requests(handle);
    assert!(requests[0].contains("format=openapi"), "{}", requests[0]);
}

#[test]
fn best_effort_warns_when_an_old_service_rejects_format() {
    let dir = project("format_400_best_effort");
    let (url, handle) = mock_server(vec![
        http_response(
            "400 Bad Request",
            r#"{"error":"Failed to deserialize query string: unknown field `format`"}"#,
        ),
        version_response("2.3.0"),
    ]);
    write(&dir.join("sanshain.yaml"), &format_yaml(&url, "asyncapi"));
    let output = run(&dir, &["require"], &[("SANSHAIN_BEST_EFFORT", "true")]);
    assert_success(&output);
    let err = stderr(&output);
    assert!(
        err.contains("format: openapi needs Sanshain Service 2.4.0 or later (server is 2.3.0)"),
        "unexpected stderr: {err}"
    );
    assert!(!dir.join("out/events.yaml").exists());
    drop(joined_requests(handle));
}

// --- Sanshain-Warning: the graph never fails a build ---

const NO_BRANCH: &str =
    "no sanshain-branch 'Release-X' - this build was not recorded in a release graph";
const NEITHER: &str = "trunk and tag are mutually exclusive - this build is recorded in neither";

/// `response` with one `Sanshain-Warning` header line per warning.
fn with_warnings(response: String, warnings: &[&str]) -> String {
    let headers: String = warnings
        .iter()
        .map(|w| format!("Sanshain-Warning: {w}\r\n"))
        .collect();
    response.replacen("\r\n", &format!("\r\n{headers}"), 1)
}

fn stored_response() -> String {
    http_response(
        "202 Accepted",
        r#"{"version":"1.4.0","stability":"snapshot","content_hash":"sha256:abc","changes":{"inserts":1,"updates":0,"deletes":0}}"#,
    )
}

/// A project providing `specs/openapi.yaml` to `url`.
fn provide_project(name: &str, url: &str) -> PathBuf {
    let dir = project(name);
    write_spec(&dir);
    write(
        &dir.join("sanshain.yaml"),
        &format!(
            "serviceName: demo-service\nsanshainUrl: {url}\nprovides:\n  - file: specs/openapi.yaml\n"
        ),
    );
    dir
}

#[test]
fn each_warning_is_printed_once_and_the_build_passes() {
    let (url, handle) = mock_server(vec![with_warnings(stored_response(), &[NO_BRANCH])]);
    let dir = provide_project("warn_single", &url);
    let output = run(&dir, &["provide", "--tag", "Release-X"], &[]);
    assert_success(&output);
    assert_eq!(stderr(&output), format!("warning: {NO_BRANCH}\n"));
    assert!(stdout(&output).contains("Stored version 1.4.0"));
    drop(joined_requests(handle));

    let second = "something else, with a comma";
    let (url, handle) = mock_server(vec![with_warnings(stored_response(), &[NO_BRANCH, second])]);
    let dir = provide_project("warn_multiple", &url);
    let output = run(&dir, &["provide"], &[]);
    assert_success(&output);
    assert_eq!(
        stderr(&output),
        format!("warning: {NO_BRANCH}\nwarning: {second}\n")
    );
    drop(joined_requests(handle));
}

#[test]
fn strict_fails_after_a_warned_call_even_with_best_effort() {
    for (name, args) in [
        ("warn_strict", &["provide"][..]),
        ("warn_strict_best_effort", &["provide", "--best-effort"][..]),
    ] {
        let (url, handle) = mock_server(vec![with_warnings(stored_response(), &[NO_BRANCH])]);
        let dir = provide_project(name, &url);
        let output = run(&dir, args, &[("SANSHAIN_STRICT", "true")]);
        assert_failure(&output);
        let err = stderr(&output);
        assert!(
            err.contains(&format!("warning: {NO_BRANCH}")),
            "{name}: {err}"
        );
        assert!(
            err.contains(&format!("strict: Sanshain warned: {NO_BRANCH}")),
            "{name}: {err}"
        );
        assert!(!err.contains("Continuing (best effort)"), "{name}: {err}");
        // The call was made: strict fails after it, not instead of it.
        assert_eq!(joined_requests(handle).len(), 1, "{name}");
    }
}

#[test]
fn strict_require_fails_on_a_warning_even_with_best_effort() {
    let dir = project("warn_require_strict");
    let (url, handle) = mock_server(vec![with_warnings(
        http_response("200 OK", "openapi: 3.0.3\n"),
        &[NO_BRANCH],
    )]);
    write(
        &dir.join("sanshain.yaml"),
        &format!(
            "serviceName: demo-service\nsanshainUrl: {url}\nstrict: true\nbestEffort: true\nrequires:\n  - serviceName: user-service\n    version: 1.2.0\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /y\n"
        ),
    );
    let output = run(&dir, &["require", "--tag", "Release-X"], &[]);
    assert_failure(&output);
    let err = stderr(&output);
    assert!(
        err.contains(&format!("strict: Sanshain warned: {NO_BRANCH}")),
        "{err}"
    );
    assert!(!err.contains("Continuing (best effort)"), "{err}");
    drop(joined_requests(handle));
}

#[test]
fn local_trunk_and_tag_warn_and_send_neither() {
    let (url, handle) = mock_server(vec![stored_response()]);
    let dir = provide_project("trunk_and_tag", &url);
    let output = run(
        &dir,
        &["provide", "--tag", "Release-X"],
        &[("SANSHAIN_TRUNK", "true")],
    );
    assert_success(&output);
    assert_eq!(stderr(&output), format!("warning: {NEITHER}\n"));
    let requests = joined_requests(handle);
    let body = requests[0].split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(!body.contains("trunk"), "{body}");
    assert!(!body.contains("\"tag\""), "{body}");
}

#[test]
fn provide_sends_the_origin_of_the_build() {
    let (url, handle) = mock_server(vec![stored_response()]);
    let dir = provide_project("origin", &url);
    let output = run(
        &dir,
        &[
            "provide",
            "--tickets",
            "ORD-7,ORD-9",
            "--note",
            "Adds users",
        ],
        &[(
            "SANSHAIN_CHANGE_URL",
            "https://github.com/acme/orders/pull/7",
        )],
    );
    assert_success(&output);
    let requests = joined_requests(handle);
    let body = requests[0].split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(body.contains(r#""tickets":["ORD-7","ORD-9"]"#), "{body}");
    assert!(body.contains(r#""note":"Adds users""#), "{body}");
    assert!(
        body.contains(r#""change_url":"https://github.com/acme/orders/pull/7""#),
        "{body}"
    );
}

#[test]
fn local_trunk_and_tag_fail_under_strict_before_any_request() {
    // Nothing listens on port 9: a request would fail differently.
    let dir = provide_project("trunk_and_tag_strict", "http://127.0.0.1:9");
    let output = run(
        &dir,
        &["provide", "--trunk", "--tag", "Release-X", "--best-effort"],
        &[("SANSHAIN_STRICT", "1")],
    );
    assert_failure(&output);
    let err = stderr(&output);
    assert!(err.contains(&format!("warning: {NEITHER}")), "{err}");
    assert!(err.contains(&format!("strict: {NEITHER}")), "{err}");
    assert!(!err.contains("Continuing (best effort)"), "{err}");
}

fn unrecorded_catalog_response(reason: &str) -> String {
    with_warnings(
        http_response(
            "200 OK",
            r#"{"system":"orders-service","trunk":false,"resources":[],"recorded":false}"#,
        ),
        &[reason],
    )
}

#[test]
fn an_unrecorded_catalog_warns_and_passes_unless_strict() {
    let reason = "no Component named 'orders-service' in catalog-info.yaml";
    let (url, handle) = mock_server(vec![unrecorded_catalog_response(reason)]);
    let dir = catalog_project(
        "catalog_unrecorded",
        &url,
        "catalogInfo: catalog-info.yaml\n",
    );
    let output = run(&dir, &["catalog"], &[]);
    assert_success(&output);
    assert_eq!(stderr(&output), format!("warning: {reason}\n"));
    assert_eq!(stdout(&output), "");
    drop(joined_requests(handle));

    let (url, handle) = mock_server(vec![unrecorded_catalog_response(reason)]);
    let dir = catalog_project(
        "catalog_unrecorded_strict",
        &url,
        "strict: true\ncatalogInfo: catalog-info.yaml\n",
    );
    let output = run(&dir, &["catalog", "--best-effort"], &[]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains(&format!("strict: Sanshain warned: {reason}")),
        "{}",
        stderr(&output)
    );
    drop(joined_requests(handle));
}
