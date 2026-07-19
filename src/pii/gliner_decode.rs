//! Pure decoding of GLiNER span output into [`PiiEntity`] spans — milestone M8.
//!
//! GLiNER is **not** token-classification (that is [`ner_decode`](super::ner_decode)):
//! it is a *zero-shot, open-label span extractor*. You pass entity **types as text**
//! (`"person"`, `"phone number"`, `"address"`, …); the model scores every candidate
//! **word-span × type** pair, and detection is a **sigmoid threshold + greedy
//! non-overlapping selection** over those scores. This module owns that decode plus
//! the label→[`PiiKind`] map and the whitespace word split — all deterministic and
//! unit-testable **without** a model or the native ONNX runtime. [`gliner`](super::gliner)
//! feeds it the real per-span logits.
//!
//! **Why GLiNER maps to *structured* kinds too (`phone number` → [`PiiKind::Phone`]),
//! unlike the token-classification NER.** [`ner_decode::label_to_kind`](super::ner_decode::label_to_kind)
//! deliberately drops structured categories, because the deterministic recognizers own
//! them and the XLM-R NER would only add noise. GLiNER is the *opposite* case: its whole
//! reason to exist (M8) is **contextual PII the deterministic layer cannot catch** — a
//! bare national phone with no `+CC` anchor, a free-form address. So it *is* allowed to
//! emit `Phone`/`Location` for those. This is safe because the hybrid
//! [`resolve_overlaps`](super::overlap::resolve_overlaps) dedups a GLiNER guess against a
//! deterministic match (the checksum-backed one wins), and an ML false positive is an
//! **over-mask, never a leak** — the standing tradeoff. Email is deliberately *not*
//! mapped: the deterministic email regex is authoritative and reliable, so a GLiNER email
//! would only add false positives.

use once_cell::sync::Lazy;
use regex::Regex;

use super::{Confidence, PiiEntity, PiiKind};

/// GLiNER's `whitespace` words-splitter, verbatim from the reference library
/// (`gliner/data_processing/tokenizer.py`): a run of word characters (with internal
/// `-`/`_`) **or** a single non-space character. Crucially it separates trailing
/// punctuation — `"Milano."` → `["Milano", "."]` — which the model was trained on, so a
/// pure-whitespace split (`"Milano."` as one word) measurably lowers its scores.
static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\w+(?:[-_]\w+)*|\S").unwrap());

/// One entity **type** handed to GLiNER: the natural-language label the model matches
/// on (`"phone number"`) plus the [`PiiKind`] a hit of it becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlinerLabel {
    /// The label text fed to the model (used to build the prompt in [`gliner`](super::gliner)).
    pub text: String,
    /// The kind a detection of this label is masked as.
    pub kind: PiiKind,
}

/// The raw score of one candidate **(word-span, type)** pair, as read off the model's
/// span logits. `start_word`/`end_word` are **inclusive** word indices into the
/// whitespace word list; `type_idx` indexes the [`GlinerLabel`] list; `logit` is the
/// pre-sigmoid score.
#[derive(Debug, Clone, Copy)]
pub struct SpanScore {
    pub start_word: usize,
    pub end_word: usize,
    pub type_idx: usize,
    pub logit: f32,
}

/// Numerically-stable logistic sigmoid, `1/(1+e^-x)`, mapping a logit to a probability.
fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// Map a GLiNER label (natural-language type) to a [`PiiKind`], case-insensitively.
///
/// Covers the named-entity kinds (person/org/location) **and** the contextual
/// structured kinds GLiNER exists to add (`phone number` → [`PiiKind::Phone`],
/// `address` → [`PiiKind::Location`]). Returns `None` for an unrecognised label — the
/// caller ([`parse_gliner_labels`]) treats that as a **config error** (fail closed),
/// never a silently-ignored label.
pub fn gliner_label_to_kind(label: &str) -> Option<PiiKind> {
    match label.trim().to_ascii_lowercase().as_str() {
        "person" | "name" | "full name" | "person name" | "people" => Some(PiiKind::Person),
        "organization" | "organisation" | "company" | "org" => Some(PiiKind::Organization),
        "location" | "address" | "city" | "place" | "gpe" | "country" => Some(PiiKind::Location),
        "phone number" | "phone" | "telephone" | "telephone number" | "mobile number" => {
            Some(PiiKind::Phone)
        }
        _ => None,
    }
}

