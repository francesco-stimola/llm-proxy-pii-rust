# M2 — Unstructured-entity NER: candidate models & evaluation plan

Prep for milestone **M2**. The deterministic recognizers (M1 Part A) own the
*structured* PII and are authoritative; the NER model only covers *unstructured*
entities — `Person`, `Organization`, `Location`. The point of this doc is to pick
that model **by measurement, not by guesswork** — the old proxy's
`openai/privacy-filter` failed precisely because it was an unmeasured, unreliable
NER. See `docs/ARCHITECTURE.md` (hybrid detection) for why the split exists.

> Model landscape moves fast and this is written at a point in time. Treat repo
> IDs / sizes / licenses below as **starting points to verify at M2**, not facts.

## Requirements (hard constraints)

- **Local** — detection must run on-box; sending text to a remote model to detect
  PII would leak the very data we protect.
- **ONNX-exportable** — runs through `ort` (see the `onnx` feature). A model with
  no clean ONNX export path is disqualified regardless of accuracy.
- **CPU-first, lean** — usable on CPU with int8 quantization; low RAM and latency
  (this is a per-request hot path). GPU is an M4 optimization, not a requirement.
- **Tokenizer compatible** with the `tokenizers` crate (HF fast tokenizer JSON).
- **Labels mappable** to `PiiKind::{Person, Organization, Location}`.
- **Locales IT + US now**, multilingual capability a strong bonus (it de-risks M5).
- **Permissive license** (MIT/Apache-2.0 preferred).

## Candidate shortlist

| Candidate | Base / type | Languages | Notes | Why consider |
|---|---|---|---|---|
| **GLiNER** (multi / PII variants) | small bi-encoder, label-conditioned NER | multilingual variants exist | zero-shot labels ("person", "organization", "location"); compact; ONNX export documented | flexible label set, small, multilingual — fits IT+US and M5 |
| **Piiranha** (`iiiorg/piiranha-*`) | mDeBERTa token-classification, PII-tuned | multilingual | purpose-built for PII categories | trained for exactly our task; strong recall baseline |
| **Multilingual NER** (XLM-R / mDeBERTa) e.g. WikiNeural, `Davlan/*-ner-hrl` | token-classification | many incl. Italian | solid PER/ORG/LOC; well-known baselines | reliable, easy to export, good IT coverage |
| **Italian-specific NER** (`dbmdz`, spaCy `it_core_news_*`) | BERT/spaCy | IT | strong on Italian names/orgs | comparison baseline for the IT half (spaCy = harder to ONNX) |
| *(escalation)* small local LLM w/ structured output | decoder LLM | multilingual | heavier; only if NER can't clear the bar | contextual recall on hard names |

**Presidio** is a *framework* (spaCy recognizers + NER), not a single model — keep
it as a conceptual reference / baseline, not an integration target (Python/spaCy
runtime doesn't fit a lean Rust binary).

## Evaluation data

The current corpus (`tests/corpus/pii_cases.json`) is structured-PII focused. M2
needs a labelled **unstructured** set — build `tests/corpus/ner_cases.json`:

- Person / Organization / Location in natural IT + EN sentences.
- Reuse the old proxy's scenarios: `Pinco Pallino`, `Futura Incognita`,
  single-word names (`Tizio`, `Caia`) that the old NER dropped.
- **Negative / false-positive guards** (REG-03): connection names / common words
  like `anubi` must NOT be tagged `Person`.
- A few multilingual names to preview M5.

## Metrics

Per entity type (`Person` / `Organization` / `Location`), span-level:

- **Recall** (weighted highest — a missed entity is a data leak).
- **Precision** / false-positive rate (over-masking hurts usability, but is safer
  than a leak — so recall > precision, within reason).
- **F1** for the summary ranking.

Plus operational cost, measured on CPU:

- latency per request (and per 1k tokens), peak RAM, model file size,
- **int8 vs fp32** delta on both accuracy and speed (informs the quantization
  choice and the M4 GPU decision).

## Method

1. Add `tests/corpus/ner_cases.json` (above).
2. For each candidate: export to ONNX, implement it behind `OnnxNerDetector`
   (the `PiiDetector` trait — no pipeline changes needed).
3. Run it **combined** with the deterministic recognizers through the existing
   overlap-resolution, so the score reflects the real hybrid, not the NER alone.
4. Record a comparison table (accuracy metrics + latency/RAM/size) per candidate,
   fp32 and int8.

## Decision criteria

Pick the candidate with the **best unstructured recall at acceptable CPU
latency/RAM**, that is ONNX-exportable and permissively licensed. Multilingual
capability breaks ties (de-risks M5). Record the choice and the numbers in
`docs/DEVLOG.md`.

## Escalation path (if no local NER clears the bar)

The `PiiDetector` trait keeps all of these swappable without touching the proxy:

- larger model + GPU execution provider (pull M4 forward),
- a small **local** LLM with constrained/structured output for hard cases,
- confidence-threshold tuning, or an ensemble of two NERs.

Do **not** reach for a heavy model pre-emptively — measure first, then escalate
only if the data says so (textbook / lean principle).
