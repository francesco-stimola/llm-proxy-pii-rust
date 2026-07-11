//! The privacy stage: detect PII in the outgoing request, mask it, teach the
//! model how to read the placeholders, and restore the originals in the incoming
//! response. The only stage wired in the current milestone.
//!
//! ## What gets masked / restored
//!
//! On the way **out** (`on_request`) every text field of the OpenAI chat payload
//! is scanned and masked with a shared per-request [`Vault`], so the same real
//! value maps to the same `[KIND_N]` token everywhere it appears:
//!
//! - `messages[].content` — string, or the `text` of each content part
//! - `messages[].tool_calls[].function.arguments` — re-masked tool-call args
//!   carried in the conversation history
//!
//! If anything was masked, a system message is injected explaining the
//! placeholders (see [`AUGMENTATION_PROMPT`]).
//!
//! On the way **back** (`on_response`) the same vault restores the originals in:
//!
//! - `choices[].message.content`
//! - `choices[].message.tool_calls[].function.arguments` — so the client runs
//!   its tools with the real values

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
        let detector = self.detector.as_ref();
        let vault = &mut ctx.vault;
        transform_request_texts(&mut req.body, &mut |text| {
            let entities = detector.detect(text);
            vault.mask(text, &entities)
        });

        // Only pollute the prompt when we actually masked something.
        if !ctx.vault.is_empty() {
            inject_augmentation(&mut req.body);
        }
    }

    fn on_response(&self, resp: &mut ProxyResponse, ctx: &mut RequestContext) {
        if ctx.vault.is_empty() {
            return;
        }
        let vault = &ctx.vault;
        transform_response_texts(&mut resp.body, &mut |text| vault.demask(text));
    }
}

/// Apply `f` to every masked-relevant text field of an outgoing request.
fn transform_request_texts(body: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        if let Some(content) = message.get_mut("content") {
            transform_content(content, f);
        }
        transform_tool_call_args(message, f);
    }
}

/// Apply `f` to every restorable text field of an incoming response.
fn transform_response_texts(body: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(choices) = body.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for choice in choices {
        let Some(message) = choice.get_mut("message") else {
            continue;
        };
        if let Some(content) = message.get_mut("content") {
            transform_content(content, f);
        }
        transform_tool_call_args(message, f);
    }
}

/// Message `content` is either a plain string or an array of typed parts; apply
/// `f` to the string / each `text` part.
fn transform_content(content: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    match content {
        Value::String(s) => *s = f(s),
        Value::Array(parts) => {
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = part.get_mut("text") {
                        transform_string_value(text, f);
                    }
                }
            }
        }
        _ => {}
    }
}

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

/// Prepend the augmentation system message to the conversation.
fn inject_augmentation(body: &mut Value) {
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        messages.insert(
            0,
            json!({ "role": "system", "content": AUGMENTATION_PROMPT }),
        );
    }
}
