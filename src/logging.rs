//! Log subscriber setup: the **default level** and the **timestamp format** (M10).
//!
//! Both defaults were wrong for the shipped binary, and both were only visible by
//! *running* it rather than by reading it:
//!
//! - **Level.** [`EnvFilter::from_default_env`] falls back to ERROR-only when `RUST_LOG`
//!   is unset, so a released binary printed **nothing at all** — not `listening on`, not
//!   `ONNX NER detector loaded`. That breaks the one check this project asks operators to
//!   make ("if the NER line is missing you are running structured-only"), because with no
//!   `RUST_LOG` *every* line is missing and a silent process looks exactly like a healthy
//!   one. The default is now [`DEFAULT_LOG_FILTER`]; `RUST_LOG` still wins.
//! - **Timestamps.** They were UTC, on a proxy that runs on the same laptop as the person
//!   reading its logs. They are now **local, with the offset always shown** — never a bare
//!   local time, which reads right on the author's box and is ambiguous everywhere else.
//!
//! This does **not** weaken the privacy bar: logs carry kinds, counts and placeholders
//! only, [`Config`](crate::config::Config)'s manual `Debug` redacts the API key, and
//! `tests/log_safety.rs` (DBG-02) enforces it at *every* level — the masked-body dump
//! stays `trace`-only.
//!
//! ## Why the offset is captured by the caller, before the runtime exists
//!
//! [`local_offset`] must be called while the process is still **single-threaded**. The
//! `time` crate refuses to answer once it is not (its guard against the
//! `localtime_r`/`setenv` data race, CVE-2020-26235), and `#[tokio::main]` builds the
//! worker threads *before* the body of `main` runs. That is why `main` is a plain `fn`
//! that reads the offset first and builds the runtime itself. The failure is
//! platform-split — Windows has a thread-safe path and usually works, Linux and macOS do
//! not — so getting this wrong ships as "it looked right on my box".

use time::format_description::BorrowedFormatItem;
use time::UtcOffset;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::EnvFilter;

/// The filter used when `RUST_LOG` is unset or empty.
///
/// `info` is the level at which the binary says what it is doing — `listening on`, the
/// loaded detectors, the debug-mode warnings — without per-request noise.
pub const DEFAULT_LOG_FILTER: &str = "info";

/// The log timestamp: exactly the parts the UTC default had (date, time, 6 sub-second
/// digits) with `Z` replaced by an **explicit numeric offset**.
///
/// ```text
/// before  2026-07-29T10:37:04.844780Z
/// after   2026-07-29T12:37:04.844780+02:00
/// ```
///
/// The offset is an *addition*, never a replacement: a bare `12:37:04.844780` would be
/// unreadable anywhere but the machine that wrote it.
const TIMESTAMP_FORMAT: &[BorrowedFormatItem<'static>] = time::macros::format_description!(
    "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]\
     [offset_hour sign:mandatory]:[offset_minute]"
);

/// The timer type this crate installs — named so `main` can hold one without spelling out
/// the format-description type.
pub type LogTimer = OffsetTime<&'static [BorrowedFormatItem<'static>]>;

/// Build the log filter from a `RUST_LOG` value, **without reading the environment** — so
/// it is testable in a parallel test run, where mutating process env is a data race.
///
/// An **empty** `RUST_LOG` counts as unset. An exported-but-blank variable is the same
/// accident as an exported-but-blank `UPSTREAM_API_KEY`, and treating it as "set" would
/// re-create the silent-binary bug this function exists to fix: an empty directive list
/// filters everything below ERROR.
///
/// Parsing stays **lossy** (as [`EnvFilter::from_default_env`] was): one malformed
/// directive in an otherwise good `RUST_LOG` drops that directive and keeps the rest,
/// rather than silently discarding the operator's whole setting.
pub fn log_filter(rust_log: Option<&str>) -> EnvFilter {
    match rust_log {
        Some(spec) if !spec.trim().is_empty() => EnvFilter::new(spec),
        _ => EnvFilter::new(DEFAULT_LOG_FILTER),
    }
}

