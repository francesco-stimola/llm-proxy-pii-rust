//! The privacy stage: detect PII in the outgoing request, mask it, teach the
//! model how to read the placeholders, and restore the originals in the incoming
//! response. The only stage wired in the current milestone.
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

use serde_json::{Value, json};

use crate::pii::PiiDetector;
use crate::pipeline::{RequestContext, Stage};
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
}

impl PrivacyStage {
    /// Build the stage around a concrete detector (deterministic, ML, or both).
    pub fn new(detector: Box<dyn PiiDetector>) -> Self {
        Self { detector }
    }
}

impl Stage for PrivacyStage {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn on_request(&self, req: &mut ProxyRequest, ctx: &mut RequestContext) {
        // Scope the vault borrow so it's released before we re-read the vault.
        // `detect_error` captures a *required* detector failure so we can fail
        // closed after the walk (its message carries no input text).
        let mut detect_error: Option<crate::pii::DetectError> = None;
        let outcome = {
            let detector = self.detector.as_ref();
            let vault = &mut ctx.vault;
            let error_slot = &mut detect_error;
            let mut mask = |text: &str| match detector.try_detect(text) {
                Ok(entities) => vault.mask(text, &entities),
                Err(err) => {
                    if error_slot.is_none() {
                        *error_slot = Some(err);
                    }
                    // Leave the text as-is; we block below and never forward it.
                    text.to_string()
                }
            };
            mask_request(&mut req.body, &mut mask)
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
            inject_augmentation(&mut req.body);
        }
    }

    fn on_response(&self, resp: &mut ProxyResponse, ctx: &mut RequestContext) {
        if ctx.vault.is_empty() {
            return;
        }
        let vault = &ctx.vault;
        demask_response(&mut resp.body, &mut |text| vault.demask(text));
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
                    return Err(
                        "message `content` array has an unrecognized element".to_string()
                    );
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
fn demask_response(body: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(choices) = body.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message") else {
            continue;
        };
        if let Some(content) = message.get_mut("content") {
            demask_content(content, f);
        }
        transform_tool_call_args(message, f);
        if let Some(args) = message.pointer_mut("/function_call/arguments") {
            transform_string_value(args, f);
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

    messages.insert(0, json!({ "role": "system", "content": AUGMENTATION_PROMPT }));
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
