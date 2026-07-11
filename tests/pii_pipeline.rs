//! Integration tests for the privacy stage over OpenAI-shaped payloads
//! (INT-01…06 in `docs/TESTING.md`). These drive `PrivacyStage` directly — no
//! network — asserting the mask / augment / restore behaviour.

use serde_json::{Value, json};

use llm_proxy_pii_rust::pii::recognizers::StructuredRecognizers;
use llm_proxy_pii_rust::pipeline::privacy::PrivacyStage;
use llm_proxy_pii_rust::pipeline::{RequestContext, Stage};
use llm_proxy_pii_rust::proxy::{ProxyRequest, ProxyResponse};

fn stage() -> PrivacyStage {
    PrivacyStage::new(Box::new(StructuredRecognizers::new()))
}

/// Content of the first `user` message in a (possibly augmented) request body.
fn user_content(body: &Value) -> &str {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("a user message")["content"]
        .as_str()
        .expect("string content")
}

/// Mask a single user message and return its masked content.
fn mask_user_message(text: &str) -> String {
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: json!({ "messages": [{ "role": "user", "content": text }] }),
    };
    stage().on_request(&mut req, &mut ctx);
    user_content(&req.body).to_string()
}

#[test]
fn int01_user_message_pii_is_masked_and_vault_populated() {
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: json!({
            "model": "gpt-x",
            "messages": [{ "role": "user", "content": "email bob@test.com phone 555-111-2222" }]
        }),
    };
    stage().on_request(&mut req, &mut ctx);

    assert!(!ctx.vault.is_empty(), "vault should be populated");
    let content = user_content(&req.body);
    assert!(content.contains("[EMAIL_1]"), "got: {content}");
    assert!(content.contains("[PHONE_1]"), "got: {content}");
    assert!(!content.contains("bob@test.com"));
    assert!(!content.contains("555-111-2222"));
}

#[test]
fn int02_tool_result_pii_is_masked() {
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: json!({
            "messages": [{
                "role": "tool",
                "tool_call_id": "c1",
                "content": "contact: a@b.com, +39 333 0000001"
            }]
        }),
    };
    stage().on_request(&mut req, &mut ctx);

    let tool = req.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    let content = tool["content"].as_str().unwrap();
    assert!(!content.contains("a@b.com"), "got: {content}");
    assert!(content.contains("[EMAIL_1]"), "got: {content}");
    assert!(content.contains("[PHONE_1]"), "got: {content}");
}

#[test]
fn int03_response_tool_call_arguments_are_deanonymized() {
    let s = stage();
    let mut ctx = RequestContext::new();

    // Request establishes bob@test.com -> [EMAIL_1] in the vault.
    let mut req = ProxyRequest {
        body: json!({ "messages": [{ "role": "user", "content": "send it to bob@test.com" }] }),
    };
    s.on_request(&mut req, &mut ctx);

    // Assistant asks to call a tool with the placeholder in its arguments.
    let mut resp = ProxyResponse {
        body: json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "1",
                        "type": "function",
                        "function": { "name": "send_email", "arguments": "{\"to\":\"[EMAIL_1]\"}" }
                    }]
                }
            }]
        }),
    };
    s.on_response(&mut resp, &mut ctx);

    let args = resp.body["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    assert!(args.contains("bob@test.com"), "got: {args}");
    assert!(!args.contains("[EMAIL_1]"), "got: {args}");
}

#[test]
fn int04_augmentation_system_message_injected_when_pii_present() {
    let mut ctx = RequestContext::new();
    let mut req = ProxyRequest {
        body: json!({ "messages": [{ "role": "user", "content": "mail a@b.com" }] }),
    };
    stage().on_request(&mut req, &mut ctx);

    let first = &req.body["messages"][0];
    assert_eq!(first["role"], "system");
    assert!(
        first["content"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("placeholder")
    );
}

#[test]
fn no_pii_means_no_injection_and_no_change() {
    let mut ctx = RequestContext::new();
    let original = json!({ "messages": [{ "role": "user", "content": "hello world, nothing here" }] });
    let mut req = ProxyRequest {
        body: original.clone(),
    };
    stage().on_request(&mut req, &mut ctx);

    assert!(ctx.vault.is_empty());
    assert_eq!(req.body, original, "clean request must be untouched");
}

#[test]
fn int05_masking_is_deterministic_across_turns() {
    let text = "from bob@test.com and alice@a.com, again bob@test.com";
    let first = mask_user_message(text);
    let second = mask_user_message(text);

    assert_eq!(first, second, "same input must yield the same tokens");
    assert_eq!(
        first,
        "from [EMAIL_1] and [EMAIL_2], again [EMAIL_1]",
        "repeated value reuses its token; numbering follows reading order"
    );
}

#[test]
fn int06_response_text_is_deanonymized() {
    let s = stage();
    let mut ctx = RequestContext::new();

    let mut req = ProxyRequest {
        body: json!({ "messages": [{ "role": "user", "content": "my email is bob@test.com" }] }),
    };
    s.on_request(&mut req, &mut ctx);

    let mut resp = ProxyResponse {
        body: json!({
            "choices": [{ "message": { "role": "assistant", "content": "I emailed [EMAIL_1] for you." } }]
        }),
    };
    s.on_response(&mut resp, &mut ctx);

    let content = resp.body["choices"][0]["message"]["content"].as_str().unwrap();
    assert_eq!(content, "I emailed bob@test.com for you.");
}
