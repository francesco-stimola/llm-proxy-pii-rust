//! The privacy stage: detect PII in the outgoing request, mask it, teach the
//! model how to read the placeholders, and restore the originals in the incoming
//! response. The only stage wired in the current milestone.
//!
//! ## Two wire schemas (M6)
//!
//! The masking *engine* (`mask_all`, the `Vault`, the fixpoint) is
//! schema-agnostic; what differs per schema is the field-by-field walk of the
//! body. The stage dispatches on [`RequestContext::schema`]:
//! [`WireSchema::OpenAi`] runs the Chat Completions walk documented below;
//! [`WireSchema::Anthropic`] runs the native `/v1/messages` walk (in the
//! *Anthropic native* section). Both share one `Vault` per request.
//!
//! ## What gets masked (request)
//!
//! Every text-bearing field of the OpenAI chat payload is masked with a shared
//! per-request [`Vault`], so the same real value maps to the same `[KIND_N]`
//! token everywhere it appears. Coverage (an unscanned text field is a leak):
//!
//! - `messages[].content` — a string, or the `text` of each content part
//! - `messages[].name` — the author/participant name
//! - `messages[].tool_calls[].function.arguments` and the legacy
//!   `messages[].function_call.arguments`
//! - `tools[].function.description` and every `description` inside
//!   `tools[].function.parameters`
//!
//! **Fail closed**: if a `content` field has a shape we can't safely interpret
//! (e.g. a bare object), or `messages` is missing/not an array, the stage marks
//! the request blocked rather than forwarding it un-masked.
//!
//! If anything was masked, a system message is injected (or merged into the
//! existing one) explaining the placeholders — see [`AUGMENTATION_PROMPT`].
//!
//! ## What gets restored (response)
//!
//! `choices[].message.content`, `choices[].message.tool_calls[].function.arguments`,
//! and the legacy `function_call.arguments` — so the client runs tools and reads
//! text with the real values.

use serde_json::{json, Value};

use crate::pii::anonymizer::Vault;
use crate::pii::PiiDetector;
use crate::pipeline::{RequestContext, Stage, WireSchema};
use crate::proxy::{ProxyRequest, ProxyResponse};

/// System instruction injected when the request contained PII, so the model
/// treats placeholders as real typed values and uses them verbatim.
const AUGMENTATION_PROMPT: &str = "\
Some real values in this conversation have been replaced with typed placeholders \
of the form [KIND_N] — for example [EMAIL_1], [PHONE_2], [PERSON_1], [IBAN_1]. \
Each placeholder stands for a real value of the named kind. Treat every \
placeholder as if it were the real value it represents: use it verbatim wherever \
you would use that value, including inside tool/function-call arguments, and \
never modify, translate, reformat, split, merge, or guess the value behind it. \
The same placeholder always refers to the same value. Placeholders are \
automatically restored to the real values after your reply, before anyone sees \
it, so you never need to ask the user to reveal the real value.";

/// Masks PII on the way out and restores it on the way back, using an
/// engine-agnostic [`PiiDetector`].
pub struct PrivacyStage {
    detector: Box<dyn PiiDetector>,
    /// Per-request validation allowance handed to every field of one request (M10-R28).
    validation_budget: usize,
}

impl PrivacyStage {
    /// Build the stage around a concrete detector (deterministic, ML, or both), with the shipped
    /// per-request validation allowance.
    pub fn new(detector: Box<dyn PiiDetector>) -> Self {
        Self::with_validation_budget(
            detector,
            crate::pii::recognizers::MAX_PHONE_VALIDATIONS_PER_REQUEST,
        )
    }

    /// Build the stage with an explicit per-request validation allowance (M10-R28).
    ///
    /// **This exists so a guard can reach a refusal without paying for one.** The shipped allowance
    /// is ~1.6 s of CPU in `--release` and ~25 s unoptimized; DOS-07, DOS-06 and E2E-05 all need to
    /// *cross* it, and three cases at 25 s each is how a guard ends up `#[ignore]`d — which in this
    /// milestone alone has hidden three findings. Lowering the number does not weaken any of them:
    /// what they assert is that the allowance belongs to the **request** and that exhausting it
    /// refuses rather than truncates, and neither claim is about the size of the number.
    ///
    /// The number itself is pinned separately, on the shipped constant, by DOS-BUD.
    pub fn with_validation_budget(detector: Box<dyn PiiDetector>, units: usize) -> Self {
        Self {
            detector,
            validation_budget: units,
        }
    }
}