/// Parse a comma-separated `GLINER_LABELS` spec into typed labels, mapping each to a
/// [`PiiKind`]. An unmappable label is an **error** (fail closed): a typo must never
/// silently disable a whole category of detection. An empty spec is an error too.
pub fn parse_gliner_labels(spec: &str) -> Result<Vec<GlinerLabel>, String> {
    let mut out = Vec::new();
    for raw in spec.split(',') {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let kind = gliner_label_to_kind(text)
            .ok_or_else(|| format!("GLINER_LABELS: label {text:?} maps to no PiiKind"))?;
        out.push(GlinerLabel {
            text: text.to_string(),
            kind,
        });
    }
    if out.is_empty() {
        return Err("GLINER_LABELS is empty".to_string());
    }
    Ok(out)
}

/// The default label set when `GLINER_LABELS` is unset: the three named-entity kinds
/// (parity with XLM-R, the successor case) plus the two contextual kinds that are the
/// point of M8 — a `phone number` the deterministic layer can't anchor, and a free-form
/// `address`.
pub fn default_gliner_labels() -> Vec<GlinerLabel> {
    [
        "person",
        "organization",
        "location",
        "phone number",
        "address",
    ]
    .into_iter()
    .map(|t| GlinerLabel {
        text: t.to_string(),
        // Every default label is in `gliner_label_to_kind`, so this cannot panic;
        // the unit test `default_labels_all_map` pins that.
        kind: gliner_label_to_kind(t).expect("default GLiNER label maps to a kind"),
    })
    .collect()
}

