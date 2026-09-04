# Architecture

## Overview

`llm-proxy-pii-rust` is a reverse proxy in front of an OpenAI-compatible LLM
provider. It inspects each request, anonymizes PII locally, forwards the
anonymized request upstream, and restores the original values in the response.

## Design principles

- **Local-first** — PII detection runs on-box; nothing leaves for the filtering step.
- **Modular pipeline** — request/response transformations are `Stage`s. Only the
  privacy stage is wired now, but auth / rate-limit / logging can be added later
  without touching the core.
- **Engine-agnostic detection** — everything sits behind the `PiiDetector` trait,
  so we can swap models or add engines without touching the proxy.
- **CPU-first, GPU optional** — correctness and reproducibility on CPU first; CPU stays the
  default and the only universally-trusted path. Hardware acceleration ([M9](ROADMAP.md#m9)) is
  **opt-in at runtime** behind `NER_EXECUTION_PROVIDER` (the platform's accelerator is already in
  the `onnx` build), and **falls back to CPU** if it isn't there — GPU behavior isn't automatic
  (it depends on the model, quantization, and hardware). See *Execution providers — hardware
  acceleration (M9)* below.
- **Textbook & lean** — idiomatic Rust, low RAM/CPU, no over-engineering.

## Hybrid detection (key decision)

Two classes of PII, handled differently:

| Class        | Examples                              | Engine |
|--------------|---------------------------------------|--------|
| Structured   | email, phone, SSN, national ID, VAT / tax ID, credit card, IBAN, secret | deterministic regex + validation (Luhn, IBAN checksum, per-scheme checksums) — high precision, no model |
| Unstructured | names, organizations, locations       | ONNX NER model (M2) |

The old proxy's ONNX `openai/privacy-filter` was unreliable on the ML part.
Keeping deterministic recognizers for structured PII removes most of the
reliability risk *and* most of the compute cost — the ML model only carries the
unstructured-entity load.

**Locale coverage (M4, extended by M11 Track A) — four tiers.** The structured recognizers split into:
- **Universal** — email, secret, credit card, IBAN (already any-country) and phone
  (US + `+CC`). Always on.
- **National identifiers** — US SSN (keeps `PiiKind::Ssn` / `[SSN_N]`) plus, under
  `PiiKind::NationalId` / `[NATID_N]`, one per XLM-R-aligned country: IT Codice Fiscale,
  GB NINO, ES DNI/NIE, FR NIR, DE Steuer-ID, NL BSN, PT NIF, LV personal code, zh China
  Resident ID. **Always on regardless of `PII_LOCALES`** (privacy-first — a national ID that
  reaches the proxy is masked even if its country isn't configured). Each is checksum- or
  rule-specific (mod-23 / mod-97 / mod-11 / ISO 7064 / NINO prefix rules) to stay near-zero
  false-positive when always on. The pure-numeric 9-/11-digit IDs (BSN/NIF, DE/LV) accept a
  small fraction of arbitrary numbers on checksum alone (~18% of 9-digit tokens); this is an
  **accepted over-mask tradeoff** (M4-R6) — privacy-first, never a leak — not context-gated
  (that would leak); the contextual precision path is GLiNER ([M8](ROADMAP.md#m8)).
- **VAT / tax identifiers ([M11](ROADMAP.md#m11-a))** — `PiiKind::TaxId` / `[TAXID_N]`: the
  Italian Partita IVA (11 digits, mod-10 with position doubling) in both its **bare** and VIES
  forms, and VIES-form DE (ISO 7064 Mod 11,10), GB (mod-97 / mod-97-55), PT (mod-11) and NL
  (format-anchored). **Always on regardless of `PII_LOCALES`**, like the national IDs it joins and
  for the same reason — and with **no configuration variable of its own**, deliberately: gating it
  would have inherited `PII_LOCALES`'s *narrowing* semantics, the subtlety M10 had to set in bold
  in the changelog, and a second tier carrying it doubles what the README must explain.
  - **A new kind rather than a reused `[NATID_N]`, and that is the decision the track turned on.**
    A VAT number identifies a *business*, and is personal data only when that business is a sole
    trader; a Codice Fiscale identifies a *person*, always. One token for both destroys that
    distinction for every consumer downstream. Reusing `NATID` now and splitting later was
    rejected because it converts a free choice today into a breaking change to the placeholder
    vocabulary tomorrow.
  - **Five countries, because five are measured.** ES, FR and LV VAT numbers are real and are
    **not** recognized: their checksums are not verified here, and the `PHONE-NAT` rule applies
    unchanged — an unmeasured recognizer does not ship. VAT-04 asserts the absence so the gap stays
    a decision rather than a discovery.
  - **NL is the one scheme with nothing to check**, so it is accepted on format (`NL` + 9 digits +
    a literal `B` + 2 digits) and tagged `Confidence::Structural` unless its body passes the
    11-proef — the 2020 sole-trader `btw-id` is randomized by design. Same honesty as an IBAN whose
    mod-97 fails: masked, and flagged rather than claimed.
  - **The bare Italian form has a measured over-mask cost: 0.100** — a mod-10 check accepts one
    arbitrary 11-digit number in ten. Accepted on the same M4-R6 grounds as the numeric national
    IDs (over-mask, never leak; the vault restores byte-identically). The prefixed forms carry no
    such cost, since they need the literal country code as well.
  - **Priority: below both `NationalId` and `Phone`** — see `PiiKind::priority`, which carries the
    two reasons. The one that is easy to get wrong: a bare P.IVA is `\d{11}`, and so are the
    compact domestic phone shapes M10 measured (`02079460958` is a real London number and it
    satisfies the P.IVA checksum). Ranking `TaxId` above `Phone` silently relabels every compact GB
    and DE number as `[TAXID_N]` — no leak, since the bytes are masked either way, but a fidelity
    regression on a measured capability. A numbering-plan lookup that confirms an *assigned* number
    is better evidence than a mod-10 check, so the plan lookup names the span. Pinned by VAT-14.
  - **The ordering is right and its price is large — both halves belong here (M11-R2).** The
    collision is not two unlucky numbers: the phone tier's separator-free `Trunk` arm is
    `\b0\d{6,11}\b`, and *every issuable P.IVA is 0-leading*, so the whole bare form is a phone
    candidate in all nine vetted regions. **Measured against the shipped default: 0.775 of issuable
    bare P.IVAs are named `[PHONE_n]`, not `[TAXID_n]`** (`VAT-17`). The split by leading pair is
    the explanation and it is why nothing caught this for a milestone: `00xx` is **0.033**,
    `0[1-9]xx` is **0.859** — a leading `00` reads to libphonenumber as the international access
    code and is rejected, and *every* real published P.IVA in this repo's corpus is `00…`-leading,
    i.e. inside the immune sub-shape. `VAT-10`'s 0.0998 is the **national-ID** collision only; its
    sweep is `1`-leading, so it cannot see this one at all. Nothing leaks either way — what the
    price buys is the decision that a *business* identifier and a *person's* must not share a
    token, and for the majority of the bare form's issuable space that distinction is not
    delivered. **The maintainer's answer, 2026-09-03: the order stands** — ROADMAP → M11 Track A,
    decision 4, which records what it costs and what it rejected. The short form: the two strings
    VAT-14 pins are *themselves* separator-free, so yielding that arm to `TaxId` would not refine
    the rule, it would undo it.
- **Domestic phone** — numbers written with **no `+CC`** (GB `020 7946 0958`, IT `347 1234567`,
  ES `91 123 45 67`). Historically the "FP-prone, opt-in" tier ([M4-R1](reviews/M4.md#m4-r1),
  [M8.1](ROADMAP.md#m81)): a bare digit run looks like an order number or a national ID, so it
  could not be always-on. **[M10](ROADMAP.md#m10) made it on by default**, because the reason
  for opt-in had already been defused by measurement — a loose regex proposes a candidate and
  the pure-Rust **`phonenumber`** crate's `is_valid()` accepts it only if it is a real,
  **assigned** number for the region, the prefix-and-length check against a real numbering plan
  that no hand-written regex can do. Nine regions ship, on by default; `PII_LOCALES` **replaces**
  that set if you set it, and a code outside the table contributes nothing (an unmeasured region
  is not a region we ship). See the *Domestic phone coverage* matrix below for what that buys and
  what it costs, and the accepted dependency tradeoff under *Supply-chain*.

So `PII_LOCALES` (`Config.pii_locales`, default = every vetted region) gates only the
*domestic-phone* tier, not "which countries" — the national IDs and the VAT tier are never gated
off. The
**language** domain for the NER is the model's declared languages (XLM-R HRL:
ar/de/en/es/fr/it/lv/nl/pt/zh — validated, see `docs/DEVLOG.md`); structured PII is
language-independent.

### Domestic phone coverage — re-measured 2026-07-29 (M10)

**Nine regions, chosen by a principle rather than by taste: exactly the countries the tool
already claims** — the ten national-ID packs plus the NER's language set. `de es fr gb it lv nl
pt cn`. **US needs nothing** (no trunk-`0` domestic form; the universal `NNN NNN NNNN` arm
covers it) and **`ar` gets nothing**, for the same reason it gets no national-ID pack: the
language spans ~20 countries with different plans, so there is no single "Arabic" numbering plan.

**Four candidate shape families, and *which regions may validate each* is the key precision
decision.** A shape is a **rendering**, not a country: several countries write numbers the same
way, and Italy writes them three ways.

| shape | example | declared by |
|---|---|---|
| `Trunk` — leading `0`, compact or 2–3 groups | `020 7946 0958`, `030 12345678`, `011 5627111` | de · fr · gb · it · nl · cn |
| `TrunkPairs` — leading `0`, five 2-digit pairs | `01 23 45 67 89` | fr |
| `Groups` — no trunk, 2–4 groups | `91 123 45 67`, `912 345 678`, `138 0013 8000` | es · it · lv · pt · cn |
| `LongBlock` — no trunk, prefix + one 6–8-digit block | `347 1234567` | it |

**Why per-region and not "trunk / non-trunk".** libphonenumber's `parse` accepts a national number
**with or without** its trunk prefix — in a trunk-prefix country you really can dial a local number
that way — so offering un-anchored groups to DE/FR/NL/GB asks *"could this be a same-area local
dial in Berlin?"*, true of an enormous slice of ordinary numeric text. Measured with every region
seeing every shape: **DE 7 of 24** digit-shaped non-phones became `Phone` spans (`512 1024 2048
4096`, `30 60 120`, `20 30 40`, …), FR 4, NL 3; restricting them to `Trunk` took all three to 0.
The same argument one level finer earns the fourth row: a single non-trunk flag handed China the
Italian-mobile `LongBlock` shape, and China's plan accepts 10-digit runs starting `1…`, so file
offsets and byte counts (`offset 100 1000000 in file`) became `Phone` spans — **0.250 of the
offsets pool, 0.156 of sizes**. Declaring shapes per region takes those to 0 with Chinese mobiles
still covered. `every_declared_shape_is_needed_by_a_real_rendering` fails if a row lists a shape no
rendering of that country needs.

**Measured, per region and for the union** (`tests/phone_eval.rs`, `--release`; 35 corpus
positives, 20 curated negatives, 433 generated digit-shaped non-phones):

| | recall | curated FP | dates | tables | codes | offsets | sizes | ports · money · refs |
|---|---|---|---|---|---|---|---|---|
| de · fr · gb · nl | **1.000** | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| es | **1.000** | 0.000 | 0.000 | 0.188 | 0.000 | 0.000 | 0.000 | 0.000 |
| it | **1.000** | 0.000 | 0.080 | 0.062 | 0.000 | 0.000 | 0.000 | 0.000 |
| lv | **1.000** | 0.000 | 0.120 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| pt | **1.000** | 0.000 | 0.000 | 0.188 | 0.091 | 0.050 | 0.000 | 0.000 |
| cn | **1.000** | 0.000 | 0.000 | 0.188 | 0.000 | 0.000 | 0.031 | 0.000 |
| **union (the shipped default)** | **1.000** | 0.000 | **0.180** | **0.375** | 0.091 | 0.050 | 0.031 | 0.000 |

> **These numbers replace an earlier, rosier set, and the correction is the more useful half.**
> The first pool reported 0.000 for `sizes`, `offsets` and `refs` and concluded "the whole
> exposure is dates". It was wrong in a way worth naming: an un-anchored candidate needs a
> **2–3-digit leading token**, and the pool's non-date entries almost never had one
> (`chunk 8192 bytes`, `order 2026 1042`, `port 8080` — all 4-digit leads, not candidates at
> all), while the `LongBlock` family had essentially no representative. So the zeros were
> reporting the *pool's* shape, not the detector's precision. *A corpus has a shape, and that
> shape is a blind spot* (M4-R13) — landing on the milestone's own deliverable measurement.
> `phone_eval` now asserts that the pool can reach every shape family before it reports on one.

**Read the FP figures per category, never blended.** A single rate over a pool whose composition
you chose is a number about the pool. What they say: **ports, money amounts and reference numbers
are untouched.** The cost is concentrated in two shapes — space- or dash-separated **dates**
(`28 01 2026` is a valid Latvian mobile; `01 02 2026` contains Milan's `02` prefix) and
space-separated **numeric tables** (`512 105 205` is a real Suzhou landline shape). ISO
(`2026-07-29`) and slash (`29/07/2026`) dates cannot collide at all — no family accepts `/`, and a
4-digit leading group is not a candidate. The `tables` category is adversarial by construction: it
was generated *from the families' own structure* to reach them, which is what makes its 0.375 a
ceiling rather than an expectation.

**Three things this trade rests on.**
1. **On real agent traffic the cost is zero.** Over the M7 fixture — a genuine 22 KiB Claude
   Code turn already in the repo, written for a different purpose and so not curated for this
   one — the shipped default yields **no `Phone` spans at all**. Pinned as PHONE-OM
   (`tests/phone_overmask.rs`) with one positive control **per shape family**, so "found
   nothing" can never be "that family is switched off".
2. **The union produces no *emergent* false positives** — a candidate the union masks is always
   one some enabled region masks alone, so a region's measured cost is also its marginal cost.
   That is a **structural property of the dispatch**, not a discovered fact (the validator is
   `.any()` over a superset), so `phone_eval` asserts it instead of reporting it. It does *not*
   mean adding a region is free: the union's FP **set** still grows by set-union, which is
   exactly what the table above shows.
3. **Over-masking a date is a *functional* nuisance, not a privacy failure** — the direction
   this project errs in on purpose. It is not free: a masked line number or port inside
   `tool_use.input` hands the model `[PHONE_1]` where it needed `8080`. That is why the guard in
   (1) is over real traffic and not over a corpus of the false positives we imagined, and why
   the three worst known over-masks are **pinned as tests** (`known_over_masks_are_still_over_masks`)
   rather than quietly dropped from the negative corpus when they stopped passing.

**Latency is not the constraint** — over the same 22 KiB turn the cost is **flat in the region
count**: **0.30 ms/turn with none enabled, 0.32 ms with all nine**, reproduced three times. That is
the point of the dispatch shape: one recognizer per shape *family*, with the region loop inside the
validator, so adding a region costs validations on candidates only, never another O(n·L) scan of
every field (see `national_phone_recognizers`). *(Conditions, per this project's rule about never
quoting a number without them: reference box, `--release`, `--test-threads=1`, **not idle**. One
noisy run once put these at 0.55/0.57 and that figure was briefly published here alongside the
other — the discrepancy is why these are the reproduced ones. The **flatness** is what the decision
rests on, and it survived every run: background load raises a flat line without sloping it.)*

**What keeps a digit-dense field affordable is memoization — and nothing else, which was itself a
finding.** The candidate rescan probes O(n) start positions, so a field of digit groups asks the
same question about the same bytes over and over, at up to five `phonenumber::parse()` calls each;
and `.any()` short-circuits only on *accept*, so a **rejection is the expensive verdict**.
Unguarded, a legal 12 MiB body cost **105 s** of CPU on an unauthenticated path. Validator results
are therefore **memoized per scan** — a call-local map, no lock, no cross-request state; the
validator is a pure function of the matched bytes, so a hit can never change a verdict. That alone
takes 4 MiB of digit groups from 45-50 s to **0.38 s**.

> **A digit-count gate was added alongside it and then deleted — both halves are worth keeping**
> ([M10-R13](reviews/M10.md#m10-r13)). Derived from libphonenumber's `possible_length` metadata to
> make rejection cheap, it was **wrong**: `parse` normalizes before it validates, stripping an
> **international prefix** and a **bare country calling code** as well as the national prefix, so a
> candidate may carry several more digits than any `possible_length`. `39 3332 2673 8858` — a real
> Italian number written with its country code — was refused before any region saw it, and for the
> `Groups` family (regex reaching 15 digits against a mask stopping at 13) recall in that band was
> **0**, two thirds of them *truncated* rather than merely missed. A **miss**, i.e. the one
> direction a privacy filter may never fail in.
>
> And it was **worthless**: measured with the gate fully open, on the very inputs it was introduced
> for — 384 ms vs 382, 257 vs 258, and 145 vs 145 on *distinct* candidates, the case memoization
> cannot help. The memoization was already doing all of the work.
>
> **The rule, and it generalizes past phones:** *a cheap filter in front of a validator must be
> derived from what the **validator** accepts, not from what the **metadata** describes — and it
> must be proved by a differential test against generated inputs, never by a list the author
> expected it to allow.* Its own guard asserted the superset property over 30 hand-written
> literals, all domestic renderings and therefore all <= 13 digits; the defect lived at 14-15.
> *An assertion made only where it cannot fail is not an assertion.* PHONE-NAT-10 now generates
> from each family's grammar and asserts the property that matters directly: **if a region that
> declares this family accepts it, we detect it.**

**And the memo alone does not bound anything — a fail-closed budget does.** A memo keyed on the
matched bytes helps only candidates that **recur**. Every DoS figure this milestone published, and
DOS-05 itself, was built with `unit.repeat(n)` — so all of it measured the *input's periodicity*
rather than the code. On a body of **distinct** digit groups the memo is inert: same shape, same
4 MiB, varying only the distinct-candidate count moved the cost **207 ms → 17,049 ms**, and a legal
15 MiB body answered in **64.5 s** at the default configuration. There is no faster validator to
reach for (the one cheap filter was the leak above), so the work is **bounded** instead:
`MAX_PHONE_VALIDATIONS_PER_REQUEST`, and exceeding it is an `Err` on the `try_detect` channel —
**the request is blocked, never forwarded with a partial scan.** Same call M5-R7 settled: *a detector
may degrade its own recall, but it may never decide for the caller that degraded output is
acceptable.* DOS-06 is the guard, and its generator is an odometer rather than a modular hash for
exactly the reason above.

> **The unit of a budget is the whole budget, and the first version of this one had the wrong unit
> (M10-R28 / M10-R30).** It was `_PER_FIELD` and it meant it: the allowance was minted **per call**,
> `PrivacyStage` calls detection once per text field, and `Vault::mask_all` calls it up to five times
> per field. So the real ceiling was `budget × fields × passes`, and **every one of those factors is
> chosen by the client**. Measured: the same 15.6 MiB that is refused as one field answered `200` in
> **57 s** split across 78 perfectly legal `messages[].content` fields — *indistinguishable from the
> build that had no budget at all*, three warm runs each. The published per-field figure was not true
> in any unit.
>
> **The rule, and it generalizes past this tier:** *a budget scoped to a unit the client can multiply
> is not a bound — it is a rate.* Its companion is on the testing side, in
> [TESTING](TESTING.md#algorithmic-complexity-guards): when a guard's unit (one string) is smaller
> than the attack's unit (one body), the guard cannot see the attack however many axes it varies
> inside its own unit. Every complexity guard this project had written measured **one string**,
> because `try_detect(&str)` is the shape they were handed; the masking path takes a **body**.
>
> There is now exactly **one `Budget` per request**, created by `PrivacyStage::on_request` beside
> the `Vault` it already owns and threaded down through `Vault::mask_all` into `try_detect` /
> `redetect`. Measured after: the 15.6 MiB body is refused in **2.24 s**.

> **The budget is a *parameter*, not an optional second seam — and that took two attempts
> (M10-R35).** The first fix added a `try_detect_within` / `redetect_within` pair whose **defaults
> delegated to the budget-less originals**. Every wrapper then carried an obligation to override
> *both*, and the penalty for missing one was invisible: the call fell through to a method that
> **minted a fresh allowance**. The round that introduced it saw the hazard clearly and closed it for
> all three wrappers — and missed the **leaf**. `StructuredRecognizers` is the only detector whose
> cost the budget bounds and the only place that mints one; it overrode `try_detect_within` and not
> `redetect_within`, so every fixpoint pass after the first started full again and a legal 15.63 MiB
> body answered `200` in **17.2 s** against a published ceiling of ~1.4 s. **The whole suite stayed
> green**, before and after the one-line difference.
>
> So the pair is gone. `try_detect` and `redetect` **take** a `&Budget`, and `redetect`'s default
> forwards *the same one*; `Vault::mask_all` lost its budget-less convenience for the identical
> reason.
>
> **And that fixed one half of the trait (M10-R44).** `try_detect`'s *own* default still routed to
> `detect` — and every production `detect` **mints** an allowance and `unwrap_or_default()`s the
> refusal. A five-line wrapper implementing only `detect` compiled, never mentioned `Budget`, and
> forwarded a body the request path must refuse, with the refusal not ignored but **erased**. Word
> for word the sharpest case M10-R35 described, surviving in the half its fix did not turn over. So
> the required method is now `try_detect` and `detect` is the derived one: `detect` is the
> convenience view, `try_detect` is the contract. A detector that cannot fail writes `Ok(..)` — but it
> writes it, and it sees the budget. **No method remains that a default could route to which would
> mint another**, and the only ways to create an allowance are `Budget::new` / `per_call` /
> `unlimited`, each visible at its call site.
>
> *An obligation that a trait default can satisfy is not carried by the type system — and "every test
> passes" is the signature of that, not evidence against it.* The general form: **when forgetting to
> override is silently valid, the API is the defect**; make the thing that must travel a parameter,
> so omitting it does not compile. Note which direction the guarantee comes from — the four
> detect-only test doubles that stopped compiling are the whole regression suite for this, and they
> are worth more than a test would be.

> **`FailOpen` is not fail-open about the budget, and the distinction is a leak if collapsed.**
> *"This detector is unavailable"* is a property of the **detector**; continuing without a
> non-critical engine is exactly what the wrapper is for. *"The allowance for scanning this request
> ran out"* is a property of the **request** — the text was not fully examined, so its PII status is
> unknown, and answering `Ok(vec![])` there forwards a partially scanned body with a clean bill of
> health. Round 4 hunted for a path where an exhausted budget fails *open* and found none, but only
> because nothing wraps the structured recognizers in `FailOpen` **today**. That was a property of
> the wiring, not of the code, and wiring changes.
>
> **It asks the error, not the budget (M10-R41).** The first version matched on
> `budget.is_exhausted()` — a property of the **request**, asked about an error that may belong to the
> **detector**. It gave the right answer, but only through an invariant nothing stated: no detector
> returns `Ok` with an exhausted budget, *and* `build_detector` happens to order the structured
> recognizers first. Same wiring-dependent argument, one level down. And it had a cost in the other
> direction — a genuine GPU or tokenizer failure arriving while the budget was coincidentally spent
> became a `400` on a proxy explicitly configured to degrade to structured-only. `DetectError` now
> carries `budget_exhausted`, built by `DetectError::budget_exhausted(..)` vs
> `DetectError::unavailable(..)`, so the wrapper reads what it means and the decision lives in one
> function. *Correlating with a global state is not the same as distinguishing a kind.*

**What the budget costs, and the number is chosen against a legal payload.** One unit is one
`phonenumber::parse()`, measured at **~3 µs** on the shipped release build, so **500,000** bounds a
request's domestic-phone *validation* at **~1.5 s** of CPU. Every row below is printed by `DOS-BUD`
(`cargo test --release --test complexity -- --ignored --nocapture budget_refusal_line`) — *a number a
reader must trust goes stale; a number they can re-run is a fact*:

| body | verdict | units | wall clock |
|---|---|---|---|
| M7 22 KiB Claude Code turn | masked | **0** | — |
| 357 KB SQL result, 5,000 rows, one phone column | masked | 5,000 | 46 ms |
| 3.7 MB SQL result, 50,000 rows | masked | 50,000 | 514 ms |
| **16 MiB SQL result, `347 XXXXXXX` column** | masked | 221,941 | 2.4 s |
| **16 MiB SQL result, same numbers written `3XX XXX XXXX`** | **refused** | 500,000 | 1.6 s |
| 62,500 grouped numbers — bare column (793 KB) · `name,phone` (2.0 MB) · 6-column (4.45 MB) | **refused** | 500,000 | — |
| 2 MiB field, *nothing but* phone-shaped groups | masked | 499,380 | 1.36 s |
| 3 MiB+ field, same shape | **refused** | 500,000 | 1.44 s |
| 1 x 200 KB field | masked | — | 151 ms |
| 5 x 200 KB fields (1 MB) | masked | — | 814 ms |
| **15.6 MiB across 78 x 200 KB fields** | **refused** | 500,000 | **1.61 s** |
| 16 MiB, phone tier **off** - the unbudgeted floor | masked | 0 | 229 ms |

> **The allowance is a count of *numbers*, and how many depends on how they are written.**
> `national_phone_valid` is `.any()` over the regions whose plans use that candidate's **shape
> family**, so the accept path is cheap — but `Scan::Overlapping` resumes one `char` past each match's
> *start*, so a grouped or pair-separated number also proposes **sub-candidates from inside itself**,
> and each of those is rejected and pays its family's whole region list.
>
> **Per row in a column of 20,000 — the figure a real payload meets:**
>
> | column of | units/row | | column of | units/row |
> |---|---|---|---|---|
> | `347 XXXXXXX` (IT `LongBlock`) | **1.00** | | `0X XX XX XX XX` (FR pairs) | 3.27 |
> | `+39 3XX XXXXXXX` (any `+CC`) | 1.02 | | `6XX XX XX XX` (ES grouped) | 3.27 |
> | | | | `3XX XXX XXXX` (**IT grouped**) | **8.00** |
>
> So **500,000 units is ~62,500 phone numbers per request** at the most expensive column rendering
> measured, and ~500,000 at the cheapest. The **bytes** at which that lands are a property of the
> payload's layout, not of the limit — the same 62,500 grouped numbers are refused at **793 KB** as a
> bare `phone` column, **2.0 MB** as `name,phone`, and **4.45 MB** as a six-column export. An ordinary
> 5,000-row export spends **8%** of the allowance; a real 22 KiB Claude Code turn spends **0**.
>
> **Per candidate in isolation the numbers are different, and the difference is the memo.** One
> `06 12 34 56 78` costs **46** units on its own — against 1 for `347 1234567`, 0 for a `+CC` form
> (that recognizer has no validator at all) and 65 for the most expensive candidate found, which no
> plan accepts. In a *column* those collapse, because the per-scan memo absorbs the repeated
> sub-candidate prefixes: FR drops from 46 to 3.27. *Isolation measures how a rendering behaves; the
> column measures what a body costs, and only the second one answers "can real traffic reach this?".*
>
> **Four published versions of this band were wrong, all in the optimistic direction, and the shape of
> the error never changed (M10-R49 · R53 · R56).** An eleven-digit column that masked nothing; then
> `347 XXXXXXX` alone, which is the cheapest legal phone in the shipped set; then a "2.6 MB" figure
> from a probe whose generator repeated, so the memo served most of it free. Each time the measurement
> was correct and the **generalization** was not. *A conclusion drawn from one point of a grid is a
> fact about that point* — so the band is now published as a count with a density note, re-run by
> `DOS-BUD` in both renderings and three layouts, rather than as a megabyte figure.

**The number is 500,000 and not 50,000 because of an ordinary tool result.** At 50,000 units a 367 KB
database result was refused. *A fail-closed
threshold whose refusal is a routine event is the wrong threshold:* every refusal costs the agent a
turn, and a bound that fires on legal traffic teaches its operator to raise it rather than to trust
it. Which is also why it is **not** an environment variable (M10-R27): a CPU bound an operator can
raise is not a bound. Headroom against a conversation is total — the M7 turn spends **zero** units,
pinned by `PHONE-BUD`.

> **The budget bounds validation, not the whole request — and saying otherwise would be M10-R30 in a
> new place.** **Three** terms make up a request's CPU, and the third was found by M11-R18 rather
> than designed:
> 1. `units x ~3 µs`, which the allowance caps at ~1.5 s.
> 2. Regex scanning and the mask rewrite, linear in body size and entity count and bounded only by
>    `MAX_BODY_BYTES` — **229 ms** for 16 MiB with the tier off.
> 3. **`free()` validator work, which is per *candidate* and charged to nothing.** The wrapper's own
>    doc justifies the exemption as *"a checksum over at most 18 bytes"*, and that was true while
>    each validator ran once per match. `shrink_on_reject` (M11-R13) retries a rejected IBAN at every
>    interior separator — up to eight `iban_case_gate` calls per match, on spans up to 44 characters
>    — and on a body of *distinct* alphanumeric groups the per-scan memo is inert, so every one is a
>    miss.
>
> **The old two-term model published "at most about 3 s" and that no longer holds.** On the shape
> term 3 is sensitive to — 4 MiB of distinct lowercase `[a-z]{2}[0-9]{2}` groups — the library
> measures a **median of 2.1 s** against a **671 ms** uppercase control of identical size, and a
> 14.6 MiB request through the real binary takes **8–10 s** against 2–3 s for the same body
> uppercased (M11-R18). A masked 16 MiB body of ordinary content still takes 2.4 s and a refused one
> 1.3–2.9 s; what changed is that those are no longer the worst shapes measured.
>
> **This is a constant factor on a linear path, not a blow-up, and not a leak** — but availability is
> a privacy property here, so the number matters. **How to bound term 3 is an open maintainer
> decision** ([M11-R18](reviews/M11.md#m11-r18) lists four options with what each costs); the
> allocation-free rewrite of `iban_mod97`/`iban_length_ok` took ~18% off it and does not close it.
> *(An earlier version of this line said "every refusal lands at ~1.4–1.9 s". That band came only from
> DOS-BUD's adversarial rows, whose candidates are rejected and whose scan therefore stops early; a
> **legal** refused body runs further and costs more — M10-R53.)*
>
> What changed is not that the ceiling vanished; it is that it stopped being **multiplied by a factor
> the client picks**. 57 s -> 1.61 s on the same 78-field body, and what remains is linear in bytes.
> *Linear is a shape, not a budget* — so the shape is published with the number beside it, rather
> than the validation term alone wearing the word "ceiling".

**The validator is not a locale *discriminator*.** National plans overlap, so a number valid in
one region can be valid in another — a GB mobile also validates as DE, and `0123456789` is a real
Paris number. That is **privacy-safe** (over-masking a real phone is never a leak); it just means
enabling one region does not reject every other region's numbers, and it is why a per-region
precision figure does not predict the union's.

> **A trunk anchor does more than reduce false positives: it constrains *where a candidate may
> begin* (M10-R1).** Remove it and an accepted span can start at an arbitrary digit — so a
> greedy over-match that swallowed the next number is rejected, a *shifted* window is accepted
> instead, and masking **truncates** the neighbour rather than shadowing it:
> `912 345 678 913 456 789` masked to `912 [PHONE_1]`, three digits of a real number upstream in
> clear. **The fixpoint recovers a value it did not touch; it can never recover one the mask
> ate** — which is why M8-R8's "the next pass masks it" argument does not carry over from the
> trunk families, and copying it forward was the actual defect.
>
> **The anchor constrains; it does not forbid** — and the first version of this note said it did
> ([M10-R26](reviews/M10.md#m10-r26)). A trunk candidate must begin at a `0` on an ASCII word
> boundary, and inside `020 7946 0958` there is one: `0958`. So a trunk span *can* start
> mid-value. What differs is the consequence: the shifted span **overlaps** the real number and
> the resolver unions them, so the bytes are covered and nothing is truncated — the outcome is an
> over-mask (`020 7946 0958 0161 496 0000` becomes one placeholder at the shipped default, two
> under `PII_LOCALES=gb`), never a leak. **That, and not the stronger claim, is why the trunk
> families need no shrink.** A targeted sweep of 3,870 variants found no truncating case.
>
> Two lessons outlive the fix. **The un-anchored families retry a rejected match one digit group
> shorter** until the validator accepts a prefix at the *same* start, which is what the anchor
> gives for free. And the predicate that catches this is *"no byte of a real value survives"*,
> not *"nothing detectable survives"* — the orphaned `912` is not detectable, which is precisely
> why it survived every existing guard, including PROP-03 (it quantifies over **accepted**
> candidates, and those bytes belonged to a rejected one). PHONE-NAT-09 asserts the stronger
> form, on the **default** region set: its sibling PHONE-NAT-04 runs on `["gb"]`, where the
> coalescing above does not occur and so the interesting case cannot arise.

**Known recall gaps, deliberate and measured.** A non-trunk number written **compactly**
(`3471234567`) is not a candidate: bare digit runs are indistinguishable from order numbers and
timestamps, and the 9-/11-digit ones are already over-masked by the national-ID tier under the
M4-R6 tradeoff. Latvia's 4+4 rendering (`6712 3456`) is out for the same reason — allowing a
4-digit leading group made every `YYYY NNNN` pair an 8-digit candidate, which Latvia's plan
accepts: four new false positives for one rendering. LV stays covered by `67 22 33 44` and
`67 123 456`.

**Word boundaries are ASCII — `(?-u:\b)`, never a bare `\b` (M4-R13).** Rust `regex`'s default
`\b` is **Unicode-aware**: a Han / Kana / Cyrillic letter *is* a word character, so there is **no
boundary between a CJK character and a digit**. Chinese and Japanese have no inter-word spaces, so
the glued form is the *natural* way to write it — and with a Unicode `\b` every anchored recognizer
was **inert** in CJK prose, forwarding the PII in clear (`我的信用卡号是4111111111111111` matched
*nothing*; the zh Resident ID pack shipped in M4 never fired in Chinese). All anchored recognizers
therefore use `(?-u:\b)`, which counts only `[0-9A-Za-z_]` as word characters. The deliberate
anti-false-positive guarantee is preserved **exactly** — an ID still cannot fire inside a longer
*ASCII* token (`card4111111111111111`, API keys, hashes, base64) — it merely stops treating a
non-ASCII **letter** as part of a number. (`Email` / `Phone` are anchored by character classes, not
`\b`, so they were never affected.)

> **…but `\d` is still Unicode, on purpose — so a validator may never index a match by *byte*
> (M11-R21).** The M4-R13 fix turned Unicode off for `\b` and left it **on** for `\d`, and that
> asymmetry is right: a digit written `٤`, `๔`, `４` or `𝟒` is a digit, and a recognizer that could not
> see one would be inert in exactly the scripts M4-R13 was about. The consequence is that **every
> pattern containing `\d` can hand its validator a match with 2-, 3- or 4-byte characters in it**, and
> `str` indexing panics on a non-`char` boundary. `iban_mod97` did precisely that — `&compact[..4]`
> after compacting — from the project's first commit until `831f916`, so `AB𝟎𝟏ABCDEFGHIJK` in a legal
> `content` field returned **500** on every release up to `v1.2.1`. Fail-*closed* (nothing is
> forwarded), never a leak — but a 30-byte unauthenticated request that kills its own response is an
> availability defect, and availability is a privacy property here.
>
> **The rule: a validator takes `&str` and must treat it as text.** Iterate `chars()`, or filter to
> ASCII bytes *before* checking a length and indexing — which is what every other validator in this
> file already does (`es_dni_nie_valid`, `fr_nir_valid`, `cf_check_valid`, `nl_bsn_valid`,
> `zh_resident_id_valid`, `de_steuerid_valid` all reject a multi-byte character by returning `false`,
> never by indexing). A byte length is not a character count, and a `.len()` gate in front of a byte
> slice only looks like a guard.
>
> **Nothing enforces this yet** — reverting `iban_mod97` to its pre-`831f916` body leaves the whole
> suite green, because every corpus in the repo is ASCII (the `non_ascii_scripts` cases carry
> non-ASCII *letters* around ASCII values, never a non-ASCII *digit inside* one). The chokepoint is a
> property test — *`try_detect` panics on no input* — over a generator whose alphabet includes
> multi-byte `\p{Nd}`; see [M11-R21](reviews/M11.md#m11-r21), open.

**Every letter-bearing recognizer carries a *decided* answer on letter case (M11-R10).** This is
the ASCII-word-boundary rule's sibling, and it went undecided for ten milestones. The patterns above
spell their letters themselves, so *how a value is capitalised* is a coverage decision — and it had
never been made: the Codice Fiscale, the ES DNI/NIE and the CN resident id folded case, while the
VAT prefixes and IBAN were written `[A-Z]`. The consequence is not a shorter span but **no span at
all**: the letters are ASCII word characters, so there is no `(?-u:\b)` between them and the digits
for a bare-digit recognizer to fall back on. Seven of thirteen renderings of values already in this
repo's corpus reached the provider **in clear**. The rule now has three parts:

- **Case is a rendering convention -> fold it.** The five VAT prefixes and the NL literal `B` do,
  spelled as explicit ASCII classes (`[Ii][Tt]`) rather than `(?i)`, so no *Unicode* case folding
  can widen them. Measured cost over 341.1 MB of third-party source: **0 added matches**.
- **Case is part of a format -> keep it.** `Secret`'s `sk-` and `AKIA` are literal prefixes the
  provider issues, not renderings of one. This is an answer, not an omission, and the guard can
  express it.
- **IBAN folds only under a verification gate, and the asymmetry is deliberate.** Folding IBAN's
  continuous arm costs **+931 masked spans** on that same corpus — hex digests, base64 blobs — and
  IBAN has no hard checksum gate (M4 masks a structurally valid one even when mod-97 fails). So
  [`iban_case_gate`] splits by rendering: canonical uppercase keeps M4's rule, while a span
  carrying **any** lowercase letter must pass mod-97 **and** the ISO 13616 length. Residue: **1 of 936** added matches over 304.9 MB — not zero, and the *bound* is the
  mod-97 rate rather than zero, because a country code the length table does not know is gated
  by mod-97 alone (M11-R19). An operator reading this should know the visible consequence — *a lowercase IBAN whose
  mod-97 fails is not masked, while its uppercase twin is.*

**A validator that rejects must not be able to delete a value (M11-R13).** The corollary, learned
immediately and the hard way. `iban_case_gate` was added to a recognizer that had previously
accepted everything, and rejection is not free when a pattern can over-reach: the grouped arm's
optional trailing group swallowed the next short word, the gate refused the over-long span for the
lowercase byte that word contributed, and the whole candidate disappeared — country code, check
digits and last group forwarded raw. The rule: **whatever a newly added validator refuses, those
bytes must still reach the resolver in some form**, or the gate is a coverage switch wearing a
precision argument. Mechanically that is `shrink_on_reject`, which retries the span one separator
shorter; the predicate that proves it is *no byte of the value survives masking* (M10-R1), not
*nothing detectable survives*.

**Candidate generation must see *overlapping* matches (M4-R17).** `Regex::find_iter` is
leftmost-**non-overlapping**: after a hit it resumes at the match's *end*. A real value that **starts
inside** an earlier match of the *same* recognizer is therefore never emitted as a candidate — and an
invariant over the candidate set is then satisfied **vacuously**, because the resolver never learns the
value exists. *An invariant is only ever as strong as the set it quantifies over.* So a recognizer
resumes one `char` past a match's **start**, and its overlapping hits are coalesced into maximal runs
(which keeps the candidate set bounded on pathological input at no cost to coverage — the resolver
would union those spans anyway).

**…but only where the pattern's length is *bounded* (M4-R19).** That rescan probes O(n) start
positions, each costing at most one maximal match, so it is **O(n · L)** for a pattern whose longest
match is L. Linear while L is bounded — a card is ≤ 19 digits, an IBAN ≤ 44 chars, every national ID ≤
18 — but the two **unbounded** patterns, `Email` (`[…]+@[…]+`) and `Secret` (`sk-[…]{6,}`), have L =
O(n), so it degenerates to **O(n²)**: a ~1 MB `content` field, far under the 16 MiB body limit, pegged a
core for *minutes* on an unauthenticated path. Those two therefore keep plain `find_iter` semantics
(`Scan::Sequential` in `recognizers.rs`), and it **costs no coverage** — a same-recognizer match that
starts inside an earlier one is *contained* in it (both run greedily to the same word boundary), and the
one shape that isn't, a chained `a@b.com@c.com`, is caught by the **fixpoint** below instead. So the two
mechanisms are complementary: **bounded recognizers rescan; unbounded ones iterate.**

> **The rule for a new recognizer:** if its match length has no upper bound, it must **not** take
> `Scan::Overlapping`. `tests/complexity.rs` (DOS-01…03) is the guard — it fails on a super-linear scan
> in seconds rather than hanging.

> **And a *bounded* recognizer can still need the fixpoint (M8-R8).** A multi-arm bounded recognizer whose
> longest arm can span into an **adjacent** value inherits the Sequential recognizers' fixpoint reliance for
> a different reason. The national-phone regex is one: its 3-group arm can greedily take the trunk of the
> *next* number (`0800 1111 0800`), an over-long span the `is_valid` validator **rejects** — and because the
> overlapping rescan resumes *forward* of that rejected match, single-pass `detect()` misses the shorter
> valid form it shadowed (the first of the two numbers). No leak on the request path: `Vault::mask_all`'s
> confirmed fixpoint re-detects, masking the second value un-shadows the first. So such a recognizer must
> **never** override `redetect` to skip later passes, and its anti-swallow guard must assert at the
> **`mask_all`** level (does the fixpoint mask *both*?), not on single-pass `detect()`.

**Masking must be linear in the entity *count*, not just the field *size* (M4-R24).** These are **two
independent dimensions**, and closing one says nothing about the other. `Vault::mask` used to splice
placeholders in right-to-left with `String::replace_range`; each splice memmoves the whole tail, so *k*
entities in *n* bytes shift Θ(n·k) bytes — and a field of many **small** values (`a@b.co `, an SSN, a
phone) has *k* growing with *n*, so it is **Θ(n²) all over again**, on the same unauthenticated path.
13 MiB of repeated emails burned **~7 minutes**; the *same* 13 MiB as **one** giant email masked in
219 ms. Detection being linear does not bound the splice — it is a separate cost, which is exactly why
M4-R19 closed without touching it. `mask` now makes **one left-to-right copy into a fresh, capacity-
reserved buffer** — O(n + k), each byte touched once. Placeholder numbering is unaffected: it follows the
entities in start order, which splice direction never determined.

> **The rule:** the complexity guards must vary the entity **count**, not just the field **size**.
> DOS-01…03 each pin a *single* entity, so a per-entity quadratic lived right underneath them for four
> milestones — the *"a corpus has a shape, and that shape is a blind spot"* lesson (M4-R13) recurring on
> the DoS guards themselves. **DOS-04** is the many-entity guard.
>
> **Scope, stated honestly:** what is linear on both axes is the **structured** (default-build) masking
> path — detect → resolve → splice → de-mask. The optional `onnx` NER is a separate, opt-in cost with its
> own scaling behavior — chunked and measured linear (M5, PERF-01; see *Hybrid detection* below), not
> covered by this section's guards.

**Masking must run to a *fixpoint* (M4-R17).** Masking **rewrites the bytes around what it replaced**,
and a value is only recognizable in context — so masking can *expose* PII that was not detectable
before:

```text
4111111111111111555 867 5309   one 19-digit run: not Luhn-valid, so correctly NOT a card
                               (an ID never fires inside a longer token)
4111111111111111[PHONE_1]      masking the phone SPLIT the run — the leftover is a clean,
                               Luhn-valid card, and it would go upstream in clear
```

`Vault::mask_all` therefore re-detects until the text yields nothing. It converges because a
placeholder is **inert** (no recognizer can match `[KIND_N]` or span across it), so each pass strictly
shrinks the un-masked text. The round-trip stays exact — every pass records raw value → placeholder, and
`demask` restores them all in one tolerant pass.

> **The NER runs only on pass 0 — later passes are structured-only (S4, CC-05/CC-08).** Masking
> *exposes* PII only by splitting a token, which is a **structured-recognizer** phenomenon (the card
> above). Masking a name to `[PERSON_1]` never reveals a *new* name, so re-running the NER buys no recall
> — measured **0** losses across the labelled corpus — while it *does* re-tag the **sub-word fragments**
> the model emits (`"lack"` of `"Slack"`, `"An"` of `"Anthropic"`; the [M7-R7](reviews/M7.md#m7-r7)
> over-mask). On a real Claude Code **system prompt**, dense with such names, those fragments chained
> **past `MAX_MASK_PASSES` and fail-closed 400'd live** (CC-05/CC-08) — offline the pass count grew
> 6 → 11 → 13 as the field grew, unbounded. So `mask_all` runs the whole detector on pass 0 and only
> `redetect` — the detectors masking can expose (the structured recognizers), never the NER — on later
> passes *and the fixpoint confirm*. The pass count is then **O(1) on a fragment-dense field**, and it is
> the latency win M4-R21 priced (the field's second full NER scan). M7-R7 called this "a latency cost, not
> a correctness one"; past four passes it was a fail-closed **availability** defect, and this is its fix. A
> **word-boundary snap** of the fragments was measured and **rejected** — it makes convergence *worse*.
> The no-recall-loss claim is a model-swap checkpoint:
> `tests/ner_perf.rs::m7_s4_dense_org_names_converge_instead_of_400`.

> **…and the NER inside it can no longer break that in the *other* way, either, because `mask_all` won't
> let it (M5-R4 / CC-08).**
> "A placeholder is inert" is proved **by construction** for the deterministic layer: `[KIND_N]` has no
> `@`, no `sk-`, nowhere near enough digits, and `[` / `]` sit outside every pattern's character classes.
> But `mask_all` runs the **`CompositeDetector`**, and the ML NER inside it is under no such constraint —
> nothing *structurally* stops a model from tagging `[PERSON_1]`, or a dense run of placeholders, as a
> `Person`. If a pass re-masked one, the text would not strictly shrink, `MAX_MASK_PASSES` would exhaust,
> and the request would **400** — fail-*closed* (M4-R20), so **never a leak**, but a hard availability
> failure on ordinary input.
>
> So the loop closes that door itself: `keep_maskable` **drops any detection that is exactly one of our
> own `[KIND_N]` tokens** — a real value can never take that shape — before it is masked. Every surviving
> detection is then genuine PII, masking genuine PII strictly shrinks the raw text, and the fixpoint
> converges **regardless of the NER**. Placeholder inertness is now a property of the *algorithm*, not of
> the chosen model.
>
> The shipped model doesn't even try — XLM-R int8 tags **zero** entities on placeholder-only text
> (`tests/ner_perf.rs`, `m5_r4_the_ner_treats_placeholders_as_inert`) — but that test is now
> **belt-and-braces**, not the sole guarantee, and it is the **durable model-swap canary**. The Backlog's
> successor is **GLiNER**, a *zero-shot, open-label, **context**-driven* extractor that could well look at
> `Contact [PERSON_1] at [ORG_1]` and tag both; `m5_r4` runs the NER **directly** on placeholder text and
> catches exactly that, independent of everything else. (The runtime `placeholder_tags_suppressed` counter
> is a *weaker* signal since **S4**: the NER runs only on pass 0, so it can never re-tag masking's *own*
> output — the counter fires only when a detector tags placeholder-shaped text already **in the raw field**,
> e.g. a client echoing placeholders in Run ON. Useful, but it is not the GLiNER canary — the test is,
> M7-R23.)
>
> M5 is also what made this *reachable at scale*: before chunking, a field over ~500 tokens never reached
> the NER at all (it errored). Chunking now routes exactly the large, placeholder-dense fields through it.

**Exhausting `MAX_MASK_PASSES` fails *closed* (M4-R20).** The bound is a safety net, not a proof: *"each
pass strictly shrinks the un-masked text"* buys **eventual** convergence, never convergence **within**
four passes. So `mask_all` **confirms** the fixpoint rather than assuming it — one final `try_detect`,
and if anything is still detectable it returns `Err` and `PrivacyStage` **blocks** the request (400).
Forwarding a *probably*-clean text is exactly the failure mode a privacy proxy must not have. (No input
has ever been shown to need more than **2** passes — an exhaustive search over 314k glued inputs never
exceeded it, because masking *fragments* a digit run rather than peeling it — so this stays a latent
path. It is fail-closed regardless, which is the point: the bar does not depend on the search being
exhaustive.)

> **This fired for real, once, and taught the block to explain itself (CC-08).** A live Claude Code
> session hit the 400 on an ordinary long turn; it has not reproduced since, in the live session or in
> a dozen synthetic reconstructions (placeholder-dense fields, the raw CSV, the chunked path — all
> converge in ≤1 pass). No leak — the block did its job — but a 400 with no *reason* is its own defect.
> With placeholder re-tagging now ruled out by construction (above), a residue can only be genuine PII
> that masking keeps re-exposing, so the fail-closed branch logs a **value-free** diagnostic to name it:
> the per-pass kind tally (is the count shrinking, i.e. a deep nest that would clear with more passes, or
> stalled?), the residue's kinds, and `placeholder_tags_suppressed`. Kinds and counts only — never the
> text, which fail-closed never forwards or logs. **That instrument then paid off:** re-running the
> battery, CC-05 hit the same 400 and the diagnostic pinned it — `placeholder_tags_suppressed=0`, a
> residue of real `ORG`/`PER` fragments, not placeholders. The cause was **NER sub-word fragmentation**,
> and the fix is **S4** (above): the NER runs only on pass 0, so the fragments can't chain past the bound.

**Detection cache (S3, `src/pii/cache.rs`, M7.1).** Claude Code re-sends 20–40 KB of **byte-identical**
system prompt + tool schemas every turn, and detecting PII in it — the NER above all — dominates the
masking latency. `CachingDetector` wraps the composite and memoizes `try_detect` **keyed on the exact
field bytes**, so turn 2+ skips the scan; the per-request vault still mints the placeholders, so numbering
is unchanged. **The fail-closed soundness argument** is the whole design: `try_detect` is a pure function
of its input (stateless regex; NER inference on the input alone), and the key is the *whole* input — so a
hit returns exactly what a fresh scan would and **can never mask less**. Only `Ok` results are cached (an
error still fails closed), the cache is bounded (a two-generation map, ~`2 × PII_CACHE_ENTRIES` live
entries, only fields 256 B–128 KiB), and `redetect` (the S4 later passes, on per-request masked text) is
never cached. `PII_CACHE_ENTRIES=0` disables it. Proven end-to-end by
`tests/proxy_e2e.rs::e2e_cache_on_a_repeated_large_field_still_masks_both_times`.

**Overlap resolution (`src/pii/overlap.rs`).** Detectors produce overlapping candidate spans;
`resolve_overlaps` reduces them to a non-overlapping set. Its governing rule is an **invariant,
not a ranking** (M4-R10 / M4-R11):

> **No structured span's bytes are ever abandoned.**

This replaced an earlier "highest priority wins" resolver that settled every overlap by **dropping
the whole loser span** — which silently left the loser's bytes *in clear*. A flat priority scalar can
only express "one of them wins"; it cannot express "**both** must be masked". On a **partial** overlap
(`555 867 5309john.doe@example.com` — the phone and the email share only `5309`) that meant whichever
side lost got forwarded unmasked, and re-tuning the priorities merely chose *which* PII leaked. Two
phases now:

1. **Structured union-merge.** Every group of transitively-overlapping structured spans collapses into
   its **union**. *Nothing structured is ever dropped*, so the invariant holds by construction. One
   sort-by-start sweep reaches the fixpoint. The union is masked as a single placeholder and restored
   verbatim, so the round-trip stays exact. It over-masks slightly (a bare `@domain` can land inside the
   placeholder) — the project's fail-safe direction: **over-mask, never leak**.
2. **NER greedy drop.** NER spans keep the whole-span drop (M2-R7): a `Person` overlapping a kept
   structured span is discarded entirely. Abandoning an *unstructured* remainder costs recall, never a leak.

**Naming the union** (`PiiKind::priority`: Secret > Iban > Card > Ssn ≈ NationalId > Phone > Email) —
the highest-priority **raw** candidate the union covers, *including one an email encloses* (M4-R15), so a
`Secret` glued to a phone isn't announced to the model as `[PHONE_1]` and the kind-only audit log doesn't
under-report it. **One exception:** when the union is *exactly* an `Email` span, the group is a genuine
email whose local part merely *looks* like a card or an ID (`4111111111111111@x.com`) — that enclosed
match is a false decomposition, not a second entity, so the email keeps the label.

| Shape | Overlap | Result |
|---|---|---|
| `4111111111111111@x.com` | the email **encloses** the card; union == the email span | one `[EMAIL_1]` — the card is a false decomposition of the local part |
| `4111 1111 1111 1111@x.com` | **partial** — a space-grouped card, and an email local part can't hold a space, so the email is only `1111@x.com` | one `[CARD_1]` over the **union** — card *and* trailing email masked |
| `555 867 5309john.doe@example.com` | **partial** — phone and email share `5309` | one `[PHONE_1]` over the union — the email's local part is **not** abandoned |
| `555 867 5309.sk-…@x.com` | **partial** phone + an email enclosing a secret | one `[SECRET_1]` — the union is named by the highest-priority raw candidate it covers |

Enclosure is a **naming** rule, not a deletion. (An earlier revision *deleted* the enclosed span before
ranking the rest — which stranded it in clear whenever the enclosing email then lost, M4-R10. The union is
identical either way, since an enclosed span merges *into* its enclosing email; expressing it as naming
keeps the behaviour and removes the trap.) The invariant is pinned by a property test (**PROP-03**,
`every_structured_candidate_byte_is_covered`): every raw structured candidate must be fully covered by
some resolved span. A winner-picking resolver cannot satisfy a per-byte invariant — which is exactly why
the two earlier priority-only fixes each passed their own case while leaking the other side.

## Anonymization

Detected spans are replaced with typed placeholders of the form `[KIND_N]` — e.g.
`[EMAIL_1]`, `[PERSON_2]` — ASCII and tokenizer-friendly. A per-request `Vault`
maps placeholder → original so the response can be restored exactly (round-trip).
Text with no PII passes through unchanged.

## Prompt augmentation (helping the model read placeholders)

The downstream model sees only masked values, so it must be told how to read
them — otherwise it can mishandle them, especially in tool calls (treating
`[EMAIL_1]` as literal noise instead of a stand-in for an email). The privacy
stage therefore **transparently injects a system instruction** into the outgoing
request, stating that:

- values like `[EMAIL_1]`, `[PERSON_2]` are placeholders standing in for real
  data of the named type;
- they must be used verbatim — including as tool-call arguments — never altered
  or guessed at;
- they will be restored downstream before anyone real sees them.

This expands the round-trip scope:

- **Tool calls are in scope** — `tool_calls` arguments in the response are
  de-anonymized (so the client runs tools with real values), and tool *result*
  messages coming back are re-anonymized before going upstream. It is not just
  chat message text.
- **Placeholder assignment is deterministic** — the same real value maps to the
  same placeholder every time it appears, so the model can correlate it across a
  multi-turn (stateless) conversation where history is re-sent and re-masked on
  each request.

## Robustness & fail-closed (M1.5)

For a privacy proxy the failure mode *is* the product: anything unexpected must
**fail closed** (block or scrub), never forward raw PII.

- **Fail-closed request handling.** A stage can set `RequestContext.block` when it
  hits something it can't safely mask — an unreadable `content` shape (a bare
  object/scalar) or a missing/!array `messages`. The proxy then returns **400**
  and never forwards. Masking always runs *before* forwarding, so a masked value
  can't leak even on a later error.
- **API scope.** `POST /v1/chat/completions` is always proxied; `POST /v1/messages`
  (the native Anthropic schema, M6) is proxied **only when `UPSTREAM_PROVIDER=anthropic`**;
  `GET /healthz` is served for liveness. Every other path/method returns **404** via the
  router `fallback` and is never forwarded — we don't proxy schemas we don't model
  (`/v1/responses`, `/v1/embeddings`, … are out of scope for now).
- **Field coverage.** The masker scans *every* text-bearing field of the chat
  schema — message `content` (string and the `text` of array parts, all roles),
  `name`, `tool_calls[].function.arguments`, legacy `function_call.arguments`,
  `tools[].function.description`, and every `description` inside
  `tools[].function.parameters`. One shared per-request `Vault` means the same
  value gets the same token even when it's split across fields.
- **Body-size limit.** `MAX_BODY_BYTES` (default 16 MiB) is applied via
  `DefaultBodyLimit`, above axum's 2 MiB default, so long-context requests aren't
  silently rejected.
- **Masking runs on the blocking pool, not on a tokio worker (M4-R19).** Detection is
  **CPU-bound** — regex scans over every text field, plus NER inference when it's on — and
  it sits on an **unauthenticated** path (it precedes any upstream auth). Run inline, a
  handful of concurrent large bodies would starve the executor and the whole proxy would
  stop serving, so the request-stage loop goes through `tokio::task::spawn_blocking`. If
  that task itself dies (a panic in a stage), the request is **blocked**, never forwarded:
  we'd be holding a body whose PII status is unknown. This bounds the *blast radius* (the
  async executor survives; `/healthz` still answers) — it does **not** bound the *cost*. The
  cost is bounded separately, by keeping the structured masking path linear on **both** of
  its axes: field size (M4-R19) **and** entity count (M4-R24). Both were quadratic, and the
  blocking pool is finite and shared, so either one alone was enough to stop the proxy
  masking — hence serving — for everyone. Measured end-to-end: a **13.4 MiB** body of ~2 M
  small emails now masks in **1.8 s** (it took ~7 minutes), and eight of them concurrently
  finish in 4.3 s while `/healthz` answers in 48 ms.
- **Tolerant de-masking.** Restore accepts model-mangled placeholders
  (`[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`) in one pass; a placeholder that looks
  like ours but isn't in the vault is logged rather than silently shipped.
- **Response headers.** Only a safe allowlist of upstream response headers is
  forwarded (`retry-after`, `x-request-id`, `x-ratelimit-*`, `openai-*`,
  `anthropic-*`); content/hop-by-hop headers are dropped because the body is
  re-serialized after de-masking.
- **Detection confidence.** `PiiEntity` carries a `Confidence` (`Verified` vs
  `Structural`). A structure-only IBAN (mod-97 fails) is still masked but tagged
  `Structural`; the signal is available to audit logging now and ML thresholds in
  M2.
- **NER fail-closed is configurable (`NER_REQUIRED`).** Structured PII is always
  fail-closed. For the M2 NER layer: by default a missing/failing NER falls back to
  structured-only (fail *open* for names — an explicit `FailOpen` wrapper). Setting
  **`NER_REQUIRED`** makes it fail *closed*: a configured-but-unloadable model is
  fatal at startup (`build_detector`/`AppState::new` return `Result`), and a
  per-request inference error blocks the request (400) via the fallible
  `PiiDetector::try_detect` error channel — whose `DetectError` carries only a
  static label, never input text.

  > **The rule that governs it (M5-R7): a detector may degrade its own recall, but it may never
  > decide *for the caller* that degraded output is acceptable.** Fail-open vs fail-closed is
  > `FailOpen`'s decision, and the **only** road to it is the `try_detect` error channel. A detector
  > that quietly returns `Ok(partial)` where it could have returned `Err` has routed *around* the
  > switch the operator set — it silently converts `NER_REQUIRED` (block) into "forward a
  > partially-scanned body", which is the one thing that operator paid to prevent. This is not
  > hypothetical: the first M5-R2 fix did exactly that, clamping an over-long NER sequence and
  > returning `Ok`. The clamp was *better* for the default posture and *fatal* to the other one —
  > and a component that cannot see the posture must not choose between them. **When in doubt, a
  > detector returns `Err` and lets the wrapper decide.**

## Debug & observability (M2.6)

Opt-in developer tools to *see* that masking holds end-to-end. Both are **off by
default** and neither weakens the fail-closed posture — request-side masking always
runs, so the upstream never sees raw PII regardless.

- **`PII_DEBUG_SKIP_DEMASK`** (on `Config.debug_skip_demask`): skips the response
  de-mask so the (local) client receives the placeholders the provider saw — proof the
  round-trip is wired. A **loud `warn!`** fires at startup when it's on, so it can't
  quietly linger in a deployment.
- **`trace!` of the masked upstream body** (`RUST_LOG=…=trace`): the exact bytes sent
  upstream, logged just before forwarding — masked at that point, so safe. `debug!`
  keeps the concise kind-only audit lines.
- **Safety boundary (rule):** the masked request and the raw provider response
  (placeholders only) are safe to log; the **final de-masked client output (real
  values) is NEVER logged**. Same bar as the future audit logging.

## Streaming & multi-provider routing (M3)

**Streaming (SSE).** When a request sets `stream:true`, the proxy masks it exactly as
a buffered request, forwards it, and streams the response back while
**de-anonymizing incrementally** (`src/stream.rs`). A placeholder like `[EMAIL_1]`
can be split across two token deltas, so `SseDemasker` keeps a **hold-back buffer** per
streamed field, emitting everything up to the last point that could still be an incomplete
placeholder and holding the rest until the next delta (or stream end) resolves it. The
line-buffering and split-placeholder hold-back (`split_demaskable`) are **schema-agnostic**;
a per-`WireSchema` rewriter knows where the text lives in each wire format (M6): OpenAI's
`choices[].delta.content` + `delta.tool_calls[].function.arguments`, or Anthropic's
`content_block_delta` (`text_delta.text` plain, `input_json_delta.partial_json` JSON-aware),
held back per content-block `index`. An Anthropic block's tail is flushed at its
`content_block_stop` — injected **before** the whole stop frame (a `content_block_delta`
after the stop is protocol-invalid), which is why the demasker holds each `event:` line until
its `data:` line so it controls frame ordering; OpenAI streams have no `event:` lines, so the
mechanism is inert there. Robustness: if the upstream answers a `stream:true` request
with a **non-SSE** body (a JSON error, or a provider that ignored `stream`), the proxy
falls back to the buffered path (real status + content-type, `on_response` de-mask)
rather than forcing an event-stream; a **mid-stream upstream error** becomes a terminal
`event: error` (after flushing buffered content) instead of a broken connection.
Streaming **never weakens fail-closed**: request-side masking runs first, so the
provider only ever sees placeholders; a clean request (nothing masked) streams through
untouched.

**Multi-provider routing — Option A.** Every provider is reached through its
**OpenAI-compatible** endpoint, so a single schema feeds the masker (no new leak
surface). The request **body** is identical across the presets; only the HTTP **envelope**
differs, and that is all a preset touches. A `UPSTREAM_PROVIDER` preset (`openai` / `copilot` / `anthropic`) sets the
per-provider *shape* — the chat path (`upstream_chat_path`; Copilot drops `/v1`), the
allowlist of client request headers to pass through (`forward_request_headers`, e.g.
`anthropic-version` or editor headers), and any required static headers
(`upstream_extra_headers`) — each overridable by env. Base URL + API key stay
env-driven. Auth: the client's own `Authorization` wins, else the configured key as
`Bearer`. Anthropic's *native* `/v1/messages` schema is **not** served here — that is the
inbound work of **[M6](ROADMAP.md#m6)** (the rest of Option B stays Backlog); see *The
wire-format boundary* next.

## The wire-format boundary — who speaks what to whom

The single most confusing question about this proxy — *"which providers does it work
with?"* — turns on one axis. The default answer: **the proxy speaks the OpenAI Chat
Completions schema (`POST /v1/chat/completions`) on *both* hops** — Option A, one schema
feeds the masker, a single shape to get right and no translation layer to leak through.
**M6 adds one *inbound* exception:** it also accepts the **native Anthropic Messages
schema** (`POST /v1/messages`) and masks it **in place**, native→native — see *Native
Anthropic Messages* below.

```
          OpenAI Chat Completions                        OpenAI Chat Completions
  client ───────────────────────────►  PROXY  ───────────────────────────►  upstream provider
  (OpenAI-compatible caller)          mask / restore     (any endpoint speaking that schema)

          Anthropic /v1/messages  ┐                  ┌  Anthropic /v1/messages   (M6, when
  Claude Code ───────────────────►│      PROXY       │──────────────────────►    UPSTREAM_PROVIDER
  (native client)                 ┘  mask / restore  └                           = anthropic)
```

So *"which providers"* is really **two** independent questions, and conflating them is the
confusion:

- **Upstream — what the proxy forwards *to*.** Either the OpenAI Chat Completions schema
  (presets `openai`, `copilot`, and `anthropic`'s *OpenAI-compatible* endpoint, plus
  anything through `UPSTREAM_BASE_URL` — Ollama / vLLM / LM Studio, Groq, Mistral, …), or —
  on the M6 route — Anthropic's **native** `/v1/messages`. The preset sets the *shape*
  (path, forwarded headers); masking is identical across the OpenAI presets.
- **Client — what speaks *to* the proxy.** Any OpenAI-compatible client (the OpenAI SDK,
  `curl`, editor agents targeting `/v1/chat/completions` — Cline, Continue), **or** a
  **native Anthropic client** — Claude Code (CLI + IDE) and the Anthropic SDK — on the M6
  `/v1/messages` route.

**The exception that used to trip everyone up — now closed for Claude Code (M6).** *"works
with Anthropic"* (as an **upstream**) was always true; *"works with Claude Code"* (as a
**client**) was not, because Claude Code speaks Anthropic's **native** Messages API
(`POST /v1/messages`, content blocks, `tool_use`/`tool_result`), which the proxy did not
serve. **M6 serves it** — registered only when `UPSTREAM_PROVIDER=anthropic` (the only
upstream that speaks it), so on any other provider `/v1/messages` still 404s (fail-closed).

> **Why this framing is worth keeping in mind when planning work.** A feature request lands
> on exactly one of the two axes. *"Support provider X"* is usually **upstream** work — a
> preset, trivial when X is OpenAI-compatible. *"Support client Y"* is **inbound** work, and
> if Y speaks a native protocol it means **a new schema on the masking path** — the
> expensive, leak-sensitive kind (M6 did exactly this for Claude Code; the remainder of
> Option B — native adapters for *other* providers — stays Backlog). Name the axis first,
> and the size and risk of the change follow.

## Native Anthropic Messages (M6) — Claude Code passthrough

A **native** Anthropic client (Claude Code, the Anthropic SDK) speaks `POST /v1/messages`,
not the OpenAI schema. M6 serves it by masking the native body **in place** and forwarding
native→native — *not* translating to the OpenAI shape and back, which would add two lossy
schema boundaries, each a leak surface. The masking *engine* is unchanged (`Vault::mask_all`,
the fixpoint, `spawn_blocking` fail-closed); M6 adds only the schema walk, the native
forward/auth, and the Anthropic SSE rewriter. The route is registered **only when
`UPSTREAM_PROVIDER=anthropic`** (the only upstream that speaks it); everywhere else
`/v1/messages` 404s like any un-modelled endpoint.

**A [`WireSchema`](../src/pipeline/mod.rs) tag on `RequestContext`** (default `OpenAi`)
selects the walk. `PrivacyStage` and `SseDemasker` dispatch on it, so the OpenAI path is
entirely undisturbed — it gets the default.

**Request coverage (a missed field is a leak).** `mask_anthropic_request` walks:
- the top-level **`system`** (a string *or* a text-block array) — masked in place;
- each `messages[].content` block, dispatched on `type`: `text` → `text`; `tool_use` →
  every **string leaf** of `input` (a JSON *object* — the tool arguments, restored in the
  response, so a replay holds real PII); `tool_result` → its `content` (string or nested
  block array, recursive); `thinking` → `thinking` (see the signature note); a `document` →
  its `title` / `context` metadata **and** its `source`, dispatched on `source.type` the way
  blocks dispatch on `type` (`text` → `source.data`; `content` → a nested block array,
  recursed like a `tool_result`; `base64` / `url` → opaque binary or a fetch target, skipped;
  **any other source type → fail closed** — M6-R1); `image` / `redacted_thinking` → non-text
  or opaque, skipped;
- `tools[].description` + every `description` in `tools[].input_schema` (the shared
  `mask_schema_descriptions`).

**Unknown block type → fail closed (400).** The known set is exhaustive for real Claude Code
traffic and pinned by a guard test, so a new Anthropic block type is a *conscious* addition,
never a silent leak — and the **same rule applies one level down**, to a `document`'s
`source.type` (M6-R1). The block-type value is **never** echoed into the block reason (it is
attacker-influenced — the never-log-raw-PII rule). *Server-side result blocks*
(`server_tool_use` / `web_search_tool_result`, sent only with server-side tools enabled)
fail closed for now — safe, a conscious future addition (Backlog).

> **`thinking` is masked on the way up but never de-masked on the way down — and that keeps
> the block signature valid.** An extended-thinking block is generated by the model over
> *already-masked* input, so it naturally contains only placeholders, and its cryptographic
> `signature` signs that placeholder text. Leaving the placeholders intact means the block's
> bytes never change across a multi-turn replay (re-masking an inert placeholder is a no-op),
> so the signature stays valid — **robustly, even if placeholder numbering shifts elsewhere**
> in the conversation. De-masking `thinking` would either invalidate the signature or make
> correctness depend on reproducing byte-identical numbering. So the request walk *masks*
> `thinking` (safety, in case a client injects fresh PII) and the response walk deliberately
> *skips* it. This is why `demask_anthropic_response` restores only `text` and
> `tool_use.input`.

**Augmentation** goes into the top-level `system` field (created / string-appended /
block-array-pushed), only when something was masked — the native analogue of the OpenAI
system-message injection.

**Response demask (buffered + streamed) mirrors the request walk.** The buffered reply is a
top-level `content[]` array; `text` blocks and `tool_use.input` string leaves are restored.
`tool_use.input` is a real JSON object, so its leaves use the **plain** demask (serde
re-escapes on serialization) — unlike the OpenAI JSON-*encoded* `arguments` string, which
needs the JSON-aware demask (M3-R2).

**Auth: the client's credential wins, the proxy key is the fallback** (`Upstream::messages_auth`).
Order: client `Authorization` (Claude Code's OAuth `Bearer sk-ant-oat01-…`, forwarded
**verbatim**) → client `x-api-key` → the proxy's configured key as `x-api-key` → **401** if
none. **An OAuth token never lands in `x-api-key`** (Anthropic 401): it only ever rides
`Authorization`. `anthropic-version` (default `2023-06-01` when the client omits it) and
`anthropic-beta` pass through the allowlist. This is the same "client credential wins"
posture as the chat path, and it is what lets the proxy front Claude Code **without ever
holding a key**.

## Module layout

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | binary entry: tracing, config, run server |
| `src/config.rs` | runtime configuration |
| `src/server.rs` | axum router + handlers (`/v1/chat/completions`; `/v1/messages` when `provider=anthropic`, M6); shared mask / fail-closed / buffered / streaming flow |
| `src/proxy.rs` | request/response value objects + the upstream HTTP client (per-provider path/headers, raw + JSON send; native `send_messages` / `forward_messages` + `messages_auth`, M6) |
| `src/stream.rs` | streaming (SSE) incremental de-anonymizer — shared split-placeholder hold-back core + a per-`WireSchema` delta rewriter (OpenAI, M3; Anthropic, M6) |
| `src/pipeline/mod.rs` | `Stage` trait; `RequestContext`; `WireSchema` (OpenAI / Anthropic, M6) |
| `src/pipeline/privacy.rs` | the privacy stage (only one wired) — OpenAI **and** Anthropic-native (M6) mask/demask walks, dispatched on `WireSchema` |
| `src/pii/mod.rs` | `PiiDetector` trait, `PiiEntity` / `PiiKind` / `Confidence` |
| `src/pii/recognizers.rs` | deterministic structured-PII recognizers (M1) |
| `src/pii/overlap.rs` | shared span overlap resolution — the *no-abandoned-bytes* invariant: structured union-merge → NER drop (`PiiKind::priority` only *labels* a union; enclosure by an email is a **naming** rule, `name_of` — the containment *gate* was removed in M4-R15) |
| `src/pii/composite.rs` | `CompositeDetector` — combine detectors behind one trait |
| `src/pii/anonymizer.rs` | `Vault`: mask / demask |
| `src/pii/ner_decode.rs` | pure NER decode (label→kind, BIO→spans) — model-independent |
| `src/pii/onnx.rs` | ONNX NER detector (M2, feature `onnx`) — tokenizer + `ort` session pool |
| `src/pii/hf.rs` | HuggingFace Hub model resolution (M2.5, feature `onnx`) — opt-in revision-pinned fetch into the standard HF cache + `id2label` parse |

## Stack

tokio (async runtime) · axum + tower (HTTP + modular layers) · reqwest (upstream,
streaming) · serde / serde_json · regex + once_cell (recognizers) · `ort` (ONNX
Runtime, M2, feature `onnx`) · `tokenizers` (M2, feature `onnx`).

**Hybrid detection (M2).** `CompositeDetector` runs the deterministic recognizers
and — when the `onnx` feature is on and the model env vars are set — the
`OnnxNerDetector` over the same text, merging spans through `overlap`. NER config
is env-driven: `NER_MODEL_PATH`, `NER_TOKENIZER_PATH`, `NER_LABELS` (comma-separated
labels in class-id order), optional `NER_POOL_SIZE` (session pool for concurrency),
`NER_INTRA_THREADS` (per-session threads — M7, see below), `NER_TOKEN_TYPE_IDS`
(BERT-family models), and `NER_REQUIRED` (fail-closed switch).
A missing/failed model logs and falls back to structured-only. The model was chosen
by *measurement* (XLM-R int8 — `docs/M2-NER-EVALUATION.md`, `docs/DEVLOG.md`).

**GLiNER — contextual / open-label detection (M8, opt-in, off by default).** A *second* ML engine,
`GLiNerDetector`, joins the composite when `GLINER_MODEL_PATH` (+ `GLINER_TOKENIZER_PATH` /
`GLINER_CONFIG_PATH`) is set. Unlike the token-classification NER, GLiNER is a *zero-shot span
extractor*: the entity types are fed to it **as text** (`"person"`, `"phone number"`, `"address"`), it
scores every candidate **word-span × type**, and detection is a sigmoid threshold + greedy
non-overlapping selection (`gliner_decode`, the span×label analogue of BIO `ner_decode`; the model I/O
contract — GLiNER span-mode `markerV0`, six named inputs → `[1, num_words, max_width, num_types]` logits
— is documented and verified against the real export in `src/pii/gliner.rs`). **It is not a successor to
XLM-R:** measured on the shipped **int8** model its Person recall (~0.58) is below XLM-R's (~0.83), so it
does not replace the NER — it *adds* what XLM-R can't do, the contextual kinds the deterministic layer
can't anchor (a **bare national phone** with no `+CC`, a free-form address). It maps
`"phone number" → Phone` / `"address" → Location` on purpose (email stays deterministic). Its overlap
behaviour follows the kind, not the engine: a GLiNER **name** guess (`Person`/`Organization`/`Location`,
so also `address`) is an NER kind and is **dropped whole** when it overlaps a structured span (M2-R7),
while a GLiNER **`Phone`** is `is_structured()`, so `overlap` **union-merges** it with any overlapping
structured span rather than dropping it — the checksum-backed kind still *names* the union, and both
spans are masked. Either way a GLiNER false positive is an **over-mask, never a leak** — the standing
tie-breaker. `NER_REQUIRED` means "**≥1** ML detector (XLM-R and/or GLiNER) must load and run unwrapped".
Tunables: `GLINER_LABELS`, `GLINER_THRESHOLD` (default **0.15** — int8 confidences run low, set by a
measured sweep), `GLINER_POOL_SIZE`, `GLINER_INTRA_THREADS`. Explicit local paths only for now (the
airtight-privacy path). Decision + numbers: `docs/DEVLOG.md` 2026-07-19.

> **GLiNER tags our own placeholders — and it is safe anyway (M5-R4, "GLiNER especially").** Being
> zero-shot and context-driven, GLiNER *does* score `[PERSON_1]` as a person (int8 XLM-R does not — that
> is why the docs singled it out). This cannot stall the fixpoint: `keep_maskable` drops an exact
> `[KIND_N]` hit **by construction** (CC-08) and **S4** keeps GLiNER off every pass after the first
> (`redetect → empty`, idempotent — masking a name never reveals a new one), so `mask_all` on
> placeholder-dense text converges unchanged rather than 400ing. The `m5_r4`-style canary is
> `tests/gliner_eval.rs::gliner_placeholder_inertness_canary`.

**NER threading — the two knobs multiply (M7).** `NER_POOL_SIZE × NER_INTRA_THREADS` is the
process's NER thread count under saturated load, and **the invariant is that the product fits the
box**, not that either factor saturates it (`intra = 6` with `pool = 2` puts 12 ONNX threads on 6
cores — oversubscription, plausibly slower than one). So `NER_INTRA_THREADS` **defaults to a
derived value**, `max(1, base / NER_POOL_SIZE)` (`onnx::default_intra_threads`); a fixed constant is
wrong on a 2-core VM and a 64-core server alike. An explicit env value wins; a `0` is treated as
unset for **both** knobs, never as ONNX Runtime's "pick for me", which would reintroduce exactly the
oversubscription the derivation prevents. Both resolve in one place
(`onnx::resolve_pool_and_intra`), which the latency harness calls too — when the harness read its
own default it silently measured a configuration the server does not ship (M7-R1). That single home
is also why `GLINER_POOL_SIZE`/`GLINER_INTRA_THREADS` inherit every change here for free.

**The base is PHYSICAL cores, capped by the parallelism the platform grants (M11 Track B, decided
2026-09-02).**

```
base  = min(physical_cores, available_parallelism())
intra = max(1, base / NER_POOL_SIZE)
```

Through M10 the base was `available_parallelism()` — the **logical** count — so on the reference box
(6 cores / 12 threads) the shipped `NER_POOL_SIZE=1` derived `intra = 12`: both SMT siblings of every
core running the same int8 GEMM, contending for one core's L1d/L2 and one set of vector units.
Physical cores is the conventional intra-op base for GEMM-bound inference, and it is **one rule at
every pool size** — the sibling contention that motivates the cap does not weaken when a second
session appears, it applies to both of them. Dividing the logical count from `pool = 2` up would put
`2 × 6 = 12` ONNX threads back on 6 physical cores, reintroducing at `pool = 2` exactly what the
change removes at `pool = 1`, and making the process's total NER thread count *double* between
`pool = 1` and `pool = 2` under a formula whose whole purpose is that the product fits the box. The
cost is named and accepted: at `pool = 2` on a 6/12 box each session falls from 6 threads to 3,
leaving the siblings to the runtime's own work (tokio, TLS, JSON), which is latency-bound and *does*
profit from SMT unlike the GEMM.

> **`min(physical, available_parallelism())`, never `physical` alone — this is the trap.**
> `available_parallelism()` honours cgroup quota, CPU affinity masks and Windows job objects; a
> physical-core count does **not**, it reports the silicon. Take it bare and a proxy in a 2-CPU
> container on a 32-core host derives `intra = 32` — the oversubscription this derivation exists to
> prevent, arriving through the fix for it. The `min` also settles CPUs whose thread count is no
> longer 2× the core count: on a hybrid P+E part (14 cores / 20 threads) it returns 14, every core
> once; with SMT disabled in firmware the two counts are equal and it is a no-op. If the platform
> will not report a physical count at all, the base is the logical one — **exactly the pre-M11
> behaviour**, so a platform that cannot answer loses nothing.

> **This is settled by decision and mechanism, NOT by this box's timings — and saying so is the
> point.** [M7-R2](reviews/M7.md#m7-r2) recorded the SMT question as **unresolved** after four runs
> whose sign flipped under a ~40% same-configuration spread, and M11 does not claim to have resolved
> it: it adopts the conventional base and stops paying for a knob no measurement on this hardware can
> read. The M11 sweep *records* the change, it does not justify it. What that sweep did measure, and
> what is worth knowing: single-request latency is a wash at the default (`1×6` 2.48× vs `1×12`
> 2.42× over the pre-M7 shape, inside the noise), while **throughput improved on both shapes** —
> `1×6` 0.485 turns/s vs `1×12` 0.419 (**+16%**) and `2×3` 0.664 vs `2×6` 0.558 (**+19%**), the new
> centralized shape being the fastest row measured. `NER_INTRA_THREADS` remains an explicit override
> and still wins, so the old shape is one env var away.

> **The startup log prints the base and where it came from, and that is not decoration (M7-R5).**
> The line carries `thread_base`, `thread_base_source` (`physical` / `parallelism-cap` /
> `physical-unknown`), `logical_cores` and `physical_cores` beside `pool_size` and `intra_threads`.
> M7-R5 rejected `pool_size=0, intra_threads=12` because no arithmetic reconciles it; since the base
> stopped being the core count an operator's task manager shows, a bare `intra_threads` became just
> as unreconcilable — on the reference box it now reads 6 where the machine advertises 12. With those
> fields the operator can redo `intra = max(1, base / pool)` from the line itself.

> **State the invariant with its domain, because it is false outside it (M7-R4).** The derivation
> bounds `pool × intra` by the **base** while `pool ≤ base`. Beyond that it *cannot*: `intra`
> floors at 1 and nothing clamps `NER_POOL_SIZE`, so `NER_POOL_SIZE=8` on a 2-core box is 8 threads
> on 2 cores and no choice of `intra` fixes it. **That is an operator error the proxy does not
> defend against** — it hits the ~270 MB-per-session RAM wall long before the thread wall. An
> invariant asserted unconditionally but true only in one regime is worse than a bounded one: the
> first version of THREAD-01 wrote the exception into a `cores.max(pool)` term, which green-lit
> exactly that case under a test name claiming the opposite.

**A single request occupies one session, so the product is the *saturated-load* count — not what a
lone request gets.** The masking path is sequential at three nested levels: the field walk holds
`&mut Vault`, `infer_chunked` loops its windows, and only then does the session run. A lone request
therefore reaches `intra`, never `pool × intra`, and the pool is **inert at concurrency 1**
(measured: `2×1` ≈ `1×1`). The right shape is a **deployment** question the proxy cannot answer for
itself — but it *can* default to the case almost everyone runs. **The shipped default is
`NER_POOL_SIZE=1` (flipped from 2 on 2026-07-17):** a personal proxy in front of Claude Code
(concurrency ≈ 1) gets every physical core on its one request (every *logical* one before M11
Track B) and ~270 MB less RAM, since each session holds
its own copy of the weights — **measured: 563 MB at `pool=1` vs 834 MB at `pool=2`** (a ~290 MB
shared base plus ~270 MB per session, so `pool=N` ≈ 290 + N×270 MB — not a clean doubling). A
**centralizing** operator serving concurrent clients sets `NER_POOL_SIZE=N` for the pooled shape. **The flip is not free, and the cost is named:** `pool=1`
measured **−23% throughput** under concurrent load — intra-op scaling is sublinear (12 threads buy
~2.2×, not 12×), so independent sessions aggregate better — but that cost lands only on concurrency
the default's target does not have, while the RAM it saves is certain. This is the `low-RAM` bar in
`CLAUDE.md` applied to the dominant deployment; a constant would be wrong on a 2-core VM and a
64-core server alike, which is why it stays overridable. Numbers: DEVLOG 2026-07-16; the flip: DEVLOG
2026-07-17.

> **Parallelize *detection*, never *minting*.** Chunk-level fan-out would be safe — windows are
> read-only w.r.t. the `Vault` and `infer_chunked` already merges them deterministically. The
> **field** walk is not, and `&mut Vault` is not merely why it's hard, it's why it's *wrong*:
> placeholder numbering follows encounter order, so racing two fields makes `[EMAIL_1]` vs
> `[EMAIL_2]` a coin flip and breaks the determinism M1 Part B pins.

> **`NER_INTRA_THREADS` changes performance only — and that it does not change *detection* is
> EMPIRICAL, not guaranteed (M7-R3).** Verified on XLM-R int8 + the CPU EP: byte-identical entity
> sets, spans included, at intra 1…12 (`ner_perf.rs::m7_r3_intra_threads_changes_speed_not_detection`).
> Nothing in ONNX Runtime promises it. Intra-op parallelism repartitions GEMM and reduction work
> across threads; floating-point addition is **not associative**, so a different partition can move
> a logit in its last bits — and the BIO decode is a per-token `argmax`, where a near-tie flips on
> nothing but thread count. A flipped `B-PER` is lost recall, and here a miss is what makes a leak.
> **A model swap or an execution-provider swap must re-run that guard** — the M9 DirectML/CUDA
> work is exactly where cross-thread non-determinism stops being theoretical (GPU floating-point
> diverges from CPU further than thread count does; see *Execution providers* below). This is
> [M5-R4](reviews/M5.md#m5-r4)'s rule about the same layer: *the NER's convenient properties are
> measured, never proved.*

**The NER over-masks sub-word fragments of organization names — accepted (M7).** Measured: the
hybrid tags `"An"` — two characters of `"Anthropic's"` — as an `Organization` in ordinary
instruction prose, so a Claude Code system prompt reaches the model as `"[ORG_1]thropic's"`. It is
**not a leak** (it fails *toward* masking) and it is the same accepted class as
[M4-R6](reviews/M4.md#m4-r6): privacy beats precision, and a precision fix needs its own recall
argument. Two things make it worth writing down rather than filing under M4-R6 and forgetting:

- **The mechanism is different, so the fix is too.** M4-R6 is a *deterministic recognizer*
  over-matching pure-numeric IDs; this is the *NER* emitting a fragment of a word it half-recognized.
  M4-R6's path out is context (GLiNER); this one's is span quality — the one of the two a **model**
  change could actually fix.
- **It corrupts boilerplate, on every turn, and it has a price we already measured.** The
  augmentation prompt then tells the model `[ORG_1]` is a real value to use verbatim. And because
  the field is no longer clean, it costs a **second fixpoint pass** — ~940 ms of a 4.2 s turn, which
  is the concrete link between this precision bug and the S4 fixpoint lead.

Nobody had seen it until M7 ran the NER over a realistic system prompt — the S0 lesson (*a corpus
has a shape, and the shape is the blind spot*) one level down.

**Execution providers — hardware acceleration (M9).** The NER and GLiNER sessions run on an
`ort` **execution provider (EP)** selected at runtime by `NER_EXECUTION_PROVIDER` (both models
share the one knob). `cpu` is the default and the only provider the CPU-first design depends on;
the rest are **opt-in accelerators**. Which ones a binary *has* is decided by the linked ONNX
Runtime distribution: each platform's natural accelerator is wired **per-target** in `Cargo.toml`
(table below), so `--features onnx` already carries it — the `ep-*` features are escape hatches
for non-default combinations, not the normal path. The selection + fallback policy has **one
home**, `onnx::build_session_pool` (built on `build_session_reporting`) — both detectors build
their session pools through it, so the behavior cannot drift between them, and the pool it returns
is homogeneous with a single known-effective provider (the `resolve_pool_and_intra` discipline,
applied to the backend).

**Falling back to CPU is the fail-closed move, not a compromise.** A requested accelerator that
cannot initialize — not present in the linked distribution, driver/device missing, registration
error — drops to CPU with a `warn!`, because *a privacy proxy must start*, and **CPU is the
reference implementation**: falling *back to* it costs latency, never masking quality. Note the
direction — the claim is not that every backend computes identically (it isn't, see the note
below); it is that the one we fall back *to* is the one everything else is measured against.
That asymmetry is what makes the fallback safe under `NER_REQUIRED` too: that knob is a
*detection-posture* knob ("the ML layer must be present and must not silently degrade"), and
dropping to the reference implementation degrades nothing it is about. Making it fatal instead
would let a driver update take down a proxy that is still masking perfectly. The dispatch is registered with `.error_on_failure()`
precisely so this fallback is **explicit and logged**, never ORT's silent per-session CPU drop
that would hide which backend actually ran. Two failures are kept distinct: a **typo** in
`NER_EXECUTION_PROVIDER` (e.g. `vulkan`, which is *not* an ORT backend) is a config error that
**fails startup**; a **known** provider that won't initialize **falls back**. Conflating them —
silently running CPU when an operator named a real-but-absent GPU — is the move `onnx::ExecutionProvider::parse`
refuses (M8-R5's rule, applied to the accelerator knob).

> **The fallback covers *initialization*, not *numerical correctness*, and not the session's later
> life — and that bound is what makes an untested EP acceptable.** Three things it does *not* catch:
>
> 1. **Numerical divergence.** An accelerator that registers but computes subtly different logits
>    than the CPU is not detected.
> 2. **Per-node partitioning.** ORT assigns nodes per-EP *after* registration and force-falls-back
>    whatever the EP handles poorly (observed on DirectML: 8+ `Force fallback to CPU execution for
>    node` lines plus ORT's own "some nodes were not assigned to the preferred execution providers"
>    warning). So a registered EP is not a promise the whole graph ran on it — see the reporting
>    caveat below.
> 3. **Mid-life failure.** `build_session_pool` runs once, at load. A provider that dies afterwards
>    (a GPU device-removed/TDR event is routine on Windows in a way "the CPU stopped working" is
>    not) surfaces as an ordinary `session.run` error and follows the existing posture: `FailOpen`
>    swallows it to structured-only under the default, `NER_REQUIRED` 400s. **There is no
>    re-initialization on CPU** — the process keeps a dead accelerator for its lifetime. That is
>    pre-existing, deliberate (M2-R1/R2) and not a leak, but M9 raises its *probability*, so it is
>    stated here rather than left as a surprise.
>
> In every case the blast radius is bounded to **NER recall**: the deterministic structured
> recognizers (email, phone, SSN, credit card, IBAN) run on CPU regex, independent of the EP, so
> **the fail-closed layer is untouched no matter what backend the NER runs on**. An untested EP can
> therefore only (a) fail to load → CPU, or (b) degrade the best-effort NER layer — never leak
> through the deterministic layer, never crash. This is why the tested-vs-untested split below is a
> *safe* trade, and why even the benchmarked one still owes the cross-thread-determinism guard
> above: **an execution-provider swap must re-run it** before the backend is trusted, because GPU
> floating-point diverges from CPU further than thread count does.

Every EP *type* compiles on every platform (only its `register()` is feature-gated inside `ort`),
so the selector needs no per-provider `#[cfg]` — an unavailable provider simply fails
registration and falls back.

> **ONE ONNX Runtime distribution, ONE set of providers — "compile in every backend" is not a
> thing.** `download-binaries` fetches a single ORT distribution and *that* decides which EPs
> exist; a cargo feature only decides whether our `register()` is compiled. **Measured**:
> enabling six `ep-*` features at once on Windows x64 builds and links fine, but the binary is
> still the DirectML distribution — at runtime `cuda` / `tensorrt` / `coreml` / `rocm` /
> `openvino` all report unavailable and fall back to CPU. Cross-platform it is impossible by construction anyway: CoreML exists
> only in macOS builds, DirectML only in Windows ones. So the only meaningful choice is **one
> natural accelerator per platform**, set as a per-target `ort` feature in `Cargo.toml` rather
> than a flag on the build command — which also means a developer's `--features onnx` build and
> the release pipeline cannot disagree about the accelerator.

**Consequently the provider list is asked of the *runtime*, not derived from cargo features**
(`bench::available_providers` → `ort::ep::ExecutionProvider::is_available`). A feature-derived
list is wrong in both directions: it would have shown five GPU rows that were really CPU in the
six-feature experiment above, and it would *miss* the per-target accelerator, which is present
without `ep-directml` ever being named. Asking the binary what it contains cannot lie either way.

**"Measured" and "trusted" are different columns, deliberately.** Only `cpu` has passed the
cross-thread determinism guard (`m7_r3_intra_threads_changes_speed_not_detection`); **no
accelerator has been run against it at all.** DirectML has been *benchmarked* — we know how fast
it is — which is not the same as knowing its logits agree with the CPU's. A single ✅ for both
would say the opposite of the paragraph above, and a future builder scans the table, not the prose.

| Provider (`NER_EXECUTION_PROVIDER`) | How it gets in | Natural OS / hardware | Status |
|---|---|---|---|
| `cpu` (default) | always | any | ✅ **trusted** — the reference implementation; the only provider that has passed the determinism guard |
| `directml` | **automatic** on Windows, both arches (per-target `ort` feature — it is *free*, see below) | Windows, any DX12 GPU (AMD/NVIDIA/Intel/iGPU) | 📊 **benchmarked, not trusted** — measured on this box's AMD DX12 iGPU (a NO-GO there); determinism guard **not** re-run |
| `coreml` | **automatic** on macOS (per-target — also free) | macOS, Apple Silicon / AMD | ⚠️ **wired, unverified** — needs a Mac |
| `cuda` | `ep-cuda` → its **own release binary** (`-cuda`), Windows x64 + Linux x64 | Linux/Windows, NVIDIA | ⚠️ **wired, unverified** — no NVIDIA hardware here. No `cu12` prebuilt exists for either arm64 target, so no artifact is offered there |
| `webgpu` | `ep-webgpu` → its **own release binary** (`-webgpu`), Windows x64 + Linux x64 + macOS arm64 | any (Vulkan/Metal/D3D12 via Dawn) | ⚠️ **wired, unverified** — obtainable, unlike ROCm/OpenVINO |
| `tensorrt` | `ep-tensorrt` | Linux/Windows, NVIDIA | ⚠️ **wired, unverified** |
| `rocm` | `ep-rocm` | Linux, AMD | ❌ **not obtainable via `download-binaries`** — `ort-sys`'s `dist.txt` has no ROCm row for any target, and on Linux the feature combines into a key (`cu12,rocm`) that matches nothing, silently falling back to the plain distribution: **strictly fewer providers than plain `--features onnx`**. Needs a from-source ORT |
| `openvino` | `ep-openvino` | Linux/Windows, Intel | ❌ **not obtainable via `download-binaries`** — not a distribution key, so the feature compiles `register()` against a runtime that isn't in the tarball |

> **The rule that governs every future wiring (M9-R13).** Adding an EP feature per-target is free
> **only when `ort-sys` does not key its *distribution* on it.** `resolve_dist` builds the download
> key from `training`, `webgpu`, `cuda|tensorrt`, `nvrtx`, `rocm` — and nothing else. Outside that
> list (DirectML, CoreML) the feature changes only which `register()` compiles, so it cannot break
> or even change a build; *inside* it, the feature swaps the downloaded tarball for every build on
> that platform, and a combination with **no row in `dist.txt` silently falls back** to the plain
> distribution — green build, absent accelerator. That asymmetry is why Windows and macOS could be
> wired without a CI run while Linux had to be scoped to `x86_64`. **Check `resolve_dist` before
> wiring a new one.**
>
> **Corollary, learned by getting it wrong (M9-R22): a result measured with several `ep-*`
> features enabled says nothing about any one of them.** `resolve_dist` keys on the *combination*,
> so a seven-feature build resolves to a key like `wgpu,cu12,rocm` — which matches no row, silently
> falls back to the plain distribution, and then fails to link because `static_link` still emits
> the WebGPU link directive from `cfg!(feature = "webgpu")`. That experiment produced the confident
> and **false** conclusion "`ep-webgpu` does not link on Windows"; `dist.txt` in fact carries `wgpu`
> rows for Windows x64, Linux x64 and macOS arm64, so `ep-webgpu` *alone* resolves fine. **Measure
> one feature at a time, or measure nothing.**

> **And the process rule M9 paid four review rounds to learn (M9-R21/R24):** when a claim is found
> wrong, **fix every site that makes it, not the site the finding cited.** Grep the *claim* — and
> the *category* it belongs to, not just the exact words. M9 corrected a claim in one file five
> separate times while the same sentence stood in three others, twice leaving two documents giving
> **opposite verdicts on the same feature**. A finding that hands over a `file:line` list is naming
> where the reviewer stopped looking, not where the claim stops.

Nothing in the table above `cpu` should be read as "safe to trust blindly": they are safe to *try*
(the fallback and the bounded blast radius above see to that), and `--bench-providers` tells you
whether trying is worth it on your box. Trusting one for masking quality means re-running the
determinism guard under it first.

**Every release target uses a *prebuilt* ONNX Runtime** (`download-binaries`); none builds it from
source. So THE RULE above does more than warn — it **partitions the accelerators**, and the release
shape follows from that partition (M9.1):

- **Free** (not a distribution key): DirectML, CoreML. Enabling them cannot change which tarball is
  fetched — they are already *inside* their platform's plain distribution. So they are wired
  **per-target** in `Cargo.toml` and every standard binary has them, at zero cost and with no CI run
  needed to prove it.
- **Key-ed** (changes the download): CUDA, WebGPU, TensorRT, nvrtx. Wiring one per-target would
  impose a different, heavier runtime on **every** user of that platform — which is what an earlier
  version of this section did with CUDA on x86_64 Linux, handing a CUDA-laden runtime to every Linux
  user with or without an NVIDIA device. A key-ed accelerator therefore belongs in **its own release
  artifact**: `release-build.yml` builds one binary per backend, and the operator downloads the one
  their machine can run (the README carries that table).

Which pairs exist is not a judgement call — it is `dist.txt`. There is no CUDA or WebGPU prebuilt for
either arm64 target, and none at all for ROCm or OpenVINO, so no artifact is offered for those and
the matrix says nothing speculative. **Only DirectML has actually been run** — on this project's
box — which is why it is the only non-CPU row marked *benchmarked* rather than *unverified*.

> **What "unverified" means for the Linux/CUDA row, concretely — the pre-tag check (M9-R13).** Two
> things about that row are *unknown*, not *believed*, and neither can be settled from this project's
> hardware (no NVIDIA device, no Linux runner):
>
> 1. **Does the `+cu12` distribution actually get fetched?** `cuda` is a distribution key, so an
>    `x86_64` Linux build should download `x86_64-unknown-linux-gnu+cu12` rather than the plain
>    tarball. A build that resolved to the plain one still compiles green — that silence is the whole
>    hazard the per-target rule above exists to name.
> 2. **Does CUDA survive packaging?** `release-build.yml` packages a **single file**
>    (`target/<triple>/release/<bin>`), while `ort`'s `copy-dylibs` places sidecar shared objects next
>    to the *build* output. Whether `NER_EXECUTION_PROVIDER=cuda` works from the **packaged artifact**
>    has never been observed. This project deliberately does **not** claim the CUDA runtime is
>    dynamically loaded — the claim was removed rather than restated, because it was never tested here.
>
> The check is cheap and already exists: on the manual release build, confirm `+cu12` in the build log,
> then run `--bench-providers` **on the downloaded artifact** (not the build tree) and confirm a `cuda`
> row. On `aarch64-unknown-linux-gnu`, expect **no** `cuda` row — it is not wired, by design. Until
> someone does this, the row stays ⚠️ and the docs must not promise more.

**On "the most compatible backend".** ONNX Runtime has **no Vulkan EP** — Vulkan is a
vendor-agnostic GPU API in general, but not one of ORT's backends, so using it would mean leaving
ORT for a different engine. The ORT-native cross-vendor answers are: **DirectML** on Windows (D3D12,
so *any* vendor's GPU, which is why it is this box's pick), and **WebGPU** everywhere (it sits on
Vulkan/Metal/D3D12 under the hood) — the latter still experimental, hence untested. There is no
single native GPU EP that spans all OSes; the universal path is, and remains, **CPU**.

**Measured on this project's box: CPU-int8 wins, so it stays the default.** DirectML-fp16 vs the
shipped CPU-int8 on the AMD DX12 iGPU came out **1.45× at seq 128, 0.45× at seq 256, 0.38× at seq
512** (DEVLOG 2026-07-19). The GPU wins only where latency is already invisible and loses ~2.6×
where it is not: fields are chunked to `MAX_WINDOW_TOKENS` = 480, so the latency-dominant
inferences run at seq ~480–512. A shared-memory iGPU is bandwidth-bound and attention grows
quadratically with sequence — 12 CPU threads on int8 beat it. **This is a fact about this iGPU, not
about GPUs**: a discrete GPU has 10–20× the bandwidth and would very likely win, which is precisely
why the selector exists even though the default did not move.

> **Quantization and backend are COUPLED, and measuring one while holding the other fixed is how
> M9 produced a confident wrong answer.** The CPU's format is int8; a GPU's is fp16 (on CPU, ORT
> up-casts fp16→fp32, so fp16 only pays off on a GPU). The first measurement ran the *shipped int8
> model* on DirectML, saw 2–5× slower, and looked like a verdict — it was a **false negative**: int8
> ops partition badly onto GPU providers. Re-run at fp16, the same GPU went from 5× slower to 1.45×
> faster at short sequences. Any future "is backend X worth it?" must vary **both** axes.

**`--bench-providers` — ship the measurement, not the number (`pii::bench`).** Because the answer
above is hardware-specific, publishing it as guidance would mislead every operator whose box
differs. The binary therefore measures the **model × provider matrix** on the machine running it
and names the winner at seq 512. The configured NER model plus `NER_BENCH_MODELS` (extra variants,
comma-separated) form the matrix, so an operator can compare CPU-int8 against GPU-fp16 — the
comparison that actually decides. Three properties make the report trustworthy rather than merely
informative:

- **A fallback is reported, never counted — at *session* granularity.** `build_session_reporting`
  returns the *effective* provider, so a row whose accelerator did not register prints
  `unavailable — fell back to cpu` instead of quietly presenting CPU timings under a GPU's name.
  **The contract covers registration, not execution:** ORT may still place individual nodes on the
  CPU inside a registered EP, so `ok` means *"the EP was registered, and these are the timings you
  will actually get"* — **not** "the whole graph ran on the accelerator". The numbers stay honest
  either way (partitioning is included in what an operator would really experience); it is the
  *guarantee* that is narrower than it looks. This is also what makes the quantization lesson
  mechanical rather than folklore: int8-on-GPU is slow **because** its ops partition back to CPU.
- **It refuses to let the int8 trap repeat.** The report always carries the quantization warning,
  and flags loudly when a run contains int8 but no fp16 — i.e. when it *cannot* answer "is the GPU
  worth it?".
- **It warns about the measurement itself.** The same box measured ~3× slower immediately after a
  long compile; the ranking survived, the absolute numbers did not. So the report says to measure
  idle and on AC.

It behaves identically in every build, and **never advises a rebuild**: the platform's accelerator
is already wired per-target, so when no accelerator was measured the report explains *this
machine's* situation (device/driver missing, or — on arm64 Linux — that none is wired for that
architecture) rather than naming a cargo feature the operator already has. Without the `onnx`
feature it still runs and explains that there is no ML layer to accelerate rather than failing.
Two guards pin that, and the split matters: **`BENCH-01`** drives `format_report` directly with a
CPU-only result set, because it is the only way to reach the no-accelerator branch on a machine
whose platform accelerator *is* present; **`CLI-03`** spawns the real binary and covers the
`cfg(not(onnx))` message end-to-end, which no unit test in `bench` can reach (that module is
`onnx`-gated). Neither alone is sufficient — assuming otherwise is what let this defect ship twice.

**NER chunking (M5, PERF-01).** `OnnxNerDetector` tokenizes a field once; if it fits, it runs
exactly as before (M2). A field that doesn't is split into overlapping token windows
(`chunk_char_ranges`, a pure function unit-tested without a model), each **re-tokenized
independently** — a middle window needs its own `<s>…</s>` framing, so it can't be a raw slice
of the whole field's token ids — and run through the same single-window path; results are
merged, sorted, and exact duplicates from the overlap region deduped. Without this, an
oversized field didn't just run slowly: it made the ONNX call **error outright** (measured —
see *Decisions & open points* above), silently downgrading NER coverage by default or
**blocking every such request** under `NER_REQUIRED`. Chunking is a **recall** mechanism only,
never leak-relevant: structured PII is detected independently, over the whole field, and is
never chunked.

**Two constants, and the difference between them is the whole point (M5-R2).**

| constant | bounds | value |
|---|---|---|
| `MAX_WINDOW_TOKENS` | the **planning window** — how much of the *field's* tokenization one chunk covers | 480 |
| `MODEL_MAX_TOKENS` | the **model's usable sequence length** — what may actually be handed to the session | 512 |

They are not two names for one budget. A window is planned in the coordinates of the *whole
field's* tokenization, but then **re-tokenized from its own text**, which adds the two special
tokens and drifts at the cut edges — so the sequence that reaches the model is `window +
specials + drift`, **measured at 481–483, i.e. always over the planning bound**. `MODEL_MAX_TOKENS`
is 512 because XLM-R declares `max_position_embeddings: 514` but RoBERTa-family position ids
start at `pad_token_id + 1 = 2`. The 32-token gap is the drift headroom, and **the headroom
itself** — not the mere ordering — is the **compile-time invariant** (M5-R10):

```rust
const MIN_DRIFT_HEADROOM_TOKENS: usize = 16;
const _: () = assert!(MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS);
```

Get it wrong and the crate does not build — including a window *over* the ceiling, which underflows
the const subtraction. This is the constraint the chunker actually relies on: `479 < 512` and
`511 < 512` satisfy `<` identically, yet a 511-token window re-tokenizes to ~514 and the `Expand`
error is back. The drift has "nowhere to go" the moment the headroom drops below it, not the moment
the window reaches the ceiling — so it is the **headroom** that must be pinned.

> **This is what two earlier wordings got wrong, and the pair is worth keeping as a warning.**
> First (M5-R2) this section said the window was *"conservatively under `max_position_embeddings`"* —
> a single budget, assumed safe; the window *was* under the limit and the **sequence was not**, on
> every chunk. Then the fix's own guard asserted only `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` (M5-R10)
> — true at any headroom ≥ 1, so it approved a 511-token window that overflows. **A bound you do not
> check is not a bound — and a compile-time invariant must encode the constraint the code relies on,
> not a weaker one that happens to hold at today's values.** `A < B` is not "A leaves room for
> drift"; and when the invariant is the *only* guard a modelless CI can run, the gap between those
> two is the whole exposure.

**The ceiling is checked at one choke point, and overflow is an `Err`, not a clamp (M5-R7).**
`run_and_decode` is the only path into the ONNX session (the direct call *and* every chunk), and
it **rejects** an over-long sequence rather than truncating it. An earlier revision clamped and
returned `Ok(partial)` — losing a window's tail instead of the whole field, which sounds strictly
better and is the wrong call: *whether a degraded NER is acceptable is a **posture** decision*,
and this codebase already has exactly one owner for it — the `FailOpen` wrapper and the
`try_detect` error channel (M2-R1/R2). Fail-open (default) swallows the error and proceeds
structured-only; **`NER_REQUIRED` unwraps the detector, so the error blocks the request (400)**,
which is what that operator asked for. Clamping returned `Ok` in *both* postures, quietly
forwarding a partially-scanned field to someone who had explicitly demanded a block — the failure
**relocated**, not closed. (The general rule this taught is in *Robustness & fail-closed* above.)
The error path is latent by design; `tests/ner_perf.rs`
(`m5_r2_…within_the_models_usable_length`) is the guard that keeps it that way.

**The one assumption the chunker rests on, stated (M5-R8).** `chunk_char_ranges` reads the
tokenizer's per-token offsets, and it takes the `(0, 0)` offset — the sentinel the tokenizer emits
for a **special token** — to mean *"this is `</s>`, i.e. the sequence end"*, which is why a window
reaching the last token uses `input.len()` rather than that offset (getting this wrong silently
dropped the whole final window; see M5-R2's history). That is sound **only because `(0, 0)`
appears exclusively at the sequence *ends***. A mid-sequence `(0, 0)` would either collapse a
window to zero length (silently unscanned) or restart it at byte 0. Verified against the real
XLM-R tokenizer over 17 adversarial inputs — CJK, combining marks, zalgo, emoji/ZWJ, zero-width
and control characters, 20 K-char single tokens, base64 runs, literal `<s>`/`<unk>` text: zero
mid-sequence sentinels. **A tokenizer that emitted one would break chunking, so a tokenizer swap
must re-check it** — the same class of model-swap checkpoint as placeholder inertness above.

> **Boundary fragments are cleaned up by the *resolver*, not by the `dedup()` — and that is
> load-bearing.** `infer_chunked`'s `dedup()` removes only **exact** duplicates. A window that
> cuts an entity in half emits a *truncated* one (`Mil` where the neighbouring window sees
> `Milan`); that is a different span, so it survives the dedup. What removes it is
> `overlap::resolve_overlaps`' NER phase (see *Overlap resolution* above): all three NER kinds share
> `PiiKind::priority() == 0`, so the phase tiebreaks on **span length, descending** — it takes the
> whole entity first and drops the overlapping fragment. **So the correctness of chunking depends
> on the NER kinds staying at equal priority.** Rank `Person` above `Location` and a truncated
> boundary fragment could outrank the entity it was cut from. (The window/stride arithmetic bounds
> how often this even arises: 480-token windows on a 448-token stride, so any entity ≤ 32 tokens is
> whole in at least one window. A longer one split across both windows is a *recall* miss — the
> OVL-02 / M2-R7 class, accepted for the best-effort NER layer.)

**Model management (M2.5, feature `onnx`).** The model file is resolved in priority
order (`src/pii/hf.rs` + `server.rs::load_onnx_ner`):

1. **Explicit local** — `NER_MODEL_PATH` (+ `NER_TOKENIZER_PATH` + `NER_LABELS`). Zero
   outbound calls; the airtight-privacy path, and it always wins.
2. **Opt-in auto-download** — when `NER_MODEL_PATH` is unset but `NER_MODEL_REPO`
   (`owner/name`) is set, the `hf-hub` crate fetches a **revision-pinned** model into
   the *standard* HF cache (`<home>/.cache/huggingface/hub`, library-managed, deduped
   across tools). Tunables: `NER_MODEL_REVISION` (default `478a2a3`), `NER_MODEL_FILE`
   (default `onnx/model_quantized.onnx`), `NER_TOKENIZER_FILE`, `NER_CONFIG_FILE`;
   `NER_LABELS` is derived from the model's `config.json` `id2label` unless set.

**Privacy note.** The auto-download is **opt-in** (only when `NER_MODEL_REPO` is set)
and fetches **model artifacts, not user data** — the one outbound call in the whole
tool, made once at startup and logged (repo + revision, never any input). It honors
`HF_HOME` / `HF_HUB_CACHE`; with neither set it pins the conventional cache instead of
`hf-hub` 1.0's `/tmp` fallback on Windows. Nothing user-supplied ever leaves the box.

**Toolchain:** Rust with the **MSVC** target on Windows. On a machine without
admin rights, install rustup per-user and the MSVC linker via portable Build Tools
— full procedure in `docs/SETUP.md`. MSVC is required to link the `ort` / ONNX
Runtime native library at M2.

## Supply-chain & dependency security

A privacy proxy inherits the vulnerabilities of everything it links, so the dependency surface is scanned
automatically — not by hope. Two independent layers, both free on a public repo, deliberately kept **separate
from the build CI** (this is cheap and needs its own cadence):

- **`cargo-deny`** (`.github/workflows/security.yml` + `deny.toml`) runs **`check advisories bans sources`**
  against the **RustSec** advisory DB — on PR / push that touch the dependency manifests, on a **weekly
  schedule** (the important trigger: a CVE can be disclosed against a dependency you *already* have, with no
  code change to fire a run), and on demand. `advisories` is the security core; `sources` pins crates to
  crates.io (an unknown git/registry host is the shape a supply-chain attack takes — fail closed); `bans`
  flags duplicate versions. It reads `Cargo.lock` (no compile), so it stays fast. The action's default cargo
  (1.71) can't parse this tree — it pulls crates needing `edition2024` — so the workflow sets
  `rust-version: stable` (cargo-deny needs a cargo new enough to read the graph, ≥ 1.85, not the project MSRV).
  - **`licenses` is off the gating command on purpose.** It is *compliance*, not *security*, and noisy against
    the `onnx` crypto stack (`ring` / `aws-lc-sys` carry non-SPDX license refs). A ready-to-enable allow-list
    sits commented in `deny.toml` (this crate is AGPL-3.0-or-later) for when compliance — not vulnerability —
    is the actual question.
- **Dependabot** (`.github/dependabot.yml`) keeps the `cargo` and `github-actions` ecosystems patched (grouped
  weekly to limit PR noise). Its vulnerability-driven side — **alerts + security updates**, from the GitHub
  Advisory DB — is a separate repo toggle the maintainer enables in *Settings → Code security & analysis*.

GitHub's native **code scanning (CodeQL — Rust is GA since 2025-10), secret scanning + push protection** are
the maintainer's one-click complement in that same Settings pane; also free for public repos. cargo-deny and
Dependabot overlap on advisories but do not duplicate: cargo-deny keys on the Rust-native **RustSec** DB and
adds source/ban policy GitHub does not, while Dependabot keys on the **GitHub Advisory** DB and opens the fix PRs.

**Workflow tokens run least-privilege.** Every Actions workflow declares an explicit `permissions:` block —
read-only by default, with `contents: write` granted *only* to the release **publish** job — so the
`GITHUB_TOKEN` never inherits the repo's (possibly read-write) default. This is CodeQL's
`actions/missing-workflow-permissions` closed as a standing rule rather than a one-off.

**PRs are gated for correctness, too.** `ci.yml` runs fmt / clippy / test (both `default` and `--features
onnx`) plus an MSRV `cargo check` on every push and PR — lightweight, one platform, **no cross-compile** — so
a dependency bump (Dependabot's especially) that fails to build or breaks a test cannot be merged green. The
all-target release build stays tag/manual-only (`release-build.yml`). This is why Dependabot version updates
are safe to run: the gate, not vigilance, catches a breaking bump — including the 0.x "minor" bumps Dependabot
cannot recognise as breaking.

**Accepted tradeoff — `phonenumber` in the DEFAULT build (M8.1).** The national-phone recognizers link the
`phonenumber` crate (libphonenumber port) in the *default* build, not behind `onnx`. It is deliberately kept
default-eligible because it is **pure Rust** — no `*-sys`, no `cc`/bindgen, nothing
`tests/dependency_footprint.rs` forbids on any target, so that guard stays green. Two real costs are named so
a future `cargo-deny` advisory isn't a surprise: (1) **~3 MB** of binary from the embedded worldwide numbering
metadata (converted to a postcard blob at build time, `Lazy`-deserialized once at runtime — no XML parsing on
the hot path); (2) a few **unmaintained transitive deps** — `oncemutex` (2016), `regex-cache` 0.2.1, an old
`regex-syntax` 0.6.29 alongside the modern 0.8. None is a known advisory today; if one is ever flagged,
**`PII_LOCALES=` (set and empty) switches the whole tier off** — that is the off switch, and it is a
different value from *unset*, which means every region. (Those two were folded together once, so the
documented mitigation turned the tier fully **on**; `CFG-01` now pins the distinction. A mitigation that
does the opposite of what it says is worse than none, because it is reached for under time pressure.) Note
the mitigation also got **weaker** in [M10](ROADMAP.md#m10): the recognizers are on by default rather than
opt-in, so a flagged advisory would affect every deployment until the operator acts, not only the ones that
had opted in. The capability bought — an assigned-range phone check no regex can do, on by default — is
judged worth it.

**And `time` in the DEFAULT build (M10).** Log timestamps are local with an explicit offset, which needs the
`time` crate's platform offset lookup (`tracing-subscriber`'s `local-time` feature). It was *assumed* to be
free because `Cargo.lock` already carried `time 0.3.53` — `cargo tree` said otherwise: it was there only under
`--features onnx`, via `hf-hub → hf-xet → tracing-appender`. So it is a genuinely new default-build dependency
(`time` + `time-core`/`time-macros`, `deranged`, `num-conv`, `powerfmt`, `num_threads`), all **pure Rust**, so
the guarantee holds unchanged. Recorded because "surely it's already there" is exactly the
class of claim this file exists to stop being made without checking.

**What that guarantee actually says — corrected 2026-07-31, and it is the same class of error again.** Three
documents, this one included, described the default build as **native-dependency-free**. That is true on
Windows and false on Linux and macOS. `reqwest` is pinned to `native-tls` precisely to keep rustls'
`aws-lc-rs` out — but `native-tls` *is* the platform's own TLS: schannel on Windows (pure-Rust declarations),
**OpenSSL on Linux** (`openssl-sys` + `cc`, linking the system `libssl`), Security.framework on macOS
(`core-foundation-sys`, `security-framework-sys`, `system-configuration-sys`). Both dependency guards asked
`cargo tree` about **the machine running them**, so on the maintainer's Windows box they were green, and the
first time CI ran them on Linux they were red — correctly, on their first honest run. `openssl-sys` was in
`Cargo.lock` long before [M10](ROADMAP.md#m10); what was new was a guard finally able to see it.

The rule in the form that is actually true, and the form the guards now check **per released target**: *the
default build reaches no native dependency except the TLS its operating system already provides.* Each
target's allowance names those crates and nothing else, so a genuinely new native dependency is still caught
everywhere. There is no pure-Rust way out through `reqwest` — rustls' crypto providers (`aws-lc-rs`, `ring`)
compile C as well — so this is a **reformulation of a claim, not a defect to fix**. The operational
consequence, worth knowing before packaging: a Linux binary links the system OpenSSL, so it is self-contained
on any ordinary distribution but not in a `scratch`/distroless image.

## Decisions & open points

- **Placeholder format: `[KIND_N]`** (e.g. `[EMAIL_1]`) — ASCII, tokenizer-friendly.
- **Coverage (M4, supersedes the original "IT + US")** — three tiers; see *Hybrid
  detection → Locale coverage* above. The NER's domain is its **model's** 10 languages;
  structured PII is language-independent and always on. `PII_LOCALES` gates only the
  *domestic-phone* tier — nine `phonenumber`-validated regions, **on by default since
  [M10](ROADMAP.md#m10)**; setting the variable replaces that set, and a code outside the
  table contributes nothing.
- **A default that detects nothing is a bug, not a conservative choice (M10).** The FP-prone
  tier shipped for two milestones with a default (`it,us`) naming regions that mapped to no
  recognizer at all — nothing regressed, the gap was simply never filled when M8.1 added the
  tier's first entries. The floor that replaces it: **a proxy started with no configuration
  must mask a domestic number**, pinned by a test that builds the detector from an empty
  environment. The branches of a coverage decision may differ in *how many* regions are on;
  they may never differ in *whether* the default detects anything.
- **Over-mask, never leak** — the standing tie-breaker. Where precision and recall
  conflict, recall wins: the pure-numeric national IDs accept ~18% of arbitrary 9-digit
  tokens (M4-R6) and a union may swallow a bare `@domain`. Both are **accepted on
  purpose**. The precision path is *context* (GLiNER, [M8](ROADMAP.md#m8)), never a keyword
  gate — gating a recognizer on nearby words reintroduces leaks.
- **Resolved (M1)**: the `Stage` signature threads a per-request `RequestContext`
  (carrying the `Vault`) from request to response.
- **Resolved (M1.5)**: the scanned text fields are fixed — see *Robustness &
  fail-closed → Field coverage* above.
- **Resolved (M4-R19)**: candidate generation was **O(n²)** on the two unbounded-length
  recognizers (Email, Secret) and masking ran **inline** on the tokio worker. Fixed: the
  overlap rescan is now bounded-patterns-only (unbounded ones rely on the fixpoint), and
  masking runs on `spawn_blocking`. Detection is linear, verified by `tests/complexity.rs`.
- **Resolved (M4-R24)**: masking was *still* **O(n²)**, but in the **entity count**, not the
  field length — `Vault::mask`'s right-to-left `replace_range` splice shifted the tail once
  per entity (13 MiB of many small values ≈ 7 min of CPU). The splice is now a single
  left-to-right copy, O(n + k), and **DOS-04** guards it by varying the entity *count* — the
  axis DOS-01…03 held fixed at one. The structured masking path is now linear on **both**
  axes, measured.
- **Resolved (M5, PERF-01)**: `OnnxNerDetector` used to feed the **whole field as one
  sequence**, and the failure mode was not the suspected "quadratic self-attention" — it was
  **worse and simpler**: RoBERTa-family absolute position embeddings top out at
  `max_position_embeddings` (514 for the picked XLM-R int8), so a sequence past that limit made
  the ONNX graph's position-embedding lookup go **out of range** — measured (`tests/ner_perf.rs`)
  as an outright `Expand` op failure, not a graceful slowdown. Any field over roughly 2 KB of
  prose (~500 tokens) failed NER entirely: silently downgraded to structured-only under the
  default fail-*open* wrapper, but a hard **block** under `NER_REQUIRED` (every such request
  would 400) — an availability gap in the same family as M4-R19/R24, though opt-in and off by
  default so never the unauthenticated DoS those were. Fixed: `OnnxNerDetector::infer` now
  splits an oversized field into overlapping token windows
  (`chunk_char_ranges` + `infer_chunked`, `src/pii/onnx.rs`), each independently tokenized (its
  own `<s>…</s>` framing) and run under the sequence budget, with results merged and
  deduplicated. Measured linear scaling: 64/256/1024 repeated-sentence multiples run in
  448 ms / 2.07 s / 7.53 s (debug profile), recall intact (≥99.6% of expected entities — the
  small excess above 100% is an occasional un-deduped near-boundary double-detection, a
  precision nit, not a recall loss). This is a **recall** mechanism only: structured PII (the
  fail-closed layer) is detected independently over the whole field and is never chunked, so
  this changes NER coverage/availability, never masking correctness.