impl Stage for PrivacyStage {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn on_request(&self, req: &mut ProxyRequest, ctx: &mut RequestContext) {
        let schema = ctx.schema;
        // Scope the vault borrow so it's released before we re-read the vault.
        // `detect_error` captures a *required* detector failure so we can fail
        // closed after the walk (its message carries no input text).
        let mut detect_error: Option<crate::pii::DetectError> = None;
        // **One validation budget for the whole request, created here beside the vault (M10-R28).**
        // The vault is already per-request for the same reason: both are state that belongs to the
        // request rather than to a field. Before this, each field minted its own allowance and each
        // of `mask_all`'s five passes minted another, so the ceiling was `budget × fields × passes`
        // — and every one of those factors is chosen by the client. A budget scoped to something the
        // caller can multiply is a rate, not a bound.
        let budget = crate::pii::Budget::new(self.validation_budget);
        let outcome = {
            let detector = self.detector.as_ref();
            let vault = &mut ctx.vault;
            let error_slot = &mut detect_error;
            let budget = &budget;
            // `mask_all`, not `mask`: masking rewrites the bytes around what it replaced and
            // can *expose* a value that was not recognizable before (a phone inside a longer
            // digit run splits it and reveals a Luhn-valid card), so it re-detects to a
            // fixpoint — M4-R17.
            let mut mask = |text: &str| match vault.mask_all(text, detector, budget) {
                Ok(masked) => masked,
                Err(err) => {
                    if error_slot.is_none() {
                        *error_slot = Some(err);
                    }
                    // Leave the text as-is; we block below and never forward it.
                    text.to_string()
                }
            };
            // Dispatch to the schema-specific walk (M6). The masking engine
            // itself (`mask_all`, the fixpoint, the vault) is schema-agnostic;
            // only the field-by-field traversal of the body differs.
            match schema {
                WireSchema::OpenAi => mask_request(&mut req.body, &mut mask),
                WireSchema::Anthropic => mask_anthropic_request(&mut req.body, &mut mask),
            }
        };

        // Fail closed: a required detector errored → block, don't forward.
        if let Some(err) = detect_error {
            ctx.block(err.to_string());
            return;
        }
        if let Err(reason) = outcome {
            ctx.block(reason);
            return;
        }

        // Only touch the prompt when we actually masked something.
        if !ctx.vault.is_empty() {
            match schema {
                WireSchema::OpenAi => inject_augmentation(&mut req.body),
                WireSchema::Anthropic => inject_augmentation_anthropic(&mut req.body),
            }
        }
    }

    fn on_response(&self, resp: &mut ProxyResponse, ctx: &mut RequestContext) {
        if ctx.vault.is_empty() {
            return;
        }
        match ctx.schema {
            WireSchema::OpenAi => demask_response(&mut resp.body, &ctx.vault),
            WireSchema::Anthropic => demask_anthropic_response(&mut resp.body, &ctx.vault),
        }
    }
}

// ── Request masking ────────────────────────────────────────────────────────

/// Mask every text-bearing field of an outgoing request. Returns `Err(reason)`
/// to fail closed on an unrecognized shape.
fn mask_request(body: &mut Value, f: &mut dyn FnMut(&str) -> String) -> Result<(), String> {
    let messages = body
        .get_mut("messages")
        .ok_or("request has no `messages` field")?
        .as_array_mut()
        .ok_or("`messages` is not an array")?;

    for message in messages {
        if let Some(content) = message.get_mut("content") {
            mask_content(content, f)?;
        }
        if let Some(name) = message.get_mut("name") {
            transform_string_value(name, f);
        }
        transform_tool_call_args(message, f);
        if let Some(args) = message.pointer_mut("/function_call/arguments") {
            transform_string_value(args, f);
        }
    }

    // Tool/function definitions: free-text descriptions can carry example PII.
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if let Some(description) = tool.pointer_mut("/function/description") {
                transform_string_value(description, f);
            }
            if let Some(parameters) = tool.pointer_mut("/function/parameters") {
                mask_schema_descriptions(parameters, f);
            }
        }
    }

    Ok(())
}

