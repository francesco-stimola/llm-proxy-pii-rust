//! Reversible anonymization: replace detected PII with typed placeholders and
//! restore them on the way back.
//!
//! The [`Vault`] maps placeholder → original for a single request/response
//! round-trip, so [`Vault::demask`] restores the exact original text. Assignment
//! is **deterministic**: the same real value always maps to the same placeholder
//! within a vault, which lets the downstream model correlate a value across a
//! multi-turn (stateless) conversation.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

use super::{DetectError, PiiDetector, PiiEntity, PiiKind};

/// How many times [`Vault::mask_all`] re-detects before giving up. Masking exposes PII at
/// most a handful of times in practice (each pass must break a token apart to reveal a new
/// one); this is a safety bound, not an expected limit. Exhausting it **blocks the
/// request** — see [`Vault::mask_all`] (M4-R20).
const MAX_MASK_PASSES: usize = 4;

/// Tolerant placeholder pattern used on the way back. It accepts the canonical
/// `[EMAIL_1]` **and** the corruptions a model tends to introduce — a space or
/// dash for the underscore, stray inner spaces, a lowercased label:
/// `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`. This keeps restore from silently
/// failing when the model reformats a placeholder.
static PLACEHOLDER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\s*([A-Za-z]+)[ _-]*([0-9]+)\s*\]").unwrap());

/// Placeholder ↔ original-value store for one request.
#[derive(Debug, Default)]
pub struct Vault {
    /// placeholder (e.g. `[EMAIL_1]`) → original value.
    to_original: HashMap<String, String>,
    /// original value → placeholder, so a repeated value reuses its token.
    to_placeholder: HashMap<String, String>,
    /// per-kind counter, so tokens are numbered `[EMAIL_1]`, `[EMAIL_2]`, …
    counters: HashMap<PiiKind, usize>,
}

impl Vault {
    /// Create an empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether anything has been masked yet — i.e. no placeholders exist.
    pub fn is_empty(&self) -> bool {
        self.to_original.is_empty()
    }

