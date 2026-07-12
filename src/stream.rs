//! Streaming (SSE) de-anonymization for chat-completions responses (milestone M3).
//!
//! The provider streams the answer as Server-Sent Events — `data: {json}` lines
//! whose `choices[].delta.content` carry the answer one token-fragment at a time.
//! A placeholder like `[EMAIL_1]` can be **split across two deltas** (e.g. `[EMA`
//! then `IL_1]`), so we can't just de-mask each delta independently.
//!
//! [`SseDemasker`] keeps a small per-choice **hold-back buffer**: it accumulates
//! delta text, emits everything up to the last point that could still be an
//! incomplete placeholder, and holds the rest until the next delta (or the end of
//! the stream) resolves it. Only response content is rewritten — request-side
//! masking already ran, so nothing raw ever leaves for the provider regardless.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{Value, json};

use crate::pii::anonymizer::Vault;

/// A suffix that could still grow into a placeholder: an unclosed `[` followed
/// only by placeholder-body characters (letters, digits, space, `_`, `-`), up to
/// a small bound. Anything else (a `]`, a stray character, or a too-long run)
/// means it is *not* a partial placeholder and is safe to emit.
static PARTIAL_PLACEHOLDER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[[A-Za-z0-9 _\-]{0,32}$").unwrap());

/// Byte index up to which `pending` is safe to de-mask and emit; `[idx..]` is a
/// possible incomplete placeholder to hold back until more text arrives.
pub(crate) fn split_demaskable(pending: &str) -> usize {
    match pending.rfind('[') {
        None => pending.len(),
        Some(open) => {
            if PARTIAL_PLACEHOLDER.is_match(&pending[open..]) {
                open
            } else {
                pending.len()
            }
        }
    }
}

/// Incremental SSE de-anonymizer for one response stream.
pub struct SseDemasker {
    vault: Vault,
    /// Bytes of a not-yet-complete line (split across upstream chunks).
    line_buf: Vec<u8>,
    /// Per-choice held-back content (a possible split placeholder tail).
    pending: HashMap<usize, String>,
    /// Whether the held-back content was already flushed (at `[DONE]`).
    flushed: bool,
}

impl SseDemasker {
    /// Build a de-masker around the request's populated [`Vault`].
    pub fn new(vault: Vault) -> Self {
        Self {
            vault,
            line_buf: Vec::new(),
            pending: HashMap::new(),
            flushed: false,
        }
    }

    /// Feed raw upstream bytes; return de-masked SSE bytes ready for the client.
    /// Only complete lines are processed; a partial trailing line is buffered.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.line_buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        // `\n` never appears inside a UTF-8 multi-byte sequence, so splitting on
        // it yields complete, decodable lines.
        while let Some(nl) = self.line_buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.line_buf.drain(..=nl).collect();
            line.pop(); // drop the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            self.handle_line(&line, &mut out);
        }
        out
    }

    /// End of stream: flush any held-back content and any partial trailing line.
    pub fn flush(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.line_buf.is_empty() {
            let line = std::mem::take(&mut self.line_buf);
            let line = String::from_utf8_lossy(&line).into_owned();
            let line = line.trim_end_matches(['\r', '\n']);
            self.handle_line(line, &mut out);
        }
        self.flush_pending(&mut out);
        out
    }

    /// Process one complete SSE line into `out` (with its trailing `\n`).
    fn handle_line(&mut self, line: &str, out: &mut Vec<u8>) {
        let Some(data) = line.strip_prefix("data:") else {
            // Blank separators, comments (`:`), `event:`/`id:` — pass through.
            emit_line(out, line);
            return;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            // Flush anything held back *before* the terminator.
            self.flush_pending(out);
            emit_line(out, line);
            return;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(mut value) => {
                self.rewrite_deltas(&mut value);
                emit_line(out, &format!("data: {value}"));
            }
            // Not JSON we understand — forward verbatim, never break the stream.
            Err(_) => emit_line(out, line),
        }
    }

    /// De-mask each `choices[].delta.content` in a parsed chunk, with hold-back.
    fn rewrite_deltas(&mut self, value: &mut Value) {
        let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) else {
            return;
        };
        for (pos, choice) in choices.iter_mut().enumerate() {
            let idx = choice
                .get("index")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(pos);
            if let Some(content) = choice.pointer_mut("/delta/content") {
                if let Value::String(text) = content {
                    *text = self.push_content(idx, text);
                }
            }
        }
    }

    /// Append `incoming` to a choice's buffer and return the safe, de-masked
    /// prefix; the possible-incomplete-placeholder tail stays buffered.
    fn push_content(&mut self, idx: usize, incoming: &str) -> String {
        let combined = {
            let pending = self.pending.entry(idx).or_default();
            pending.push_str(incoming);
            std::mem::take(pending)
        };
        let split = split_demaskable(&combined);
        let emitted = self.vault.demask(&combined[..split]);
        self.pending.insert(idx, combined[split..].to_string());
        emitted
    }

    /// Emit any held-back content as synthesized delta chunks (once).
    fn flush_pending(&mut self, out: &mut Vec<u8>) {
        if self.flushed {
            return;
        }
        self.flushed = true;
        let mut indices: Vec<usize> = self.pending.keys().copied().collect();
        indices.sort_unstable();
        for idx in indices {
            let pending = self.pending.get_mut(&idx).map(std::mem::take).unwrap_or_default();
            if pending.is_empty() {
                continue;
            }
            let text = self.vault.demask(&pending);
            if text.is_empty() {
                continue;
            }
            let chunk = json!({ "choices": [ { "index": idx, "delta": { "content": text } } ] });
            out.extend_from_slice(format!("data: {chunk}\n\n").as_bytes());
        }
    }
}

