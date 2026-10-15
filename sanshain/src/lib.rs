//! Client library for the Sanshain Service: **provide** full API specifications
//! (OpenAPI, AsyncAPI, gRPC/Proto) under a producer-declared version, and
//! **require** pinned endpoint snippets.
//!
//! Compatibility: **sanshain 2.x speaks Sanshain Service 2.x.** Only
//! [`Client::catalog`] needs Sanshain Service 2.4.0 or later.
//!
//! Configuration lives in `sanshain.yaml`, the same file every other Sanshain
//! client reads.
//!
//! The spec version is never configured here — it lives in the spec file itself
//! (`info.version` for OpenAPI/AsyncAPI, a `// sanshain-version:` comment for
//! proto). Every provide declares a stability: `snapshot` by default, `ga` only
//! via the explicit switch (`SANSHAIN_GA=true` or the CLI's `--ga`).

use serde::Deserialize;
use serde_json::{Value, json};
use serde_yaml_ng::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use thiserror::Error;

/// Environment variable that switches provides to GA stability (`true` or `1`).
pub const GA_ENV_VAR: &str = "SANSHAIN_GA";
/// Environment variable overriding the configured Sanshain server URL.
pub const URL_ENV_VAR: &str = "SANSHAIN_URL";
/// Environment variable carrying the API token (sent as `Authorization: Bearer`).
pub const TOKEN_ENV_VAR: &str = "SANSHAIN_TOKEN";
/// Environment variable declaring this build as the trunk stream's (`true` or `1`).
pub const TRUNK_ENV_VAR: &str = "SANSHAIN_TRUNK";
/// Environment variable naming the sanshain-branch this build belongs to.
pub const TAG_ENV_VAR: &str = "SANSHAIN_TAG";
/// Environment variable turning "warn and skip" on missing configuration into
/// a failure (`true` or `1`); overrides `strict` in `sanshain.yaml` when set.
pub const STRICT_ENV_VAR: &str = "SANSHAIN_STRICT";
/// Environment variable turning failed calls into warnings (`true` or `1`);
/// overrides `bestEffort` in `sanshain.yaml` when set.
pub const BEST_EFFORT_ENV_VAR: &str = "SANSHAIN_BEST_EFFORT";

/// The default configuration file name, in the project root.
pub const CONFIG_FILE: &str = "sanshain.yaml";

/// The response header through which Sanshain reports something worth a build
/// warning — an unknown sanshain-branch, an unreadable catalog. It may appear
/// several times, one warning per header line. See [`Client::take_warnings`].
pub const WARNING_HEADER: &str = "Sanshain-Warning";

/// What [`Stream::resolve`] reports for a build declaring both trunk and a tag.
pub const TRUNK_AND_TAG_WARNING: &str =
    "trunk and tag are mutually exclusive - this build is recorded in neither";

#[derive(Error, Debug)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// A provide/require the server refused; the message is fully formatted.
    #[error("{0}")]
    Api(String),
    /// The server answered, but it is a pre-2.0 Sanshain instance.
    #[error("{0}")]
    WrongServer(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Which of exactly two states a stored version is in. Declared by the caller
/// on every provide; never part of the version string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    Snapshot,
    Ga,
}

impl Stability {
    pub fn as_str(self) -> &'static str {
        match self {
            Stability::Snapshot => "snapshot",
            Stability::Ga => "ga",
        }
    }
}

impl std::fmt::Display for Stability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a `SANSHAIN_*` boolean switch is on: `true` or `1`, trimmed and
/// case-insensitive. Anything else — including unset — is off.
pub fn is_switch_on(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        v.eq_ignore_ascii_case("true") || v == "1"
    })
}

/// Resolves a switch that `sanshain.yaml` can also set (`strict`,
/// `bestEffort`): an explicit flag turns it on; otherwise a set, non-empty
/// environment variable decides, either way; otherwise the configured value.
pub fn resolve_switch_with(flag: bool, env: Option<&str>, configured: bool) -> bool {
    if flag {
        return true;
    }
    match env {
        Some(value) if !value.is_empty() => is_switch_on(Some(value)),
        _ => configured,
    }
}

/// Resolves the stability of a provide: the explicit `--ga` flag or
/// `SANSHAIN_GA=true` switch to GA; everything else is a snapshot.
pub fn resolve_stability(ga_flag: bool) -> Stability {
    resolve_stability_with(ga_flag, std::env::var(GA_ENV_VAR).ok().as_deref())
}

/// [`resolve_stability`] with the environment value passed in (testable).
pub fn resolve_stability_with(ga_flag: bool, ga_env: Option<&str>) -> Stability {
    if ga_flag || is_switch_on(ga_env) {
        Stability::Ga
    } else {
        Stability::Snapshot
    }
}

/// Which dependency graph a build's calls belong to: trunk, a named
/// sanshain-branch, or neither.
///
/// The stream is a property of the invocation, not of the repository, so it is
/// declared by the pipeline (`--trunk` / [`TRUNK_ENV_VAR`], `--tag` /
/// [`TAG_ENV_VAR`]) rather than committed to `sanshain.yaml`. A developer's
/// laptop and the trunk CI job build the same checkout, and the trunk pin store
/// is last-writer-wins — inferring the stream would let a local build overwrite
/// what CI recorded.
///
/// Trunk and a tag are mutually exclusive, so this type cannot hold both; a
/// build declaring both resolves to [`Stream::None`] with a warning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Stream {
    /// The build speaks for no graph — the default.
    #[default]
    None,
    /// The build maintains the main graph.
    Trunk,
    /// The build updates one sanshain-branch's graph.
    Tag(String),
}

impl Stream {
    /// The stream declared by a pipeline, preferring an explicit flag over the
    /// environment.
    ///
    /// Reads [`TRUNK_ENV_VAR`] and [`TAG_ENV_VAR`]; see [`Stream::resolve_with`]
    /// for the testable form.
    pub fn resolve(trunk_flag: bool, tag: Option<&str>) -> (Stream, Option<&'static str>) {
        Stream::resolve_with(
            trunk_flag,
            tag,
            std::env::var(TRUNK_ENV_VAR).ok().as_deref(),
            std::env::var(TAG_ENV_VAR).ok().as_deref(),
        )
    }

    /// [`Stream::resolve`] with the environment values passed in.
    ///
    /// A build declaring both trunk and a tag is recorded in neither graph:
    /// the graph is informational and never fails a build, so this resolves
    /// to [`Stream::None`] together with [`TRUNK_AND_TAG_WARNING`] — for the
    /// caller to print, and to fail on under strict mode.
    pub fn resolve_with(
        trunk_flag: bool,
        tag: Option<&str>,
        trunk_env: Option<&str>,
        tag_env: Option<&str>,
    ) -> (Stream, Option<&'static str>) {
        let trunk = trunk_flag || is_switch_on(trunk_env);
        let tag = tag
            .or(tag_env)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);

        match (trunk, tag) {
            (true, Some(_)) => (Stream::None, Some(TRUNK_AND_TAG_WARNING)),
            (true, None) => (Stream::Trunk, None),
            (false, Some(tag)) => (Stream::Tag(tag), None),
            (false, None) => (Stream::None, None),
        }
    }

    /// The sanshain-branch this build belongs to, if any.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Stream::Tag(tag) => Some(tag),
            _ => None,
        }
    }

    /// Whether this build speaks for the trunk stream.
    pub fn is_trunk(&self) -> bool {
        matches!(self, Stream::Trunk)
    }

    /// The stream as require query parameters — empty when nothing is declared,
    /// since the server defaults `trunk` to false and treats an absent `tag` as
    /// "no branch".
    fn query_params(&self) -> Vec<(&'static str, &str)> {
        match self {
            Stream::None => Vec::new(),
            Stream::Trunk => vec![("trunk", "true")],
            Stream::Tag(tag) => vec![("tag", tag.as_str())],
        }
    }

    /// Adds the stream to a provide payload. Absent fields rather than explicit
    /// nulls: the payload denies unknown shapes and defaults these itself.
    fn apply_to_payload(&self, map: &mut serde_json::Map<String, Value>) {
        match self {
            Stream::None => {}
            Stream::Trunk => {
                map.insert("trunk".to_string(), Value::Bool(true));
            }
            Stream::Tag(tag) => {
                map.insert("tag".to_string(), Value::String(tag.clone()));
            }
        }
    }
}

impl std::fmt::Display for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stream::None => f.write_str("none"),
            Stream::Trunk => f.write_str("trunk"),
            Stream::Tag(tag) => write!(f, "branch '{tag}'"),
        }
    }
}

/// The API type of a provide/require entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiType {
    #[default]
    OpenApi,
    AsyncApi,
    Proto,
}

impl ApiType {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiType::OpenApi => "openapi",
            ApiType::AsyncApi => "asyncapi",
            ApiType::Proto => "proto",
        }
    }

    fn provide_path(self) -> &'static str {
        match self {
            ApiType::OpenApi => "/provide",
            ApiType::AsyncApi => "/provide/asyncapi",
            ApiType::Proto => "/provide/grpc",
        }
    }

    fn require_path(self) -> &'static str {
        match self {
            ApiType::OpenApi => "/require",
            ApiType::AsyncApi => "/require/asyncapi",
            ApiType::Proto => "/require/grpc",
        }
    }

    fn payload_key(self) -> &'static str {
        match self {
            ApiType::OpenApi => "openapi_yaml",
            ApiType::AsyncApi => "asyncapi_yaml",
            ApiType::Proto => "proto_content",
        }
    }

    fn file_extension(self) -> &'static str {
        match self {
            ApiType::OpenApi | ApiType::AsyncApi => "yaml",
            ApiType::Proto => "proto",
        }
    }

    /// Where the version lives in a spec file of this type.
    fn version_slot(self) -> &'static str {
        match self {
            ApiType::OpenApi | ApiType::AsyncApi => "info.version",
            ApiType::Proto => "the // sanshain-version: marker",
        }
    }
}

impl std::fmt::Display for ApiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One specification this project publishes. The version is read from the spec
/// file by the server (`info.version` / `// sanshain-version:`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvideEntry {
    pub file: PathBuf,
    #[serde(default)]
    pub api_type: ApiType,
    /// This project no longer provides the family — retire the capability
    /// instead of publishing.
    ///
    /// Deleting the entry would say nothing: Sanshain cannot tell a dropped
    /// protocol from a pipeline that merely stopped running, so absence leaves
    /// the capability, graph edges and contracts in place. Keeping the entry and
    /// marking it retired is the explicit act. See [`Client::retire`].
    #[serde(default)]
    pub retired: bool,
}

/// One endpoint/channel/RPC of a requires entry.
#[derive(Debug, Clone, Deserialize)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
}

