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
use serde_json::{json, Value};

use crate::pii::anonymizer::Vault;
use crate::pipeline::WireSchema;

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

/// Which streamed text field a hold-back buffer belongs to. Each field that can
/// stream independently — and could therefore split a placeholder across two
/// chunks — gets its own buffer.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum StreamKey {
    /// OpenAI `choices[choice].delta.content`.
    Content { choice: usize },
    /// OpenAI `choices[choice].delta.tool_calls[tool].function.arguments`.
    ToolArg { choice: usize, tool: u64 },
    /// Anthropic `content_block_delta` for one content block `index`. `json` is
    /// true for a `tool_use` block streamed as `input_json_delta` (a JSON
    /// fragment — de-masked JSON-aware), false for a `text_delta`.
    Block { index: u64, json: bool },
}

/// Build a synthesized delta chunk carrying the flushed de-masked `text` for a
/// key — the schema is implied by the key variant.
fn flush_chunk(key: &StreamKey, text: &str) -> Value {
    match key {
        StreamKey::Content { choice } => {
            json!({ "choices": [ { "index": choice, "delta": { "content": text } } ] })
        }
        StreamKey::ToolArg { choice, tool } => json!({
            "choices": [ {
                "index": choice,
                "delta": { "tool_calls": [ { "index": tool, "function": { "arguments": text } } ] }
            } ]
        }),
        StreamKey::Block { index, json: false } => json!({
            "type": "content_block_delta",
            "index": index,
            "delta": { "type": "text_delta", "text": text }
        }),
        StreamKey::Block { index, json: true } => json!({
            "type": "content_block_delta",
            "index": index,
            "delta": { "type": "input_json_delta", "partial_json": text }
        }),
    }
}

/// Emit a synthesized flush chunk as a full SSE event. Anthropic events carry an
/// `event:` line the client dispatches on; OpenAI's do not.
fn emit_synth_chunk(key: &StreamKey, text: &str, out: &mut Vec<u8>) {
    let chunk = flush_chunk(key, text);
    match key {
        StreamKey::Block { .. } => out
            .extend_from_slice(format!("event: content_block_delta\ndata: {chunk}\n\n").as_bytes()),
        _ => out.extend_from_slice(format!("data: {chunk}\n\n").as_bytes()),
    }
}

/// Incremental SSE de-anonymizer for one response stream.
///
/// The line-buffering and split-placeholder hold-back are shared; the
/// per-`schema` [rewriter](Self::rewrite_deltas) knows where the text lives in
/// each wire format (OpenAI `choices[].delta` vs Anthropic `content_block_delta`).
pub struct SseDemasker {
    vault: Vault,
    /// Which wire schema the stream speaks (M6).
    schema: WireSchema,
    /// Bytes of a not-yet-complete line (split across upstream chunks).
    line_buf: Vec<u8>,
    /// Held-back text per streamed field (a possible split-placeholder tail).
    /// An entry is *removed* when flushed, so flushing is idempotent.
    pending: HashMap<StreamKey, String>,
    /// An SSE `event:` line held until its `data:` line, so an Anthropic block's
    /// flushed tail can be injected *before* the whole `content_block_stop` frame
    /// (a `content_block_delta` after the stop is protocol-invalid). Always `None`
    /// for OpenAI, whose stream has no `event:` lines.
    pending_event: Option<String>,
}

