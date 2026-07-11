# Reference PII tests — from `llmproxy-extended`

These two files are copied **verbatim** from the previous project
[`llmproxy-extended`](https://github.com/francesco-stimola/llmproxy-extended)
(`tests/`), same author, MIT-licensed. They are **reference material only** — not
part of the Rust build or test suite (`cargo test` ignores non-`.rs` files here).

They capture the PII scenarios the old proxy validated, which the Rust
implementation must reproduce and improve on. Use them as the source of truth
when building the Rust fixtures.

## What they cover

Structured PII categories, each masked to a typed placeholder and restored on a
round-trip (mask → demask):

| Category    | Example                       | Notes |
|-------------|-------------------------------|-------|
| Email       | `john@example.com`            | |
| Phone (US)  | `555-123-4567`                | pattern `[2-9]\d{2}-[2-9]\d{2}-\d{4}` |
| SSN         | `123-45-6789`                 | pattern `[1-8]\d{2}-\d{2}-\d{4}` |
| Credit card | `4111 1111 1111 1111`         | **Luhn-validated** |
| IBAN (DE)   | `DE89 3704 0044 0532 0130 00` | label must be IBAN; country prefix must not leak |

Behaviours asserted:
- Detection (is PII present?) and **no false positives** on plain alphabetic text.
- Masking to typed tokens (`[PII_EMAIL_…]`, `[PII_PHONE_…]`, …).
- A **vault** storing originals; `demask` restores the exact original text.
- Multiple PII values in one string all masked and restored.
- Passthrough unchanged when there is no PII (or the layer is disabled).

## Key lesson carried over

The old proxy detected these **structured** categories with deterministic
regex + validation (Luhn, IBAN), not with the ML model. The ONNX
`openai/privacy-filter` model was reserved for unstructured entities (names,
organizations, locations) — and it is that model that proved unreliable. The
Rust design keeps the same split: deterministic recognizers for structured PII,
ML NER only where it is actually needed.
