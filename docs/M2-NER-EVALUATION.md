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

## Decision — first evaluation round (2026-07-12)

Locked after the M2 review. Evaluate **two candidates head-to-head first**, chosen
by the key engineering discriminator: whether the model drops into today's
`OnnxNerDetector` (standard token-classification — `input_ids` + `attention_mask`
→ per-token logits → BIO decode) or needs extra integration work.

1. **XLM-R multilingual token-classification NER** — the *drop-in baseline*.
   Starting-point IDs (verify at export): `Davlan/xlm-roberta-base-ner-hrl`
   (10 languages incl. IT; PER/ORG/LOC) or `Babelscape/wikineural-multilingual-ner`
   (9 languages incl. IT). Feeds only `input_ids`+`attention_mask`, so it runs
   through `onnx.rs` unchanged — establishes the recall floor at ~zero integration
   cost.
2. **Piiranha** — the *PII specialist*, best-recall bet (recall is metric #1: a
   missed entity is a leak). `iiiorg/piiranha-v1-detect-personal-information`
   (mDeBERTa-v3, PII-tuned, multilingual). Integration cost: DeBERTa-v3 needs
   `token_type_ids` → pulls in review finding **M2-R4**; its granular PII labels
   must be mapped to `Person/Org/Location` (overlaps with the structured layer are
   resolved by the hybrid — structured wins) → touches **M2-R3**.

**GLiNER stays in escalation** — often the best zero-shot recall on rare/single-word
names, but it is *not* token-classification (span×label scoring needs a different
decode path), so it requires a separate detector. Not justified until the two above
miss the recall bar.

Both are scored **through the hybrid resolver** (not the NER alone), **fp32 and
int8**, on `tests/corpus/ner_cases.json`: recall / precision / F1 per type + CPU
latency / RAM / model size. Pick by recall-at-acceptable-cost; multilingual breaks
ties (de-risks M5).

**First build task = the evaluation harness.** `tests/ner_corpus.rs` today enforces
only the REG-03 negatives (the deterministic layer emits no unstructured entity); it
does *not* score a live model. Add a harness (an `#[ignore]`d test or a small bin,
gated on `NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS`) that runs a candidate
against the corpus and prints the P/R/F1 + latency/RAM table — the artifact this
decision is made from. Wiring the live model here is also where the fail-closed
hardening (M2-R1…R4) naturally lands.

### Pre-converted ONNX exists — export step is skipped (verified 2026-07-12)

Both candidates are already on the Hub in ONNX **with int8 variants and a fast
`tokenizer.json`**, so the Python export/quantization step is **not needed**. The
builder just downloads three artifacts (one `.onnx` + `tokenizer.json` + the labels
from `config.json`) and points `NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS`
at them. **Pin the exact repo revision (commit hash)** used, for reproducibility.

- **XLM-R baseline** → [`jiting/xlm-roberta-base-ner-hrl_onnx`](https://huggingface.co/jiting/xlm-roberta-base-ner-hrl_onnx)
  (Xenova/Transformers.js conversion): `onnx/model.onnx` (fp32, 1.11 GB) +
  `onnx/model_quantized.onnx` (**int8**, 279 MB) + `tokenizer.json` + `config.json`
  (clean PER/ORG/LOC labels — feeds today's `onnx.rs` unchanged). Alternate mirror:
  `tjruesch/xlm-roberta-base-ner-hrl-onnx`.
- **Piiranha** → [`onnx-community/piiranha-v1-detect-personal-information-ONNX`](https://huggingface.co/onnx-community/piiranha-v1-detect-personal-information-ONNX):
  `onnx/` ships the full spread — `model.onnx`, `model_fp16.onnx`, `model_int8.onnx`,
  `model_quantized.onnx`, `model_uint8.onnx`, `model_q4*.onnx` — plus `tokenizer.json`
  and `config.json`. **Caveat:** Piiranha's labels are PII-granular (given name /
  surname / city / email / phone / …), **not** plain PER/ORG/LOC — `label_to_kind`
  must map given+surname → Person, city/etc. → Location, and Piiranha may **not**
  cover `Organization` the same way (the eval quantifies this). Its email/phone/etc.
  labels overlap the structured layer and are resolved by the hybrid. This is the
  **M2-R3** label-mapping work; being mDeBERTa-v3 it also needs `token_type_ids`
  (**M2-R4**).

Trust note: these are community/auto conversions, not the original authors'. No
separate trust step is required — scoring each against `ner_cases.json` **is** the
correctness check (a bad conversion surfaces as low recall) — but pin the revision.

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