impl SseDemasker {
    /// Build a de-masker around the request's populated [`Vault`] for a schema.
    pub fn new(vault: Vault, schema: WireSchema) -> Self {
        Self {
            vault,
            schema,
            line_buf: Vec::new(),
            pending: HashMap::new(),
            pending_event: None,
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
        self.flush_pending_event(&mut out);
        out
    }

    /// Process one complete SSE line into `out` (with its trailing `\n`).
    fn handle_line(&mut self, line: &str, out: &mut Vec<u8>) {
        // Hold an `event:` line until its `data:` line arrives, so a flushed
        // content-block tail can be injected ahead of a whole `content_block_stop`
        // frame. OpenAI streams carry no `event:` lines, so this is inert there.
        if line.starts_with("event:") {
            self.flush_pending_event(out);
            self.pending_event = Some(line.to_string());
            return;
        }
        let Some(data) = line.strip_prefix("data:") else {
            // Blank separators, comments (`:`), `id:` — pass through.
            self.flush_pending_event(out);
            emit_line(out, line);
            return;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            // OpenAI's terminator: flush anything held back *before* it.
            self.flush_pending(out);
            self.flush_pending_event(out);
            emit_line(out, line);
            return;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(mut value) => {
                // Anthropic: a block's held-back tail is flushed at its
                // `content_block_stop` — emitted *before* the held `event:` line so
                // the synthesized `content_block_delta` precedes the stop frame.
                if self.schema == WireSchema::Anthropic {
                    if let Some(index) = anthropic_block_stop_index(&value) {
                        self.flush_block(index, out);
                    }
                }
                self.flush_pending_event(out);
                self.rewrite_deltas(&mut value);
                emit_line(out, &format!("data: {value}"));
            }
            // Not JSON we understand — forward verbatim, never break the stream.
            Err(_) => {
                self.flush_pending_event(out);
                emit_line(out, line);
            }
        }
    }

    /// Emit a held `event:` line, if any. Called right before the line that
    /// follows it (its `data:` line, a separator, or stream end).
    fn flush_pending_event(&mut self, out: &mut Vec<u8>) {
        if let Some(event) = self.pending_event.take() {
            emit_line(out, &event);
        }
    }

    /// De-mask the text fields of one parsed SSE `data:` chunk, with hold-back —
    /// dispatched to the schema-specific rewriter.
    fn rewrite_deltas(&mut self, value: &mut Value) {
        match self.schema {
            WireSchema::OpenAi => self.rewrite_openai(value),
            WireSchema::Anthropic => self.rewrite_anthropic(value),
        }
    }

    /// OpenAI: de-mask each `choices[].delta.content` **and** each streamed
    /// `delta.tool_calls[].function.arguments`, with per-field hold-back.
    fn rewrite_openai(&mut self, value: &mut Value) {
        let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) else {
            return;
        };
        for (pos, choice) in choices.iter_mut().enumerate() {
            let choice_idx = choice
                .get("index")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(pos);
            let Some(delta) = choice.get_mut("delta") else {
                continue;
            };
            if let Some(Value::String(text)) = delta.get_mut("content") {
                *text = self.push_buffered(StreamKey::Content { choice: choice_idx }, text);
            }
            if let Some(tool_calls) = delta.get_mut("tool_calls").and_then(Value::as_array_mut) {
                for (tpos, tool_call) in tool_calls.iter_mut().enumerate() {
                    let tool = tool_call
                        .get("index")
                        .and_then(Value::as_u64)
                        .unwrap_or(tpos as u64);
                    if let Some(Value::String(args)) = tool_call.pointer_mut("/function/arguments")
                    {
                        *args = self.push_buffered(
                            StreamKey::ToolArg {
                                choice: choice_idx,
                                tool,
                            },
                            args,
                        );
                    }
                }
            }
        }
    }