/// Split `text` into words the way GLiNER's `whitespace` words-splitter does
/// ([`WORD_RE`]): word-character runs, with punctuation as its own word. Returns **byte**
/// `(start, end)` spans; empty for all-whitespace.
///
/// GLiNER scores spans over **words**, not sub-word tokens, so this is the coordinate
/// system every span index in [`SpanScore`] refers to. It must match the reference
/// splitter exactly — a mismatch (e.g. `"Milano."` kept whole) lowers the model's scores.
/// Kept here (model-independent) so it is unit-tested without a tokenizer.
pub fn split_words(text: &str) -> Vec<(usize, usize)> {
    WORD_RE
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Decode GLiNER span logits into [`PiiEntity`] spans.
///
/// For each candidate `(word-span, type)` whose `sigmoid(logit) >= threshold`, take the
/// span, then resolve conflicts **greedily**: highest probability first, and a span is
/// accepted only if none of its words are already covered (flat NER — GLiNER does not
/// nest by default). This mirrors GLiNER's reference greedy decoder. Ties are broken
/// deterministically (probability, then earliest start, then longest span, then kind) so
/// placeholder numbering downstream is reproducible.
///
/// `word_spans` are the byte spans from [`split_words`] on the **same** `text`; a
/// selected word-span `[i..=j]` becomes the byte range `word_spans[i].0 .. word_spans[j].1`.
/// All hits are [`Confidence::Structural`] — an ML guess is never checksum-verified.
pub fn decode_spans(
    text: &str,
    word_spans: &[(usize, usize)],
    labels: &[GlinerLabel],
    scores: &[SpanScore],
    threshold: f32,
) -> Vec<PiiEntity> {
    struct Cand {
        prob: f32,
        start_word: usize,
        end_word: usize,
        kind: PiiKind,
    }

    let mut cands: Vec<Cand> = Vec::new();
    for s in scores {
        // A malformed span (start after end, or off the end of the word list) is
        // dropped, never indexed — the offsets come from the model, so treat them as
        // untrusted (the M2-R6 / M5-R3 discipline).
        if s.start_word > s.end_word || s.end_word >= word_spans.len() {
            continue;
        }
        let Some(label) = labels.get(s.type_idx) else {
            continue;
        };
        let prob = sigmoid(s.logit);
        if prob < threshold {
            continue;
        }
        cands.push(Cand {
            prob,
            start_word: s.start_word,
            end_word: s.end_word,
            kind: label.kind,
        });
    }

    cands.sort_by(|a, b| {
        b.prob
            .total_cmp(&a.prob)
            .then(a.start_word.cmp(&b.start_word))
            .then(b.end_word.cmp(&a.end_word)) // longer span first
            .then((a.kind as usize).cmp(&(b.kind as usize)))
    });

    let mut used = vec![false; word_spans.len()];
    let mut out = Vec::new();
    for c in cands {
        if (c.start_word..=c.end_word).any(|w| used[w]) {
            continue;
        }
        used[c.start_word..=c.end_word].fill(true);
        let start = word_spans[c.start_word].0;
        let end = word_spans[c.end_word].1;
        match text.get(start..end) {
            Some(slice) => {
                // Word spans exclude whitespace by construction, but trim defensively so
                // the masked span matches the value exactly (mirrors ner_decode).
                let lead = slice.len() - slice.trim_start().len();
                let trimmed = slice.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let start = start + lead;
                out.push(PiiEntity {
                    kind: c.kind,
                    span: start..start + trimmed.len(),
                    text: trimmed.to_string(),
                    confidence: Confidence::Structural,
                });
            }
            // Off a char boundary — surface it, never index-panic, never log the text.
            None => {
                tracing::warn!(kind = ?c.kind, "GLiNER span off a char boundary; entity dropped")
            }
        }
    }

    out.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then(a.span.end.cmp(&b.span.end))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(start_word: usize, end_word: usize, type_idx: usize, logit: f32) -> SpanScore {
        SpanScore {
            start_word,
            end_word,
            type_idx,
            logit,
        }
    }

    #[test]
    fn split_words_handles_padding_and_multibyte() {
        assert_eq!(split_words("Mario Rossi"), vec![(0, 5), (6, 11)]);
        // Leading / trailing / repeated whitespace collapses to word runs only.
        assert_eq!(split_words("  a   bb "), vec![(2, 3), (6, 8)]);
        assert_eq!(split_words(""), Vec::<(usize, usize)>::new());
        assert_eq!(split_words("   "), Vec::<(usize, usize)>::new());
        // Multi-byte: "café Öl" — bytes: café=5 (é is 2 bytes), space, Öl=3 (Ö is 2).
        let t = "café Öl";
        let w = split_words(t);
        assert_eq!(w.len(), 2);
        assert_eq!(&t[w[0].0..w[0].1], "café");
        assert_eq!(&t[w[1].0..w[1].1], "Öl");
    }

    #[test]
    fn split_words_separates_trailing_punctuation_like_the_reference() {
        // The whole reason to match GLiNER's regex splitter: "Milano." must be
        // ["Milano", "."], not one word, or the model scores the entity lower.
        let t = "in Milano.";
        let w = split_words(t);
        assert_eq!(
            w.iter().map(|&(a, b)| &t[a..b]).collect::<Vec<_>>(),
            vec!["in", "Milano", "."]
        );
        // Hyphen/underscore stay inside a word (e-mail-style).
        assert_eq!(
            split_words("well-known e_mail")
                .iter()
                .map(|&(a, b)| &"well-known e_mail"[a..b])
                .collect::<Vec<_>>(),
            vec!["well-known", "e_mail"]
        );
    }

    #[test]
    fn label_mapping_covers_names_and_contextual_structured() {
        assert_eq!(gliner_label_to_kind("person"), Some(PiiKind::Person));
        assert_eq!(
            gliner_label_to_kind("Organization"),
            Some(PiiKind::Organization)
        );
        assert_eq!(gliner_label_to_kind("location"), Some(PiiKind::Location));
        // The point of M8: contextual structured kinds the deterministic layer misses.
        assert_eq!(gliner_label_to_kind("phone number"), Some(PiiKind::Phone));
        assert_eq!(gliner_label_to_kind("address"), Some(PiiKind::Location));
        // Email stays with the deterministic layer — deliberately unmapped.
        assert_eq!(gliner_label_to_kind("email"), None);
        assert_eq!(gliner_label_to_kind("misc"), None);
    }

    #[test]
    fn default_labels_all_map() {
        // default_gliner_labels `.expect()`s each label maps — pin that it can't panic.
        let labels = default_gliner_labels();
        assert!(labels.iter().any(|l| l.kind == PiiKind::Phone));
        assert!(labels.iter().any(|l| l.kind == PiiKind::Person));
        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn parsing_labels_fails_closed_on_a_bad_label() {
        let ok = parse_gliner_labels("person, phone number , address").unwrap();
        assert_eq!(ok.len(), 3);
        assert_eq!(ok[1].kind, PiiKind::Phone);
        // A typo maps to nothing → error, not a silently-dropped category.
        assert!(parse_gliner_labels("person, phon numbr").is_err());
        assert!(parse_gliner_labels("   ").is_err());
        assert!(parse_gliner_labels("").is_err());
    }

    #[test]
    fn a_single_span_above_threshold_becomes_an_entity() {
        let text = "call Mario Rossi now";
        let words = split_words(text); // [call][Mario][Rossi][now]
        let labels = default_gliner_labels();
        let person = labels
            .iter()
            .position(|l| l.kind == PiiKind::Person)
            .unwrap();
        // Span words 1..=2 = "Mario Rossi".
        let scores = [s(1, 2, person, 3.0)];
        let got = decode_spans(text, &words, &labels, &scores, 0.5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Person);
        assert_eq!(got[0].text, "Mario Rossi");
        assert_eq!(got[0].span, 5..16);
        assert_eq!(got[0].confidence, Confidence::Structural);
    }

    #[test]
    fn below_threshold_is_dropped() {
        let text = "call Mario now";
        let words = split_words(text);
        let labels = default_gliner_labels();
        // logit 0.0 → sigmoid 0.5; with threshold 0.6 it's below.
        let scores = [s(1, 1, 0, 0.0)];
        assert!(decode_spans(text, &words, &labels, &scores, 0.6).is_empty());
        // …but at threshold 0.5 exactly, 0.5 >= 0.5 passes.
        assert_eq!(decode_spans(text, &words, &labels, &scores, 0.5).len(), 1);
    }

    #[test]
    fn greedy_keeps_the_higher_score_on_overlap() {
        let text = "New York City center";
        let words = split_words(text); // [New][York][City][center]
        let labels = default_gliner_labels();
        let loc = labels.iter().position(|l| l.text == "location").unwrap();
        // Two overlapping location spans: "New York" (0..=1, weaker) and
        // "New York City" (0..=2, stronger). The stronger one wins; the weaker,
        // overlapping it, is dropped.
        let scores = [s(0, 1, loc, 1.0), s(0, 2, loc, 2.5)];
        let got = decode_spans(text, &words, &labels, &scores, 0.5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "New York City");
    }

    #[test]
    fn adjacent_non_overlapping_spans_are_both_kept() {
        let text = "Mario Rossi met Anna Bianchi";
        let words = split_words(text); // [Mario][Rossi][met][Anna][Bianchi]
        let labels = default_gliner_labels();
        let person = labels
            .iter()
            .position(|l| l.kind == PiiKind::Person)
            .unwrap();
        let scores = [s(0, 1, person, 3.0), s(3, 4, person, 3.0)];
        let got = decode_spans(text, &words, &labels, &scores, 0.5);
        assert_eq!(
            got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            vec!["Mario Rossi", "Anna Bianchi"]
        );
    }

    #[test]
    fn phone_label_yields_a_phone_kind() {
        // The recall gap M8 exists for: an un-anchored national phone tagged by context.
        let text = "ring me on 020 7946 0958 today";
        let words = split_words(text); // [ring][me][on][020][7946][0958][today]
        let labels = default_gliner_labels();
        let phone = labels
            .iter()
            .position(|l| l.kind == PiiKind::Phone)
            .unwrap();
        let scores = [s(3, 5, phone, 4.0)];
        let got = decode_spans(text, &words, &labels, &scores, 0.5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Phone);
        assert_eq!(got[0].text, "020 7946 0958");
    }

    #[test]
    fn out_of_range_or_malformed_spans_are_skipped_not_indexed() {
        let text = "a b";
        let words = split_words(text); // 2 words
        let labels = default_gliner_labels();
        let scores = [
            s(0, 9, 0, 5.0),   // end_word off the end
            s(2, 2, 0, 5.0),   // start_word off the end
            s(1, 0, 0, 5.0),   // start after end
            s(0, 0, 999, 5.0), // type off the end
        ];
        assert!(decode_spans(text, &words, &labels, &scores, 0.5).is_empty());
    }
}