/// Append a line plus its `\n` terminator.
fn emit_line(out: &mut Vec<u8>, line: &str) {
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::PiiDetector;
    use crate::pii::recognizers::StructuredRecognizers;

    #[test]
    fn split_holds_back_only_a_possible_placeholder_tail() {
        assert_eq!(split_demaskable("hello world"), 11); // no '[' → emit all
        assert_eq!(split_demaskable("hi [EMA"), 3); // hold from the '['
        assert_eq!(split_demaskable("hi [EMAIL_1]"), 12); // closed ']' → emit all
        assert_eq!(split_demaskable("a [EMAIL_1] b [PH"), 14); // hold the trailing partial
        // A '[' followed by clearly non-placeholder text is emitted, not held.
        assert_eq!(split_demaskable("see [note] here"), 15);
        // Bounded: a very long '[' run is not treated as a placeholder.
        let long = format!("x [{}", "A".repeat(40));
        assert_eq!(split_demaskable(&long), long.len());
    }

    /// Build a vault that maps `[EMAIL_1]` → the real email.
    fn vault_with_email() -> Vault {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        let entities = detector.detect("bob@test.com");
        vault.mask("bob@test.com", &entities);
        vault
    }

    /// Collect the de-masked `delta.content` across all emitted `data:` chunks.
    fn collect_content(bytes: &[u8]) -> String {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let mut acc = String::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    continue;
                }
                let v: Value = serde_json::from_str(data).unwrap();
                if let Some(c) = v.pointer("/choices/0/delta/content").and_then(Value::as_str) {
                    acc.push_str(c);
                }
            }
        }
        acc
    }

    #[test]
    fn placeholder_split_across_deltas_is_restored() {
        let mut d = SseDemasker::new(vault_with_email());
        let mut out = Vec::new();
        // `[EMAIL_1]` arrives split across two SSE events.
        out.extend(d.push(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"mail [EMA\"}}]}\n\n"));
        out.extend(d.push(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"IL_1] now\"}}]}\n\n"));
        out.extend(d.push(b"data: [DONE]\n\n"));
        out.extend(d.flush());

        let content = collect_content(&out);
        assert_eq!(content, "mail bob@test.com now");
        let raw = String::from_utf8(out).unwrap();
        assert!(!raw.contains("[EMAIL_1]"), "placeholder leaked to client: {raw}");
        assert!(raw.contains("data: [DONE]"), "terminator must be preserved");
    }

    #[test]
    fn non_data_lines_and_done_pass_through() {
        let mut d = SseDemasker::new(vault_with_email());
        let out = d.push(b": keep-alive comment\n\n");
        assert_eq!(String::from_utf8(out).unwrap(), ": keep-alive comment\n\n");
    }

    #[test]
    fn bytes_split_mid_line_are_reassembled() {
        let mut d = SseDemasker::new(vault_with_email());
        let mut out = Vec::new();
        // The SSE line itself is chopped mid-way between two byte chunks.
        out.extend(d.push(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"con"));
        out.extend(d.push(b"tent\":\"hi [EMAIL_1]\"}}]}\n\n"));
        out.extend(d.flush());
        assert_eq!(collect_content(&out), "hi bob@test.com");
    }
}