    /// Anthropic: de-mask a `content_block_delta` — `text_delta.text` (plain) and
    /// `input_json_delta.partial_json` (JSON-aware) — held back per content block
    /// `index`. A block's held-back tail is flushed at its `content_block_stop`
    /// (in [`handle_line`](Self::handle_line), which owns the frame ordering), so
    /// nothing is ever held past the block it belongs to. Every other event
    /// (`message_*`, `content_block_start`, pings) passes through untouched.
    fn rewrite_anthropic(&mut self, value: &mut Value) {
        if value.get("type").and_then(Value::as_str) != Some("content_block_delta") {
            return;
        }
        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(delta) = value.get_mut("delta") else {
            return;
        };
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                if let Some(Value::String(text)) = delta.get_mut("text") {
                    *text = self.push_buffered(StreamKey::Block { index, json: false }, text);
                }
            }
            Some("input_json_delta") => {
                if let Some(Value::String(pj)) = delta.get_mut("partial_json") {
                    *pj = self.push_buffered(StreamKey::Block { index, json: true }, pj);
                }
            }
            _ => {}
        }
    }

    /// Append `incoming` to a field's buffer and return the safe, de-masked
    /// prefix; the possible-incomplete-placeholder tail stays buffered.
    fn push_buffered(&mut self, key: StreamKey, incoming: &str) -> String {
        let combined = {
            let pending = self.pending.entry(key.clone()).or_default();
            pending.push_str(incoming);
            std::mem::take(pending)
        };
        let split = split_demaskable(&combined);
        let emitted = self.demask_for(&key, &combined[..split]);
        self.pending.insert(key, combined[split..].to_string());
        emitted
    }

    /// De-mask for a field: JSON-aware where the held text is itself a
    /// JSON-encoded string — an OpenAI tool-call `arguments` or an Anthropic
    /// `input_json_delta` fragment (keep a `"`/`\` value valid, M3-R2) — plain
    /// otherwise (`content` / `text_delta`).
    fn demask_for(&self, key: &StreamKey, text: &str) -> String {
        let json_escape = matches!(
            key,
            StreamKey::ToolArg { .. } | StreamKey::Block { json: true, .. }
        );
        if json_escape {
            self.vault.demask_json_string(text)
        } else {
            self.vault.demask(text)
        }
    }

    /// Flush the held-back tail of a single Anthropic content block (at its
    /// `content_block_stop`). At most one buffer matches an `index`.
    fn flush_block(&mut self, index: u64, out: &mut Vec<u8>) {
        let keys: Vec<StreamKey> = self
            .pending
            .keys()
            .filter(|k| matches!(k, StreamKey::Block { index: i, .. } if *i == index))
            .cloned()
            .collect();
        for key in keys {
            self.flush_key(&key, out);
        }
    }

    /// Emit all remaining held-back text as synthesized chunks, in a stable order
    /// so multi-field streams are deterministic. Idempotent — entries are removed
    /// as they flush, so a second call (`[DONE]` then stream end) emits nothing.
    fn flush_pending(&mut self, out: &mut Vec<u8>) {
        let mut keys: Vec<StreamKey> = self.pending.keys().cloned().collect();
        keys.sort_unstable();
        for key in keys {
            self.flush_key(&key, out);
        }
    }

    /// Remove one field's held-back tail, de-mask it, and emit it as a synthesized
    /// delta chunk (skipping an empty result).
    fn flush_key(&mut self, key: &StreamKey, out: &mut Vec<u8>) {
        let Some(pending) = self.pending.remove(key) else {
            return;
        };
        if pending.is_empty() {
            return;
        }
        let text = self.demask_for(key, &pending);
        if text.is_empty() {
            return;
        }
        emit_synth_chunk(key, &text, out);
    }
}

/// Append a line plus its `\n` terminator.
fn emit_line(out: &mut Vec<u8>, line: &str) {
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
}