/// The machine's current UTC offset, or `None` if it cannot be determined.
///
/// **Call this before any thread is spawned** — see the module docs. `None` is not an
/// error to paper over: the caller falls back to UTC and *says so*, because a silently
/// wrong local time is the kind of thing that costs an hour during an incident. Never
/// guess an offset.
pub fn local_offset() -> Option<UtcOffset> {
    UtcOffset::current_local_offset().ok()
}

/// The timer for a given offset, falling back to UTC (`+00:00`) when the offset is
/// unknown.
///
/// The fallback prints `+00:00` rather than `Z` on purpose: one format, one code path, and
/// the reader still gets an explicit offset — which is the property that matters.
pub fn timer(offset: Option<UtcOffset>) -> LogTimer {
    OffsetTime::new(offset.unwrap_or(UtcOffset::UTC), TIMESTAMP_FORMAT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **LOG-01.** `RUST_LOG` unset ⇒ `info`, not the ERROR-only silence a released binary
    /// shipped with. Driven through the builder, never through process env.
    #[test]
    fn an_unset_rust_log_defaults_to_info() {
        assert_eq!(log_filter(None).to_string(), "info");
    }

    /// **LOG-02.** An exported-but-blank `RUST_LOG` is "unset", not "no directives" — the
    /// latter is ERROR-only, i.e. the silent binary through a second door.
    #[test]
    fn a_blank_rust_log_is_treated_as_unset() {
        assert_eq!(log_filter(Some("")).to_string(), "info");
        assert_eq!(log_filter(Some("   ")).to_string(), "info");
    }

    /// **LOG-03.** A set `RUST_LOG` still wins — the default is a fallback, not a floor.
    #[test]
    fn a_set_rust_log_still_wins() {
        assert_eq!(log_filter(Some("warn")).to_string(), "warn");
        assert_eq!(
            log_filter(Some("llm_proxy_pii_rust=debug")).to_string(),
            "llm_proxy_pii_rust=debug"
        );
    }

    /// **LOG-04.** The offset lookup is a pure query: two calls agree. It is *also* the
    /// only thing about it a test can assert portably — under `cargo test` the process is
    /// already multi-threaded, so on Linux/macOS the answer is legitimately `None`, and
    /// asserting a wall-clock offset would assert the test runner's threading model.
    /// The real regression (a timestamp with no offset at all) is caught end-to-end by
    /// CLI-06 in `tests/binary_smoke.rs`, against the actual binary.
    #[test]
    fn the_offset_lookup_is_stable() {
        assert_eq!(local_offset(), local_offset());
    }

    /// **LOG-05.** Both timers print an **explicit** offset and keep every part the UTC
    /// default had — date, time, six sub-second digits. Formatting a known instant is
    /// deterministic; formatting *now* would not be.
    #[test]
    fn the_timestamp_always_carries_an_explicit_offset() {
        use time::macros::datetime;
        use tracing_subscriber::fmt::format::Writer;
        use tracing_subscriber::fmt::time::FormatTime;

        // `FormatTime` reads the clock, so exercise the format description directly —
        // that is the part under test.
        fn rendered(offset: UtcOffset) -> String {
            datetime!(2026-07-29 10:37:04.844780 UTC)
                .to_offset(offset)
                .format(&TIMESTAMP_FORMAT)
                .expect("the format description must apply to an OffsetDateTime")
        }

        assert_eq!(
            rendered(UtcOffset::from_hms(2, 0, 0).unwrap()),
            "2026-07-29T12:37:04.844780+02:00"
        );
        assert_eq!(rendered(UtcOffset::UTC), "2026-07-29T10:37:04.844780+00:00");

        // And the timer really is built from that description (a `FormatTime` impl, so it
        // writes *some* current time — we assert the shape, not the value).
        let mut out = String::new();
        timer(None).format_time(&mut Writer::new(&mut out)).unwrap();
        assert!(
            out.len() == "2026-07-29T10:37:04.844780+00:00".len() && out.ends_with("+00:00"),
            "the UTC fallback timer must print a full timestamp with an explicit offset, got {out:?}"
        );
    }
}