/// Mask a message `content`: a string, or the `text` of each content part.
/// Fails closed on an object/scalar we can't interpret as text.
fn mask_content(content: &mut Value, f: &mut dyn FnMut(&str) -> String) -> Result<(), String> {
    match content {
        Value::String(s) => {
            *s = f(s);
            Ok(())
        }
        // Multimodal content is an array of parts. Fail closed on anything we
        // can't account for, so a stray element never smuggles PII past us.
        Value::Array(parts) => {
            for part in parts {
                if let Value::String(s) = part {
                    // A bare string element *is* text — mask it (skipping = leak).
                    *s = f(s);
                } else if part.is_object() {
                    // A part object: mask any `text` it carries (covers
                    // `{"type":"text"}` and future text-bearing parts); a part
                    // with no `text` (image_url, input_audio, …) is non-text.
                    if let Some(text) = part.get_mut("text") {
                        transform_string_value(text, f);
                    }
                } else {
                    // number / bool / null / nested array — uninterpretable.
                    return Err("message `content` array has an unrecognized element".to_string());
                }
            }
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err("message `content` has an unrecognized shape".to_string()),
    }
}

/// Recursively mask every `description` string inside a JSON-Schema value,
/// leaving structural fields (`enum`, `const`, `default`, …) untouched.
fn mask_schema_descriptions(schema: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    match schema {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if key == "description" {
                    transform_string_value(value, f);
                } else {
                    mask_schema_descriptions(value, f);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                mask_schema_descriptions(item, f);
            }
        }
        _ => {}
    }
}

// ── Response restoring ─────────────────────────────────────────────────────

/// Restore placeholders in every text-bearing field of a response.
///
/// `content` is plain text (plain demask); `tool_calls[].function.arguments` and the
/// legacy `function_call.arguments` are **JSON-encoded strings**, so they get the
/// JSON-aware demask that keeps a value with a `"`/`\` valid inner JSON (M3-R2).
fn demask_response(body: &mut Value, vault: &Vault) {
    let Some(choices) = body.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message") else {
            continue;
        };
        if let Some(content) = message.get_mut("content") {
            demask_content(content, &mut |text| vault.demask(text));
        }
        if let Some(tool_calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut) {
            for tool_call in tool_calls {
                if let Some(args) = tool_call.pointer_mut("/function/arguments") {
                    transform_string_value(args, &mut |text| vault.demask_json_string(text));
                }
            }
        }
        if let Some(args) = message.pointer_mut("/function_call/arguments") {
            transform_string_value(args, &mut |text| vault.demask_json_string(text));
        }
    }
}

/// Restore a response `content`: a string, or the `text` of each content part.
///
/// Mirrors `mask_content` so a placeholder can't reach the client un-restored,
/// including a bare-string element in a content array. De-masking is the safe
/// direction, so an unrecognized element is left as-is rather than blocking.
fn demask_content(content: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    match content {
        Value::String(s) => *s = f(s),
        Value::Array(parts) => {
            for part in parts {
                if let Value::String(s) = part {
                    *s = f(s);
                } else if let Some(text) = part.get_mut("text") {
                    transform_string_value(text, f);
                }
            }
        }
        _ => {}
    }
}

// ── Anthropic native `/v1/messages` (M6) ────────────────────────────────────
//
// The proxy masks the Anthropic-native body **in place** — no OpenAI round-trip —
// so there is no lossy translation boundary to leak through. The masking engine
// (`mask_all`, the vault, the fixpoint) is identical to the OpenAI path; only the
// schema walk below is new. Coverage must be exhaustive: an unscanned text field
// is a leak, and an **unknown content-block type fails closed** (400) so a new
// Anthropic block type is a conscious addition, never a silent leak.

/// Mask every text-bearing field of an Anthropic `/v1/messages` request.
///
/// Covers the top-level `system` (string or text-block array), every
/// `messages[].content` block (dispatched on `type`), and `tools[].description`
/// plus every `description` inside `tools[].input_schema`. Returns `Err(reason)`
/// to fail closed on an unrecognized shape or an unknown content-block type.
fn mask_anthropic_request(
    body: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    // Top-level `system` — a string or an array of text blocks (optional).
    if let Some(system) = body.get_mut("system") {
        mask_anthropic_system(system, f)?;
    }

    let messages = body
        .get_mut("messages")
        .ok_or("request has no `messages` field")?
        .as_array_mut()
        .ok_or("`messages` is not an array")?;
    for message in messages {
        if let Some(content) = message.get_mut("content") {
            mask_anthropic_content(content, f)?;
        }
    }

    // Tool definitions: free-text `description` and every `description` nested in
    // the tool's JSON-Schema `input_schema` can carry example PII. (Anthropic
    // tools are flat — `name`/`description`/`input_schema` — unlike OpenAI's
    // `function.*` nesting.)
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if let Some(description) = tool.get_mut("description") {
                transform_string_value(description, f);
            }
            if let Some(schema) = tool.get_mut("input_schema") {
                mask_schema_descriptions(schema, f);
            }
        }
    }

    Ok(())
}