/// The content-block `index` if `value` is an Anthropic `content_block_stop`
/// event, else `None`. Drives the pre-stop tail flush.
fn anthropic_block_stop_index(value: &Value) -> Option<u64> {
    if value.get("type").and_then(Value::as_str) == Some("content_block_stop") {
        Some(value.get("index").and_then(Value::as_u64).unwrap_or(0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::recognizers::StructuredRecognizers;
    use crate::pii::PiiDetector;

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
                if let Some(c) = v
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
                    acc.push_str(c);
                }
            }
        }
        acc
    }

    #[test]
    fn placeholder_split_across_deltas_is_restored() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::OpenAi);
        let mut out = Vec::new();
        // `[EMAIL_1]` arrives split across two SSE events.
        out.extend(d.push(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"mail [EMA\"}}]}\n\n",
        ));
        out.extend(d.push(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"IL_1] now\"}}]}\n\n",
        ));
        out.extend(d.push(b"data: [DONE]\n\n"));
        out.extend(d.flush());

        let content = collect_content(&out);
        assert_eq!(content, "mail bob@test.com now");
        let raw = String::from_utf8(out).unwrap();
        assert!(
            !raw.contains("[EMAIL_1]"),
            "placeholder leaked to client: {raw}"
        );
        assert!(raw.contains("data: [DONE]"), "terminator must be preserved");
    }

    /// Sum a streamed `tool_calls[0].function.arguments` across `data:` chunks.
    fn collect_tool_args(bytes: &[u8]) -> String {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let mut acc = String::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    continue;
                }
                let v: Value = serde_json::from_str(data).unwrap();
                if let Some(a) = v
                    .pointer("/choices/0/delta/tool_calls/0/function/arguments")
                    .and_then(Value::as_str)
                {
                    acc.push_str(a);
                }
            }
        }
        acc
    }

    fn sse_event(v: Value) -> Vec<u8> {
        format!("data: {v}\n\n").into_bytes()
    }

    #[test]
    fn tool_call_arguments_split_across_deltas_are_restored() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::OpenAi);
        let mut out = Vec::new();
        // The tool-call arguments JSON carries `[EMAIL_1]`, split across two events.
        out.extend(d.push(&sse_event(json!({
            "choices": [ { "index": 0, "delta": {
                "tool_calls": [ { "index": 0, "function": { "arguments": "{\"to\":\"[EMA" } } ]
            } } ]
        }))));
        out.extend(d.push(&sse_event(json!({
            "choices": [ { "index": 0, "delta": {
                "tool_calls": [ { "index": 0, "function": { "arguments": "IL_1]\"}" } } ]
            } } ]
        }))));
        out.extend(d.push(b"data: [DONE]\n\n"));
        out.extend(d.flush());

        assert_eq!(collect_tool_args(&out), r#"{"to":"bob@test.com"}"#);
        let raw = String::from_utf8(out).unwrap();
        assert!(
            !raw.contains("[EMAIL_1]"),
            "placeholder leaked in tool args: {raw}"
        );
    }

    #[test]
    fn tool_call_arguments_deanon_stays_valid_json() {
        // M3-R2: a value with a `"` de-masked into streamed tool-call arguments
        // must keep the arguments valid inner JSON.
        use crate::pii::{Confidence, PiiEntity, PiiKind};
        let value = r#"Ac"me"#;
        let mut vault = Vault::new();
        let entity = PiiEntity {
            kind: PiiKind::Person,
            span: 0..value.len(),
            text: value.to_string(),
            confidence: Confidence::Structural,
        };
        vault.mask(value, std::slice::from_ref(&entity));

        let mut d = SseDemasker::new(vault, WireSchema::OpenAi);
        let mut out = Vec::new();
        out.extend(d.push(&sse_event(json!({
            "choices": [ { "index": 0, "delta": {
                "tool_calls": [ { "index": 0, "function": { "arguments": "{\"vendor\":\"[PERSON_1]\"}" } } ]
            } } ]
        }))));
        out.extend(d.push(b"data: [DONE]\n\n"));
        out.extend(d.flush());

        let args = collect_tool_args(&out);
        let parsed: Value = serde_json::from_str(&args).expect("valid streamed tool args JSON");
        assert_eq!(parsed["vendor"], r#"Ac"me"#);
    }

    #[test]
    fn non_data_lines_and_done_pass_through() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::OpenAi);
        let out = d.push(b": keep-alive comment\n\n");
        assert_eq!(String::from_utf8(out).unwrap(), ": keep-alive comment\n\n");
    }

    #[test]
    fn bytes_split_mid_line_are_reassembled() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::OpenAi);
        let mut out = Vec::new();
        // The SSE line itself is chopped mid-way between two byte chunks.
        out.extend(d.push(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"con"));
        out.extend(d.push(b"tent\":\"hi [EMAIL_1]\"}}]}\n\n"));
        out.extend(d.flush());
        assert_eq!(collect_content(&out), "hi bob@test.com");
    }

    // ── Anthropic native `/v1/messages` SSE (M6) ────────────────────────────

    /// A full Anthropic SSE frame: an `event:` line + a `data:` line + blank.
    fn anthropic_event(event: &str, data: Value) -> Vec<u8> {
        format!("event: {event}\ndata: {data}\n\n").into_bytes()
    }

    /// Sum `content_block_delta` → `text_delta.text` across the emitted stream.
    fn collect_anthropic_text(bytes: &[u8]) -> String {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let mut acc = String::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                    if v.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                        if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                            acc.push_str(t);
                        }
                    }
                }
            }
        }
        acc
    }

    /// Sum `content_block_delta` → `input_json_delta.partial_json` for one block.
    fn collect_anthropic_partial_json(bytes: &[u8], index: u64) -> String {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let mut acc = String::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                    let is_block = v.get("type").and_then(Value::as_str)
                        == Some("content_block_delta")
                        && v.get("index").and_then(Value::as_u64) == Some(index);
                    if is_block {
                        if let Some(pj) = v.pointer("/delta/partial_json").and_then(Value::as_str) {
                            acc.push_str(pj);
                        }
                    }
                }
            }
        }
        acc
    }

    #[test]
    fn anthropic_text_delta_split_across_events_is_restored() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::Anthropic);
        let mut out = Vec::new();
        // `[EMAIL_1]` arrives split across two `text_delta` events for block 0.
        out.extend(d.push(&anthropic_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "mail [EMA" } }),
        )));
        out.extend(d.push(&anthropic_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "IL_1] now" } }),
        )));
        out.extend(d.push(&anthropic_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 0 }),
        )));
        out.extend(d.flush());

        assert_eq!(collect_anthropic_text(&out), "mail bob@test.com now");
        let raw = String::from_utf8(out).unwrap();
        assert!(!raw.contains("[EMAIL_1]"), "placeholder leaked: {raw}");
        assert!(raw.contains("content_block_stop"), "stop preserved");
    }

    #[test]
    fn anthropic_input_json_delta_split_is_restored_and_valid() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::Anthropic);
        let mut out = Vec::new();
        // A `tool_use` block streams its arguments as `input_json_delta` fragments
        // carrying `[EMAIL_1]`, split across two events.
        out.extend(d.push(&anthropic_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "input_json_delta", "partial_json": "{\"to\":\"[EMA" } }),
        )));
        out.extend(d.push(&anthropic_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "input_json_delta", "partial_json": "IL_1]\"}" } }),
        )));
        out.extend(d.push(&anthropic_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 1 }),
        )));
        out.extend(d.flush());

        let pj = collect_anthropic_partial_json(&out, 1);
        assert_eq!(pj, r#"{"to":"bob@test.com"}"#);
        let parsed: Value = serde_json::from_str(&pj).expect("reassembled JSON is valid");
        assert_eq!(parsed["to"], "bob@test.com");
    }

    #[test]
    fn anthropic_held_back_tail_is_flushed_before_its_block_stop() {
        // A block ending on a partial-placeholder-looking tail (`[EM`) holds it
        // back; the `content_block_stop` must flush it, and the flushed delta must
        // precede the stop frame (a delta after the stop is protocol-invalid).
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::Anthropic);
        let mut out = Vec::new();
        out.extend(d.push(&anthropic_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "done [EM" } }),
        )));
        out.extend(d.push(&anthropic_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 0 }),
        )));
        out.extend(d.flush());

        assert_eq!(collect_anthropic_text(&out), "done [EM");
        let raw = String::from_utf8(out).unwrap();
        let flushed = raw.find("text_delta").expect("tail flushed as a delta");
        let stop = raw.find("content_block_stop").expect("stop present");
        assert!(flushed < stop, "flush must precede the stop frame:\n{raw}");
    }

    #[test]
    fn anthropic_non_delta_events_pass_through() {
        let mut d = SseDemasker::new(vault_with_email(), WireSchema::Anthropic);
        let out = d.push(&anthropic_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "m1" } }),
        ));
        let raw = String::from_utf8(out).unwrap();
        assert!(
            raw.contains("event: message_start"),
            "event line kept: {raw}"
        );
        assert!(raw.contains("\"id\":\"m1\""), "payload kept: {raw}");
    }
}