/// One dependency on another service, pinned to an exact version.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequireEntry {
    pub service_name: String,
    #[serde(default)]
    pub api_type: ApiType,
    pub version: String,
    pub output_directory: PathBuf,
    pub endpoints: Vec<Endpoint>,
    /// The format to receive the snippet in, when it differs from the API
    /// type's own. Only [`RequireFormat::OpenApi`] for an AsyncAPI entry.
    #[serde(default)]
    pub format: Option<RequireFormat>,
}

/// A format a requires entry can ask the server to convert its snippet to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequireFormat {
    /// An AsyncAPI snippet as an OpenAPI document (its message schemas), for
    /// OpenAPI code generators. Needs Sanshain Service 2.4.0 or later.
    OpenApi,
}

impl RequireFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            RequireFormat::OpenApi => "openapi",
        }
    }
}

/// Configuration from `[package.metadata.sanshain]` in `Cargo.toml`.
#[derive(Debug, Clone)]
pub struct Config {
    pub sanshain_url: Option<String>,
    /// The name this project provides and requires as. Absent is not a parse
    /// error: a command warns and skips without it, or fails under `strict`.
    pub service_name: Option<String>,
    pub provides: Vec<ProvideEntry>,
    pub requires: Vec<RequireEntry>,
    /// `strict` from `sanshain.yaml`: missing configuration fails instead of
    /// warning and skipping. See [`STRICT_ENV_VAR`].
    pub strict: bool,
    /// `bestEffort` from `sanshain.yaml`: failed calls become warnings. See
    /// [`BEST_EFFORT_ENV_VAR`].
    pub best_effort: bool,
    /// `catalogInfo`: the project's Backstage `catalog-info.yaml`, pushed by
    /// [`Client::catalog`].
    pub catalog_info: Option<PathBuf>,
    /// Fields no Sanshain client knows, as paths in file order
    /// (`provides[1].verison`). Parsing ignores them; see [`puzzled_cat`].
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    sanshain_url: Option<String>,
    service_name: Option<String>,
    provide: Option<ProvideEntry>,
    #[serde(default)]
    provides: Vec<ProvideEntry>,
    #[serde(default)]
    requires: Vec<RequireEntry>,
    #[serde(default)]
    strict: bool,
    #[serde(default)]
    best_effort: bool,
    catalog_info: Option<PathBuf>,
}

/// Config fields removed in the 2.0 model, rejected by name with a migration hint.
const REMOVED_FIELDS: &[(&str, &str)] = &[
    (
        "branch",
        "'branch' is no longer supported — the branch model was removed in Sanshain 2.0; replace it with an exact 'version' pin",
    ),
    (
        "timeout",
        "'timeout' is no longer supported — Sanshain 2.0 resolves pins immediately instead of long-polling; remove it",
    ),
    (
        "baseVersion",
        "'baseVersion' is no longer supported — Sanshain 2.0 dropped optimistic concurrency; the version is read from the spec file itself; remove it",
    ),
    (
        "base_version",
        "'base_version' is no longer supported — Sanshain 2.0 dropped optimistic concurrency; the version is read from the spec file itself; remove it",
    ),
    (
        "releaseBranches",
        "'releaseBranches' is no longer supported — stability no longer derives from git branches; snapshot is the default and GA is an explicit switch (SANSHAIN_GA=true or --ga)",
    ),
    (
        "release_branches",
        "'release_branches' is no longer supported — stability no longer derives from git branches; snapshot is the default and GA is an explicit switch (SANSHAIN_GA=true or --ga)",
    ),
    (
        "stability",
        "'stability' is not configured in sanshain.yaml — snapshot is the default; publish GA explicitly via SANSHAIN_GA=true or --ga",
    ),
    (
        "spec_id",
        "'spec_id' is no longer supported — Sanshain 2.0 addresses specs by 'serviceName' and the exact 'version' pinned in each requires entry",
    ),
    (
        "api_key",
        "'api_key' is no longer supported — pass the token via the SANSHAIN_TOKEN environment variable; secrets do not belong in sanshain.yaml",
    ),
    (
        "service_url",
        "'service_url' is no longer supported — use 'sanshainUrl' (or the SANSHAIN_URL environment variable)",
    ),
];

/// Every field any Sanshain client knows, per level. A field known here but not
/// supported by this client (`compression`, `insecure`, …) is accepted silently.
const KNOWN_TOP_FIELDS: &[&str] = &[
    "sanshainUrl",
    "serviceName",
    "clientName",
    "compression",
    "bestEffort",
    "strict",
    "force",
    "insecure",
    "combine",
    "catalogInfo",
    "provide",
    "provides",
    "requires",
];
const KNOWN_PROVIDE_FIELDS: &[&str] = &[
    "file",
    "apiType",
    "retired",
    "openApiFile",
    "asyncApiFile",
    "protoFile",
    "combine",
];
const KNOWN_REQUIRE_FIELDS: &[&str] = &[
    "serviceName",
    "apiType",
    "version",
    "outputDirectory",
    "endpoints",
    "format",
];
const KNOWN_ENDPOINT_FIELDS: &[&str] = &["method", "path"];

/// Appends `prefix` + each key of `value` not in `known` to `out`, in order.
fn collect_unknown(value: &YamlValue, known: &[&str], prefix: &str, out: &mut Vec<String>) {
    for key in value.as_mapping().into_iter().flat_map(|m| m.keys()) {
        let key = yaml_key(key);
        if !known.contains(&key.as_str()) {
            out.push(format!("{prefix}{key}"));
        }
    }
}

fn yaml_key(key: &YamlValue) -> String {
    match key.as_str() {
        Some(key) => key.to_string(),
        None => serde_yaml_ng::to_string(key)
            .map(|k| k.trim_end().to_string())
            .unwrap_or_default(),
    }
}

/// Every unknown field of a configuration, as a path, in document order —
/// read from the YAML value, whose mappings keep the order of the file.
fn unknown_fields(value: &YamlValue) -> Vec<String> {
    let mut out = Vec::new();
    for (key, value) in value.as_mapping().into_iter().flatten() {
        let key = yaml_key(key);
        match key.as_str() {
            "provide" => collect_unknown(value, KNOWN_PROVIDE_FIELDS, "provide.", &mut out),
            "provides" => {
                for (i, entry) in value.as_sequence().into_iter().flatten().enumerate() {
                    let prefix = format!("provides[{i}].");
                    collect_unknown(entry, KNOWN_PROVIDE_FIELDS, &prefix, &mut out);
                }
            }
            "requires" => {
                for (i, entry) in value.as_sequence().into_iter().flatten().enumerate() {
                    for (field, field_value) in entry.as_mapping().into_iter().flatten() {
                        let field = yaml_key(field);
                        if field == "endpoints" {
                            for (j, endpoint) in
                                field_value.as_sequence().into_iter().flatten().enumerate()
                            {
                                let prefix = format!("requires[{i}].endpoints[{j}].");
                                collect_unknown(endpoint, KNOWN_ENDPOINT_FIELDS, &prefix, &mut out);
                            }
                        } else if !KNOWN_REQUIRE_FIELDS.contains(&field.as_str()) {
                            out.push(format!("requires[{i}].{field}"));
                        }
                    }
                }
            }
            other if !KNOWN_TOP_FIELDS.contains(&other) => out.push(other.to_string()),
            _ => {}
        }
    }
    out
}

/// The puzzled cat: what a command prints, once, when `sanshain.yaml` carries
/// fields no Sanshain client knows. `None` when there are none. Under strict
/// the fields are rejected and the command must fail after printing it.
pub fn puzzled_cat(unknown_fields: &[String], strict: bool) -> Option<String> {
    if unknown_fields.is_empty() {
        return None;
    }
    let noun = if unknown_fields.len() == 1 {
        "field"
    } else {
        "fields"
    };
    let names = unknown_fields
        .iter()
        .map(|f| format!("'{f}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let verdict = if strict {
        "rejected (strict)"
    } else {
        "ignored"
    };
    Some(format!(
        "   /\\_/\\  ?\n  ( O_o )   {CONFIG_FILE}: unknown {noun} {names} - {verdict}\n ---U-U---"
    ))
}

fn check_removed_fields(obj: &serde_json::Map<String, Value>, context: &str) -> Result<()> {
    for (field, message) in REMOVED_FIELDS {
        if obj.contains_key(*field) {
            return Err(Error::Config(if context.is_empty() {
                (*message).to_string()
            } else {
                format!("{context}: {message}")
            }));
        }
    }
    Ok(())
}

/// Whether a pin is a version the server will accept: `MAJOR[.MINOR[.PATCH]]`,
/// optionally `v`-prefixed.
///
/// Deliberately does not normalize. The server canonicalizes to three parts and
/// answers with the canonical form; a second normalizer here could only ever
/// disagree with it about what `v2.1` means.
fn is_pinnable_version(version: &str) -> bool {
    let version = version.strip_prefix('v').unwrap_or(version);
    let parts: Vec<&str> = version.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// A `sanshain.yaml` that parsed as YAML and passed the legacy-field
/// rejections. Its unknown fields are known at this point, whether or not the
/// remaining validation succeeds — so a command can show them either way.
#[derive(Debug)]
pub struct LoadedConfig {
    /// Fields no Sanshain client knows, as paths in file order.
    pub unknown_fields: Vec<String>,
    /// `strict: true` as written in the file, readable even when the rest of
    /// the validation fails.
    pub strict: bool,
    /// The validated configuration, or the first validation error.
    pub config: Result<Config>,
}

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Config(format!("invalid {CONFIG_FILE}: {e}"))
}

fn as_mapping(value: &Value) -> Result<&serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| Error::Config("sanshain.yaml must be a mapping".to_string()))
}

/// Rejects every leftover pre-2.0 field, at every level, by name with a
/// migration hint. Runs before anything else looks at the document.
fn check_legacy_fields(obj: &serde_json::Map<String, Value>) -> Result<()> {
    check_removed_fields(obj, "")?;
    if let Some(single) = obj.get("provide").and_then(Value::as_object) {
        check_removed_fields(single, "provide")?;
    }
    if let Some(Value::Array(list)) = obj.get("provides") {
        for (i, entry) in list.iter().enumerate() {
            if let Some(entry_obj) = entry.as_object() {
                check_removed_fields(entry_obj, &format!("provides[{i}]"))?;
            }
        }
    }
    if let Some(Value::Array(list)) = obj.get("requires") {
        for (i, entry) in list.iter().enumerate() {
            if let Some(entry_obj) = entry.as_object() {
                check_removed_fields(entry_obj, &require_context(i, entry_obj))?;
            }
        }
    }
    Ok(())
}

fn require_context(i: usize, entry_obj: &serde_json::Map<String, Value>) -> String {
    let name = entry_obj
        .get("serviceName")
        .and_then(Value::as_str)
        .unwrap_or("?");
    format!("requires[{i}] ({name})")
}