/// Mask the top-level `system` field: a string, or an array of text blocks.
/// Fails closed on a shape we can't interpret as text.
fn mask_anthropic_system(
    system: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    match system {
        Value::String(s) => {
            *s = f(s);
            Ok(())
        }
        // `system` blocks are text-only in the Anthropic schema; mask each block's
        // `text` (covers a plain `{type:text}` and a cache-control block). Anything
        // without a `text` field — a non-object element *or* an object of an
        // unrecognized shape — **fails closed** (M6-R2), matching the block-level
        // rule, so a future non-text system block leaks nothing.
        Value::Array(blocks) => {
            for block in blocks {
                if let Some(text) = block.get_mut("text") {
                    transform_string_value(text, f);
                } else {
                    return Err("`system` array has an unrecognized element".to_string());
                }
            }
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err("`system` has an unrecognized shape".to_string()),
    }
}

/// Mask a `messages[].content`: a string, or an array of content blocks.
fn mask_anthropic_content(
    content: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    match content {
        Value::String(s) => {
            *s = f(s);
            Ok(())
        }
        Value::Array(blocks) => {
            for block in blocks {
                mask_anthropic_block(block, f)?;
            }
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err("message `content` has an unrecognized shape".to_string()),
    }
}

/// Mask one Anthropic content block, dispatched on its `type`. **The known set is
/// exhaustive for real Claude Code traffic and an unknown type fails closed** —
/// pinned by a guard test, so a new Anthropic block type can never silently leak.
///
/// - `text` → the `text` string;
/// - `tool_use` → every string leaf of `input` (a JSON *object* — the tool
///   arguments the model chose, restored to real values in the response, so on a
///   replay they hold real PII);
/// - `tool_result` → its `content` (string or nested block array — recursive; a
///   client-run tool's output, the CSV-export leak class E2E-02);
/// - `thinking` → the `thinking` string (see the note below);
/// - `document` with a **text** source → its `source.data` (plaintext with PII);
/// - `image` / other `document` / `redacted_thinking` → non-text or opaque, skipped.
///
/// **`thinking` is masked but never *de*-masked** (the response walk leaves it
/// alone). A thinking block is generated by the model over already-masked input,
/// so it naturally contains only placeholders; leaving them intact keeps the
/// block's cryptographic `signature` valid across a multi-turn replay (re-masking
/// an inert placeholder is a no-op, so the bytes — and the signature — never
/// change). Masking here only bites if a client injects *fresh* real PII into a
/// thinking block, which we still refuse to forward in clear.
fn mask_anthropic_block(
    block: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    // A bare string element in a content array *is* text (skipping = leak).
    if let Value::String(s) = block {
        *s = f(s);
        return Ok(());
    }
    if !block.is_object() {
        return Err("message `content` array has an unrecognized element".to_string());
    }
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            if let Some(text) = block.get_mut("text") {
                transform_string_value(text, f);
            }
            Ok(())
        }
        Some("tool_use") => {
            if let Some(input) = block.get_mut("input") {
                transform_string_leaves(input, f);
            }
            Ok(())
        }
        Some("tool_result") => {
            if let Some(content) = block.get_mut("content") {
                mask_content_block_array(content, f)?;
            }
            Ok(())
        }
        Some("thinking") => {
            if let Some(text) = block.get_mut("thinking") {
                transform_string_value(text, f);
            }
            Ok(())
        }
        Some("document") => mask_anthropic_document(block, f),
        // Non-text / opaque blocks — nothing to mask, but *known* (not a leak).
        Some("image") | Some("redacted_thinking") => Ok(()),
        // Fail closed. The message deliberately carries **no** client value (the
        // `type` is attacker-influenced), per never-log-raw-PII.
        Some(_) => Err("unknown Anthropic content block type".to_string()),
        None => Err("Anthropic content block has no `type`".to_string()),
    }
}

/// Mask a nested **content block array**: a string, or an array of `text`/`image`
/// blocks (with bare strings allowed). Fails closed on an unknown sub-block,
/// matching the top-level dispatch. Shared by a `tool_result`'s `content` **and**
/// a `content`-source `document`'s `content` (M6-R1), so its fail-closed messages
/// are **caller-neutral** — they become a client-facing 400 reason for either, and
/// naming just one would misreport the other (M6-R6).
fn mask_content_block_array(
    content: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    match content {
        Value::String(s) => {
            *s = f(s);
            Ok(())
        }
        Value::Array(blocks) => {
            for block in blocks {
                if let Value::String(s) = block {
                    *s = f(s);
                    continue;
                }
                if !block.is_object() {
                    return Err("nested content array has an unrecognized element".to_string());
                }
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get_mut("text") {
                            transform_string_value(text, f);
                        }
                    }
                    Some("image") => {}
                    Some(_) => return Err("unknown Anthropic nested content block".to_string()),
                    None => return Err("Anthropic nested content block has no `type`".to_string()),
                }
            }
            Ok(())
        }
        Value::Null => Ok(()),
        _ => Err("nested `content` has an unrecognized shape".to_string()),
    }
}

