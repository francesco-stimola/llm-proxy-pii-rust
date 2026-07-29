//! Runtime configuration.

use std::fmt;
use std::net::SocketAddr;

use anyhow::Context;

/// Where the proxy listens and which upstream it forwards to.
#[derive(Clone)]
pub struct Config {
    /// Address the proxy listens on.
    pub listen: SocketAddr,
    /// Base URL of the upstream OpenAI-compatible provider (no trailing
    /// `/v1/...` — the proxy appends the path).
    pub upstream_base_url: String,
    /// Optional API key injected as `Authorization: Bearer …` when the client
    /// did not send its own `Authorization` header.
    pub upstream_api_key: Option<String>,
    /// Maximum request body size in bytes. Tuned above axum's 2 MiB default so
    /// long-context requests aren't silently rejected with 413.
    pub max_body_bytes: usize,
    /// Which upstream provider preset is active (`openai` / `copilot` /
    /// `anthropic` / a custom name) — informational, drives the defaults below.
    pub provider: String,
    /// Path appended to `upstream_base_url` for chat completions. Per-provider
    /// (M3): OpenAI/Anthropic use `/v1/chat/completions`, Copilot `/chat/completions`.
    pub upstream_chat_path: String,
    /// Path appended to `upstream_base_url` for the native Anthropic Messages API
    /// (M6). Only used by the `/v1/messages` route, which is registered **only**
    /// when `provider == "anthropic"`. Defaults to `/v1/messages`.
    pub upstream_messages_path: String,
    /// Extra static headers to add to every upstream request (e.g. editor headers
    /// some providers require). Never includes secrets by default.
    pub upstream_extra_headers: Vec<(String, String)>,
    /// Allowlist of **client request** header names (lowercased) to pass through
    /// to the upstream — beyond `Authorization`, which is always handled. Lets a
    /// provider receive required headers (`anthropic-version`, editor headers, …)
    /// without blindly forwarding everything.
    pub forward_request_headers: Vec<String>,
    /// Active locales for the **domestic-phone** recognizers (M4's FP-prone tier) —
    /// numbers written with no `+CC`. Universal recognizers (email, IBAN, card, `+CC`
    /// phone) and the national IDs always run regardless (M4-R1).
    ///
    /// Defaults to **every vetted region** (M10); setting `PII_LOCALES` **replaces** that
    /// set rather than adding to it, so an operator who set it keeps exactly the behaviour
    /// they asked for.
    pub pii_locales: Vec<String>,
    /// **Debug only (M2.6), off by default.** When set, the response de-mask is
    /// skipped so the client receives the placeholders (`[EMAIL_1]`, …) the
    /// provider saw — visual proof the round-trip is wired. Request-side masking
    /// still runs, so it never weakens the fail-closed posture; a loud startup
    /// warning fires when it's on. Never enable in production.
    pub debug_skip_demask: bool,
    /// Entries for the content-keyed detection cache (S3, M7.1): the byte-identical
    /// system prompt Claude Code re-sends every turn is detected once and reused,
    /// saving the dominant NER scan. `0` disables it; otherwise up to ~`2×` this
    /// many large fields are held. From `PII_CACHE_ENTRIES`; default
    /// [`DEFAULT_PII_CACHE_ENTRIES`]. Sound because a hit is keyed on the *exact*
    /// bytes of a deterministic scan, so it can never mask less (see `pii::cache`).
    pub pii_cache_entries: usize,
}

/// Default max request body: 16 MiB — comfortably above long-context payloads
/// without inviting unbounded memory use.
pub const DEFAULT_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Default size of the S3 detection cache (M7.1): enough for a system prompt plus a
/// handful of tool schemas, with ~`2×` this many entries live at once. Overridable
/// via `PII_CACHE_ENTRIES`; `0` disables the cache entirely.
pub const DEFAULT_PII_CACHE_ENTRIES: usize = 16;

impl Config {
    /// Load configuration from environment variables, with sensible defaults:
    /// `LISTEN_ADDR` (default `127.0.0.1:8080`), `UPSTREAM_BASE_URL`
    /// (default `https://api.openai.com`), and optional `UPSTREAM_API_KEY`.
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_raw =
            std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        let listen = listen_raw
            .parse()
            .with_context(|| format!("invalid LISTEN_ADDR: {listen_raw:?}"))?;

        let upstream_base_url = std::env::var("UPSTREAM_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());

        // An empty value is treated as "unset" so an exported-but-blank var
        // doesn't send `Authorization: Bearer `.
        let upstream_api_key = std::env::var("UPSTREAM_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());

        let max_body_bytes = match std::env::var("MAX_BODY_BYTES") {
            Ok(raw) => raw
                .parse()
                .with_context(|| format!("invalid MAX_BODY_BYTES: {raw:?}"))?,
            Err(_) => DEFAULT_MAX_BODY_BYTES,
        };

        let debug_skip_demask = env_flag("PII_DEBUG_SKIP_DEMASK");

        let pii_cache_entries = match std::env::var("PII_CACHE_ENTRIES") {
            Ok(raw) => raw
                .trim()
                .parse()
                .with_context(|| format!("invalid PII_CACHE_ENTRIES: {raw:?}"))?,
            Err(_) => DEFAULT_PII_CACHE_ENTRIES,
        };

        // Provider routing (M3, Option A): a preset picks OpenAI-compatible
        // defaults (path + which client headers to pass through), each overridable.
        let provider = std::env::var("UPSTREAM_PROVIDER")
            .ok()
            .map(|p| p.trim().to_ascii_lowercase())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "openai".to_string());
        let preset = ProviderPreset::for_name(&provider);

        let upstream_chat_path = std::env::var("UPSTREAM_CHAT_PATH")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| preset.chat_path.to_string());