/// `format` exists for one combination only: `format: openapi` on an
/// `apiType: asyncapi` entry.
fn check_format(context: &str, format: &Value, api_type: Option<&Value>) -> Result<()> {
    const ALLOWED: &str = "the only combination is 'format: openapi' with 'apiType: asyncapi'";
    if format.as_str() != Some("openapi") {
        let shown = format
            .as_str()
            .map_or_else(|| format.to_string(), str::to_string);
        return Err(Error::Config(format!(
            "{context}: format '{shown}' is not supported — {ALLOWED}"
        )));
    }
    if api_type.and_then(Value::as_str) != Some("asyncapi") {
        let shown = api_type
            .and_then(Value::as_str)
            .unwrap_or(ApiType::default().as_str());
        return Err(Error::Config(format!(
            "{context}: 'format: openapi' is not valid with apiType '{shown}' — {ALLOWED}"
        )));
    }
    Ok(())
}

/// Everything after the legacy rejections and the unknown-field detection:
/// pins, endpoints, and the shape of each field.
fn validate(value: &Value, unknown_fields: Vec<String>) -> Result<Config> {
    let obj = as_mapping(value)?;
    if let Some(Value::Array(list)) = obj.get("requires") {
        for (i, entry) in list.iter().enumerate() {
            let Some(entry_obj) = entry.as_object() else {
                continue;
            };
            let name = entry_obj
                .get("serviceName")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let context = require_context(i, entry_obj);
            match entry_obj.get("version") {
                None => {
                    return Err(Error::Config(format!(
                        "{context}: missing 'version' — Sanshain 2.0 pins exact versions; add version = \"1.2.0\" (list available: GET /producers/{name}/versions)"
                    )));
                }
                Some(version) => {
                    let version = version.as_str().unwrap_or("");
                    if !is_pinnable_version(version) {
                        return Err(Error::Config(format!(
                            "{context}: version '{version}' is not a version — pin MAJOR[.MINOR[.PATCH]], optionally v-prefixed (no ranges, no 'latest')"
                        )));
                    }
                }
            }
            match entry_obj.get("endpoints") {
                Some(Value::Array(endpoints)) if !endpoints.is_empty() => {}
                _ => {
                    return Err(Error::Config(format!(
                        "{context}: 'endpoints' must list at least one endpoint (method + path)"
                    )));
                }
            }
            if let Some(format) = entry_obj.get("format") {
                check_format(&context, format, entry_obj.get("apiType"))?;
            }
        }
    }

    let raw: RawConfig = serde_json::from_value(value.clone()).map_err(invalid)?;
    let mut provides = Vec::new();
    if let Some(single) = raw.provide {
        provides.push(single);
    }
    provides.extend(raw.provides);
    Ok(Config {
        sanshain_url: raw.sanshain_url,
        service_name: raw.service_name,
        provides,
        requires: raw.requires,
        strict: raw.strict,
        best_effort: raw.best_effort,
        catalog_info: raw.catalog_info,
        unknown_fields,
    })
}

impl Config {
    /// Loads configuration from a `sanshain.yaml` file.
    ///
    /// Every relative path in it (`file`, `outputDirectory`, `catalogInfo`) is
    /// resolved against the (absolute) directory containing the file, never the
    /// working directory, so a build pointed at a config elsewhere finds the
    /// same files as one started next to it. Absolute paths stay as they are.
    pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_yaml_file(path)?.config
    }

    /// [`Config::from_yaml_file`] in two phases: the error is an unreadable
    /// file, invalid YAML or a legacy field; anything validated later is
    /// in [`LoadedConfig::config`], next to the already known unknown fields.
    pub fn load_yaml_file(path: impl AsRef<Path>) -> Result<LoadedConfig> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|e| {
            Error::Config(format!(
                "cannot read {}: {e} — every Sanshain client reads its configuration from {CONFIG_FILE} in the project root",
                path.display()
            ))
        })?;
        let mut loaded = Self::load_yaml_str(&text)?;
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if let (Ok(config), Some(dir)) = (&mut loaded.config, absolute.parent()) {
            config.resolve_paths_against(dir);
        }
        Ok(loaded)
    }

    /// Rebases every relative path on `dir`; `Path::join` keeps an absolute
    /// path absolute.
    fn resolve_paths_against(&mut self, dir: &Path) {
        for entry in &mut self.provides {
            entry.file = dir.join(&entry.file);
        }
        for entry in &mut self.requires {
            entry.output_directory = dir.join(&entry.output_directory);
        }
        if let Some(catalog_info) = &mut self.catalog_info {
            *catalog_info = dir.join(&*catalog_info);
        }
    }

    /// Parses and validates a `sanshain.yaml` document.
    pub fn from_yaml_str(yaml: &str) -> Result<Self> {
        Self::load_yaml_str(yaml)?.config
    }

    /// [`Config::from_yaml_str`] in two phases, see [`Config::load_yaml_file`].
    /// Order: YAML parse, legacy-field rejections, unknown-field detection,
    /// then every other validation.
    pub fn load_yaml_str(yaml: &str) -> Result<LoadedConfig> {
        let yaml_value: YamlValue = serde_yaml_ng::from_str(yaml).map_err(invalid)?;
        let value = serde_json::to_value(&yaml_value).map_err(invalid)?;
        let obj = as_mapping(&value)?;
        check_legacy_fields(obj)?;
        let unknown_fields = unknown_fields(&yaml_value);
        Ok(LoadedConfig {
            strict: obj.get("strict").and_then(Value::as_bool).unwrap_or(false),
            config: validate(&value, unknown_fields.clone()),
            unknown_fields,
        })
    }

    /// Parses and validates a configuration value. All validation happens here,
    /// before any network call: a requires entry without `version` and every
    /// leftover pre-2.0 field are hard errors carrying a migration hint. Any
    /// other field no client knows is collected in [`Config::unknown_fields`]
    /// (in the map's order — file order only when loaded from YAML).
    pub fn from_value(value: &Value) -> Result<Self> {
        check_legacy_fields(as_mapping(value)?)?;
        let unknown_fields = serde_yaml_ng::to_value(value)
            .map(|yaml| unknown_fields(&yaml))
            .unwrap_or_default();
        validate(value, unknown_fields)
    }
}

/// Resolves the server base URL: the `SANSHAIN_URL` environment variable wins
/// over the configured `sanshainUrl`; missing both is a configuration error.
pub fn resolve_url(config_url: Option<&str>, env_url: Option<&str>) -> Result<String> {
    env_url
        .or(config_url)
        .map(|u| u.trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Error::Config(
                "missing 'sanshainUrl' — set it in sanshain.yaml or via the SANSHAIN_URL environment variable"
                    .to_string(),
            )
        })
}

/// The error body every failure response carries; `proposed_version` appears
/// only on a 409 refusing a provide by the version rules.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ErrorBody {
    pub error: String,
    pub proposed_version: Option<String>,
}

/// What the server stored for a successful provide.
#[derive(Debug, Clone, Deserialize)]
pub struct ProvideResponse {
    pub version: String,
    pub stability: String,
    pub content_hash: String,
    pub changes: Changes,
    /// AsyncAPI `subscribe` operations harvested from this document as
    /// version-less consumer edges. Present only for AsyncAPI provides that
    /// declared any.
    #[serde(default)]
    pub harvested_subscriptions: Vec<HarvestedSubscription>,
}

/// Endpoint diff relative to what this exact version stored before.
#[derive(Debug, Clone, Deserialize)]
pub struct Changes {
    pub inserts: i64,
    pub updates: i64,
    pub deletes: i64,
}

/// One AsyncAPI `subscribe` operation harvested from a provide.
///
/// Each is checked against the publishing Producer's GA channel contract.
/// Reading fewer fields than the contract offers is fine; expecting a field the
/// contract does not guarantee is drift — reported here on a snapshot, refused
/// with `409` on a GA provide.
#[derive(Debug, Clone, Deserialize)]
pub struct HarvestedSubscription {
    pub channel: String,
    pub message_name: String,
    /// The Producer owning the channel's PUB contract, absent while none does.
    #[serde(default)]
    pub owner: Option<String>,
    /// Why the expectation is not satisfiable by the current contract.
    #[serde(default)]
    pub drift: Option<String>,
}

impl HarvestedSubscription {
    /// Whether this line deserves a warning rather than an informational note —
    /// drift against the contract, or no publisher at all.
    pub fn is_advisory(&self) -> bool {
        self.drift.is_some() || self.owner.is_none()
    }
}

impl std::fmt::Display for HarvestedSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} / {}", self.channel, self.message_name)?;
        match &self.owner {
            Some(owner) => write!(f, " <- {owner}")?,
            None => f.write_str(" <- (no publisher yet)")?,
        }
        if let Some(drift) = &self.drift {
            write!(f, " — {drift}")?;
        }
        Ok(())
    }
}

/// What retiring an API family shed. A retire publishes no version, so it
/// reports none; all counts are zero for a dry run.
#[derive(Debug, Clone, Deserialize)]
pub struct RetiredProtocol {
    /// The capability tag removed (`messaging`/`grpc`), absent for OpenAPI.
    #[serde(default)]
    pub tag_cleared: Option<String>,
    /// Open trunk pins closed — they leave the current main graph, their closed
    /// rows stay as timeline history.
    pub trunk_pins_closed: i64,
    /// AsyncAPI channel-message contracts released for another Producer to claim.
    pub contracts_released: i64,
}

/// Sanshain compares provided content byte for byte, so a CRLF checkout of an
/// otherwise identical file hashes differently from an LF one and provokes a
/// spurious version conflict. Normalising before upload means a Windows and a
/// Linux runner publishing the same commit agree.
fn normalize_line_endings(content: &str) -> String {
    if content.contains('\r') {
        content.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        content.to_string()
    }
}

/// Builds the JSON payload for a provide call. Carries `stability` and the
/// declared stream; the 1.x fields (`branch`, `base_version`, `force`) no longer
/// exist.
pub(crate) fn provide_payload(
    producername: &str,
    api_type: ApiType,
    spec_content: &str,
    stability: Stability,
    stream: &Stream,
) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "producername".to_string(),
        Value::String(producername.to_string()),
    );
    map.insert(
        api_type.payload_key().to_string(),
        Value::String(normalize_line_endings(spec_content)),
    );
    map.insert(
        "stability".to_string(),
        Value::String(stability.as_str().to_string()),
    );
    stream.apply_to_payload(&mut map);
    Value::Object(map)
}

/// Builds the JSON payload for a retire.
///
/// A retire carries no document and no stability, and belongs to no stream — the
/// server answers `400` for a request combining `retired` with any of them, so
/// this payload has no field that could express one.
pub(crate) fn retire_payload(producername: &str) -> Value {
    json!({ "producername": producername, "retired": true })
}