/// Mask a `document` block (M6-R1). A document carries text in several places, and
/// **every text-bearing one must be covered** — a missed one is a leak:
///
/// - `title` / `context` — optional citation metadata strings;
/// - `source`, dispatched on `source.type` the way blocks dispatch on `type`:
///   - `text` → the plaintext `source.data`;
///   - `content` → a nested block array (text / image), recursed like a
///     `tool_result` content — this is the shape M6-R1 forwarded in clear;
///   - `base64` / `url` → opaque binary or a fetch target that must not be
///     rewritten (the accepted non-text tradeoff, same as `image`) — skipped;
///   - **any other source type → fail closed**, matching the block-level rule, so a
///     new Anthropic document source is a conscious addition, never a silent leak.
fn mask_anthropic_document(
    block: &mut Value,
    f: &mut dyn FnMut(&str) -> String,
) -> Result<(), String> {
    if let Some(title) = block.get_mut("title") {
        transform_string_value(title, f);
    }
    if let Some(context) = block.get_mut("context") {
        transform_string_value(context, f);
    }
    let Some(source) = block.get_mut("source") else {
        return Ok(()); // no source → no body to leak (a malformed doc Anthropic rejects)
    };
    match source.get("type").and_then(Value::as_str) {
        Some("text") => {
            if let Some(data) = source.get_mut("data") {
                transform_string_value(data, f);
            }
            Ok(())
        }
        Some("content") => {
            if let Some(content) = source.get_mut("content") {
                mask_content_block_array(content, f)?;
            }
            Ok(())
        }
        Some("base64") | Some("url") => Ok(()),
        Some(_) => Err("unknown Anthropic document source type".to_string()),
        None => Err("Anthropic document `source` has no `type`".to_string()),
    }
}

/// Append the augmentation to the top-level `system` field (string or text-block
/// array), or create it — the native analogue of [`inject_augmentation`]. Called
/// only when something was masked.
fn inject_augmentation_anthropic(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    match obj.get_mut("system") {
        Some(Value::String(s)) => *s = format!("{s}\n\n{AUGMENTATION_PROMPT}"),
        Some(Value::Array(blocks)) => {
            blocks.push(json!({ "type": "text", "text": AUGMENTATION_PROMPT }));
        }
        // No `system` yet, or a shape we don't recognize → set it to the prompt.
        Some(other) => *other = json!(AUGMENTATION_PROMPT),
        None => {
            obj.insert("system".to_string(), json!(AUGMENTATION_PROMPT));
        }
    }
}

/// Restore placeholders in an Anthropic `/v1/messages` response: the top-level
/// `content[]` array — `text` blocks and `tool_use.input` string leaves.
///
/// Mirrors the request walk, with two deliberate omissions: `thinking` is left
/// with placeholders (keeps its `signature` valid on replay — see
/// [`mask_anthropic_block`]), and `tool_use.input` is a real JSON *object*, so its
/// string leaves are restored with the **plain** demask — serde re-escapes on
/// serialization, unlike OpenAI's JSON-encoded `arguments` string (M3-R2).
fn demask_anthropic_response(body: &mut Value, vault: &Vault) {
    let Some(content) = body.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get_mut("text") {
                    transform_string_value(text, &mut |t| vault.demask(t));
                }
            }
            Some("tool_use") => {
                if let Some(input) = block.get_mut("input") {
                    transform_string_leaves(input, &mut |t| vault.demask(t));
                }
            }
            _ => {}
        }
    }
}

// ── Shared helpers ─────────────────────────────────────────────────────────

/// Apply `f` to each `tool_calls[].function.arguments` string on a message.
fn transform_tool_call_args(message: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(tool_calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut) else {
        return;
    };
    for tool_call in tool_calls {
        if let Some(arguments) = tool_call.pointer_mut("/function/arguments") {
            transform_string_value(arguments, f);
        }
    }
}

/// Apply `f` to a `Value` expected to be a string; ignore anything else.
fn transform_string_value(value: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    if let Value::String(s) = value {
        *s = f(s);
    }
}

