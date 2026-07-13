# Development log

Newest first. One entry per meaningful change — note *what* and *why*, not just
*what*. This is the running history so context is never lost between sessions.

## 2026-07-13 — M4-R19/R20/R22/R23: a safety fix has a cost, and the cost is part of the fix

**M4 is done — all 23 findings closed, and M5 is unblocked.**

**M4-R19 (BLOCKER) — the fix for M4-R17 was a denial of service.** Making candidate generation see *every*
overlapping match meant resuming the regex one `char` past each match's **start** — O(n) start positions.
Fine while a match is **bounded** (a card is ≤ 19 digits), but the two patterns with **no length bound** —
`Email` (`[…]+@[…]+`) and `Secret` (`sk-[…]{6,}`) — re-matched an O(n)-long value at every one of them:
**O(n²)**. A ~1 MB `content` field, far under the 16 MiB body limit, pegged a core for **minutes** on an
**unauthenticated** path (151 s at 200 KB; masking runs *before* any upstream auth). A proxy that is down
forwards nothing — and protects nothing. **Availability is a privacy property here.**

The fix is a `Scan` enum on `Recognizer`, decided by one property of the pattern — *is its match length
bounded?* The ten bounded patterns keep the rescan (`Scan::Overlapping`, O(n·L), linear — **and the M4-R17
repro is a `CreditCard`, so it stays closed**); the two unbounded ones go back to plain `find_iter`
(`Scan::Sequential`).

**The hard part was proving that costs no coverage** — because shrinking a candidate set is *exactly* how
M4-R17 made PROP-03 pass vacuously, so this needed an argument, not an assertion. A same-recognizer match
that starts inside an earlier one is **contained** in it (both run greedily to the same word boundary), so
it adds no bytes — for `Secret` that is the whole story. For `Email` there is one shape that isn't
contained, a chained `a@b.com@c.com`, whose second email starts inside the first's *domain* and reaches
past its end. **The fixpoint catches it**: masking the first leaves `[EMAIL_1]@c.com`, and `mask_all`
re-detects until nothing is left, so any surviving `local@domain` is masked on the next pass
(`a@b.com+x@c.com` → `[EMAIL_1][EMAIL_2]`). What remains is a bare `@domain` — not PII (M4-R11). So the two
mechanisms turn out to be **complementary**: *bounded recognizers rescan; unbounded ones iterate.*

Measured (release, 1 MB inputs): email **15 ms**, secret **23 ms**, and the *bounded* card row **160 ms** —
and doubling N doubles the time (2.1 → 3.7 → 7.7 → 15.3 → 29.7 ms across N = 125 K → 2 M), so it is
**linear by measurement**, not just "fast". Masking also moved to `tokio::task::spawn_blocking`: it is
CPU-bound (regex scans, plus NER inference when on), and inline it could starve the executor. A panicking
stage now **blocks** the request — we'd be holding a body of unknown PII status.

New `tests/complexity.rs` (**DOS-01…03**) pins it. They are *timing* guards on a worker thread with a
wall-clock budget, so a quadratic regression fails in **seconds** rather than hanging for hours — and each
also asserts the value is still masked and round-trips, so a "fix" that buys speed with blindness fails
too. **Verified non-vacuous: on the pre-fix code DOS-01/02 time out while DOS-03 passes** — precisely the
bounded/unbounded split the finding predicted.

**M4-R20 — the fixpoint is now *confirmed*, not assumed.** Exhausting `MAX_MASK_PASSES` used to return the
text anyway, forwarding anything still un-masked — a fail-*open* in a fail-closed product. The reassuring
comment ("hitting it can only mean over-masking") was **unproven**: *"each pass strictly shrinks the
un-masked text"* buys **eventual** convergence, never convergence **within four passes**. `mask_all` now
runs one final `try_detect`; anything still detectable → `Err` → `PrivacyStage` blocks (400). No cost on
the normal path (a converging text — every real one, ≤ 2 passes — runs exactly as many detections as
before). Guarded by a synthetic `NeverConverges` detector.

**M4-R21 — closed as *not a bug*.** `mask_all` runs the detector ≥2× (~2× NER inference). That second pass
**is** the fail-closed confirmation above, so it is a deliberate correctness cost, not an oversight.
Carried into M5's PERF-01 as a *measurement*.

**M4-R22/R23 — the small ones.** `rust-version = "1.82"` declared (`Option::is_none_or` sets the floor;
M5's CI would have discovered it by failing), and the four code comments left dangling by the docs refactor
now point at `docs/reviews/` anchors instead of ROADMAP sections that no longer hold the explanation.

**125 tests green (default) / 133 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R16/R17/R18: an invariant is only as strong as the set it quantifies over

Sixth review of this area. The theme this time is **the limits of the guard itself**.

**M4-R17 — `find_iter` hid values from the invariant.** `Regex::find_iter` is
leftmost-**non-overlapping**: after a hit it resumes at the match's *end*. A real value that **starts
inside** an earlier match of the *same* recognizer is therefore never emitted as a candidate at all — so
PROP-03 ("every raw candidate is covered") was satisfied **vacuously** for it. The resolver was fine; it
simply never learned the value existed.

```
4111 1111 1111 1111@123-45-6789-4111 1111 1111 1111
                        └─ the shifted window `6789-4111 1111 1111` is Luhn-valid and matches first,
                           so the REAL trailing card (which begins inside it) was never a candidate
                        →  masked: [CARD_1]@[CARD_2] 1111   ← a card digit group in clear
```

Fixed where the finding says the cause is — **candidate generation**: each recognizer now resumes one
`char` past a match's **start**, and its overlapping hits are coalesced into maximal runs (bounded
candidate set on pathological input, same coverage). *Note: the reviewer's suggested "re-scan the
uncovered gaps" would **not** have closed this repro — the gap is ` 1111`, and the hidden card starts
inside the already-covered region, so no gap re-scan can surface it.*

**And then PROP-04 found a live leak on its first run.** The reviewer asked for a companion property —
"re-running the detector on the masked output must yield nothing" — and it immediately failed:

```
4111111111111111555 867 5309   → one 19-digit run: NOT Luhn-valid, so the card is correctly not a
                                 candidate (an ID never fires inside a longer token)
4111111111111111[PHONE_1]      → masking the phone SPLIT the run, exposing a clean, Luhn-valid card
                                 that would go upstream IN CLEAR
```

**Masking rewrites the bytes around what it replaced, and a value is only recognizable in context.** So
masking now runs to a **fixpoint** (`Vault::mask_all`, wired into `PrivacyStage`): re-detect until the
text yields nothing. It converges because a placeholder is inert (no recognizer can match `[KIND_N]` or
span across it), and the round-trip stays exact. This is the deeper form of the same lesson as the
union-merge: *the property you assert must quantify over something the bug can't hide behind.* PROP-03
quantifies over the **candidate set**; PROP-04 quantifies over the **output bytes**. Only the second one
could have caught this.

**M4-R16 — the ASCII blind spot was still in the guard.** The corpus grew `non_ascii_scripts` at M4-R13,
but **PROP-03's own tables were still 100% ASCII** — the exact blind spot that let M4-R13 survive four
reviews, sitting in the one test the whole no-abandoned-bytes guarantee rests on. Added CJK / Cyrillic /
accented glue and non-ASCII-context samples, so the invariant is exercised on multi-byte input.

**M4-R18 — the code still described the deleted containment gate.** Five comments and two test names
still called it a deletion. Renamed and rewritten to the naming-rule mechanism — worth the churn,
because "the gate deletes the enclosed span" is precisely the mental model that produced the M4-R10 leak.
**119 tests green (default) / 127 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R13/R14/R15: the recognizers were **inert in Chinese** (Unicode `\b`)

The fifth review of this area, and it found the worst leak yet — not in the overlap code everyone had been
staring at, but in a single character of every regex.

**M4-R13 (blocker).** Rust `regex`'s `\b` is **Unicode-aware**: a Han / Kana / Cyrillic letter *is* a word
character, so there is **no word boundary between a CJK character and a digit**. Chinese and Japanese have no
inter-word spaces, so the glued form is the **natural** way to write it — not an evasion. Every `\b`-anchored
structured recognizer was therefore **completely inert** in CJK prose:

```
我的信用卡号是4111111111111111    → matched NOTHING — 16 Luhn-valid card digits upstream in clear
我的身份证号是11010519491231002X  → the zh Resident ID pack we shipped in M4 never fired, in Chinese
密钥sk-abcdef123456              → API secret in clear;  账号DE89370400440532013000 → IBAN in clear
カード番号は4111111111111111です  → card in clear;      Карта4111111111111111      → card in clear
```

The same values mask the instant a space is inserted, which pins the cause exactly. This is squarely inside the
**declared M4 domain**: `zh` is one of the ten declared languages and ARCHITECTURE claims "structured PII is
language-independent". Fix: `(?-u:\b)` on all 12 anchored recognizers. The anti-FP guarantee is preserved
**exactly** (an ID still can't fire inside a longer *ASCII* token — `card4111111111111111`, hashes, base64);
only a non-ASCII *letter* stops counting as part of a number. `Email`/`Phone` were never affected (character
classes, not `\b`).

**Why it survived four reviews — the real lesson.** `tests/corpus/pii_cases.json` contained **zero non-ASCII
characters**, and M4's "validate across the declared domain" pass validated the **NER** on zh — never the
*structured* recognizers. The test suite was ASCII-shaped, so an entire class of total failure was invisible to
it. Every review, mine included, kept auditing the code we had tests for. Added a `non_ascii_scripts` corpus
category (CJK/JA/Cyrillic positives + ASCII-token negatives) and non-ASCII round-trips. **Non-vacuity:**
restoring the Unicode `\b` makes the detector return `[]` on the Chinese card sentence and makes corpus CJK-01 +
RT-05 fail with *"PII must be masked"*.

**M4-R14 (hardening).** The merge's one fallback degraded *toward* the leak class it exists to prevent: if the
union re-slice failed it returned the **winning candidate alone**, abandoning the other constituents' bytes. Now
the union is first **widened out to enclosing `char` boundaries** (widening only ever *adds* bytes, so it can't
abandon a constituent), which makes the slice total; the remaining unreachable arm returns the **whole group
unmerged**, with a `debug_assert!` + kind-only `warn!`. Never a panic — this is a proxy on attacker-influenced
input.

**M4-R15 — and it deleted the containment gate.** The union was named by whichever candidate *survived*, so a
`Secret` enclosed by an email came out as `[PHONE_1]`: no leak, but the model is told the blob is a phone and the
audit log under-reports a secret. The fix is to name the union by the highest-priority **raw** candidate it
covers — and doing so revealed that **the gate was never needed**: it never affected a *span* (an enclosed span,
if not deleted, simply merges *into* its enclosing email → identical union), only the *label*. So
`drop_spans_contained_in_an_email` is **gone**, replaced by a naming rule in `name_of` — highest-priority raw
candidate, **except** when the union is exactly an `Email` span (a genuine email whose local part merely looks
like a card/ID keeps the `Email` label, preserving M4-R7/R9). Deleting nothing also **structurally removes the
M4-R10 trap**: with no deletion, no span can be stranded. Less code, same behaviour, one fewer booby trap.
**115 tests green (default) / 123 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R10/R11: the resolver gets an *invariant* instead of a ranking (union-merge)

Third review on the same overlap code, and the one that found the **root cause** the previous two fixes
kept dancing around. Worth recording, because the lesson is bigger than the bug.

**The pattern.** M4-R7 made `Email` win an overlap; the card leaked. M4-R9 made the structured span win;
the **email** leaked. Both fixes were "correct" against their own test — and both were wrong, because they
were tuning *which side of a partial overlap gets abandoned*. The root cause was never the priorities:
`resolve_overlaps` settled an overlap by **dropping the whole loser span**, so the loser's bytes were left
**in clear**. A flat `priority()` scalar can only express *"one of them wins"*; it cannot express *"both must
be masked"*. Two leaks the reviewer reproduced through `Vault::mask`:

```
555 867 5309john.doe@example.com      → [PHONE_1]john.doe@example.com     (M4-R11: a deliverable email in clear)
555 867 5309.4111111111111111@x.com   → [PHONE_1].4111111111111111@x.com  (M4-R10: 16 card digits in clear)
```

M4-R10 is the nastier one and it was **introduced by my own M4-R9 gate**: the gate deletes a span contained in
an email *before* the priority sort — but the containing email was **not guaranteed to survive** that sort. A
third span partially overlapping the email dropped the email too, so the already-deleted card was masked by
**nothing at all**. (`Secret` is the sharpest case: the highest-priority kind, deleted before priority ever ran.)

**Fix — replace the ranking with an invariant:** *no structured span's bytes are ever abandoned.*
- **Structured union-merge** — two overlapping structured spans now collapse into their **union** (labelled by
  the highest-priority kind) instead of one being dropped. One sort-by-start sweep reaches the fixpoint. The
  union is masked as one placeholder and restored verbatim, so the round-trip stays exact; it over-masks a
  little (a bare `@domain` can land inside the placeholder) — the project's stated direction: over-mask, never
  leak.
- **The containment gate stays and is now provably safe:** an email is never *dropped*, only absorbed into a
  union covering at least its own span, so a span the gate deleted can no longer be stranded.
- **NER keeps the whole-span drop** (M2-R7) — a lost `Person` remainder costs recall, never a leak.
- `resolve_overlaps` now takes `input` (the union's `text` must be re-sliced from the source: `Vault` keys on
  `entity.text` and splices by `span`). `PiiKind::priority` now ranks **labels, not survivors** — a lower
  priority can no longer cost coverage.

**The test that should have existed from the start (PROP-03).** `every_structured_candidate_byte_is_covered`:
glue PII values (incl. the grouped shapes) in arbitrary orders, then assert **every raw structured candidate is
fully covered by some resolved span**. A per-byte invariant *cannot* be satisfied by picking a winner — which is
exactly why the two priority-only fixes each passed their hand-written case while leaking the other side. Proof
it bites: with the union-extension disabled it **independently rediscovered** the M4-R11 leak, shrinking straight
to `555 867 5309john.doe@example.com` ("Email at 8..32 is left in clear"). All 7 M4-R10/R11 tests fail under that
probe. **110 tests green (default) / 118 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4-R9: containment-gate the Email priority (fixes a leak my M4-R7 introduced)

The review caught a **real leak in my own M4-R7 fix**, and it's worth recording *why* the reasoning
failed. M4-R7 made `Email` the top structured priority, justified by: "no other structured kind carries
`@`, so a card/IBAN/secret can only overlap an email by being a **substring of its local part**." That
claim is true only for the **continuous** forms my test covered. The email local-part class
`[A-Za-z0-9._%+-]` **excludes the space** — so against a *space-grouped* card or IBAN glued to a domain,
the email match forms from **only the trailing group** and merely **partially overlaps** the structured
span. `resolve_overlaps` drops the *whole* lower-priority span, so a top-priority `Email` discarded the
entire card:

```
card 4111 1111 1111 1111@example.com   →   card 4111 1111 1111 [EMAIL_1]   ← 12 card digits IN CLEAR
iban DE89 3704 0044 0532 0130 00@…     →   iban DE89 3704 0044 0532 0130 [EMAIL_1]
```

A **regression**: pre-M4-R7 the card won and masked whole. Lesson: "kind X can never contain `@`" bounds
*containment*, not *overlap* — I generalized from the shape my test happened to use.

**Fix — the recommended containment gate (not the minimal revert), so M4-R7's benefit survives.** Two
complementary mechanisms:
- **`drop_spans_contained_in_an_email`** in `resolve_overlaps`, run **before** the priority sort: a
  structured span lying **entirely inside** an `Email` span is a false decomposition of its local part →
  dropped, so the email wins (`4111111111111111@x.com`, `123456789@x.com`). Partial overlaps are untouched.
- **`Email` moved to the lowest structured priority** (below `Phone`, still above NER): every *partial*
  overlap now falls through to priority, where the checksum-backed structured span wins — digits masked,
  never fragmented.

Containment → Email wins; partial overlap → structured wins. This also closes the space-grouped **NINO**
(`AB 12 34 56 C@x.com`), which was a latent leak even *before* M4-R7 (`Email` already outranked
`NationalId`). New `PiiKind::is_structured()` backs the gate. **Tests:**
`grouped_forms_attached_to_a_domain_do_not_leak` (recognizers), `grouped_pii_glued_to_a_domain_leaks_nothing`
(`tests/adversarial.rs` — asserts on the **masked body** that no card/IBAN/NINO group survives in clear),
and the two resolver units. **Verified non-vacuous** by re-raising `Email` above the structured kinds: they
fail. **103 tests green (default) / 111 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

## 2026-07-13 — M4 review follow-ups closed: M4-R6 / M4-R7 / M4-R8

A review session opened three non-blocking precision follow-ups on the completed M4 (all fail-safe —
over-mask/utility, never a leak). Closed all three. **99 tests green (default) / 107 + 1 `#[ignore]`d
(`--features onnx`), no warnings** on both profiles.

- **M4-R6 — accepted-FP tradeoff on the pure-numeric IDs (documented + pinned).** `\b\d{9}\b`
  (BSN ∪ NIF) and `\b\d{11}\b` (DE ∪ LV) over-mask a fraction of ordinary numbers on checksum alone
  (~2/11 ≈ 18% of arbitrary 9-digit tokens; the LV `32…` form adds an unconditional ~1% at 11 digits).
  Resolved by **documenting** the accepted magnitude (code comments on both recognizers, like the LV
  shape-only note) — deliberately **not** context-gating: gating a national ID on a nearby keyword would
  reintroduce leaks and contradict the always-on M4-R1 decision. The clean precision path is the
  contextual **GLiNER** detector (Backlog). Test `bare_numeric_national_ids_are_masked_by_design`:
  `524287244` (an arbitrary PT-NIF-valid number) is masked by design; `524287245` (fails both checksums)
  is left in clear — so it's not a blanket "mask every number".
- **M4-R7 — Email is now the top structured priority (generalized the substring fix).** The earlier
  Email>national-ID reorder is generalized to Card/Iban/Secret: `PiiKind::priority` now ranks
  **Email > Secret > Iban > CreditCard > Ssn ≈ NationalId > Phone**. An `@`-token that parses as an email
  *is* an email, and no other structured kind carries `@`, so a card/IBAN/secret can only overlap an email
  by being a **substring of its local part** — there the whole email is correct and must win, else the
  `@domain` forwards in clear (`4111111111111111@x.com` → previously `[CARD_1]@x.com`). Non-email spans
  never share this tier, so lifting Email regresses nothing outside the containment case (confirmed: no
  corpus/adversarial/proptest change). Test `email_beats_a_card_iban_or_secret_local_part`.
- **M4-R8 — DE Steuer-ID consecutive-triple exclusion.** `de_steuerid_valid` now enforces the 2016+ rule:
  a digit appearing three times in the first 10 must **not** occupy three *consecutive* positions. Pure
  precision gain (rejects a look-alike), zero recall cost (a valid ID never has one). Self-verifying test
  `de_steuerid_rejects_a_consecutive_triple` (same digits + valid checksum: consecutive → rejected,
  non-consecutive → accepted). **M4 review follow-ups done.** Next: M5.

## 2026-07-13 — Relicensed MIT → AGPL-3.0-or-later; version 0.1.0 → 0.4.0

The project is gaining traction, so it moved from MIT to the **GNU Affero GPL v3-or-later**
(`AGPL-3.0-or-later`) — a network-copyleft that keeps the ecosystem open: anyone running a *modified*
version **as a service** must share their changes, which fits a privacy proxy served over the network
(running it unmodified carries no obligation). `LICENSE` is the **official FSF text** (fetched from
gnu.org, byte-exact); `Cargo.toml` `license`, both READMEs (EN/IT), and ROADMAP M0 updated. No per-file
source headers added yet (optional, can follow). Version bumped **0.1.0 → 0.4.0** to reflect M0–M4;
**`1.0.0` is reserved for the first tagged release** (the M5 CI/release pass). No functional code changed.

## 2026-07-13 — M4 COMPLETE: all national-ID packs, CF/FR checks, IBAN per-country, provider-agnostic

Closed every remaining M4 item. All tests green (default + `--features onnx`), no warnings.

- **National-ID packs for all XLM-R-aligned countries** (always-on, checksum-gated): added **DE**
  Steuer-ID (ISO 7064 Mod 11,10 + the one-repeated-digit structural rule), **NL** BSN (11-proef) / **PT**
  NIF (mod-11) behind one 9-digit recognizer, **LV** personal code (classic mod-11 + post-2017 `32…`
  shape-only form), **zh** China Resident ID (ISO 7064 MOD 11-2, 18 chars). `ar` gets no pack (no single
  Arabic national ID). Each validator hand-checked against an official test number.
- **M4-R3 — IT Codice Fiscale check character** (`cf_check_valid`, odd/even table + mod-26): a
  wrong-checksum look-alike is now rejected, consistent with the other national IDs.
- **M4-R5 — FR NIR completeness**: the month alternation admits the INSEE special codes (`20`, `30–42`,
  `50–99`) so those real NIRs aren't missed on the always-on tier (mod-97 key still gates precision).
  Corsica `2A`/`2B` documented as a known limitation.
- **IBAN per-country length**: `confidence_of` tags an IBAN `Verified` only when mod-97 **and** its
  country's ISO 13616 fixed length both check out (`iban_country_length` table); otherwise `Structural`
  (still masked). Unknown countries rely on mod-97 alone.
- **Overlap priority fix (found by proptest).** The new pure-numeric recognizers (`\d{9}`, `\d{11}`,
  18-digit) can match a numeric **email local part** (`123456789@x.com`), and `NationalId` (then priority
  3) out-ranked `Email` (2) → the email got fragmented and PROP-01 failed. Fix: reordered
  `PiiKind::priority` to **Secret > Iban > Card > Email > Ssn ≈ NationalId > Phone** — a national ID never
  *is* an email, so the email (the complete match) must win the substring overlap. This also fixes a
  latent SSN-in-email case. Guard: `numeric_email_local_part_is_not_hijacked_by_a_national_id`.
- **Provider-agnostic verification**: `e2e_masking_is_provider_agnostic` — the same request via the
  `openai` vs `anthropic` presets yields a byte-identical masked body upstream (masking is schema-based;
  presets only affect routing). **M4 is done.** Next: M5 (integration & performance testing).

## 2026-07-13 — M4 continued: national IDs always-on + ES/FR packs + 10-language NER validation

Advanced M4 per the decided three-tier scope. **86 tests green (default), 94 + 1 `#[ignore]`d
(`--features onnx`), no warnings.**

- **M4-R2 (tighten GB NINO).** Added a `validate` fn with the official prefix rules (1st letter
  ∉ D/F/I/Q/U/V; 2nd ∉ D/F/I/O/Q/U/V; invalid pairs BG/GB/KN/NK/NT/TN/ZZ), so the shape regex no
  longer masks look-alikes (`PO123456A`, `GB…`, `DA…`). Prerequisite for always-on.
- **M4-R1 (three-tier structure).** National-ID recognizers now run **always**, independent of
  `PII_LOCALES` (privacy-first — a national ID that reaches the proxy is masked even if its country
  isn't configured). Split into `national_id_recognizers()` (always-on) and `fp_prone_recognizers(code)`
  (opt-in via `PII_LOCALES` — empty seam for future national *phone* formats). `PII_LOCALES` now gates
  only *ambiguous* recognizers, not "which countries".
- **National-ID packs (always-on, checksum-specific).** ES DNI/NIE (mod-23 check letter, NIE X/Y/Z →
  0/1/2) and FR NIR (15 digits + mod-97 key). DE Steuer-ID **deferred** (needs the ISO 7064 check +
  structural rules to hit near-zero FP as an always-on recognizer). Tests: check-letter / key validators
  + detection (a wrong-check look-alike is not masked).
- **NER validated across its declared 10-language domain.** Added `multilingual_preview` cases for
  ar/es/fr/lv/nl/pt/zh (de + en/it already present) — one Person + Location each — and scored the picked
  XLM-R int8 through the hybrid via the `#[ignore]`d harness:

  | kind | recall | prec | F1 |
  |---|---|---|---|
  | Person | 0.83 | 0.71 | 0.77 |
  | Organization | 1.00 | 1.00 | 1.00 |
  | Location | 0.91 | 0.91 | 0.91 |

  The five added Latin-script European languages (es/fr/pt/nl/lv) match **cleanly**; ar/zh find the
  names/cities but with a minor boundary artifact (a preposition token — Arabic `ب`, Chinese `在北京`).
  Confirms the model genuinely covers its declared domain; structured PII remains authoritative and the
  NER stays fail-open best-effort. **Remaining M4:** more national-ID packs, FP-prone locale phone
  formats, IBAN per-country checks (all documented in ROADMAP).

## 2026-07-12 — M4 first landing: locale-parametrized recognizers + national IDs (IT/GB)

Started M4 (broad locale coverage) with the recognizer-architecture change that is its
barycenter. **83 tests green (default), 91 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Locale-parametrized recognizers.** `StructuredRecognizers` split into `universal_recognizers()`
  (email, secret, credit card, IBAN — already any-country — and phone US/`+CC`) plus
  `locale_recognizers(code)` national-identifier packs. New `with_locales(&[codes])` (kept `new()` =
  default `it, us`, backward-compatible). Active locales come from **`PII_LOCALES`** (default `it,us`,
  on `Config.pii_locales`), threaded into `server.rs::build_detector`.
- **National IDs (new `PiiKind::NationalId`, placeholder `[NATID_N]`).** IT **Codice Fiscale**
  (`[A-Za-z]{6}\d{2}[A-Za-z]\d{2}[A-Za-z]\d{3}[A-Za-z]`) and GB **NINO** (compact + space-grouped) —
  both deliberately specific (interleaved letters/digits) for a low false-positive rate. `PiiKind`
  gained the variant + `label`/`priority`(national-ID tier = 3, shared with SSN)/`from_label`.
- **Tests.** `italian_codice_fiscale_detected_by_default`, `uk_nino_needs_the_gb_locale` (incl. that it
  is *off* without the GB locale), `locale_selection_is_scoped` (a US-only set ignores a CF).
- **Deferred (documented in ROADMAP M4):** more locale national-ID packs (ES/FR/DE), locale phone
  *national* formats (FP-prone without a `+CC` anchor), IBAN per-country length checks, and validating
  the already-multilingual XLM-R against a wider corpus. **Next: continue M4 (widen the locale seam).**

## 2026-07-12 — M3-R2: JSON-aware de-mask for tool-call arguments

Fixed a pre-existing correctness bug surfaced by the M3 review. **80 tests green (default),
88 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Problem.** `Vault::demask` did a plain string substitution `[KIND_N]` → raw value. In a
  **JSON-encoded string** field — `tool_calls[].function.arguments` / legacy
  `function_call.arguments` — a value containing a `"`, `\`, or control char produced **invalid
  inner JSON**, so the client couldn't parse the tool-call arguments. Not a leak (client-side;
  request masking unaffected), but a real correctness gap on both the buffered and streaming paths.
- **Fix.** Added `Vault::demask_json_string`, which substitutes the **JSON-string-escaped** value
  (`json_string_body` = `serde_json::to_string` minus the outer quotes). Wired it into the
  `arguments` fields only: buffered `demask_response` (`src/pipeline/privacy.rs`) now uses it for
  `tool_calls`/`function_call` args, and streaming `SseDemasker::demask_for` picks it for
  `StreamKey::ToolArg`. `content` keeps the plain `demask` (it's not a JSON string).
- **Tests.** `demask_json_string_keeps_inner_json_valid` (vault, incl. asserting plain demask *would*
  break it); `tool_call_arguments_demask_stays_valid_json` + `content_demask_is_not_json_escaped`
  (buffered); `tool_call_arguments_deanon_stays_valid_json` (streaming). **Next: M4.**

## 2026-07-12 — M3 close-out: tool-call-arg streaming de-anon, M3-R1 fallback, SSE error events

Closed the remaining M3 follow-ups + the M3-R1 review nit (request-level routing stays deferred,
per user). **76 tests green (default), 84 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Streaming de-anon of tool-call arguments.** `SseDemasker`'s hold-back buffers are now keyed by
  field — `StreamKey::Content { choice }` **and** `StreamKey::ToolArg { choice, tool }` — so a
  placeholder split across streamed `delta.tool_calls[].function.arguments` deltas is reassembled
  and de-masked, not just `delta.content`. `flush_pending` synthesizes the right chunk shape per key.
  Test `tool_call_arguments_split_across_deltas_are_restored`.
- **M3-R1 — non-SSE fallback.** `stream_chat_completions` branches on the upstream response
  `content-type`: when it isn't `text/event-stream` (a JSON 401/429, or a provider that ignored
  `stream`), it falls back to `buffered_fallback` — forward the real status + content-type and run
  the `on_response` de-mask — instead of forcing an event-stream. Test
  `e2e_streaming_non_sse_error_falls_back_to_json` (429 `application/json` reaches the client intact).
- **Terminal SSE error events.** A mid-stream upstream error is now turned into a terminal
  `event: error` (after flushing buffered content) and the stream ends cleanly, rather than aborting
  the client connection. `demasking_sse_body` was made generic over the stream error type so it is
  unit-tested with a synthetic erroring stream (no HTTP): `mid_stream_upstream_error_becomes_terminal_sse_event`.
- **Deferred (unchanged):** request-level provider routing — per-instance today; documented in ROADMAP.

## 2026-07-12 — M3: SSE streaming de-anon + multi-provider routing (Option A)

Streaming and provider routing landed together (real Copilot/Anthropic usage is streamed).
**73 tests green (default), 81 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **Streaming (SSE) with incremental de-anon** (`src/stream.rs`, new). `stream:true` is now
  forwarded (no more 400): the handler masks the request as usual, then streams the response
  back through `SseDemasker`, which parses `data:` lines and de-anonymizes each
  `choices[].delta.content`. A placeholder can be **split across two token deltas**
  (`[EMA` + `IL_1]`), so it keeps a per-choice **hold-back buffer** — `split_demaskable`
  finds the last point that could still be an incomplete placeholder, emits everything before
  it, and holds the tail until the next delta (or stream end) closes it. `[DONE]` and non-data
  lines pass through; a clean request (nothing masked) or `PII_DEBUG_SKIP_DEMASK` streams
  through untouched. **Fail-closed intact:** request-side masking runs first, so the provider
  only ever sees placeholders — streaming de-anon is a client-side usability step, never a
  privacy gate. `server.rs` builds the response body with `Body::from_stream` over a
  `futures_util::stream::unfold` adapter (new dep `futures-util`).
- **Multi-provider routing — Option A** (`config.rs` + `proxy.rs`). `UPSTREAM_PROVIDER`
  (`openai`/`copilot`/`anthropic`) selects a preset for the per-provider *shape*: chat path
  (`upstream_chat_path` — Copilot drops `/v1`), a client-header passthrough allowlist
  (`forward_request_headers` — e.g. `anthropic-version`, Copilot editor headers), and static
  `upstream_extra_headers`. All overridable (`UPSTREAM_CHAT_PATH` / `UPSTREAM_FORWARD_HEADERS`
  / `UPSTREAM_EXTRA_HEADERS`); base URL + key stay env-driven. `Upstream` gained a raw `send`
  (used by streaming) plus the configurable path/headers; `Config::Debug` redacts extra-header
  values (may be secrets).
- **Tests.** `stream.rs` units (hold-back split, split-placeholder reassembly, mid-line byte
  splits, passthrough) + e2e `e2e_streaming_deanonymizes_split_placeholder` (client gets the
  real value from a `[EMAIL_1]` split across SSE events; the upstream saw only the masked body).
- **Deferred (documented, not leaks):** streaming de-anon of `delta.tool_calls[].function.arguments`
  (streamed tool args currently pass through), request-level provider routing, and terminal
  SSE error events. See ROADMAP M3 follow-ups. **Next: M4 (broad locale coverage).**

## 2026-07-12 — M2.5-R1 / M2.6 review nits: footprint guard, log-safety test, env_flag dedup

Closed the last M2.5/M2.6 review items. **68 tests green (default), 76 + 1 `#[ignore]`d
(`--features onnx`), no warnings.**

- **M2.5-R1 builder tasks (decision: Option A — keep `hf-hub 1.0`).** Corrected the three
  inaccurate "no new native deps" claims (`Cargo.toml` comment, this log's M2.5 entry, the
  ROADMAP M2.5 dep bullet) to the real onnx-only footprint — under `--features onnx` hf-hub 1.x
  pulls a second `reqwest 0.13` + `hf-xet` + `rustls`/`aws-lc-rs`. Added **`tests/dependency_footprint.rs`**:
  a `cargo tree` guard that fails if the **default** build ever pulls
  `hf-hub`/`hf-xet`/`aws-lc`/`ort`/`tokenizers` (they must stay `onnx`-gated). Verified
  non-vacuous — all five appear under `--features onnx`, none in the default tree.
- **Log-safety regression test (M2.6).** `tests/log_safety.rs` captures the crate's
  `trace`-level logs during a real PII round-trip and asserts the `trace!` masked-body log shows
  `[EMAIL_1]` and **never** the raw value, while the reply really did carry the de-masked value
  (so a leak would be caught). Turns the DBG-01 inspection rule into an automated guard.
- **`env_flag` de-dup.** One `pub(crate) config::env_flag`; `server.rs` imports it, so
  `PII_DEBUG_SKIP_DEMASK` / `NER_REQUIRED` / `NER_TOKEN_TYPE_IDS` share a single `1`/`true`/`yes`/`on`
  parser and can't diverge.

## 2026-07-12 — M2.5 review follow-ups (R1 parked, R2 fixed) + M2.6 debug modes

Closed the M2.5 review's R2 and the new M2.6 milestone; R1 investigated and parked.
**66 tests green (default), 74 + 1 `#[ignore]`d (`--features onnx`), no warnings.**

- **M2.5-R1 (hf-hub footprint) — investigated, PARKED (user's call: no downgrade).**
  `cargo tree --features onnx` confirmed `hf-hub 1.0.0` pulls a **second `reqwest 0.13.4`**,
  **`hf-xet 1.5.3`**, `rustls 0.23` → **`aws-lc-rs`/`aws-lc-sys`** (native crypto),
  `reqwest-middleware`, and `ureq 3` — so the "no new native deps" claim is inaccurate.
  Verified in hf-hub 1.0.0's `Cargo.toml` that `hf-xet` + `reqwest 0.13` are **non-optional**
  (its only features are `blocking`/`rustls-tls`/`socks`/`default=[]`), so the DECIDED
  `default-features = false` trim is a **no-op** — the only way to shed them is a downgrade
  to the pre-Xet `hf-hub 0.4.3` (reuses the in-tree `reqwest 0.12`, no xet/aws-lc). The user
  chose **not** to pin an older API; parked for a reviewer session. **No code changed.**
  Details + the correction to-do recorded in ROADMAP M2.5-R1. The **default** build stays
  native-dep-free regardless (hf-hub is `onnx`-gated).
- **M2.5-R2 (fail-closed hygiene) fixed.** `parse_id2label` now `ensure!(!pairs.is_empty())`
  after the contiguity check, so an **empty** `id2label` fails closed at load instead of
  returning `Ok(vec![])` and only surfacing later (and only under `NER_REQUIRED`). Test
  `empty_id2label_is_an_error`.
- **M2.6 — debug & observability modes** (opt-in, off by default; neither weakens
  fail-closed — request-side masking always runs). (1) **`PII_DEBUG_SKIP_DEMASK`** on
  `Config.debug_skip_demask` (not a bare env read → isolated + testable): skips the response
  de-mask so the client sees the placeholders the provider saw; a **loud `warn!`** fires at
  startup. `chat_completions` guards the `on_response` loop on it. (2) **`trace!` of the
  masked upstream body** just before forwarding (masked → safe), `debug!` kept for the
  kind-only audit. **Safety boundary upheld:** the de-masked client output is never logged
  (only the `trace!` masked request + a body-less `debug!` on skip). Test
  `e2e_debug_skip_demask_returns_placeholders_to_client`; new `Config.debug_skip_demask`
  threaded through `spawn_proxy_cfg` in the e2e harness.

## 2026-07-12 — M2.5: HuggingFace model auto-download (`hf-hub`) + M2-R10 — COMPLETE

Model management is now library-managed and reproducible, and the last M2 harness
nit is closed. **65 tests green (default), 72 + 1 `#[ignore]`d (`--features onnx`),
no warnings.** Verified end-to-end against a live download.

- **M2-R10 (harness precision) closed.** `tests/ner_eval.rs` scored TP/FP/FN by
  `Vec::contains` (set membership), so a duplicate `(kind, text)` could inflate recall.
  Replaced with a `tally` helper that counts each `(kind, text)` as a **multiset**
  (`tp = min(expected, detected)`). Non-network test `tally_counts_duplicates_as_multiset`
  pins recall 0.5 (not 1.0) when two expected entities meet one detection. Recorded
  numbers were unaffected (no corpus case has a duplicate in one sentence), as predicted.
- **M2.5 — opt-in `hf-hub` auto-download** (`src/pii/hf.rs`, feature `onnx`). The model
  file is resolved in priority order in `server.rs::load_onnx_ner`: (1) explicit
  `NER_MODEL_PATH` (+ tokenizer + labels) — zero outbound calls, always wins; (2) when
  unset but `NER_MODEL_REPO` (`owner/name`) is set, `HfModelSpec::resolve` fetches a
  **revision-pinned** model (`NER_MODEL_REVISION` default `478a2a3`, `NER_MODEL_FILE`
  default `onnx/model_quantized.onnx`, `NER_TOKENIZER_FILE`, `NER_CONFIG_FILE`) into the
  standard HF cache. **`NER_LABELS` is derived from `config.json` `id2label`** (class-id
  order, contiguity-checked → fail closed) unless set explicitly — this removes the
  error-prone hand-typed 9-label list. `hf-hub` 1.0 uses the async API;
  `AppState::new`/`build_detector`/`load_onnx_ner` became `async` for the startup fetch.
  *(Correction, M2.5-R1: an earlier version of this entry claimed hf-hub "reuses the
  reqwest already in the tree — no new native deps". That is **inaccurate**: under
  `--features onnx` hf-hub 1.x pulls a second `reqwest 0.13` + `hf-xet` +
  `rustls`/`aws-lc-rs`. The **default** build is still native-dep-free. See the M2.5-R1
  entry above and ROADMAP M2.5-R1.)*
- **Standard-cache pin (real bug found by running it).** `hf-hub` 1.0 falls back to
  `/tmp/.cache/huggingface` on Windows when `HOME` is unset — a non-shared, drive-relative
  location (`C:\tmp\…`) that defeats the point. `build_client` now sets
  `cache_dir = <home>/.cache/huggingface/hub` (the `huggingface_hub` convention) when
  `HF_HOME`/`HF_HUB_CACHE` are unset, otherwise defers to them. The model now lands in
  `%USERPROFILE%\.cache\huggingface\hub`, deduped with every other tool. Unit-tested via
  `standard_hub_cache_dir` (no network).
- **Verified live + consolidated.** Ran the eval through the new download path: `hf-hub`
  fetched `jiting/xlm-roberta-base-ner-hrl_onnx@478a2a3` into the standard cache and the
  hybrid scored **Org 1.00 / Loc 1.00** and **Person 0.75/0.60/0.67 on the required M2
  corpus** — identical to the manual run (the cached blob is byte-for-byte the manual
  `model_quantized.onnx`, 278,709,677 B). *Note:* the harness's aggregate table also
  counts the DE `multilingual_preview` case ("Herr Müller" → the model returns "Müller",
  no title), which is labelled *not required at M2*; with it, the printed Person line is
  0.60/0.50/0.545. Model/detections unchanged — purely scoring scope.
- **Manual models removed** (user-authorized): deleted the hand-downloaded `ner-models\`
  (XLM-R + rejected Piiranha, ~600 MB) and a mislocated `C:\tmp\.cache` artifact from the
  pre-fix run (~564 MB); the only surviving copy is the `hf-hub`-managed one in the
  standard cache. ~1.16 GB freed.
- **Docs:** ROADMAP M2.5 ✅ + M2-R10 closed; ARCHITECTURE (model-management env contract +
  privacy note); SETUP (§4 — auto-download vs explicit-local). **Next: M3 (streaming).**

## 2026-07-12 — M2 model chosen (measured): XLM-R int8; R6 closed — M2 COMPLETE

Downloaded both locked candidates from the Hub and scored them **through the hybrid
resolver** on `tests/corpus/ner_cases.json` (8 cases incl. the DE non-ASCII preview),
per `docs/M2-NER-EVALUATION.md`. int8 (`model_quantized.onnx`), ORT CPU EP.

| Model | repo @ rev | Person R/P/F1 | Org R/P/F1 | Loc R/P/F1 | latency | size |
|---|---|---|---|---|---|---|
| **XLM-R** (winner) | `jiting/xlm-roberta-base-ner-hrl_onnx` @ `478a2a3` | 0.75 / 0.60 / 0.67 | **1.00 / 1.00 / 1.00** | **1.00 / 1.00 / 1.00** | ~23 ms/case | 266 MB |
| Piiranha | `onnx-community/piiranha-…-ONNX` | 0.00 | 0.00 (no ORG label) | 0.00 | ~37 ms/case | 302 MB |

**Pick: XLM-R int8.** Perfect Org/Loc; Person misses only the pathological single-token
"Caia" (tokenized `▁Cai`+`a`, tagged as two spans) and the "Herr" title on "Herr Müller".
Multilingual (10 langs incl. IT), drop-in (PER/ORG/LOC labels, no `token_type_ids`, no
label mapping). **Piiranha rejected:** ~0 recall on natural-sentence NER (it fires only
subword fragments — it's a form/structured-PII model) **and has no Organization label**.
GLiNER escalation not needed. (Piiranha's granular labels were wired anyway via an
extended `label_to_kind`; `type_vocab_size:0` → it needs no `token_type_ids`.)

- **R6 closed by the live run.** Confirmed non-ASCII "Müller" (2-byte `ü`) extracts on
  exact byte boundaries → the HF tokenizer emits **byte** offsets and the `&str`
  slicing is correct. Added a **whitespace trim** in `decode_entities`: SentencePiece
  includes the leading space in a token offset (`▁Mario` spans " Mario"), so the raw
  span was " Mario Rossi"; trimming shifts the span to the real content (masking now
  preserves the space, and the span text matches the value). This is what took recall
  from 0.00 (exact-text mismatch) to the numbers above. Test:
  `leading_space_in_token_offset_is_trimmed`.
- **`label_to_kind` broadened** to cover granular PII schemes (GIVENNAME/SURNAME → Person,
  CITY/STATE/COUNTRY → Location) while keeping structured categories (EMAIL/PHONE/…) →
  `None` (owned by the deterministic layer). Test updated.
- **To run the NER** (off-repo model, ~266 MB, not committed): download
  `onnx/model_quantized.onnx` + `tokenizer.json` from the XLM-R repo, then set
  `NER_MODEL_PATH` / `NER_TOKENIZER_PATH` / `NER_LABELS="O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC"`
  and build `--features onnx`.
- **M2 COMPLETE.** 65 tests green (default), `--features onnx` builds + runs a live model
  clean, no warnings. Next milestone: M3 (streaming).

## 2026-07-12 — M2 review findings closed (8/9) + fail-closed NER + eval harness

Closed the M2 review (M2-R1…R9 except the model-gated R6), all with tests.
**64 tests green (default), `--features onnx` builds clean, no warnings.**

- **Fail-closed NER (R1/R2).** New `PiiDetector::try_detect(&str) -> Result<_,
  DetectError>` (default delegates to `detect`). `CompositeDetector` propagates a
  sub-detector error; `PrivacyStage::on_request` sets `ctx.block` (→ 400) when a
  *required* detector errors. `FailOpen(Box<dyn PiiDetector>)` opts a non-critical
  detector out (logs + empty). `NER_REQUIRED` drives it: `build_detector`/
  `AppState::new` now return `Result`, so a configured-but-unloadable required NER
  is fatal at startup; unset → old fail-open via `FailOpen`. `DetectError` carries
  only a static label, never input (R8).
- **Decode robustness.** `is_begin` accepts `B_`/underscore prefixes (R5, else
  adjacent same-type entities glue); `validate_label_count` rejects a
  `NER_LABELS`/model class-count mismatch instead of silently dropping entities
  (R3); `decode_entities` `warn`s (kind only) on an off-boundary offset rather
  than silently dropping a name (R6 mitigation — full non-ASCII verification stays
  open, needs a real tokenizer).
- **ONNX I/O (R4).** `outputs.get("logits")` → graceful `Err`, no panic;
  `token_type_ids` threaded when `NER_TOKEN_TYPE_IDS` is set (BERT-family, e.g.
  Piiranha); required input/output contract documented. `.lock()` recovers a
  poisoned session mutex (R9).
- **Overlap remainder (R7).** Documented + tested the deliberate choice: an NER
  span overlapping a kept structured span is dropped whole (structured never lost;
  only the non-overlapping unstructured remainder).
- **Eval harness.** `tests/ner_eval.rs` (`--features onnx`, `#[ignore]`d) scores a
  live model against `ner_cases.json` through the hybrid resolver
  (recall/precision/F1 per type + timing). Run:
  `cargo test --features onnx --test ner_eval -- --ignored --nocapture`.
- **Still open:** R6 (non-ASCII offset verification) + the measured model
  selection — both genuinely need a real model/tokenizer file.

## 2026-07-12 — M2 part 2: OnnxNerDetector behind the `onnx` feature (compiles)

The ONNX NER detector is implemented and **`cargo build --features onnx` is
green with no warnings** — `ort` 2.0.0-rc.12 (native ONNX Runtime, downloaded at
build) + `tokenizers` 0.23 both build under MSVC here. The default build stays
native-dep-free (feature off).

- **Pure decode** (`src/pii/ner_decode.rs`, **not** feature-gated, unit-tested on
  the default build): `label_to_kind` (strips `B-`/`I-`, maps PER/ORG/LOC + GPE),
  and `decode_entities` (BIO merge → char spans via token offsets; a `B-` label
  always starts a fresh entity so adjacent same-type entities don't glue). NER
  hits are tagged `Confidence::Structural`.
- **`OnnxNerDetector`** (`src/pii/onnx.rs`, `onnx` feature): HF fast tokenizer +
  a **pool of `ort` sessions** (round-robin, `NER_POOL_SIZE`) so inference isn't
  single-threaded (the M2 concurrency item). `detect` tokenizes with offsets,
  runs the session, argmaxes per-token logits (`num_labels = logits.len()/seq`,
  avoiding the Shape API), and hands off to `ner_decode`. ort's non-`Send`/`Sync`
  builder errors are converted to strings before entering `anyhow`.
- **Wiring** (`src/server.rs`): `build_detector()` composes the structured
  recognizers with the NER when the feature is on and `NER_MODEL_PATH` /
  `NER_TOKENIZER_PATH` / `NER_LABELS` (+ optional `NER_POOL_SIZE`) are set; a load
  error logs and falls back to structured-only.
- **Cargo**: `onnx = ["dep:ort", "dep:tokenizers"]`; `ort` pinned `=2.0.0-rc.12`
  with `download-binaries`; `tokenizers` `default-features = false` + `fancy-regex`
  (pure-Rust regex, no C regex backend).
- **Still pending (needs a model file):** the *measured* model selection (#2) and
  the fail-closed-on-NER-error decision (both noted in ROADMAP). No numbers are
  fabricated.
- **Tests: 57 green (default), no warnings; `--features onnx` compiles clean.**

## 2026-07-12 — M2 part 1: hybrid-detection infrastructure (model-independent)

Landed the M2 architecture that doesn't depend on a specific model, so the ONNX
model becomes a drop-in — implementing it *before* the measured model choice, per
`docs/M2-NER-EVALUATION.md` (measure, don't guess; don't fabricate an evaluation).

- **Shared overlap resolution** (`src/pii/overlap.rs`): extracted `resolve_overlaps`
  from the recognizers into a reusable function keyed on `PiiKind::priority()`.
  Structured PII (Secret>Iban>Card>Ssn>Email>Phone) outranks NER
  (Person/Org/Location = 0), so a deterministic email/IBAN always wins a span an
  ML guess overlaps. Recognizers now delegate to it (behaviour unchanged).
- **`CompositeDetector`** (`src/pii/composite.rs`): a `PiiDetector` that fans out
  to N detectors and merges their spans through the shared resolver — the hybrid
  seam. The server now builds one (structured-only today; the NER joins once
  wired). Tested with the real recognizers + a fake NER: merge works and the
  deterministic layer wins overlaps.
- **NER corpus** (`tests/corpus/ner_cases.json` + `tests/ner_corpus.rs`):
  labelled Person/Org/Location in IT+EN, single-word names (Tizio/Caia), REG-03
  negatives (`anubi` must not be a Person), a DE multilingual preview. Positive
  *recall* is measured once a model lands; the enforced-now guard is that the
  **deterministic layer never emits an unstructured entity**.
- **Symmetric response de-masking** (M1.5-review micro): `demask_content` now
  mirrors `mask_content`, restoring a bare-string element in a response content
  array too.
- **Tests: 53 green, no warnings.**
- **Next (M2 part 2):** `OnnxNerDetector` behind the `onnx` feature (`ort` +
  `tokenizers`, CPU EP, session pool for concurrency, pure BIO-decode unit-tested)
  — then the measured model selection with real files.

## 2026-07-11 — M1.5 code-review follow-ups closed

Three follow-ups from the M1.5 review (all in code from the previous session):

- **Array-content fail-closed gap** (`src/pipeline/privacy.rs`, `mask_content`):
  a content-array element that wasn't an object was silently skipped — a leak for
  a bare-string element carrying PII. Now: bare strings are masked, object parts
  have their `text` masked (non-text parts like `image_url` skipped), and any
  other element (number/bool/null/nested array) **fails closed**. Consistent with
  the top-level content rule.
- **Phone international over-match** (`src/pii/recognizers.rs`): the open
  `\+\d{1,3}(?: \d{2,7}){1,4}` swallowed a trailing number group
  (`+39 333 0000001 12345` → masked `12345` too). Replaced with two canonical
  shapes (three-group `+39 333 000 0001`, two-group `+39 333 0000001`), tried
  three-group first — same fix pattern as the IBAN over-match.
- **`Confidence` was write-only** (`src/pii/mod.rs` / `recognizers.rs`): now
  consumed — `detect` emits a `debug` audit log for each `Structural` match (KIND
  only, never the value). Field is read; richer use (audit sink, ML thresholds)
  deferred to M2.
- **Tests: 46 green, no warnings.** +`content_array_*` pipeline cases,
  +`phone_international_span_stops_at_the_number` adversarial case.
- **Next: M2** — ONNX NER.

## 2026-07-11 — M1.5: robustness & fail-closed (+ M1 code-review fixes)

Hardened M1 so the proxy fails *closed*, and folded in the M1 code-review items.

- **Fail-closed pipeline.** `RequestContext` gained a `block: Option<String>`. The
  privacy stage sets it on an unreadable `content` shape (bare object/scalar) or a
  missing/!array `messages`; the handler returns 400 and never forwards. Unproxied
  paths now 404 via a router `fallback` (only chat/completions + healthz are in
  scope). Masking still runs before forwarding, so nothing leaks on later errors.
- **Full field coverage** (`src/pipeline/privacy.rs`): added `messages[].name`,
  legacy `function_call.arguments`, `tools[].function.description`, and recursive
  `description`s inside `tools[].function.parameters`; content-array parts now mask
  any part carrying a `text` string (robust to new part types). One shared vault →
  a value split across fields still collapses to one token.
- **Tolerant de-mask** (`src/pii/anonymizer.rs`): replaced the two-pass
  contains+replace loop (code-review CR-4) with a single regex pass that also
  tolerates model corruption — `[EMAIL 1]`, `[email-1]`, `[ EMAIL_1 ]`. An
  unresolved but known-kind placeholder is `warn!`-logged, never silently shipped.
- **IBAN over-match fixed** (code review): the regex now matches the two canonical
  IBAN shapes (continuous / space-grouped-in-4s) instead of "optional space before
  any char", so `IBAN IT60…456 EUR` no longer swallows `EUR`. Corpus `IBAN-04` +
  unit test guard it.
- **`iban_mod97` wired** (code review): used in `confidence_of` — a mod-97-valid
  IBAN is `Verified`, a structure-only one `Structural` (still masked). New
  `PiiEntity.confidence` + `Confidence` enum + `PiiKind::from_label`.
- **Single system message** (code review): augmentation now merges into an existing
  `system`/`developer` message instead of inserting a duplicate.
- **Response headers forwarded** (code review): safe allowlist (`retry-after`,
  `x-request-id`, `x-ratelimit-*`, `openai-*`, `anthropic-*`); content/hop-by-hop
  dropped. `forward_chat_completions` now returns the upstream `HeaderMap`.
- **Body-size limit**: `MAX_BODY_BYTES` (default 16 MiB) via `DefaultBodyLimit`.
- **Broadened phone recognizers** (evasion recall): `(555) 867-5309`, `555.867.5309`,
  `+1 …`, extra Italian grouping. Obfuscated emails documented as an accepted gap.
- **Tests: 43 green, no warnings.** New `tests/adversarial.rs`; corpus `PHONE-03..05`,
  `IBAN-04`; pipeline INT-07 + coverage/fail-closed/split-field cases; e2e header/
  404/fail-closed cases; lib demask-tolerance + IBAN-span + phone-shape + confidence.
- **Next: M2** — ONNX NER (see `docs/M2-NER-EVALUATION.md`).

## 2026-07-11 — M1 complete: pipeline, server, prompt-augmentation round-trip

Finished the rest of M1 (Part A wiring + Part B, the ⭐ primary feature).

- **`Stage` trait finalized** (`src/pipeline/mod.rs`): resolved the open design
  point — stages now share a per-request **`RequestContext`** that carries the
  `Vault` from `on_request` to `on_response`. Stages run in order out, reverse
  order back. (Added `Vault::is_empty`.)
- **`PrivacyStage` implemented** (`src/pipeline/privacy.rs`): masks every text
  field of the outgoing OpenAI payload with one shared vault — `messages[].content`
  (string *or* `text` parts) and `messages[].tool_calls[].function.arguments` — so
  the same value maps to the same `[KIND_N]` everywhere. On the way back it
  restores `choices[].message.content` and `choices[].message.tool_calls[].function.arguments`
  (INT-03: the client runs its tools with real values). Clean requests are left
  byte-for-byte unchanged.
- **Prompt augmentation** (Part B): when anything was masked, a system message is
  prepended (`AUGMENTATION_PROMPT`) telling the model that `[KIND_N]` are typed
  real values to use verbatim (incl. tool-call args), never altered — and that
  they're auto-restored downstream. Injected only when PII is present, so clean
  prompts aren't polluted.
- **Determinism across turns** (INT-05): masking is a deterministic function of
  value; re-sending the same history yields identical tokens, and a repeated
  value reuses its token in reading order.
- **Server + forwarding** (`src/server.rs`, `src/proxy.rs`, `src/config.rs`):
  axum router (`POST /v1/chat/completions`, `GET /healthz`), `AppState` holding an
  `Arc<Upstream>` (reqwest) + the stage list, `TraceLayer`. Streaming requests
  get a clear 400 (M3). `Config::from_env` (`LISTEN_ADDR`, `UPSTREAM_BASE_URL`,
  optional `UPSTREAM_API_KEY`) with a **secret-redacted `Debug`** so the key never
  hits the logs. Upstream errors map to a JSON 502.
- **Tests: 26 green, no warnings.** Added `tests/pii_pipeline.rs` (INT-01…06 +
  clean-passthrough, stage-level, no network) and `tests/proxy_e2e.rs` (real
  client → proxy → mock upstream: asserts the upstream saw only masked values +
  the augmentation message, and the client got the originals back; plus a
  clean-passthrough case). Smoke-tested the real binary: `/healthz` → 200,
  chat vs a dead upstream → 502 JSON error.
- **Next: M2** — ONNX NER for names/orgs/locations (see `docs/M2-NER-EVALUATION.md`).

## 2026-07-11 — M1 Part A: structured-PII masking core

- **Recognizers implemented** (`src/pii/recognizers.rs`): email, phone (US dashed
  + IT/international), US SSN, credit card (Luhn-gated), IBAN, and a deterministic
  **SECRET** recognizer (`sk-…`, `sk-ant-…`, `AKIA…`) the old ML model missed.
- **Overlap resolution** by priority (Secret > IBAN > CreditCard > SSN > Email >
  Phone), then span length — this is what fixes the old proxy's REG-01 bug (IBAN
  mis-masked as PHONE): IBAN outranks phone/card and wins the shared digits.
- **Validators**: pure `luhn_valid` (checksum only; length enforced by the CC
  regex) and `iban_mod97`. IBAN detection is **structure-based**; mod-97 is a
  confidence signal only, so synthetic-but-shaped IBANs are still masked
  (privacy > strict validation), matching corpus case IBAN-03.
- **`Vault`** (`src/pii/anonymizer.rs`): `[KIND_N]` placeholders, numbered in
  reading order, spliced right-to-left so byte offsets stay valid. Deterministic —
  the same value reuses its token (VAULT-05). Exact `demask(mask(x)) == x`; the
  `]` terminator prevents `[EMAIL_1]` matching inside `[EMAIL_11]`.
- **`PiiKind`** gained a `Secret` variant, a `label()` for placeholders, and
  serde derives so the JSON corpus deserializes straight into it.
- **Tests green (16)**: inline unit tests + corpus-driven integration
  (`tests/pii_corpus.rs`, all `recognizers`/`validators`/`vault_roundtrip` cases)
  + property tests (`tests/pii_properties.rs`, proptest — PROP-01 detect+roundtrip
  for generated email/phone/SSN, PROP-02 no false positives on alphabetic text).
  Added `proptest` as a dev-dependency only. `cargo build --all-targets` clean, no
  warnings.
- **Next (Part B / rest of Part A)**: wire the privacy stage into the pipeline,
  add the axum server + reqwest forwarding of `/v1/chat/completions`, then the
  prompt-augmentation round-trip (system-prompt injection, tool_calls de-anon,
  deterministic placeholders across turns) with INT-01…06.

## 2026-07-11 — Toolchain up, first green build

- **Rust + portable MSVC installed, no admin.** rustup per-user (`cargo`/`rustc`
  1.97); MSVC linker via PortableBuildTools into `C:\Lavoro\Tools\MSVC` (`link.exe`,
  Windows SDK 10.0.26100, VC 14.44) with `env=user` (HKCU). See `docs/SETUP.md`.
- **First `cargo build` is green** — 177 deps, ~2 min, no warnings; even C-linking
  crates (`ring`) build, confirming the MSVC C toolchain works.
- **`ort`/`tokenizers` removed from M1**: `ort` 2.x is prerelease-only and not
  needed until M2, so the `onnx` feature is now empty and re-adds them at M2.
  Default build has zero native deps.
- **Decisions locked**: placeholder format `[KIND_N]` (e.g. `[EMAIL_1]`), ASCII;
  locale coverage IT + US. ARCHITECTURE and TESTING updated accordingly.
- **Roadmap refined** (session paused at baseline): prompt augmentation & round-trip
  promoted to a highlighted **primary** sub-milestone (M1 · Part B, right after the
  masking core) rather than a buried checkbox; kept in M1 because it is coupled to
  masking. Added a future **M5 — broad locale & language coverage** (beyond IT + US),
  relevant when we move off the OpenAI model.

## 2026-07-11 — Project bootstrap & scaffold

- Created repo docs: `README.md` (EN, canonical) + `README.it.md` (IT).
  Description kept generic (no ONNX) so the detection tech can change later.
  Convention adopted: Italian docs are named `<basename>.it.md`.
- Confirmed **MIT** license.
- Masked the git identity repo-locally — commits must never use the real/work
  email (global config was the Capgemini address).
- Ported PII test cases from the old `llmproxy-extended` into
  `tests/reference/old-proxy/` (PII only; "headroom" compression tests excluded).
  Key lesson extracted: structured PII was regex/Luhn/IBAN-based; the ONNX model
  (unreliable) only handled unstructured entities → adopted **hybrid detection**.
- Locked decisions: modular pipeline, **CPU-first**, hybrid detection, stack
  (tokio / axum / tower / reqwest / serde / ort / tokenizers). Roadmap M1→M4.
- Wrote the Rust module scaffold — trait/type definitions with `todo!()` bodies.
  ⚠️ **Unverified**: the Rust toolchain is not yet installed, nothing has been
  compiled. First job once MSVC is in place: get `cargo build` green, fix any
  scaffold errors, and verify the dependency versions in `Cargo.toml`.
- **Doc policy**: internal docs stay English-only; only root-level files get an
  Italian version (`README.it.md`). Removed the `docs/*.it.md` mirrors.
- **Prompt augmentation decided**: the privacy stage will transparently inject a
  system instruction so the model treats placeholders as typed real values and
  uses them verbatim (incl. tool calls). This expands the round-trip to
  `tool_calls` arguments and tool results, and requires deterministic placeholder
  assignment — see ARCHITECTURE.md.
- **Toolchain install started** — no admin rights on this PC. Rust core installs
  per-user without admin; the MSVC linker does not (VS Build Tools needs admin),
  so MSVC will come via portable extraction of the build tools + Windows SDK,
  with the GNU toolchain as a fallback that unblocks M1 immediately.
- **Rust installed** (per-user, no admin): `cargo`/`rustc` 1.97, host
  `x86_64-pc-windows-msvc`. Confirmed the only missing piece is the **MSVC
  linker** (`error: linker 'link.exe' not found`).
- **MSVC linker plan**: portable Build Tools via PortableBuildTools v2.10.2
  (`accept_license env=user target=x64 host=x64 path=<folder>` — writes
  INCLUDE/LIB/Path to HKCU, no admin; `devcmd.ps1` sets the env per-session).
  Awaiting the user's chosen install folder. Procedure in `docs/SETUP.md`.
- **Testing doc added** (`docs/TESTING.md`) from the old `docs/guide/testing.md`
  (TC-01…04 PII; headroom excluded) — captures the old proxy's real failures
  (IBAN→PHONE, secrets missed, `anubi`→PERSON false positive) as explicit test
  guards, and flags a new SECRET recognizer to add.
- **Test battery drafted**: expanded `docs/TESTING.md` with a full test catalog
  (unit / property / integration / e2e / regression) and added
  `tests/corpus/pii_cases.json` (data-driven detection / validator / roundtrip
  cases, reusing the old test values). Surfaced open decisions: locale scope
  (IT + US), placeholder format, IBAN validation strictness, SECRET patterns.

### Pending / next

- User provides the MSVC install folder → run PortableBuildTools → `cargo build` green.
- Then start M1: recognizers (incl. SECRET) + vault + privacy stage + prompt
  injection + forwarding, validated against the ported tests.