/// Builds the query parameters for a single-endpoint require call. Carries the
/// exact pinned `version`; the 1.x fields (`branch`, `timeout`, …) no longer exist.
pub(crate) fn require_query<'a>(
    consumername: &'a str,
    entry: &'a RequireEntry,
    endpoint: &'a Endpoint,
    stream: &'a Stream,
) -> Vec<(&'static str, &'a str)> {
    let mut params = vec![
        ("consumername", consumername),
        ("producername", entry.service_name.as_str()),
        ("version", entry.version.as_str()),
        ("path", endpoint.path.as_str()),
        ("method", endpoint.method.as_str()),
    ];
    params.extend(stream.query_params());
    params.extend(format_query(entry));
    params
}

/// The `format` query parameter, sent on both require calls — empty when the
/// entry asks for none, so nothing changes for it.
fn format_query(entry: &RequireEntry) -> Vec<(&'static str, &'static str)> {
    entry
        .format
        .map(|format| ("format", format.as_str()))
        .into_iter()
        .collect()
}

/// Whether a require answer is an OpenAPI document (a top-level `openapi`
/// key). A service predating `format` ignores the parameter and answers in
/// the entry's own format instead.
fn is_openapi_document(body: &str) -> bool {
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(body)
        .ok()
        .and_then(|doc| doc.as_mapping().map(|m| m.contains_key("openapi")))
        .unwrap_or(false)
}

/// Whether a failed `format: openapi` require is a pre-2.4.0 service refusing
/// the parameter: its single route answers `400` naming `format` (unknown
/// query field). A 2.4.0+ service's own `400` passes through unchanged.
fn rejected_format_on_old_server(
    entry: &RequireEntry,
    status: u16,
    body: &str,
    server_version: Option<&str>,
) -> bool {
    entry.format == Some(RequireFormat::OpenApi)
        && status == 400
        && body.to_ascii_lowercase().contains("format")
        && server_version
            .and_then(major_minor)
            .is_none_or(|v| v < (2, 4))
}