/// Apply `f` to **every string leaf** of a JSON value, recursing through arrays
/// and object *values* (never object keys — those are field names, not PII).
/// Used for an Anthropic `tool_use.input` object, whose leaves are the tool
/// arguments; numbers / bools / null are left untouched.
fn transform_string_leaves(value: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    match value {
        Value::String(s) => *s = f(s),
        Value::Array(items) => items.iter_mut().for_each(|v| transform_string_leaves(v, f)),
        Value::Object(map) => map.values_mut().for_each(|v| transform_string_leaves(v, f)),
        _ => {}
    }
}

/// Prepend the augmentation system message — or merge it into an existing
/// `system`/`developer` message so the upstream sees exactly one.
fn inject_augmentation(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for message in messages.iter_mut() {
        let role = message.get("role").and_then(Value::as_str);
        if role == Some("system") || role == Some("developer") {
            merge_augmentation(message);
            return;
        }
    }

    messages.insert(
        0,
        json!({ "role": "system", "content": AUGMENTATION_PROMPT }),
    );
}

/// Append the augmentation to an existing system/developer message's content.
fn merge_augmentation(message: &mut Value) {
    match message.get_mut("content") {
        Some(Value::String(s)) => *s = format!("{s}\n\n{AUGMENTATION_PROMPT}"),
        Some(Value::Array(parts)) => {
            parts.push(json!({ "type": "text", "text": AUGMENTATION_PROMPT }));
        }
        _ => message["content"] = json!(AUGMENTATION_PROMPT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::recognizers::StructuredRecognizers;
    use crate::pii::{Budget, Confidence, PiiEntity, PiiKind};

    /// A vault mapping `[PERSON_1]` to `value` (built via `mask` from one entity).
    fn vault_for(value: &str) -> Vault {
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
    fn tool_call_arguments_demask_stays_valid_json() {
        // M3-R2: a Person value with a `"` de-masked into tool-call `arguments`
        // must keep `arguments` a valid JSON string the client can parse.
        let vault = vault_for(r#"Ac"me Corp"#);
        let mut body = json!({
            "choices": [ { "message": {
                "role": "assistant",
                "tool_calls": [ {
                    "function": { "name": "lookup", "arguments": "{\"vendor\":\"[PERSON_1]\"}" }
                } ]
            } } ]
        });

        demask_response(&mut body, &vault);

        let args = body
            .pointer("/choices/0/message/tool_calls/0/function/arguments")
            .and_then(Value::as_str)
            .expect("arguments string");
        let parsed: Value = serde_json::from_str(args).expect("valid tool-call arguments JSON");
        assert_eq!(parsed["vendor"], r#"Ac"me Corp"#);
    }

    #[test]
    fn content_demask_is_not_json_escaped() {
        // Plain content is not a JSON string, so it must be restored verbatim
        // (no escaping of a quote in a name).
        let vault = vault_for(r#"O'Ac"me"#);
        let mut body = json!({
            "choices": [ { "message": { "role": "assistant", "content": "from [PERSON_1] today" } } ]
        });
        demask_response(&mut body, &vault);
        assert_eq!(
            body.pointer("/choices/0/message/content")
                .and_then(Value::as_str),
            Some(r#"from O'Ac"me today"#)
        );
    }

    // ── Anthropic native `/v1/messages` (M6) ────────────────────────────────

    /// Mask `body` in place through the real structured recognizers, returning
    /// the populated vault. Panics on a fail-closed shape (tests that want the
    /// error call `mask_anthropic_request` directly).
    fn mask_anthropic(body: &mut Value) -> Vault {
        let detector = StructuredRecognizers::new();
        let mut vault = Vault::new();
        {
            let mut mask = |t: &str| vault.mask_all(t, &detector, &Budget::per_call()).unwrap();
            mask_anthropic_request(body, &mut mask).expect("known Anthropic shape");
        }
        vault
    }

    #[test]
    fn anthropic_masks_every_text_bearing_field() {
        // Coverage: a value in every place M6 must scan is masked; a value in a
        // non-text leaf (image data, a numeric tool arg, a thinking `signature`)
        // is left untouched. A miss here is a leak.
        let mut body = json!({
            "model": "claude-x",
            "system": "ops at sys@corp.com",
            "messages": [
                { "role": "user", "content": "top-level string a@b.com" },
                { "role": "user", "content": [
                    { "type": "text", "text": "text block c@d.com" },
                    { "type": "tool_use", "id": "t1", "name": "f",
                      "input": { "to": "e@f.com", "count": 3, "nested": { "cc": "g@h.com" } } },
                    { "type": "tool_result", "tool_use_id": "t1",
                      "content": "csv row i@j.com" },
                    { "type": "thinking", "thinking": "reasoning k@l.com", "signature": "SIG==" },
                    { "type": "image", "source": { "type": "base64", "data": "AAAA" } },
                    { "type": "redacted_thinking", "data": "ENCRYPTED" }
                ] }
            ],
            "tools": [
                { "name": "lookup", "description": "desc m@n.com",
                  "input_schema": { "type": "object",
                    "properties": { "x": { "type": "string", "description": "prop o@p.com" } } } }
            ]
        });

        let _vault = mask_anthropic(&mut body);
        let dump = body.to_string();
        for raw in [
            "sys@corp.com",
            "a@b.com",
            "c@d.com",
            "e@f.com",
            "g@h.com",
            "i@j.com",
            "k@l.com",
            "m@n.com",
            "o@p.com",
        ] {
            assert!(!dump.contains(raw), "leaked {raw} upstream: {dump}");
        }
        // Non-text leaves are preserved verbatim.
        assert_eq!(body["messages"][1]["content"][1]["input"]["count"], 3);
        assert_eq!(body["messages"][1]["content"][3]["signature"], "SIG==");
        assert_eq!(body["messages"][1]["content"][4]["source"]["data"], "AAAA");
        assert_eq!(body["messages"][1]["content"][5]["data"], "ENCRYPTED");
        // The masked email is a placeholder the model can read back.
        assert!(dump.contains("[EMAIL_1]"), "no placeholder minted: {dump}");
    }

    #[test]
    fn anthropic_masks_document_text_and_content_sources_title_and_context() {
        // Every text-bearing part of a `document` is masked (M6-R1): a `text`
        // source's `data`, a `content` source's nested block array, and the
        // `title` / `context` metadata. A base64 file source stays opaque.
        let mut body = json!({
            "messages": [ { "role": "user", "content": [
                { "type": "document", "title": "re: q@r.com", "context": "from s@t.com",
                  "source": { "type": "text", "media_type": "text/plain", "data": "leak a1@b.com" } },
                { "type": "document",
                  "source": { "type": "content", "content": [
                      { "type": "text", "text": "content-src c1@d.com" },
                      { "type": "image", "source": { "type": "base64", "data": "IMG" } }
                  ] } },
                { "type": "document",
                  "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBERi0=" } }
            ] } ]
        });
        let _vault = mask_anthropic(&mut body);
        let dump = body.to_string();
        for raw in ["q@r.com", "s@t.com", "a1@b.com", "c1@d.com"] {
            assert!(!dump.contains(raw), "leaked {raw}: {dump}");
        }
        // base64 file source + nested image data are opaque, untouched.
        assert_eq!(
            body["messages"][0]["content"][2]["source"]["data"],
            "JVBERi0="
        );
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["content"][1]["source"]["data"],
            "IMG"
        );
    }

    #[test]
    fn anthropic_unknown_document_source_type_fails_closed() {
        // A `document` source type we don't model fails closed — the same rule as
        // an unknown block type, so a new source is a conscious addition (M6-R1).
        let mut f = |t: &str| t.to_string();

        let mut unknown_source = json!({
            "messages": [ { "role": "user", "content": [
                { "type": "document", "source": { "type": "hologram", "data": "x" } }
            ] } ]
        });
        assert!(mask_anthropic_request(&mut unknown_source, &mut f).is_err());

        // …and the fail-closed rule holds at the next depth down: a `content`
        // source with an unrecognized nested sub-block also blocks (M6-R6).
        let mut unknown_nested = json!({
            "messages": [ { "role": "user", "content": [
                { "type": "document", "source": { "type": "content", "content": [
                    { "type": "telepathy" }
                ] } }
            ] } ]
        });
        assert!(mask_anthropic_request(&mut unknown_nested, &mut f).is_err());
    }

    #[test]
    fn anthropic_masks_nested_tool_result_block_array() {
        // `tool_result.content` can itself be a block array — recurse into it.
        let mut body = json!({
            "messages": [ { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": [
                    { "type": "text", "text": "first s@t.com" },
                    { "type": "image", "source": { "type": "base64", "data": "BBBB" } },
                    "bare u@v.com"
                ] } ]
            } ]
        });
        let _vault = mask_anthropic(&mut body);
        let dump = body.to_string();
        assert!(!dump.contains("s@t.com"), "text sub-block leaked: {dump}");
        assert!(
            !dump.contains("u@v.com"),
            "bare-string sub-block leaked: {dump}"
        );
        assert_eq!(
            body["messages"][0]["content"][0]["content"][1]["source"]["data"],
            "BBBB"
        );
    }

    #[test]
    fn anthropic_unknown_block_type_fails_closed_without_echoing_it() {
        // Strict: an unknown content block blocks the request, and the reason
        // carries no client-controlled value (never-log-raw-PII).
        let mut body = json!({
            "messages": [ { "role": "user", "content": [
                { "type": "video-with-ssn-123-45-6789", "url": "x" }
            ] } ]
        });
        let mut f = |t: &str| t.to_string();
        let err = mask_anthropic_request(&mut body, &mut f).expect_err("unknown block → Err");
        assert!(!err.contains("123-45-6789"), "reason echoed input: {err}");
        assert!(
            !err.contains("video-with-ssn"),
            "reason echoed the type: {err}"
        );
    }

    #[test]
    fn anthropic_block_without_type_and_missing_messages_fail_closed() {
        let mut f = |t: &str| t.to_string();

        let mut no_type =
            json!({ "messages": [ { "role": "user", "content": [ { "text": "hi" } ] } ] });
        assert!(mask_anthropic_request(&mut no_type, &mut f).is_err());

        let mut no_messages = json!({ "model": "claude-x" });
        assert!(mask_anthropic_request(&mut no_messages, &mut f).is_err());

        let mut messages_not_array = json!({ "messages": "nope" });
        assert!(mask_anthropic_request(&mut messages_not_array, &mut f).is_err());

        // A `system` array object with no `text` fails closed too (M6-R2) —
        // an unrecognized sub-shape is never skipped open.
        let mut system_no_text = json!({ "system": [ { "type": "mystery" } ], "messages": [] });
        assert!(mask_anthropic_request(&mut system_no_text, &mut f).is_err());
    }

    #[test]
    fn anthropic_augmentation_covers_absent_string_and_array_system() {
        // Absent → created.
        let mut absent = json!({ "messages": [] });
        inject_augmentation_anthropic(&mut absent);
        assert!(absent["system"].as_str().unwrap().contains("placeholder"));

        // String → appended (existing content preserved first).
        let mut string_sys = json!({ "system": "base rules", "messages": [] });
        inject_augmentation_anthropic(&mut string_sys);
        let s = string_sys["system"].as_str().unwrap();
        assert!(
            s.starts_with("base rules") && s.contains("placeholder"),
            "got: {s}"
        );

        // Array → pushed as a trailing text block.
        let mut array_sys =
            json!({ "system": [ { "type": "text", "text": "base" } ], "messages": [] });
        inject_augmentation_anthropic(&mut array_sys);
        let blocks = array_sys["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["type"], "text");
        assert!(blocks[1]["text"].as_str().unwrap().contains("placeholder"));
    }

    #[test]
    fn anthropic_response_demasks_text_and_tool_use_input_but_not_thinking() {
        let vault = vault_for("real-value");
        let mut body = json!({
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "see [PERSON_1] now" },
                { "type": "tool_use", "id": "t", "name": "f",
                  "input": { "who": "[PERSON_1]", "count": 1 } },
                { "type": "thinking", "thinking": "about [PERSON_1]", "signature": "SIG==" }
            ]
        });
        demask_anthropic_response(&mut body, &vault);

        assert_eq!(body["content"][0]["text"], "see real-value now");
        assert_eq!(body["content"][1]["input"]["who"], "real-value");
        assert_eq!(body["content"][1]["input"]["count"], 1);
        // Thinking keeps the placeholder so its `signature` stays valid on replay.
        assert_eq!(body["content"][2]["thinking"], "about [PERSON_1]");
        assert_eq!(body["content"][2]["signature"], "SIG==");
    }

    #[test]
    fn anthropic_tool_use_input_demask_stays_valid_json_through_a_quote() {
        // `tool_use.input` is a real JSON object, so a value with a `"` restored
        // into a string leaf must survive re-serialization (serde escapes it).
        let vault = vault_for(r#"Ac"me"#);
        let mut body = json!({
            "content": [ { "type": "tool_use", "input": { "vendor": "[PERSON_1]" } } ]
        });
        demask_anthropic_response(&mut body, &vault);
        assert_eq!(body["content"][0]["input"]["vendor"], r#"Ac"me"#);
        // …and the whole thing round-trips through a serialize → parse cycle.
        let reparsed: Value = serde_json::from_str(&body.to_string()).unwrap();
        assert_eq!(reparsed["content"][0]["input"]["vendor"], r#"Ac"me"#);
    }
}