        // Native Anthropic Messages path (M6) — only the `anthropic`-gated
        // `/v1/messages` route uses it; overridable for parity with the chat path.
        let upstream_messages_path = std::env::var("UPSTREAM_MESSAGES_PATH")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "/v1/messages".to_string());

        let forward_request_headers = match std::env::var("UPSTREAM_FORWARD_HEADERS") {
            Ok(raw) => parse_header_list(&raw),
            Err(_) => preset
                .forward_headers
                .iter()
                .map(|h| h.to_string())
                .collect(),
        };

        let upstream_extra_headers = match std::env::var("UPSTREAM_EXTRA_HEADERS") {
            Ok(raw) => parse_extra_headers(&raw),
            Err(_) => Vec::new(),
        };

        // Domestic-phone regions (M4's FP-prone tier): comma-separated codes. Unset — or
        // set to nothing usable — means every vetted region (M10).
        let pii_locales = match std::env::var("PII_LOCALES") {
            Ok(raw) => {
                let list = parse_header_list(&raw); // same shape: comma-split, lowercased
                if list.is_empty() {
                    default_locales()
                } else {
                    list
                }
            }
            Err(_) => default_locales(),
        };

        Ok(Self {
            listen,
            upstream_base_url,
            upstream_api_key,
            max_body_bytes,
            provider,
            upstream_chat_path,
            upstream_messages_path,
            upstream_extra_headers,
            forward_request_headers,
            pii_locales,
            debug_skip_demask,
            pii_cache_entries,
        })
    }
}

/// The recognizer locales when `PII_LOCALES` is unset: **every vetted region** (M10).
///
/// It used to be the literal `["it", "us"]` — a placeholder chosen by M4 when the FP-prone
/// tier was empty, and never revisited when M8.1 filled it. Both codes mapped to no
/// recognizer, so the shipped default masked **no** domestic phone number at all.
///
/// Resolving it from [`PHONE_REGIONS`] rather than re-typing the list means the default and
/// the code that implements it cannot disagree — and because the resolved list is what
/// `Config`'s `Debug` prints at startup, an operator can *see* which regions are active
/// instead of inferring them from the absence of a variable.
fn default_locales() -> Vec<String> {
    crate::pii::recognizers::PHONE_REGIONS
        .iter()
        .map(|region| region.code.to_string())
        .collect()
}

/// OpenAI-compatible defaults per provider (M3, Option A). Base URL + API key stay
/// env-driven; only the *shape* differences (path, required client headers) are
/// preset here. Everything is overridable via `UPSTREAM_CHAT_PATH` /
/// `UPSTREAM_FORWARD_HEADERS` / `UPSTREAM_EXTRA_HEADERS`.
struct ProviderPreset {
    chat_path: &'static str,
    /// Client request headers (lowercased) safe & useful to pass through.
    forward_headers: &'static [&'static str],
}

impl ProviderPreset {
    fn for_name(name: &str) -> Self {
        match name {
            // GitHub Copilot's OpenAI-compat endpoint has no `/v1` and expects the
            // editor identification headers the client sends.
            "copilot" => ProviderPreset {
                chat_path: "/chat/completions",
                forward_headers: &[
                    "editor-version",
                    "editor-plugin-version",
                    "copilot-integration-id",
                    "openai-intent",
                ],
            },
            // Anthropic's OpenAI-compat layer keeps `/v1/chat/completions`.
            "anthropic" => ProviderPreset {
                chat_path: "/v1/chat/completions",
                forward_headers: &["anthropic-version", "anthropic-beta"],
            },
            // OpenAI (default) and any unknown name.
            _ => ProviderPreset {
                chat_path: "/v1/chat/completions",
                forward_headers: &["openai-organization", "openai-project"],
            },
        }
    }
}

/// Parse a comma-separated header-name list into a lowercased allowlist.
fn parse_header_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse `Key=Value` pairs separated by `;` into `(name, value)` header tuples.
/// Malformed entries (no `=`, empty name) are skipped.
fn parse_extra_headers(raw: &str) -> Vec<(String, String)> {
    raw.split(';')
        .filter_map(|entry| {
            let (name, value) = entry.split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Read a boolean-ish env flag (`1` / `true` / `yes` / `on`, case-insensitive).
///
/// Shared so every flag (`PII_DEBUG_SKIP_DEMASK`, `NER_REQUIRED`,
/// `NER_TOKEN_TYPE_IDS`) parses identically and can't diverge.
pub(crate) fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Manual `Debug` so the API key is never written to logs — `main` logs the
/// whole config at startup, and this is a privacy tool.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen", &self.listen)
            .field("upstream_base_url", &self.upstream_base_url)
            .field(
                "upstream_api_key",
                &self.upstream_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("max_body_bytes", &self.max_body_bytes)
            .field("provider", &self.provider)
            .field("upstream_chat_path", &self.upstream_chat_path)
            .field("upstream_messages_path", &self.upstream_messages_path)
            // Names only — an extra header value may be a secret (e.g. a token).
            .field(
                "upstream_extra_headers",
                &self
                    .upstream_extra_headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("forward_request_headers", &self.forward_request_headers)
            .field("pii_locales", &self.pii_locales)
            .field("debug_skip_demask", &self.debug_skip_demask)
            .field("pii_cache_entries", &self.pii_cache_entries)
            .finish()
    }
}