/// `MAJOR.MINOR` of a reported version, `None` when it does not parse.
fn major_minor(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

/// The failure for a `format: openapi` answer that is not OpenAPI.
pub(crate) fn format_unsupported_message(server_version: Option<&str>) -> String {
    match server_version {
        Some(version) => {
            format!("format: openapi needs Sanshain Service 2.4.0 or later (server is {version})")
        }
        None => "format: openapi needs Sanshain Service 2.4.0 or later (server version unknown)"
            .to_string(),
    }
}

/// Builds the JSON payload for a `POST /require-bundle` call. Unlike the
/// single-endpoint GET, this endpoint reads the stream from the body — the
/// server's bundle handler has no query extractor, so query parameters would be
/// silently ignored and a trunk build would record no trunk pins.
pub(crate) fn require_bundle_payload(
    consumername: &str,
    entry: &RequireEntry,
    stream: &Stream,
) -> Value {
    let mut payload = json!({
        "consumername": consumername,
        "producername": entry.service_name,
        "version": entry.version,
        "api_type": entry.api_type.as_str(),
        "endpoints": entry
            .endpoints
            .iter()
            .map(|e| json!({ "path": e.path, "method": e.method }))
            .collect::<Vec<_>>(),
    });
    if let Some(map) = payload.as_object_mut() {
        stream.apply_to_payload(map);
    }
    payload
}

fn fallback_message(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "(no details from server)".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Formats a failed provide. A 409 surfaces the server's message and the
/// `proposed_version` — the fix is always a version bump in your own spec file;
/// the client never modifies it.
pub(crate) fn provide_error_message(
    status: u16,
    body: &str,
    api_type: ApiType,
    spec_file: &Path,
) -> String {
    let parsed: Option<ErrorBody> = serde_json::from_str(body).ok();
    let server_message = parsed
        .as_ref()
        .map_or_else(|| fallback_message(body), |b| b.error.clone());
    match status {
        409 => {
            let mut message = format!("provide rejected (409): {server_message}");
            if let Some(proposed) = parsed.and_then(|b| b.proposed_version) {
                message.push_str(&format!(
                    "\nPublish as {proposed} — update {} in {} and provide again.",
                    api_type.version_slot(),
                    spec_file.display()
                ));
            }
            message
        }
        400 => format!("provide rejected (400, invalid document): {server_message}"),
        422 => format!("provide rejected (422, wrong request shape): {server_message}"),
        // Releasing is a permission, not merely an authenticated act, and the
        // remedy is a role grant — nothing the Producer can change in its own
        // repository. Naming it is the whole value of catching this status.
        403 => format!(
            "provide refused (403): {server_message}\nPublishing a GA version requires the 'releaser' role. Grant it to the user or token this build authenticates as (administrators and root always hold it), or publish as a snapshot by leaving --ga / {GA_ENV_VAR} unset."
        ),
        _ => format!("provide failed ({status}): {server_message}"),
    }
}

/// Formats a failed retire. Retiring is gated like releasing, but reachable two
/// ways, so the refusal names both.
pub(crate) fn retire_error_message(status: u16, body: &str, producername: &str) -> String {
    let parsed: Option<ErrorBody> = serde_json::from_str(body).ok();
    let server_message = parsed.map_or_else(|| fallback_message(body), |b| b.error);
    match status {
        403 => format!(
            "retire refused (403): {server_message}\nRetiring an API family requires the 'releaser' role — which a release pipeline already holds — or a maintainer grant on '{producername}'. Administrators and root hold both."
        ),
        404 => format!(
            "retire failed (404): {server_message}\nNothing was ever provided as '{producername}', so there is no family to retire."
        ),
        _ => format!("retire failed ({status}): {server_message}"),
    }
}

/// Formats a failed require, keeping the two failure modes distinct:
/// 404 = the producer or pinned version is unknown (a config error, fix the pin);
/// 410 = the version exists but deliberately lacks the endpoint(s).
pub(crate) fn require_error_message(
    status: u16,
    body: &str,
    entry: &RequireEntry,
    base_url: &str,
) -> String {
    let parsed: Option<ErrorBody> = serde_json::from_str(body).ok();
    let server_message = parsed.map_or_else(|| fallback_message(body), |b| b.error);
    let producer = &entry.service_name;
    let version = &entry.version;
    match status {
        404 => format!(
            "require failed (404, unknown): {server_message}\n'{producer}' has no version {version} on the server — the pin is a configuration error; fix 'version' in the requires entry (list available versions: GET {base_url}/producers/{producer}/versions)."
        ),
        410 => format!(
            "require failed (410, absent): {server_message}\nVersion {version} of '{producer}' exists but deliberately does not include the requested endpoint(s) — pin a version that provides them."
        ),
        _ => format!("require failed ({status}): {server_message}"),
    }
}

/// What the server recorded for a catalog push.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogResponse {
    pub system: String,
    /// Whether the main graph was updated as well as the dev graph.
    pub trunk: bool,
    /// The recorded refs, `resource:<namespace>/<name>`, lower-cased and sorted.
    pub resources: Vec<String>,
    /// Whether anything was recorded. `false` when Sanshain could not read
    /// the file as this project's Component — the previous periphery stays,
    /// and a [`WARNING_HEADER`] names the reason.
    #[serde(default = "recorded_by_default")]
    pub recorded: bool,
}

/// A service predating the `recorded` field only answered `200` on success.
fn recorded_by_default() -> bool {
    true
}

/// Builds the JSON payload for a catalog push. `trunk` appears only for a
/// trunk build, and a tag never does: the payload denies unknown fields.
pub(crate) fn catalog_payload(content: &str, stream: &Stream) -> Value {
    let mut payload = json!({ "catalog_info_yaml": content });
    if stream.is_trunk() {
        if let Some(map) = payload.as_object_mut() {
            map.insert("trunk".to_string(), Value::Bool(true));
        }
    }
    payload
}

/// Formats a failed catalog push. A `404` means the service predates the
/// catalog endpoint; a file Sanshain cannot read is no failure but a `200`
/// with `recorded: false` and a warning.
pub(crate) fn catalog_error_message(
    status: u16,
    body: &str,
    server_version: Option<&str>,
) -> String {
    let parsed: Option<ErrorBody> = serde_json::from_str(body).ok();
    let server_message = parsed.map_or_else(|| fallback_message(body), |b| b.error);
    match status {
        404 => match server_version {
            Some(version) => format!(
                "sanshain catalog needs Sanshain Service 2.4.0 or later (server is {version})"
            ),
            None => {
                "sanshain catalog needs Sanshain Service 2.4.0 or later (server version unknown)"
                    .to_string()
            }
        },
        _ => format!("catalog failed ({status}): {server_message}"),
    }
}

/// Percent-encodes one URL path segment, keeping only RFC 3986 unreserved
/// characters as they are.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// If the reported instance version is pre-2.0, the message that replaces the
/// original (confusing) error.
pub(crate) fn wrong_server_message(base_url: &str, reported: &str) -> Option<String> {
    let major: u64 = reported.split('.').next()?.trim().parse().ok()?;
    (major < 2).then(|| {
        format!(
            "Sanshain server at {base_url} is {reported}; this client requires Sanshain 2.x — upgrade the server."
        )
    })
}

/// HTTP client for one Sanshain Service instance.
pub struct Client {
    base_url: String,
    token: Option<String>,
    stream: Stream,
    /// Every [`WARNING_HEADER`] value received and not yet taken.
    warnings: Mutex<Vec<String>>,
}

impl Client {
    /// Creates a client for the given base URL; `token` is sent as
    /// `Authorization: Bearer` when present. The stream defaults to
    /// [`Stream::None`] — see [`Client::with_stream`].
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            token,
            stream: Stream::None,
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// Declares which graph this build's calls belong to.
    ///
    /// Set once for the whole run rather than per call: the stream is a property
    /// of the invocation, unlike `stability`, which genuinely varies between one
    /// provide and the next.
    pub fn with_stream(mut self, stream: Stream) -> Self {
        self.stream = stream;
        self
    }

    /// The stream this client declares on every call.
    pub fn stream(&self) -> &Stream {
        &self.stream
    }

    /// Creates a client from the configuration plus the `SANSHAIN_URL` /
    /// `SANSHAIN_TOKEN` environment variables.
    pub fn from_config(config: &Config) -> Result<Self> {
        let url = resolve_url(
            config.sanshain_url.as_deref(),
            std::env::var(URL_ENV_VAR).ok().as_deref(),
        )?;
        Ok(Self::new(url, std::env::var(TOKEN_ENV_VAR).ok()))
    }

    /// Every [`WARNING_HEADER`] line the server sent since the last call of
    /// this, in order, success or failure alike. A warning never fails a
    /// call by itself; printing it, and failing under strict mode, is the
    /// caller's job.
    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.lock_warnings())
    }

    fn lock_warnings(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        // A panic elsewhere cannot leave a Vec<String> half-written.
        self.warnings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the response's warnings — read before the body consumes it.
    fn record_warnings(&self, response: &ureq::Response) {
        let found = response.all(WARNING_HEADER);
        if !found.is_empty() {
            self.lock_warnings()
                .extend(found.into_iter().map(str::to_string));
        }
    }

    fn with_auth(&self, request: ureq::Request) -> ureq::Request {
        match &self.token {
            Some(token) => request.set("Authorization", &format!("Bearer {token}")),
            None => request,
        }
    }

    /// Provides one spec file under the declared stability. The version is read
    /// from the spec file by the server.
    pub fn provide(
        &self,
        producername: &str,
        entry: &ProvideEntry,
        stability: Stability,
    ) -> Result<ProvideResponse> {
        let content = fs::read_to_string(&entry.file)?;
        let payload = provide_payload(
            producername,
            entry.api_type,
            &content,
            stability,
            &self.stream,
        );
        let url = format!("{}{}", self.base_url, entry.api_type.provide_path());
        match self.with_auth(ureq::post(&url)).send_json(payload) {
            Ok(response) => {
                self.record_warnings(&response);
                Ok(response.into_json()?)
            }
            Err(err) => Err(self.map_failure(err, |status, body, _| {
                provide_error_message(status, body, entry.api_type, &entry.file)
            })),
        }
    }

    /// Declares that this Producer no longer provides an API family.
    ///
    /// The call is an ordinary provide for that family carrying `retired: true`
    /// and no document — the endpoint already names the family. The capability
    /// tag is cleared, the family leaves the current dependency graph and its
    /// AsyncAPI channel contracts are released; version history and existing
    /// Consumer pins are untouched.
    ///
    /// Gated like releasing: the caller needs the `releaser` role — which a
    /// release pipeline already holds — or a maintainer grant on this Producer,
    /// or the server answers `403`.
    pub fn retire(&self, producername: &str, api_type: ApiType) -> Result<RetiredProtocol> {
        let url = format!("{}{}", self.base_url, api_type.provide_path());
        match self
            .with_auth(ureq::post(&url))
            .send_json(retire_payload(producername))
        {
            Ok(response) => {
                self.record_warnings(&response);
                Ok(response.into_json()?)
            }
            Err(err) => Err(self.map_failure(err, |status, body, _| {
                retire_error_message(status, body, producername)
            })),
        }
    }

    /// Requires the pinned endpoints of one entry and writes the snippet to
    /// `outputDirectory`. Returns the path of the written file. A single
    /// endpoint uses `GET /require*`; several use `POST /require-bundle`.
    pub fn require(&self, consumername: &str, entry: &RequireEntry) -> Result<PathBuf> {
        let result = if entry.endpoints.len() == 1 {
            let url = format!("{}{}", self.base_url, entry.api_type.require_path());
            let mut request = self.with_auth(ureq::get(&url));
            for (key, value) in
                require_query(consumername, entry, &entry.endpoints[0], &self.stream)
            {
                request = request.query(key, value);
            }
            request.call()
        } else {
            let url = format!("{}/require-bundle", self.base_url);
            let mut request = self.with_auth(ureq::post(&url));
            for (key, value) in format_query(entry) {
                request = request.query(key, value);
            }
            request.send_json(require_bundle_payload(consumername, entry, &self.stream))
        };
        match result {
            Ok(response) => {
                self.record_warnings(&response);
                let body = response.into_string()?;
                // Checked before anything is written: an older service that
                // ignored `format` must not leave AsyncAPI where the build
                // expects OpenAPI.
                if entry.format == Some(RequireFormat::OpenApi) && !is_openapi_document(&body) {
                    return Err(Error::Api(format_unsupported_message(
                        self.server_version().as_deref(),
                    )));
                }
                fs::create_dir_all(&entry.output_directory)?;
                let out = entry.output_directory.join(format!(
                    "{}.{}",
                    entry.service_name,
                    entry.api_type.file_extension()
                ));
                fs::write(&out, body)?;
                Ok(out)
            }
            Err(err) => Err(self.map_failure(err, |status, body, server_version| {
                if rejected_format_on_old_server(entry, status, body, server_version) {
                    format_unsupported_message(server_version)
                } else {
                    require_error_message(status, body, entry, &self.base_url)
                }
            })),
        }
    }

    /// Pushes the project's Backstage `catalog-info.yaml` with
    /// `PUT /catalog/<serviceName>`, so the network graph shows the peripheral
    /// resources the system depends on. Returns what the server recorded.
    ///
    /// Only a trunk stream is sent (`trunk: true` updates the main graph as
    /// well as the dev graph); a tag is never sent — the server refuses
    /// unknown fields. Needs Sanshain Service 2.4.0 or later: an older 2.x
    /// service answers `404`, reported as such. A file Sanshain cannot read
    /// as this project's Component is no error: the answer has
    /// [`CatalogResponse::recorded`] `false` and a warning (see
    /// [`Client::take_warnings`]).
    pub fn catalog(&self, service_name: &str, catalog_info: &Path) -> Result<CatalogResponse> {
        let content = fs::read_to_string(catalog_info).map_err(|e| {
            Error::Config(format!(
                "cannot read catalogInfo {}: {e}",
                catalog_info.display()
            ))
        })?;
        let url = format!(
            "{}/catalog/{}",
            self.base_url,
            encode_path_segment(service_name)
        );
        match self
            .with_auth(ureq::put(&url))
            .send_json(catalog_payload(&content, &self.stream))
        {
            Ok(response) => {
                self.record_warnings(&response);
                Ok(response.into_json()?)
            }
            Err(err) => Err(self.map_failure(err, |status, body, server_version| {
                catalog_error_message(status, body, server_version)
            })),
        }
    }

    /// On any failure, does one lazy `GET /version`: a pre-2.0 instance replaces
    /// the confusing error with a clear upgrade message. There is no preflight —
    /// successful calls never trigger this. `describe` also gets the reported
    /// version, when the server named one.
    fn map_failure<F>(&self, err: ureq::Error, describe: F) -> Error
    where
        F: FnOnce(u16, &str, Option<&str>) -> String,
    {
        if let ureq::Error::Status(_, response) = &err {
            self.record_warnings(response);
        }
        let server_version = self.server_version();
        if let Some(message) = server_version
            .as_deref()
            .and_then(|reported| wrong_server_message(&self.base_url, reported))
        {
            return Error::WrongServer(message);
        }
        match err {
            ureq::Error::Status(status, response) => {
                let body = response.into_string().unwrap_or_default();
                Error::Api(describe(status, &body, server_version.as_deref()))
            }
            transport => Error::Http(transport.to_string()),
        }
    }

    fn server_version(&self) -> Option<String> {
        let url = format!("{}/version", self.base_url);
        let response = self.with_auth(ureq::get(&url)).call().ok()?;
        let body: Value = response.into_json().ok()?;
        Some(body.get("version")?.as_str()?.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config_value() -> Value {
        json!({
            "sanshainUrl": "http://localhost:3000",
            "serviceName": "gateway-service",
            "provides": [
                { "file": "specs/openapi.yaml" },
                { "file": "specs/gateway.proto", "apiType": "proto" }
            ],
            "requires": [
                {
                    "serviceName": "user-service",
                    "version": "2.3.0",
                    "outputDirectory": "generated/user-service",
                    "endpoints": [
                        { "method": "GET", "path": "/api/v1/users/{id}" }
                    ]
                }
            ]
        })
    }

    fn require_entry() -> RequireEntry {
        RequireEntry {
            service_name: "user-service".to_string(),
            api_type: ApiType::OpenApi,
            version: "2.3.0".to_string(),
            output_directory: PathBuf::from("generated/user-service"),
            endpoints: vec![Endpoint {
                method: "GET".to_string(),
                path: "/api/v1/users/{id}".to_string(),
            }],
            format: None,
        }
    }

    fn config_error(value: &Value) -> String {
        match Config::from_value(value) {
            Err(Error::Config(message)) => message,
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    #[test]
    fn parses_valid_config() {
        let config = match Config::from_value(&valid_config_value()) {
            Ok(config) => config,
            Err(err) => panic!("expected valid config, got {err}"),
        };
        assert_eq!(config.service_name.as_deref(), Some("gateway-service"));
        assert_eq!(
            config.sanshain_url.as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(config.provides.len(), 2);
        assert_eq!(config.provides[0].api_type, ApiType::OpenApi);
        assert_eq!(config.provides[1].api_type, ApiType::Proto);
        assert_eq!(config.requires.len(), 1);
        assert_eq!(config.requires[0].version, "2.3.0");
        assert_eq!(config.requires[0].endpoints[0].method, "GET");
    }

    #[test]
    fn supports_single_provide_object() {
        let value = json!({
            "serviceName": "user-service",
            "provide": { "file": "specs/openapi.yaml" }
        });
        let config = match Config::from_value(&value) {
            Ok(config) => config,
            Err(err) => panic!("expected valid config, got {err}"),
        };
        assert_eq!(config.provides.len(), 1);
        assert_eq!(config.provides[0].file, PathBuf::from("specs/openapi.yaml"));
    }

    #[test]
    fn missing_version_in_requires_names_entry_and_hints_migration() {
        let value = json!({
            "serviceName": "gateway-service",
            "requires": [
                {
                    "serviceName": "user-service",
                    "outputDirectory": "generated/user-service",
                    "endpoints": [{ "method": "GET", "path": "/api/v1/users/{id}" }]
                }
            ]
        });
        let message = config_error(&value);
        assert_eq!(
            message,
            "requires[0] (user-service): missing 'version' — Sanshain 2.0 pins exact versions; add version = \"1.2.0\" (list available: GET /producers/user-service/versions)"
        );
    }

    #[test]
    fn branch_era_fields_are_rejected_by_name_with_hint() {
        let mut requires_entry = json!({
            "serviceName": "user-service",
            "version": "1.2.0",
            "outputDirectory": "generated/user-service",
            "endpoints": [{ "method": "GET", "path": "/x" }]
        });
        let cases = [
            ("branch", "replace it with an exact 'version' pin"),
            ("timeout", "instead of long-polling"),
            ("baseVersion", "dropped optimistic concurrency"),
        ];
        for (field, hint) in cases {
            if let Some(obj) = requires_entry.as_object_mut() {
                obj.insert(field.to_string(), json!("something"));
            }
            let value = json!({
                "serviceName": "gateway-service",
                "requires": [requires_entry.clone()]
            });
            let message = config_error(&value);
            assert!(
                message.contains(&format!("'{field}' is no longer supported")),
                "message for {field} should name the field: {message}"
            );
            assert!(
                message.contains(hint),
                "message for {field} should hint the migration: {message}"
            );
            assert!(
                message.starts_with("requires[0] (user-service): "),
                "message for {field} should name the entry: {message}"
            );
            if let Some(obj) = requires_entry.as_object_mut() {
                obj.remove(field);
            }
        }
    }

    #[test]
    fn release_branches_rejected_at_top_level() {
        let mut value = valid_config_value();
        if let Some(obj) = value.as_object_mut() {
            obj.insert("releaseBranches".to_string(), json!(["main", "master"]));
        }
        let message = config_error(&value);
        assert_eq!(
            message,
            "'releaseBranches' is no longer supported — stability no longer derives from git branches; snapshot is the default and GA is an explicit switch (SANSHAIN_GA=true or --ga)"
        );
    }

    #[test]
    fn stability_in_provides_entry_rejected() {
        let value = json!({
            "serviceName": "gateway-service",
            "provides": [{ "file": "specs/openapi.yaml", "stability": "ga" }]
        });
        let message = config_error(&value);
        assert_eq!(
            message,
            "provides[0]: 'stability' is not configured in sanshain.yaml — snapshot is the default; publish GA explicitly via SANSHAIN_GA=true or --ga"
        );
    }

    #[test]
    fn legacy_client_fields_rejected_with_hints() {
        let cases = [
            ("spec_id", "'serviceName' and the exact 'version'"),
            ("api_key", "SANSHAIN_TOKEN environment variable"),
            ("service_url", "'sanshainUrl'"),
        ];
        for (field, hint) in cases {
            let mut value = valid_config_value();
            if let Some(obj) = value.as_object_mut() {
                obj.insert(field.to_string(), json!("legacy"));
            }
            let message = config_error(&value);
            assert!(
                message.contains(&format!("'{field}' is no longer supported")),
                "message for {field} should name the field: {message}"
            );
            assert!(
                message.contains(hint),
                "message for {field} should hint the migration: {message}"
            );
        }
    }

    #[test]
    fn non_semver_pin_rejected() {
        let value = json!({
            "serviceName": "gateway-service",
            "requires": [
                {
                    "serviceName": "user-service",
                    "version": "latest",
                    "outputDirectory": "generated/user-service",
                    "endpoints": [{ "method": "GET", "path": "/x" }]
                }
            ]
        });
        let message = config_error(&value);
        assert_eq!(
            message,
            "requires[0] (user-service): version 'latest' is not a version — pin MAJOR[.MINOR[.PATCH]], optionally v-prefixed (no ranges, no 'latest')"
        );
    }

    #[test]
    fn empty_endpoints_rejected() {
        let value = json!({
            "serviceName": "gateway-service",
            "requires": [
                {
                    "serviceName": "user-service",
                    "version": "1.0.0",
                    "outputDirectory": "generated/user-service",
                    "endpoints": []
                }
            ]
        });
        let message = config_error(&value);
        assert_eq!(
            message,
            "requires[0] (user-service): 'endpoints' must list at least one endpoint (method + path)"
        );
    }

    /// The command decides what a missing name means (warn and skip, or fail
    /// under strict), so parsing accepts it.
    #[test]
    fn missing_service_name_parses_as_absent() {
        let value = json!({ "provides": [{ "file": "specs/openapi.yaml" }] });
        match Config::from_value(&value) {
            Ok(config) => assert_eq!(config.service_name, None),
            Err(err) => panic!("expected the config to parse, got {err}"),
        }
    }

    #[test]
    fn strict_and_best_effort_default_off_and_load_from_yaml() {
        let plain = Config::from_yaml_str("serviceName: s\n").ok();
        assert_eq!(
            plain.as_ref().map(|c| (c.strict, c.best_effort)),
            Some((false, false))
        );
        let both = Config::from_yaml_str("serviceName: s\nstrict: true\nbestEffort: true\n").ok();
        assert_eq!(both.map(|c| (c.strict, c.best_effort)), Some((true, true)));
    }

    fn unknown(yaml: &str) -> Vec<String> {
        match Config::from_yaml_str(yaml) {
            Ok(config) => config.unknown_fields,
            Err(err) => panic!("expected the config to parse, got {err}"),
        }
    }

    #[test]
    fn a_single_unknown_field_gets_the_cat() {
        let fields = unknown("serviceName: s\ncatalogInf: catalog-info.yaml\n");
        assert_eq!(fields, vec!["catalogInf".to_string()]);
        assert_eq!(
            puzzled_cat(&fields, false).as_deref(),
            Some(
                "   /\\_/\\  ?\n  ( O_o )   sanshain.yaml: unknown field 'catalogInf' - ignored\n ---U-U---"
            )
        );
    }

    #[test]
    fn several_unknown_fields_are_listed_in_file_order_with_their_paths() {
        let fields = unknown(
            r#"
zebra: 1
serviceName: s
provide:
  file: a.yaml
  fiel: b.yaml
provides:
  - file: c.yaml
  - file: d.yaml
    verison: 1.0.0
requires:
  - serviceName: u
    version: 1.0.0
    outputDirectroy: out
    outputDirectory: out
    endpoints:
      - method: GET
        path: /a
      - method: GET
        path: /b
      - methd: GET
        method: GET
        path: /c
catalogInf: x
"#,
        );
        assert_eq!(
            fields,
            vec![
                "zebra",
                "provide.fiel",
                "provides[1].verison",
                "requires[0].outputDirectroy",
                "requires[0].endpoints[2].methd",
                "catalogInf",
            ]
        );
        assert_eq!(
            puzzled_cat(&fields[..2], false).as_deref(),
            Some(
                "   /\\_/\\  ?\n  ( O_o )   sanshain.yaml: unknown fields 'zebra', 'provide.fiel' - ignored\n ---U-U---"
            )
        );
    }

    #[test]
    fn under_strict_the_cat_rejects() {
        let cat = puzzled_cat(&["catalogInf".to_string()], true);
        assert_eq!(
            cat.as_deref(),
            Some(
                "   /\\_/\\  ?\n  ( O_o )   sanshain.yaml: unknown field 'catalogInf' - rejected (strict)\n ---U-U---"
            )
        );
    }

    #[test]
    fn fields_another_client_knows_get_no_cat() {
        let fields = unknown(
            r#"
sanshainUrl: http://x
serviceName: s
clientName: c
compression: true
bestEffort: false
strict: false
force: true
insecure: true
combine: true
catalogInfo: catalog-info.yaml
provide:
  file: a.yaml
  apiType: openapi
  retired: false
  openApiFile: a.yaml
  asyncApiFile: b.yaml
  protoFile: c.proto
  combine: true
requires:
  - serviceName: u
    apiType: openapi
    version: 1.0.0
    outputDirectory: out
    endpoints:
      - method: GET
        path: /a
"#,
        );
        assert!(fields.is_empty(), "got: {fields:?}");
        assert_eq!(puzzled_cat(&fields, true), None);
    }

    #[test]
    fn a_legacy_field_still_errors_before_any_cat() {
        let err = Config::from_yaml_str(
            "serviceName: s\ncatalogInf: x\nrequires:\n  - serviceName: u\n    branch: main\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /a\n",
        );
        match err {
            Err(Error::Config(message)) => {
                assert!(
                    message.contains("'branch' is no longer supported"),
                    "{message}"
                );
            }
            other => panic!("expected the legacy error, got {other:?}"),
        }
    }

    #[test]
    fn catalog_payload_sends_trunk_only_for_trunk_and_never_a_tag() {
        let trunk = catalog_payload("kind: Component", &Stream::Trunk);
        assert_eq!(
            trunk,
            json!({ "catalog_info_yaml": "kind: Component", "trunk": true })
        );
        let plain = catalog_payload("kind: Component", &Stream::None);
        assert_eq!(plain, json!({ "catalog_info_yaml": "kind: Component" }));
        let tagged = catalog_payload("kind: Component", &Stream::Tag("R".to_string()));
        assert_eq!(tagged, json!({ "catalog_info_yaml": "kind: Component" }));
    }

    #[test]
    fn catalog_errors_name_the_fix() {
        // A 400 is only an old pre-release service's answer: no special case.
        assert_eq!(
            catalog_error_message(
                400,
                r#"{"error":"no Component named 'svc'"}"#,
                Some("2.4.0")
            ),
            "catalog failed (400): no Component named 'svc'"
        );
        assert_eq!(
            catalog_error_message(404, "", Some("2.3.1")),
            "sanshain catalog needs Sanshain Service 2.4.0 or later (server is 2.3.1)"
        );
        assert_eq!(
            catalog_error_message(404, "", None),
            "sanshain catalog needs Sanshain Service 2.4.0 or later (server version unknown)"
        );
        assert_eq!(
            catalog_error_message(413, r#"{"error":"too large"}"#, None),
            "catalog failed (413): too large"
        );
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(
            encode_path_segment("orders-service_1.x~"),
            "orders-service_1.x~"
        );
        assert_eq!(encode_path_segment("a b/c?ä"), "a%20b%2Fc%3F%C3%A4");
    }

    #[test]
    fn catalog_info_loads_from_yaml() {
        let config = Config::from_yaml_str("serviceName: s\ncatalogInfo: catalog-info.yaml\n").ok();
        assert_eq!(
            config.and_then(|c| c.catalog_info),
            Some(PathBuf::from("catalog-info.yaml"))
        );
    }

    #[test]
    fn unknown_fields_are_known_even_when_a_later_validation_fails() {
        let loaded = match Config::load_yaml_str(
            "serviceName: s\nrequires:\n  - serviceName: u\n    verison: 1.2.0\n    outputDirectory: out\n    endpoints:\n      - method: GET\n        path: /a\n",
        ) {
            Ok(loaded) => loaded,
            Err(err) => panic!("only the later validation should fail, got {err}"),
        };
        assert_eq!(loaded.unknown_fields, vec!["requires[0].verison"]);
        match loaded.config {
            Err(Error::Config(message)) => {
                assert!(message.contains("missing 'version'"), "{message}");
            }
            other => panic!("expected the missing-version error, got {other:?}"),
        }
    }

    #[test]
    fn strict_in_the_file_is_readable_when_validation_fails() {
        let loaded = Config::load_yaml_str("strict: true\nrequires:\n  - serviceName: u\n");
        assert!(loaded.as_ref().is_ok_and(|l| l.strict && l.config.is_err()));
    }

    fn asyncapi_entry_yaml(extra: &str) -> String {
        format!(
            "serviceName: s\nrequires:\n  - serviceName: events\n    version: 1.0.0\n    outputDirectory: out\n{extra}    endpoints:\n      - method: SUB\n        path: user/signup\n"
        )
    }

    #[test]
    fn format_openapi_on_an_asyncapi_entry_is_known_and_parsed() {
        let loaded = Config::load_yaml_str(&asyncapi_entry_yaml(
            "    apiType: asyncapi\n    format: openapi\n",
        ));
        let Ok(loaded) = loaded else {
            panic!("the config should load");
        };
        assert!(
            loaded.unknown_fields.is_empty(),
            "no cat: {:?}",
            loaded.unknown_fields
        );
        let format = loaded.config.ok().and_then(|c| c.requires[0].format);
        assert_eq!(format, Some(RequireFormat::OpenApi));
        let plain = Config::from_yaml_str(&asyncapi_entry_yaml("    apiType: asyncapi\n")).ok();
        assert_eq!(plain.and_then(|c| c.requires[0].format), None);
    }

    #[test]
    fn format_with_another_api_type_is_a_configuration_error() {
        for extra in [
            "    format: openapi\n",
            "    apiType: proto\n    format: openapi\n",
        ] {
            match Config::from_yaml_str(&asyncapi_entry_yaml(extra)) {
                Err(Error::Config(message)) => {
                    assert!(message.starts_with("requires[0] (events): "), "{message}");
                    assert!(
                        message.contains("'format: openapi' with 'apiType: asyncapi'"),
                        "{message}"
                    );
                }
                other => panic!("expected a configuration error, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_unknown_format_value_is_a_configuration_error() {
        match Config::from_yaml_str(&asyncapi_entry_yaml(
            "    apiType: asyncapi\n    format: json-schema\n",
        )) {
            Err(Error::Config(message)) => {
                assert!(
                    message
                        .starts_with("requires[0] (events): format 'json-schema' is not supported"),
                    "{message}"
                );
            }
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    #[test]
    fn the_format_rides_the_query_only_when_set() {
        let mut entry = require_entry();
        let plain = require_query("c", &entry, &entry.endpoints[0], &Stream::None);
        assert!(plain.iter().all(|(k, _)| *k != "format"), "got: {plain:?}");
        entry.api_type = ApiType::AsyncApi;
        entry.format = Some(RequireFormat::OpenApi);
        let query = require_query("c", &entry, &entry.endpoints[0], &Stream::None);
        assert!(query.contains(&("format", "openapi")), "got: {query:?}");
    }

    #[test]
    fn only_a_pre_2_4_service_rejecting_format_counts_as_too_old() {
        let mut entry = require_entry();
        entry.api_type = ApiType::AsyncApi;
        entry.format = Some(RequireFormat::OpenApi);
        let body = r#"{"error":"unknown field `format`"}"#;
        assert!(rejected_format_on_old_server(
            &entry,
            400,
            body,
            Some("2.3.9")
        ));
        assert!(rejected_format_on_old_server(&entry, 400, body, None));
        assert!(rejected_format_on_old_server(
            &entry,
            400,
            "Unknown FORMAT",
            Some("v2")
        ));
        assert!(!rejected_format_on_old_server(
            &entry,
            400,
            body,
            Some("2.4.0")
        ));
        assert!(!rejected_format_on_old_server(
            &entry,
            400,
            body,
            Some("3.0.0")
        ));
        assert!(!rejected_format_on_old_server(
            &entry,
            400,
            "bad payload",
            Some("2.3.0")
        ));
        assert!(!rejected_format_on_old_server(
            &entry,
            404,
            body,
            Some("2.3.0")
        ));
        entry.format = None;
        assert!(!rejected_format_on_old_server(
            &entry,
            400,
            body,
            Some("2.3.0")
        ));
    }

    #[test]
    fn only_a_document_with_a_top_level_openapi_key_is_openapi() {
        assert!(is_openapi_document("openapi: 3.0.3\ninfo: {}\n"));
        assert!(is_openapi_document(r#"{"openapi":"3.1.0"}"#));
        assert!(!is_openapi_document("asyncapi: 2.6.0\nchannels: {}\n"));
        assert!(!is_openapi_document("info:\n  openapi: 3.0.3\n"));
        assert!(!is_openapi_document("not: [yaml"));
    }

    #[test]
    fn a_set_environment_overrides_the_yaml_switch_and_the_flag_overrides_both() {
        assert!(!resolve_switch_with(false, None, false));
        assert!(resolve_switch_with(false, None, true));
        assert!(resolve_switch_with(false, Some("1"), false));
        assert!(resolve_switch_with(false, Some(" TRUE "), false));
        // A set environment variable decides either way, as in sanshain-js.
        assert!(!resolve_switch_with(false, Some("false"), true));
        assert!(!resolve_switch_with(false, Some("yes"), true));
        // An empty variable counts as unset.
        assert!(resolve_switch_with(false, Some(""), true));
        assert!(resolve_switch_with(true, Some("false"), false));
    }

    #[test]
    fn stability_defaults_to_snapshot() {
        assert_eq!(resolve_stability_with(false, None), Stability::Snapshot);
        assert_eq!(
            resolve_stability_with(false, Some("false")),
            Stability::Snapshot
        );
        assert_eq!(
            resolve_stability_with(false, Some("yes")),
            Stability::Snapshot
        );
    }

    #[test]
    fn ga_switch_via_env_or_flag() {
        assert_eq!(resolve_stability_with(false, Some("true")), Stability::Ga);
        assert_eq!(resolve_stability_with(false, Some("TRUE")), Stability::Ga);
        assert_eq!(resolve_stability_with(false, Some("1")), Stability::Ga);
        assert_eq!(resolve_stability_with(true, None), Stability::Ga);
        // The explicit flag wins even when the environment says otherwise.
        assert_eq!(resolve_stability_with(true, Some("false")), Stability::Ga);
    }

    #[test]
    fn switches_are_true_or_1_trimmed_and_case_insensitive() {
        for on in ["1", "true", "TRUE", " true ", "True\n"] {
            assert!(is_switch_on(Some(on)), "{on:?} should switch on");
        }
        for off in ["yes", "on", "0", "false", "", " ", "truely"] {
            assert!(!is_switch_on(Some(off)), "{off:?} should stay off");
        }
        assert!(!is_switch_on(None));
    }

    #[test]
    fn ga_and_trunk_read_their_switch_the_same_way() {
        for (value, on) in [
            ("1", true),
            ("TRUE", true),
            (" true ", true),
            ("yes", false),
        ] {
            let expected = if on {
                Stability::Ga
            } else {
                Stability::Snapshot
            };
            assert_eq!(resolve_stability_with(false, Some(value)), expected);
            let stream = Some(Stream::resolve_with(false, None, Some(value), None).0);
            let expected = if on { Stream::Trunk } else { Stream::None };
            assert_eq!(stream, Some(expected), "SANSHAIN_TRUNK={value:?}");
        }
    }

    #[test]
    fn provide_payload_carries_stability_and_no_branch_era_fields() {
        let payload = provide_payload(
            "gateway-service",
            ApiType::OpenApi,
            "openapi: 3.0.3",
            Stability::Snapshot,
            &Stream::None,
        );
        assert_eq!(payload["producername"], "gateway-service");
        assert_eq!(payload["openapi_yaml"], "openapi: 3.0.3");
        assert_eq!(payload["stability"], "snapshot");
        let keys: Vec<&str> = payload
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for gone in ["branch", "base_version", "baseVersion", "force", "timeout"] {
            assert!(!keys.contains(&gone), "payload must not carry '{gone}'");
        }
    }

    #[test]
    fn provide_payload_key_follows_api_type() {
        let asyncapi = provide_payload(
            "s",
            ApiType::AsyncApi,
            "asyncapi: 2.6.0",
            Stability::Ga,
            &Stream::None,
        );
        assert_eq!(asyncapi["asyncapi_yaml"], "asyncapi: 2.6.0");
        assert_eq!(asyncapi["stability"], "ga");
        let proto = provide_payload(
            "s",
            ApiType::Proto,
            "syntax = \"proto3\";",
            Stability::Snapshot,
            &Stream::None,
        );
        assert_eq!(proto["proto_content"], "syntax = \"proto3\";");
    }

    #[test]
    fn require_query_carries_version_and_no_branch_era_params() {
        let entry = require_entry();
        let query = require_query(
            "gateway-service",
            &entry,
            &entry.endpoints[0],
            &Stream::None,
        );
        assert_eq!(
            query,
            vec![
                ("consumername", "gateway-service"),
                ("producername", "user-service"),
                ("version", "2.3.0"),
                ("path", "/api/v1/users/{id}"),
                ("method", "GET"),
            ]
        );
        for (key, _) in &query {
            assert!(
                ![
                    "branch",
                    "timeout",
                    "pull_from_branch",
                    "source_protected_branch"
                ]
                .contains(key),
                "query must not carry '{key}'"
            );
        }
    }

    #[test]
    fn require_bundle_payload_shape() {
        let mut entry = require_entry();
        entry.endpoints.push(Endpoint {
            method: "POST".to_string(),
            path: "/api/v1/users".to_string(),
        });
        let payload = require_bundle_payload("gateway-service", &entry, &Stream::None);
        assert_eq!(payload["consumername"], "gateway-service");
        assert_eq!(payload["producername"], "user-service");
        assert_eq!(payload["version"], "2.3.0");
        assert_eq!(payload["api_type"], "openapi");
        assert_eq!(payload["endpoints"][1]["method"], "POST");
        assert_eq!(payload["endpoints"][1]["path"], "/api/v1/users");
        assert!(payload.get("trunk").is_none());
        assert!(payload.get("tag").is_none());
    }

    /// The bundle endpoint reads the stream from the body, not the query — the
    /// server's handler has no query extractor, so a stream on the query string
    /// would be silently dropped and a trunk build would record no trunk pins.
    #[test]
    fn require_bundle_payload_carries_the_stream_in_the_body() {
        let entry = require_entry();
        let trunk = require_bundle_payload("c", &entry, &Stream::Trunk);
        assert_eq!(trunk["trunk"], true);
        let tagged = require_bundle_payload("c", &entry, &Stream::Tag("R".to_string()));
        assert_eq!(tagged["tag"], "R");
    }

    #[test]
    fn conflict_message_surfaces_proposed_version() {
        let body = r#"{"error":"GA 1.4.0 already exists with different content","proposed_version":"1.5.0"}"#;
        let message =
            provide_error_message(409, body, ApiType::OpenApi, Path::new("specs/openapi.yaml"));
        assert_eq!(
            message,
            "provide rejected (409): GA 1.4.0 already exists with different content\nPublish as 1.5.0 — update info.version in specs/openapi.yaml and provide again."
        );
    }

    #[test]
    fn conflict_message_points_at_proto_marker_for_proto() {
        let body = r#"{"error":"already released","proposed_version":"2.0.0"}"#;
        let message = provide_error_message(409, body, ApiType::Proto, Path::new("api.proto"));
        assert!(
            message
                .contains("Publish as 2.0.0 — update the // sanshain-version: marker in api.proto"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn require_unknown_and_absent_messages_are_distinct() {
        let entry = require_entry();
        let unknown = require_error_message(
            404,
            r#"{"error":"unknown version"}"#,
            &entry,
            "http://localhost:3000",
        );
        assert!(
            unknown.contains("404, unknown") && unknown.contains("configuration error"),
            "unexpected 404 message: {unknown}"
        );
        assert!(
            unknown.contains("GET http://localhost:3000/producers/user-service/versions"),
            "404 message should point at the version listing: {unknown}"
        );
        let absent = require_error_message(
            410,
            r#"{"error":"endpoint absent"}"#,
            &entry,
            "http://localhost:3000",
        );
        assert!(
            absent.contains("410, absent") && absent.contains("deliberately"),
            "unexpected 410 message: {absent}"
        );
        assert_ne!(unknown, absent);
    }

    #[test]
    fn wrong_server_message_only_for_pre_2_versions() {
        let message = wrong_server_message("http://old.example", "1.7.3");
        assert_eq!(
            message.as_deref(),
            Some(
                "Sanshain server at http://old.example is 1.7.3; this client requires Sanshain 2.x — upgrade the server."
            )
        );
        assert_eq!(wrong_server_message("http://x", "2.0.0"), None);
        assert_eq!(wrong_server_message("http://x", "3.1.4"), None);
        assert_eq!(wrong_server_message("http://x", "garbage"), None);
    }

    #[test]
    fn url_resolution_prefers_environment() {
        let url = match resolve_url(Some("http://config.example/"), Some("http://env.example")) {
            Ok(url) => url,
            Err(err) => panic!("expected a URL, got {err}"),
        };
        assert_eq!(url, "http://env.example");
        let from_config = match resolve_url(Some("http://config.example/"), None) {
            Ok(url) => url,
            Err(err) => panic!("expected a URL, got {err}"),
        };
        assert_eq!(from_config, "http://config.example");
        match resolve_url(None, None) {
            Err(Error::Config(message)) => {
                assert!(message.contains("'sanshainUrl'"), "unexpected: {message}");
                assert!(message.contains("SANSHAIN_URL"), "unexpected: {message}");
            }
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    // --- 2.2: streams, retire, harvested subscriptions, version leniency ---

    #[test]
    fn stream_defaults_to_none_and_prefers_the_flag_over_the_environment() {
        assert_eq!(
            Stream::resolve_with(false, None, None, None),
            (Stream::None, None)
        );
        assert_eq!(
            Stream::resolve_with(false, None, Some("true"), None),
            (Stream::Trunk, None)
        );
        assert_eq!(
            Stream::resolve_with(false, Some("from-flag"), None, Some("from-env")),
            (Stream::Tag("from-flag".to_string()), None)
        );
        assert_eq!(
            Stream::resolve_with(false, None, None, Some("R")),
            (Stream::Tag("R".to_string()), None)
        );
        // A blank tag declares nothing rather than a branch named "".
        assert_eq!(
            Stream::resolve_with(false, Some("   "), None, None),
            (Stream::None, None)
        );
    }

    #[test]
    fn declaring_both_trunk_and_tag_records_neither_with_a_warning() {
        let neither = (
            Stream::None,
            Some("trunk and tag are mutually exclusive - this build is recorded in neither"),
        );
        assert_eq!(Stream::resolve_with(true, Some("R"), None, None), neither);
        // The collision is caught across sources too, not only within one.
        assert_eq!(Stream::resolve_with(true, None, None, Some("R")), neither);
        assert_eq!(
            Stream::resolve_with(false, Some("R"), Some("1"), None),
            neither
        );
    }

    #[test]
    fn undeclared_stream_sends_no_fields_at_all() {
        let payload = provide_payload(
            "svc",
            ApiType::OpenApi,
            "openapi: 3.0.3",
            Stability::Snapshot,
            &Stream::None,
        );
        let keys: Vec<&str> = payload
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert!(!keys.contains(&"trunk"), "got: {keys:?}");
        assert!(!keys.contains(&"tag"), "got: {keys:?}");
        assert!(
            require_query(
                "c",
                &require_entry(),
                &require_entry().endpoints[0],
                &Stream::None
            )
            .iter()
            .all(|(k, _)| *k != "trunk" && *k != "tag")
        );
    }

    #[test]
    fn a_declared_stream_rides_the_payload_and_the_query() {
        let trunk = provide_payload(
            "svc",
            ApiType::OpenApi,
            "openapi: 3.0.3",
            Stability::Snapshot,
            &Stream::Trunk,
        );
        assert_eq!(trunk["trunk"], true);
        let tagged = provide_payload(
            "svc",
            ApiType::OpenApi,
            "openapi: 3.0.3",
            Stability::Snapshot,
            &Stream::Tag("Maribou".to_string()),
        );
        assert_eq!(tagged["tag"], "Maribou");

        let entry = require_entry();
        let query = require_query("c", &entry, &entry.endpoints[0], &Stream::Trunk);
        assert!(query.contains(&("trunk", "true")), "got: {query:?}");
        let branch = Stream::Tag("R".to_string());
        let tagged = require_query("c", &entry, &entry.endpoints[0], &branch);
        assert!(tagged.contains(&("tag", "R")), "got: {tagged:?}");
    }

    /// Content is compared byte for byte, so a CRLF checkout must hash the same
    /// as an LF one — otherwise the same commit conflicts with itself depending
    /// on which runner published it.
    #[test]
    fn provided_content_is_normalized_to_lf() {
        let payload = provide_payload(
            "svc",
            ApiType::OpenApi,
            "openapi: 3.0.3\r\ninfo:\r\n  title: T\r",
            Stability::Snapshot,
            &Stream::None,
        );
        assert_eq!(
            payload["openapi_yaml"],
            "openapi: 3.0.3\ninfo:\n  title: T\n"
        );
        assert_eq!(normalize_line_endings("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn a_retire_payload_cannot_carry_a_document_a_stability_or_a_stream() {
        let payload = retire_payload("svc");
        assert_eq!(payload["producername"], "svc");
        assert_eq!(payload["retired"], true);
        let keys: Vec<&str> = payload
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys.len(), 2, "got: {keys:?}");
    }

    #[test]
    fn version_pins_accept_short_and_v_prefixed_forms() {
        for accepted in ["1.2.3", "v1.2.3", "2.1", "v2", "0.0.0"] {
            assert!(is_pinnable_version(accepted), "should accept {accepted}");
        }
        for refused in ["latest", "", "1.2.3.4", "1..3", "v", "1.x", "^1.2.0"] {
            assert!(!is_pinnable_version(refused), "should refuse {refused}");
        }
    }

    #[test]
    fn a_ga_refusal_names_the_releaser_role_and_the_snapshot_way_out() {
        let message = provide_error_message(
            403,
            r#"{"error":"publishing GA for 'svc' requires the 'releaser' role"}"#,
            ApiType::OpenApi,
            Path::new("openapi.yaml"),
        );
        assert!(message.contains("releaser"), "unexpected: {message}");
        assert!(message.contains("snapshot"), "unexpected: {message}");
    }

    #[test]
    fn a_retire_refusal_names_both_ways_in() {
        let message = retire_error_message(
            403,
            r#"{"error":"retiring requires the 'releaser' role"}"#,
            "svc",
        );
        assert!(message.contains("releaser"), "unexpected: {message}");
        assert!(message.contains("maintainer"), "unexpected: {message}");
        let unknown =
            retire_error_message(404, r#"{"error":"Producer 'ghost' not found"}"#, "ghost");
        assert!(
            unknown.contains("no family to retire"),
            "unexpected: {unknown}"
        );
    }

    #[test]
    fn harvested_subscriptions_separate_advisories_from_resolved_edges() {
        let response: ProvideResponse = serde_json::from_str(
            r#"{"version":"1.0.0","stability":"snapshot","content_hash":"sha256:abc",
                "changes":{"inserts":0,"updates":0,"deletes":0},
                "harvested_subscriptions":[
                  {"channel":"user/signup","message_name":"UserSignedUp","owner":"accounts"},
                  {"channel":"order/placed","message_name":"OrderPlaced","drift":"expects 'total'"}]}"#,
        )
        .expect("response should parse");
        assert_eq!(response.harvested_subscriptions.len(), 2);

        let resolved = &response.harvested_subscriptions[0];
        assert!(!resolved.is_advisory());
        assert!(resolved.to_string().contains("accounts"));

        let drifting = &response.harvested_subscriptions[1];
        assert!(drifting.is_advisory(), "drift is an advisory");
        assert!(drifting.to_string().contains("no publisher yet"));
        assert!(drifting.to_string().contains("total"));
    }

    /// Absent for OpenAPI and for documents declaring none — never a parse error.
    #[test]
    fn a_response_without_harvested_subscriptions_still_parses() {
        let response: ProvideResponse = serde_json::from_str(
            r#"{"version":"1.0.0","stability":"ga","content_hash":"sha256:abc",
                "changes":{"inserts":1,"updates":0,"deletes":0}}"#,
        )
        .expect("response should parse");
        assert!(response.harvested_subscriptions.is_empty());
    }

    // --- sanshain.yaml ---

    #[test]
    fn config_loads_from_yaml_like_every_other_client() {
        let config = Config::from_yaml_str(
            r#"
serviceName: order-service
sanshainUrl: https://sanshain.example.com
provides:
  - file: api/openapi.yaml
  - file: api/events.yaml
    apiType: asyncapi
requires:
  - serviceName: user-service
    version: v2.3
    outputDirectory: target/generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users
"#,
        )
        .expect("config should parse");
        assert_eq!(config.service_name.as_deref(), Some("order-service"));
        assert_eq!(
            config.sanshain_url.as_deref(),
            Some("https://sanshain.example.com")
        );
        assert_eq!(config.provides.len(), 2);
        assert_eq!(config.provides[1].api_type, ApiType::AsyncApi);
        // The short pin reaches the server verbatim; only it canonicalizes.
        assert_eq!(config.requires[0].version, "v2.3");
    }

    #[test]
    fn a_retired_entry_is_config_not_a_publish() {
        let config = Config::from_yaml_str(
            r#"
serviceName: order-service
sanshainUrl: http://localhost:8080
provides:
  - file: api/events.yaml
    apiType: asyncapi
    retired: true
"#,
        )
        .expect("config should parse");
        assert!(config.provides[0].retired);
        assert_eq!(config.provides[0].api_type, ApiType::AsyncApi);
    }

    #[test]
    fn branch_era_yaml_is_still_refused_by_name() {
        let err = Config::from_yaml_str(
            r#"
serviceName: order-service
sanshainUrl: http://localhost:8080
requires:
  - serviceName: user-service
    branch: main
    outputDirectory: out
    endpoints:
      - method: GET
        path: /users
"#,
        );
        match err {
            Err(Error::Config(message)) => {
                assert!(message.contains("'branch'"), "unexpected: {message}");
            }
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    #[test]
    fn malformed_yaml_names_the_file_it_could_not_read() {
        match Config::from_yaml_str("serviceName: [unclosed") {
            Err(Error::Config(message)) => {
                assert!(message.contains(CONFIG_FILE), "unexpected: {message}");
            }
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }
}