    /// Mask everything `detector` finds in `text`, **iterating to a fixpoint** (M4-R17).
    ///
    /// A single pass is not enough, because **masking rewrites the bytes around what it
    /// replaced**, and a value is only recognizable in context. Replacing a phone that sits
    /// inside a longer digit run splits that run apart and *exposes* a card the recognizers
    /// could not see before:
    ///
    /// ```text
    /// 4111111111111111555 867 5309   → one 19-digit run: not Luhn-valid, so NOT a card
    ///                                   (the anti-false-positive rule — an ID never fires
    ///                                   inside a longer token)
    /// 4111111111111111[PHONE_1]      → after masking the phone, the leftover IS a clean,
    ///                                   Luhn-valid card — and it would go upstream in clear
    /// ```
    ///
    /// So we re-detect on the masked text until it yields nothing. Two things bound that loop:
    ///
    /// **The NER runs only on pass 0 (S4, CC-05/CC-08).** Masking *exposes* PII only by splitting a
    /// token — a **structured-recognizer** phenomenon (the card above). Masking a name to
    /// `[PERSON_1]` never reveals a *new* name, so re-running the NER buys no recall (measured: **0**
    /// losses across the labelled corpus) — while it *does* re-tag the sub-word fragments it emits
    /// (`"lack"` of `"Slack"`), which on a dense system prompt drove masking past the pass bound into
    /// a fail-closed 400. So later passes call [`redetect`](PiiDetector::redetect), which runs only
    /// the detectors masking can expose (the structured recognizers), never the NER. That makes the
    /// pass count **O(1) on a fragment-dense field** instead of growing with it.
    ///
    /// **Placeholders are inert by construction (M5-R4, CC-08).** A recognizer can't match a
    /// `[KIND_N]` token (no `@`, no `sk-`, nowhere near enough digits; `[` / `]` are outside every
    /// pattern's character classes), and even if a future NER tagged one, this loop no longer trusts
    /// it: [`keep_maskable`] **drops any detection that is exactly one of our own tokens** before
    /// masking, which a real value can never be. So every surviving detection is genuine PII, masking
    /// it strictly shrinks the raw text, and the fixpoint converges. The shipped model doesn't even
    /// try — `tests/ner_perf.rs::m5_r4_the_ner_treats_placeholders_as_inert` still pins that, now
    /// belt-and-braces (see `ARCHITECTURE.md` → *Masking must run to a fixpoint*).
    ///
    /// **Exhausting [`MAX_MASK_PASSES`] fails *closed* (M4-R20).** The bound is a safety
    /// net, not a proof: "each pass strictly shrinks the un-masked text" buys *eventual*
    /// convergence, never convergence **within** four passes. So the loop **confirms** the
    /// fixpoint instead of assuming it — if anything is still detectable when the passes run
    /// out, this returns `Err` and [`PrivacyStage`](crate::pipeline::privacy::PrivacyStage)
    /// blocks the request. Forwarding a "probably clean" text is exactly the failure mode a
    /// privacy proxy must not have. (With the NER out after pass 0, later passes are
    /// structured-only and converge fast — no realistic input approaches the bound; *before* S4, a
    /// dense system prompt's NER fragments needed >4 passes and 400'd. It stays fail-closed regardless.)
    ///
    /// On that branch it also logs a **value-free** diagnostic — the per-pass kind tally, the
    /// residue's kinds, and `placeholder_tags_suppressed`, a count of any placeholder-shaped tokens
    /// [`keep_maskable`] dropped. Kinds and counts only, never the offending text; it is the only
    /// signal there is, since fail-closed blocks *before* the forward-trace that would otherwise show
    /// what didn't settle. The **converging** path carries that count at `debug` too, via
    /// [`note_suppressed_placeholders`]. (Note, M7-R23: since **S4** the NER runs only on pass 0, so
    /// this counts placeholder-shaped text already in the *raw* field — a client echoing placeholders
    /// — never a model re-tagging masking's own output. The model-swap canary is the `m5_r4` test.)
    ///
    /// The round-trip stays exact: each pass records raw value → placeholder, and
    /// [`demask`](Self::demask) restores every placeholder in one tolerant pass.
    pub fn mask_all(
        &mut self,
        text: &str,
        detector: &dyn PiiDetector,
    ) -> Result<String, DetectError> {
        let mut current = text.to_string();
        // Per-pass tally of the **maskable** (real-PII) detections, kept only to explain a
        // non-convergence on the fail-closed branch below. Value-free: kinds and counts, never
        // the text — the granularity `Config`/`Confidence` already log at.
        let mut per_pass: Vec<Vec<(&'static str, usize)>> = Vec::new();
        // Count of placeholder-shaped tokens `keep_maskable` dropped. Zero for the shipped NER.
        // Since S4 the NER runs only on pass 0, so this counts placeholder-shaped text already in
        // the raw field (a client echoing placeholders), not a model re-tagging our output (M7-R23).
        // Rides the fail-closed diagnostic below and a converging-path `debug` note.
        let mut placeholder_tags_suppressed = 0usize;
        for pass in 0..MAX_MASK_PASSES {
            // Pass 0 runs the whole detector; later passes run only `redetect` — the detectors
            // masking can *expose* (the structured recognizers, where a split digit run reveals a
            // card), never the NER, which is idempotent after pass 0 (S4). That is what bounds the
            // pass count on a fragment-dense field instead of letting the NER re-tag forever.
            let raw = if pass == 0 {
                detector.try_detect(&current)?
            } else {
                detector.redetect(&current)?
            };
            let entities = keep_maskable(raw, &mut placeholder_tags_suppressed);
            if entities.is_empty() {
                note_suppressed_placeholders(placeholder_tags_suppressed);
                return Ok(current);
            }
            per_pass.push(kind_histogram(&entities));
            current = self.mask(&current, &entities);
        }
        // The passes ran out having masked real PII on every one of them, so we do NOT know
        // `current` is clean. Confirm it with `redetect` (M4-R20) — and block if it isn't.
        let remaining = keep_maskable(
            detector.redetect(&current)?,
            &mut placeholder_tags_suppressed,
        );
        if remaining.is_empty() {
            note_suppressed_placeholders(placeholder_tags_suppressed);
            return Ok(current);
        }
        // Non-convergence, diagnosed without ever logging a value (M4-R20). Placeholders are inert
        // by construction (`keep_maskable` drops them) and the NER is out after pass 0 (S4), so a
        // residue here is *real structured* PII that masking keeps re-exposing: the per-pass tally
        // shows whether its count is shrinking (a deep nest that would clear with more passes) or
        // stalled, and `placeholder_tags_suppressed` flags a model that tagged our own output.
        let remaining_kinds = kind_histogram(&remaining);
        tracing::warn!(
            passes = MAX_MASK_PASSES,
            ?per_pass,
            remaining = ?remaining_kinds,
            placeholder_tags_suppressed,
            "masking did not reach a fixpoint; blocking the request (fail closed)"
        );
        Err(DetectError {
            detector: "vault",
            message: format!("masking did not reach a fixpoint in {MAX_MASK_PASSES} passes"),
        })
    }

    /// Replace each entity in `text` with a typed placeholder, recording the
    /// original in the vault. Returns the anonymized text.
    ///
    /// This is **one pass**. Prefer [`mask_all`](Self::mask_all), which re-detects until the
    /// text stops changing — masking can expose PII that was not recognizable before.
    ///
    /// **One left-to-right copy into a fresh buffer — O(n + k), never O(n·k) (M4-R24).**
    /// This used to splice in place, right-to-left, with `String::replace_range`. That is
    /// correct but **quadratic in the entity count**: every splice memmoves the entire tail
    /// of the string, so *k* entities in *n* bytes shift Θ(n·k) bytes — and when a field
    /// holds many small values (`a@b.co `, an SSN, a phone) *k* grows with *n*, so it is
    /// Θ(n²). A 13 MiB body of repeated emails burned **~7 minutes** of CPU; the *same*
    /// 13 MiB as one giant email masked in 219 ms. Linear *detection* does not bound this —
    /// the splice is a separate cost on the same unauthenticated path, which is why M4-R19
    /// (the quadratic in *candidate generation*) closed without touching it. Copying forward
    /// once touches each byte exactly once instead.
    ///
    /// Placeholders are still numbered in **reading order**: we walk the entities sorted by
    /// start, so the *n*-th distinct value seen left-to-right gets `[KIND_n]`, exactly as
    /// before. Splice order was never what made numbering deterministic.
    ///
    /// **Precondition** (guaranteed by [`resolve_overlaps`](crate::pii::overlap::resolve_overlaps),
    /// the only production caller): spans are in-bounds, on `char` boundaries, and pairwise
    /// **non-overlapping**. A caller that breaks it gets no panic and no leak — see the guard
    /// below. Text with no entities is returned unchanged.
    pub fn mask(&mut self, text: &str, entities: &[PiiEntity]) -> String {
        if entities.is_empty() {
            return text.to_string();
        }

        let mut ordered: Vec<&PiiEntity> = entities.iter().collect();
        ordered.sort_by_key(|e| e.span.start);

        let mut out = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for entity in ordered {
            let (start, end) = (entity.span.start, entity.span.end);

            // Unreachable via the resolver (see the precondition). If it ever happens, the
            // one thing we must not do is emit the span's bytes in clear, so we advance past
            // them rather than copying them — drop, never leak — and we never panic on a
            // slice: this is a proxy, and the input is attacker-influenced.
            let well_formed = cursor <= start
                && start <= end
                && end <= text.len()
                && text.is_char_boundary(start)
                && text.is_char_boundary(end);
            if !well_formed {
                debug_assert!(false, "mask(): spans must be in-bounds and non-overlapping");
                tracing::warn!(kind = ?entity.kind, "malformed span in mask(); skipping it");
                // Widening to a `char` boundary only ever drops *more*, so it can't leak —
                // and it keeps `cursor` sliceable for the copies below.
                cursor = cursor.max(char_boundary_at_or_after(text, end));
                continue;
            }

            let placeholder = self.placeholder_for(entity);
            out.push_str(&text[cursor..start]);
            out.push_str(&placeholder);
            cursor = end;
        }
        out.push_str(&text[cursor..]);
        out
    }

    /// Restore placeholders in `text` back to their original values.
    ///
    /// A single tolerant pass (see [`PLACEHOLDER_RE`]): every placeholder-shaped
    /// token is normalized to its canonical `[LABEL_N]` form and looked up. A
    /// token that isn't in the vault is left untouched — and if it still looks
    /// like one of our kinds (so the model probably mangled or invented it), a
    /// warning is logged rather than silently shipping a broken placeholder.
    pub fn demask(&self, text: &str) -> String {
        self.demask_inner(text, false)
    }

    /// Like [`demask`](Self::demask), but for text that is itself a **JSON-encoded
    /// string** — notably a tool-call `arguments` value. The substituted value is
    /// JSON-string-escaped so a value containing a `"`, `\`, or control character
    /// keeps the surrounding inner JSON valid (M3-R2); otherwise the client fails
    /// to parse the tool-call arguments.
    pub fn demask_json_string(&self, text: &str) -> String {
        self.demask_inner(text, true)
    }

    /// Shared demask pass; `json_escape` escapes the substituted value as a
    /// JSON-string body (for `arguments`-style fields).
    fn demask_inner(&self, text: &str, json_escape: bool) -> String {
        if self.to_original.is_empty() {
            return text.to_string();
        }
        PLACEHOLDER_RE
            .replace_all(text, |caps: &Captures| {
                let canonical = format!("[{}_{}]", caps[1].to_ascii_uppercase(), &caps[2]);
                if let Some(original) = self.to_original.get(&canonical) {
                    return if json_escape {
                        json_string_body(original)
                    } else {
                        original.clone()
                    };
                }
                if PiiKind::from_label(&caps[1]).is_some() {
                    tracing::warn!(
                        placeholder = %&caps[0],
                        "unresolved PII placeholder in response; left as-is"
                    );
                }
                caps[0].to_string()
            })
            .into_owned()
    }

    /// Look up (or mint) the placeholder for an entity's value.
    fn placeholder_for(&mut self, entity: &PiiEntity) -> String {
        if let Some(existing) = self.to_placeholder.get(&entity.text) {
            return existing.clone();
        }
        let counter = self.counters.entry(entity.kind).or_insert(0);
        *counter += 1;
        let placeholder = format!("[{}_{}]", entity.kind.label(), counter);
        self.to_placeholder
            .insert(entity.text.clone(), placeholder.clone());
        self.to_original
            .insert(placeholder.clone(), entity.text.clone());
        placeholder
    }
}

/// The first `char` boundary at or after `i`, clamped to `s.len()`. Used only by
/// [`Vault::mask`]'s malformed-span guard, to keep its cursor sliceable.
fn char_boundary_at_or_after(s: &str, i: usize) -> usize {
    let mut at = i.min(s.len());
    while at < s.len() && !s.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// Whether `text` is **exactly one of our own placeholder tokens** — `[KIND_N]` for a known
/// [`PiiKind`] label, plus the tolerant corruptions [`PLACEHOLDER_RE`] accepts, and ignoring
/// surrounding whitespace. Nothing else: a partial match, a foreign label like `[TODO_1]`, or
/// two tokens in a row all return `false`.
///
/// This is the key to placeholder inertness *by construction* (M5-R4): a real PII value can
/// never take this shape, so [`keep_maskable`] can drop such a detection with no risk of
/// dropping real PII — see [`Vault::mask_all`].
fn is_placeholder_token(text: &str) -> bool {
    let trimmed = text.trim();
    PLACEHOLDER_RE.captures(trimmed).is_some_and(|caps| {
        let whole = caps.get(0).expect("group 0 always present");
        whole.start() == 0
            && whole.end() == trimmed.len()
            && super::PiiKind::from_label(&caps[1]).is_some()
    })
}

/// From a detection pass's `entities`, **drop any that is one of our own placeholders**
/// (see [`is_placeholder_token`]), tallying how many were dropped into `suppressed`.
///
/// The recognizers cannot match a placeholder, but an ML NER is under no such constraint, and
/// one that re-tagged `[PERSON_1]` would keep [`Vault::mask_all`] from ever reaching a fixpoint.
/// Filtering here makes convergence a property of the algorithm, not of the model — and it takes
/// the already-detected list (not the detector) so `mask_all` can feed it either `try_detect`
/// (pass 0) or `redetect` (later passes, the NER dropped) results (S4).
fn keep_maskable(entities: Vec<PiiEntity>, suppressed: &mut usize) -> Vec<PiiEntity> {
    let mut maskable = Vec::with_capacity(entities.len());
    for entity in entities {
        if is_placeholder_token(&entity.text) {
            *suppressed += 1;
        } else {
            maskable.push(entity);
        }
    }
    maskable
}

/// Value-free `debug` note (M7-R21): when masking *converges*, record that some detector handed back a
/// placeholder-shaped token that [`keep_maskable`] dropped. Silent in the shipped case
/// (`suppressed == 0` for XLM-R int8). Since **S4** the NER runs only on pass 0, so this counts
/// placeholder-shaped text already in the *raw* field (e.g. a client echoing placeholders in Run ON),
/// **not** a model re-tagging masking's own output — that can no longer happen (M7-R23). The
/// model-swap canary is `tests/ner_perf.rs::m5_r4_the_ner_treats_placeholders_as_inert`, which runs
/// the NER directly on placeholder text.
fn note_suppressed_placeholders(suppressed: usize) {
    if suppressed > 0 {
        tracing::debug!(
            placeholder_tags_suppressed = suppressed,
            "detector tagged our own placeholders; dropped as inert (masking still converged)"
        );
    }
}

/// A value-free tally of one detection pass: `(kind-label, count)` sorted by label for
/// stable output. Used **only** to explain a non-convergence in [`Vault::mask_all`] — labels
/// and counts carry no PII (the never-log-raw-values rule), and the sort keeps the log line
/// diffable across passes and runs.
fn kind_histogram(entities: &[PiiEntity]) -> Vec<(&'static str, usize)> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for e in entities {
        *counts.entry(e.kind.label()).or_insert(0) += 1;
    }
    let mut out: Vec<(&'static str, usize)> = counts.into_iter().collect();
    out.sort_unstable_by_key(|(label, _)| *label);
    out
}

/// Escape `s` as the **body** of a JSON string (no surrounding quotes) — i.e. the
/// form it must take when substituted into an already-quoted JSON-string field.
/// `serde_json::to_string` yields `"…escaped…"`; we drop the outer quotes.
fn json_string_body(s: &str) -> String {
    let quoted = serde_json::to_string(s).unwrap_or_default();
    quoted
        .get(1..quoted.len().saturating_sub(1))
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;
    use crate::pii::composite::CompositeDetector;
    use crate::pii::recognizers::StructuredRecognizers;
    use crate::pii::{Confidence, DetectError, PiiDetector, PiiEntity, PiiKind};

    /// Capture this crate's `debug`-and-up logs emitted while `f` runs, as one string. A
    /// **scoped** subscriber (`with_default`, thread-local) so it never fights the process-global
    /// one `tests/log_safety.rs` installs — different binary, and different mechanism regardless.
    fn capture_debug_logs(f: impl FnOnce()) -> String {
        type Buf = Arc<Mutex<Vec<u8>>>;
        #[derive(Clone)]
        struct BufMaker(Buf);
        struct BufSink(Buf);
        impl std::io::Write for BufSink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for BufMaker {
            type Writer = BufSink;
            fn make_writer(&'a self) -> Self::Writer {
                BufSink(self.0.clone())
            }
        }
        let buf: Buf = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(BufMaker(buf.clone()))
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buf.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// Build a vault mapping a `[PERSON_1]` placeholder to a value with a quote.
    fn vault_with_quoted_person(value: &str) -> Vault {
        let mut vault = Vault::new();
        let entity = PiiEntity {
            kind: PiiKind::Person,
            span: 0..value.len(),
            text: value.to_string(),
            confidence: Confidence::Structural,
        };
        vault.mask(value, std::slice::from_ref(&entity));
        vault
    }

    #[test]
    fn demask_json_string_keeps_inner_json_valid() {
        // M3-R2: a value with a `"` de-masked into a tool-call `arguments` string
        // must stay valid inner JSON (plain demask would break it).
        let vault = vault_with_quoted_person(r#"Ac"me Corp"#);
        let args = r#"{"vendor":"[PERSON_1]"}"#;

        let restored = vault.demask_json_string(args);
        assert_eq!(restored, r#"{"vendor":"Ac\"me Corp"}"#);
        // …and it really parses, carrying the exact value.
        let parsed: serde_json::Value = serde_json::from_str(&restored).expect("valid inner JSON");
        assert_eq!(parsed["vendor"], r#"Ac"me Corp"#);

        // Plain demask would have produced invalid inner JSON here.
        assert!(serde_json::from_str::<serde_json::Value>(&vault.demask(args)).is_err());
    }

    fn mask_roundtrip(input: &str) -> (String, String) {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect(input);
        let masked = vault.mask(input, &entities);
        let restored = vault.demask(&masked);
        (masked, restored)
    }

    #[test]
    fn no_pii_is_unchanged() {
        let (masked, restored) = mask_roundtrip("Hello world, no PII here");
        assert_eq!(masked, "Hello world, no PII here");
        assert_eq!(restored, "Hello world, no PII here");
    }

    #[test]
    fn masks_and_restores_multiple_pii() {
        let input = "My email is bob@test.com and my phone is 555-111-2222";
        let (masked, restored) = mask_roundtrip(input);
        assert!(!masked.contains("bob@test.com"));
        assert!(!masked.contains("555-111-2222"));
        assert!(masked.contains("[EMAIL_1]"));
        assert!(masked.contains("[PHONE_1]"));
        assert_eq!(restored, input);
    }

    #[test]
    fn same_value_gets_same_placeholder() {
        // VAULT-05: determinism within a text.
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let input = "write to a@b.com, again a@b.com";
        let entities = detector.detect(input);
        let masked = vault.mask(input, &entities);
        assert_eq!(masked, "write to [EMAIL_1], again [EMAIL_1]");
    }

    #[test]
    fn demask_tolerates_model_corrupted_placeholders() {
        // The model may reformat a token; restore must still work.
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect("mail bob@test.com");
        let _ = vault.mask("mail bob@test.com", &entities);

        for corrupted in ["[EMAIL_1]", "[EMAIL 1]", "[email-1]", "[ EMAIL_1 ]"] {
            assert_eq!(
                vault.demask(&format!("sent to {corrupted}.")),
                "sent to bob@test.com."
            );
        }
    }

    /// A detector that always reports the **first character** as PII — so masking it just
    /// exposes a new first character and the fixpoint is never reached. It models exactly
    /// the shape `mask_all`'s pass bound exists for: a transform that keeps re-creating what
    /// it removes (M4-R20). No real detector behaves this way — no input has been shown to
    /// need more than 2 passes — which is why the fail-open was *latent* rather than live.
    struct NeverConverges;

    impl PiiDetector for NeverConverges {
        fn detect(&self, input: &str) -> Vec<PiiEntity> {
            let Some(first) = input.chars().next() else {
                return Vec::new();
            };
            let end = first.len_utf8();
            vec![PiiEntity {
                kind: PiiKind::Person,
                span: 0..end,
                text: input[..end].to_string(),
                confidence: Confidence::Structural,
            }]
        }
    }

    #[test]
    fn mask_all_blocks_when_it_cannot_reach_a_fixpoint() {
        // M4-R20: exhausting MAX_MASK_PASSES used to return the text anyway — so anything
        // still detectable was forwarded IN CLEAR, contradicting the fail-closed bar. The
        // pass bound gives *eventual* convergence, never convergence within four passes, so
        // the fixpoint must be **confirmed**, not assumed.
        let mut vault = Vault::new();
        let err = vault
            .mask_all("still here", &NeverConverges)
            .expect_err("a text that never converges must fail closed, not be forwarded");

        // The block reason must carry no input text (the never-log-raw-PII rule).
        let rendered = err.to_string();
        assert!(
            !rendered.contains("still here"),
            "error leaked the input: {rendered}"
        );

        // …and a detector that *does* converge still returns Ok, so this didn't just break
        // masking for everyone.
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let masked = vault
            .mask_all("mail bob@test.com", &detector)
            .expect("converges");
        assert_eq!(masked, "mail [EMAIL_1]");
    }

    /// A detector that tags **every** placeholder token it sees as a `Person` — modelling
    /// exactly the NER pathology placeholder-inertness defends against (M5-R4 / CC-08). A model
    /// that did this would drive `mask_all` to re-mask its own output forever and 400.
    struct TagsPlaceholders;

    impl PiiDetector for TagsPlaceholders {
        fn detect(&self, input: &str) -> Vec<PiiEntity> {
            PLACEHOLDER_RE
                .find_iter(input)
                .map(|m| PiiEntity {
                    kind: PiiKind::Person,
                    span: m.start()..m.end(),
                    text: m.as_str().to_string(),
                    confidence: Confidence::Structural,
                })
                .collect()
        }
    }

    #[test]
    fn mask_all_converges_even_if_a_detector_tags_its_own_placeholders() {
        // M5-R4 / CC-08: inertness is now BY CONSTRUCTION. Pair a real recognizer (which mints
        // `[EMAIL_1]`) with a detector that re-tags that placeholder every pass. Before the
        // filter this looped to MAX_MASK_PASSES and 400'd; now `mask_all` drops the placeholder
        // detection and converges — masking only the genuine email, exactly once.
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(TagsPlaceholders),
        ]);
        let mut vault = Vault::new();
        let masked = vault
            .mask_all("mail bob@test.com", &composite)
            .expect("a placeholder-tagging detector must not break convergence");
        assert_eq!(masked, "mail [EMAIL_1]");
        // The round-trip is untouched: the placeholder still restores to the real value.
        assert_eq!(vault.demask(&masked), "mail bob@test.com");
    }

    #[test]
    fn is_placeholder_token_matches_only_our_own_tokens() {
        // Our tokens, including the tolerant corruptions and surrounding whitespace.
        assert!(is_placeholder_token("[EMAIL_1]"));
        assert!(is_placeholder_token("  [ PERSON 2 ]  "));
        assert!(is_placeholder_token("[email-3]"));
        assert!(is_placeholder_token("[ORG_11]"));
        // Not ours: real PII, a foreign label, a partial match, two tokens.
        assert!(!is_placeholder_token("bob@test.com"));
        assert!(!is_placeholder_token("[TODO_1]"));
        assert!(!is_placeholder_token("see [EMAIL_1] here"));
        assert!(!is_placeholder_token("[EMAIL_1] and [EMAIL_2]"));
    }

    /// A fake NER that tags the **first ASCII-alphabetic character** as an Organization — so
    /// masking it exposes the next character, which the next detection tags, ad infinitum. It
    /// models the sub-word fragmentation the real NER does on dense system prompts (`"Slack"` →
    /// `"lack"` → …), the mechanism that pushed masking past `MAX_MASK_PASSES` and 400'd
    /// (CC-05/CC-08). `idempotent` mirrors S4: an idempotent detector (the NER after pass 0)
    /// contributes nothing to `redetect`, so masking converges; one that re-runs every pass never
    /// does.
    struct Fragmenter {
        idempotent: bool,
    }

    impl PiiDetector for Fragmenter {
        fn detect(&self, input: &str) -> Vec<PiiEntity> {
            for (i, c) in input.char_indices() {
                if c.is_ascii_alphabetic() {
                    return vec![PiiEntity {
                        kind: PiiKind::Organization,
                        span: i..i + c.len_utf8(),
                        text: c.to_string(),
                        confidence: Confidence::Structural,
                    }];
                }
            }
            Vec::new()
        }

        fn redetect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
            if self.idempotent {
                Ok(Vec::new())
            } else {
                self.try_detect(input)
            }
        }
    }

    #[test]
    fn s4_a_re_running_fragmenter_exhausts_the_bound() {
        // The bug S4 fixes: a detector that keeps tagging a fresh fragment on every pass (the NER
        // on a dense system prompt) never lets the fixpoint settle → fail-closed 400. This holds
        // when the detector re-runs on later passes (idempotent=false, the default `redetect`).
        let composite = CompositeDetector::new(vec![Box::new(Fragmenter { idempotent: false })]);
        let mut vault = Vault::new();
        let err = vault
            .mask_all("Slack", &composite)
            .expect_err("a detector that re-fragments every pass must exhaust the bound");
        assert!(
            !err.to_string().contains("Slack"),
            "the block reason must not leak the input"
        );
    }

    #[test]
    fn s4_an_idempotent_ner_lets_masking_converge() {
        // The fix: the same fragmenter, idempotent under `redetect` like the real NER after pass 0.
        // It tags once on pass 0; later passes call `redetect` (empty), so masking converges — only
        // the pass-0 hit is masked, the fragment chain never forms.
        let composite = CompositeDetector::new(vec![Box::new(Fragmenter { idempotent: true })]);
        let mut vault = Vault::new();
        let masked = vault
            .mask_all("Slack", &composite)
            .expect("an idempotent NER must let masking converge");
        assert_eq!(masked, "[ORG_1]lack");
    }

    #[test]
    fn converging_mask_emits_the_value_free_suppression_canary() {
        // M7-R21: when a detector re-tags our placeholders but masking still converges, the
        // runtime canary must surface at `debug` (`note_suppressed_placeholders`) — and, being on
        // the request path, it must carry no raw value. Without it, a filter-leaning model that
        // never 400s is invisible until it does.
        let composite = CompositeDetector::new(vec![
            Box::new(StructuredRecognizers::new()),
            Box::new(TagsPlaceholders),
        ]);
        let logs = capture_debug_logs(|| {
            let mut vault = Vault::new();
            let masked = vault
                .mask_all("mail bob@test.com", &composite)
                .expect("converges");
            assert_eq!(masked, "mail [EMAIL_1]");
        });
        assert!(
            logs.contains("placeholder_tags_suppressed"),
            "the converging path must emit the suppression canary; logs:\n{logs}"
        );
        assert!(
            !logs.contains("bob@test.com"),
            "the canary must be value-free; logs:\n{logs}"
        );
    }

    #[test]
    fn a_clean_convergence_stays_silent() {
        // Nothing re-tagged → nothing suppressed → no canary line. It must not fire spuriously on
        // the overwhelmingly common case (the shipped model suppresses zero).
        let detector = StructuredRecognizers::new();
        let logs = capture_debug_logs(|| {
            let mut vault = Vault::new();
            let _ = vault
                .mask_all("mail bob@test.com", &detector)
                .expect("converges");
        });
        assert!(
            !logs.contains("placeholder_tags_suppressed"),
            "a clean convergence must stay silent; logs:\n{logs}"
        );
    }

    #[test]
    fn kind_histogram_tallies_by_label_and_is_value_free() {
        // The non-convergence diagnostic summarizes a pass as `(label, count)`, sorted by
        // label. Only labels and counts come out — a raw value has no path into the tally,
        // which is what keeps the fail-closed log line safe to emit (M4-R20).
        let e = |kind: PiiKind| PiiEntity {
            kind,
            span: 0..1,
            text: "secret-value".to_string(),
            confidence: Confidence::Structural,
        };
        let hist = kind_histogram(&[e(PiiKind::Person), e(PiiKind::Email), e(PiiKind::Email)]);

        assert_eq!(hist, vec![("EMAIL", 2), ("PERSON", 1)]);
        // The offending text never rides along.
        assert!(!format!("{hist:?}").contains("secret-value"));
    }

    #[test]
    fn demask_leaves_unknown_bracketed_text_untouched() {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect("mail bob@test.com");
        let _ = vault.mask("mail bob@test.com", &entities);

        // Not a known placeholder → passed through verbatim.
        assert_eq!(
            vault.demask("see [TODO 3] and [EMAIL_1]"),
            "see [TODO 3] and bob@test.com"
        );
    }
}
