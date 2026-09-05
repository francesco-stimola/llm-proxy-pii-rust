# Development log

Newest first. One entry per meaningful change — note *what* and *why*, not just
*what*. This is the running history so context is never lost between sessions.

## 2026-09-05 — M11 round 14: the axis had a cardinality, and a constant contradicted its own doc

### M11-R51 — a separator is a run, and every pattern spelled exactly one

Every gap-bearing pattern spelled its separator as **one character** while every validator behind it
normalises a **run**: `phonenumber::parse` discards each ignorable character independently,
`iban_mod97` filters `char::is_whitespace()` over the whole string. The proof needed no new
character at all — `+39 347 1234567` was masked and `+39  347  1234567`, with two spaces, was not;
`030 / 12345678`, the ordinary German business rendering, is three separator characters in a row.
Measured over 13 000 valid numbers: international **0.923**, domestic **1.000**, and **all ten**
recorded positives leaked on a doubled separator drawn from their own alphabet.

The placeholders now expand to `[...]{1,4}`. **Bounded, and not out of timidity:**
`Scan::Overlapping` is linear only while a match length is (M4-R19), and an unbounded `+` would take
the rescan to O(n²) — the DoS that finding exists for. Four covers every rendering measured; the
residue, a run of five or more, is column alignment rather than a rendering and is named beside the
guard.

**And the cost is measured, because M11-R38 is open on exactly it.** Against round 10's grid:
`3XX XXX XXXX` goes **8 → 9.8** units/row, the rejected id column **12 → 13.8** — **+22%**, so the
allowance is reached ~18% sooner, wall-clock per row unchanged. That number is added to R38 rather
than left in a commit message: fixing a leak widened the term that decision is about.

### M11-R52 — `(` and `)` were in the constant's doc comment and not in the constant

`+49 (0)30 12345678` leaked at **0.714**, `(020) 7946 0958` at **0.923** — while `+1 (415) 555-2671`
was masked, because the US arm spells `\(\d{3}\)` itself. **The repository held the answer and
three of the four phone recognizers did not have it.** All four take `{PGAP}` now, and there are
three classes, each named for its validator: `{GAP}` for `iban_mod97`, `{CGAP}` for `luhn_valid`,
`{PGAP}` for `phonenumber::parse`. The card gained `{CGAP}` because the widened matrix demanded
it: `4111--1111--1111--1111` failed an arm that spelled *one hyphen or a whitespace run*, while
`luhn_valid` filters to digits and accepts any run.

### M11-R53 — and the guards could express neither coordinate

Three parts, all closed. The matrix varies **cardinality** as well as the character.
`looks_like_a_rendering`'s alphabet is **derived** from `PHONE_ALPHABET` — hand-listing the four
missing characters would have fixed those four; deriving it means the filter can never again be
narrower than the code it is asked about, which is the defect, since a row whose `not_detected` is
empty was asserting a completeness the registry had **no way to contradict**. And the `+CC` row now
carries punctuated and doubled renderings, so removing `/` from both constants — **green at
255/0/5** — is red on the span assertion.

That third part is the durable one: *a decision recorded in two lists that only check each other is
bookkeeping; it becomes a guard the moment one of them is a behaviour.* The first two are the third
and fourth instances of M11-R31's shape — **scope, alphabet and cardinality are three separate
decisions, and each must come from the thing it is about.**

### M11-R54 — and the fifth consecutive overstatement in one README cell

Same cause every time: the sentence was written from the fix's *intent* rather than its measurement.
It now enumerates what the code enumerates.

### Numbers

**255 / 0 / 5** over 24 binaries, twice, identical; `cargo test-onnx` **291 / 0 / 22**; `fmt` and
`clippy --all-targets -D warnings` clean. The lib suite's wall time roughly **doubled** (38 s → 74 s)
— the same widening R51 priced, visible in the harness.

Also fixed on the way, and mine: five `why:` literals I had written as single 280–830-character
lines carrying literal `
` escapes. Rewrapped to the file's own style; `cargo fmt` cannot see inside
a string literal, so nothing had complained.

**Tally**, counted from the ledger: **32 on the net or the docs, 23 in the product**, 55 rows.

**Open: [M11-R18](reviews/M11.md#m11-r18) and [M11-R38](reviews/M11.md#m11-r38)** — the same
maintainer decision in two rows, now re-priced by R51.

## 2026-09-05 — M11 round 13: the axis was decided, the alphabet was not

### M11-R48 — a `+CC` number written with `-`, `.` or `/` goes upstream in clear

`phonenumber::Mode` has **four** variants. Rounds 11 and 12 both swept three by name — `E164`,
`International`, `National`, all now 0.000 — and the fourth is `Mode::Rfc3966`, whose normative RFC
example is `tel:+1-201-555-0123`. On the same generator and predicate, over 13 000 valid numbers:
**Rfc3966 0.385**, and `International` rendered with `.` **0.846**.

The cause is a rule this repo states two paragraphs above the arm. `{GAP}` is **whitespace**,
because M11-R25 derived it from `iban_mod97`; `phonenumber::parse` also discards `-` `.` `/` `(` `)`.
The shortest proof needs no corpus at all: `06.12.34.56.78` is masked and `+33.6.12.34.56.78` — the
same number — is not.

**An axis has an alphabet, and deciding the axis is not deciding the alphabet.** The class is now
**per recognizer, derived from that recognizer's validator**: `{PGAP}` for the `+CC` arm, `{GAP}`
for the rest, one definition each shared by the pattern, the shrink and the validator. That sentence
is promoted into `ARCHITECTURE.md`, because one class for everyone is how a closed axis keeps
leaking.

Not a regression — it predates M11 — but round 12's closure *claimed* every `+CC` rendering, and that
claim is what made this a finding rather than a gap. **Third time this milestone a fix's claim outran
its reach** (R41 → R43 → R48), all three on the same family.

The card, IBAN and SSN results on this axis (`4111.1111.1111.1111` in clear;
`DE89-3704-0044-0532-0130-00` mangled) are **recorded and not fixed**: unlike RFC 3966 those
renderings are not published by their issuers, so admitting them is a coverage decision.

### M11-R49 — and the guard could not have seen it

`SEPARATOR-01` took **both** its scope and its matrix from `GAP_CHARS`, so every substitution it could
make was whitespace-for-space. Deleting hyphen support from a shipped phone shape at all three sites
left the suite **green at 255/0/5**. Each `SeparatorAnswer` now names its own alphabet — `SPACE_ONLY`,
`WITH_HYPHEN`, `WITH_HYPHEN_DOT`, `PHONE_ALPHABET` — matching what that recognizer's validator
accepts, and the matrix substitutes over it. The mutation is red now, naming `U+002D`.

**One thing I tried and reverted, stated on the guard.** Widening the *scope* filter to the union
pulls in `Secret`, `Email` and the SSN, whose `-` and `.` are part of the **token** rather than
separators between groups. Scope stays whitespace; the residue — a recognizer separated *only* by
punctuation — is named, and none exists.

Third instance of one shape after M11-R31 and M11-R40. Together they are one rule: **a guard's scope
and a guard's alphabet are separate decisions, and each must come from the thing it is about.**

### M11-R50 — third consecutive round in which that README cell overstates

*"Every `+CC` rendering — any grouping"*, written in the round-12 commit and false in it. It now names
the alphabet instead of claiming a universal. R45 corrected *"over-mask, never a miss"*, R47 *"no
validator whatsoever"*, this one *"any grouping"* — the same shape three times: **the sentence
described the intent of the fix being written rather than its reach.**

### Numbers

**255 / 0 / 5** over 24 binaries, twice, identical; `cargo test-onnx` **291 / 0 / 22**; `fmt` and
`clippy --all-targets -D warnings` clean. No test count moved — `SEPARATOR-01` gained an alphabet per
row rather than a sibling.

**Tally**, counted from the ledger: **30 on the net or the docs, 21 in the product**, 51 rows.

**Open: [M11-R18](reviews/M11.md#m11-r18) and [M11-R38](reviews/M11.md#m11-r38)**, plus the coverage
questions recorded beside them (R23, R27, R30, R35, and now R48's card/IBAN/SSN punctuation).

## 2026-09-05 — M11 round 12: a fix that answered the headline number, not the finding

### M11-R43 — round 11 closed one rendering of the three its own finding measured

M11-R41 measured E.164 at 0.918 missed, `Mode::International` at 0.154, and quoted
`+55 11 91234 5678 -> [PHONE_1] 5678` as **worse** than a miss. The fix I shipped was
`\+\d{8,15}` — **one unbroken digit run**, which cannot match a spaced rendering by construction.
So `+33 6 12 34 56 78` was untouched, the Brazilian truncation was untouched, and the ledger row,
the closure note, `TESTING.md` and `CHANGELOG.md` all read as closed — the changelog quoting the
Brazilian case *inside a bullet headed "is now masked"*. Re-measured on the finding's own
methodology: **973 of 3 250** international renderings still carried digits upstream, 613 whole and
360 truncated; both French templates 0 of 250 clean.

The arm now spans the gap class — `\+\d{1,3}(?:{GAP}\d{1,8}){1,6}` beside the compact one —
still gated by `parse(None, ..).is_valid()`, and with **`shrink_on_reject: true`**, which the
unbroken version did not need and this one does: a grouped arm is greedy across separators and
over-reaches into what follows. That is the M11-R13 / M11-R33 pair, stated as a rule two rounds ago
and applied here without being rediscovered — the first time this milestone that has happened.

**The group cap is 8 because `RENDER-01` refused 5.** `+31 6 12345678` has an eight-digit second
group; a cap chosen from the renderings one happens to think of is the same defect one level down,
caught in the minute it was written.

**What generalises, and no guard catches it:** *a fix that answers a finding's headline number is not
the same as a fix that answers the finding.* The guard was green and the box was ticked; only
re-running the finding's own methodology showed it. So: **when a finding measures N renderings, the
closure reports N numbers.**

### M11-R44 — a filter that fails by silence, on the family that had just leaked twice

`looks_like_a_rendering` allowed only `[A-Za-z0-9 -]`, so **no `+CC` rendering could ever bind** to
`RENDER-01`'s `why` audit. It decides *whether* to assert, so a token it wrongly rejects is never
checked and everything stays green — round 12 proved it by quoting `+33 6 12 34 56 78` in a `why`
with both lists empty: **157/0**.

`+` is in the set now, and the function has a **matrix** — which failed on its first run, on
`4-4-4-4`: the doc beside the filter claimed that was excluded and it never was. The matrix corrected
the documentation of the function it was written to check, in the same minute. The rule, and its
corollary, are both on the function now: *prove such a filter by a token it MUST accept*, and *where
acceptance and rejection pull against each other, the tie goes to accepting* — over-acceptance costs
a list entry, under-acceptance costs an assertion nobody notices is missing.

### M11-R45 / R46 / R47 — and the fourth consecutive round of stale prose

- Both READMEs called the spaced `+CC` arm *"over-mask, never a miss"* — written in the round-11
  commit and false in it. With R43 closed the sentence is obsolete rather than merely wrong: only the
  **US 3-3-4** arm is shape-only now, and both tables say so.
- `RENDERING_ANSWERS` said *"24 rows for 24 pairs"* and a 25th recognizer shipped a round later. The
  number is **deleted, not incremented**: the audit enforces the correspondence both ways, so prose
  restating it adds only something to go stale. `UTF8-01`'s count is re-measured at **2 572**, floor
  2 000.
- `ARCHITECTURE.md` had two false `+CC` claims and this is the **fourth** round to find that file
  stale (R17, R26, R32, R47). Four rounds is not four oversights; it is one rule stated too narrowly.
  R32 wrote *"a status belongs in the ledger, never in prose"* — and this round found a **measurement**
  asserted in prose right beside it. The general form now stands in both files: **if something is
  enforced or measured elsewhere, prose links it and does not restate it.**

### Numbers

**255 / 0 / 5** over 24 binaries, twice, identical (round 12 measured 254; the +1 is the filter
matrix); `cargo test-onnx` **291 / 0 / 22**; `fmt` and `clippy --all-targets -D warnings` clean.
`national_phone_does_not_swallow_an_adjacent_number` stayed green — that is the failure mode the
card's general arm hit in M11-R30, and it was checked here first for that reason.

**Tally**, counted from the ledger: **29 on the net or the docs, 19 in the product**, 48 rows.

**Open: [M11-R18](reviews/M11.md#m11-r18) and [M11-R38](reviews/M11.md#m11-r38)** — both maintainer
decisions about the budget's unit, plus the coverage questions recorded beside them (R23, R27, R30,
R35).

## 2026-09-05 — M11 round 11: the rendering nobody wrote down

### M11-R41 — a phone number in the form every API stores it

    client sends : Chiamami al +393471234567 oppure scrivi a mario.rossi@example.com
    upstream sees: Chiamami al +393471234567 oppure scrivi a [EMAIL_1]
    CONTROL      : Chiamami al +39 347 1234567 ...  ->  Chiamami al [PHONE_1] ...

The `+CC` family is two hand-enumerated groupings, both needing **two** separators, so the compact
**E.164** rendering — what an address book, a CRM export or a JSON payload contains — proposed no
candidate at all. Measured over 13 000 numbers libphonenumber confirms valid, in its own three
renderings: **E.164 0.918 missed, International 0.154, National 0.000**. `+55 11 91234 5678` was
worse than missed: `[PHONE_1] 5678`, four digits of a real mobile in clear. **Not a regression** —
byte-identical at every tag from `v0.4.0`, and of the 35 phone positives in this repo's corpus **not
one** is E.164.

The fix is a chokepoint rather than a third grouping: a `+CC` number carries its own country code, so
`phonenumber::parse(None, ..)` needs no region hint. It is the *easiest* case for the check the nine
domestic families already run, on the only phone family that had no validator. Measured over
358.4 MB: **5 masked spans → 83, all 83 real numbers**; unvalidated, 273, the extra 190 expanded-year
ISO dates and floats — which is why the arm is validated and not merely widened.

A **separate** recognizer, because the existing one has `validate: None` and gating it would narrow
the US 3-3-4 arm too, which masks `555-867-5309` on purpose. `RENDER-01` refused to go green until
the new recognizer had a rendering answer, which is the registry doing its job.

**And the structural limit it exposes matters more than the fix.** `RENDER-01` had a row for the
`+CC` recognizer, and it passed — because a row lists the renderings *somebody wrote down*, and the
compact form was in neither list. **An unasked question looks exactly like an answered one.** No
further guard closes that, because the missing entry is by definition the one nobody thought of; it
is now stated on the guard rather than left to be rediscovered.

### M11-R42 — and the README credited two arms with a check they do not run

*"The country's real numbering plan says the number is assigned"* is true of the nine domestic
families and now of E.164; it is **not** true of the spaced `+CC` groupings or the US 3-3-4 arm,
which mask on shape alone — `+99 999 999 9999` is masked though country code 99 belongs to nobody.
Both README tables now split the claim and name the direction: over-mask, never a miss.

### Numbers

**254 / 0 / 5** over 24 binaries, twice, identical; `cargo test-onnx` **290 / 0 / 22**; `fmt` and
`clippy --all-targets -D warnings` clean. The count does not move: the fix added a recognizer and a
registry row, and `RENDER-01` covers it — which is what a registry is for.

**Tally**, by the ledger's severity cell: round 11 is **2 in the product** — R41 `leak` and R42
`precision/docs`, whose leading term is `precision`, so it counts as product under the rule stated in
round 4's entry. Running total, recomputed from the ledger rather than carried forward: **26 on the net or
the docs, 17 in the product**, 43 rows. *(Earlier entries' running totals were
incremented by hand and drifted; this one is counted.)*

**Open: [M11-R18](reviews/M11.md#m11-r18) and [M11-R38](reviews/M11.md#m11-r38)** — both maintainer
decisions, both about the same thing: what the budget's unit costs and what to do about it.

## 2026-09-05 — M11 round 10: no leak, and two guards that were asking someone else's question

Round 10 could not break round 9's fix — 93 600 generated cards x published groupings x 18 following
tokens, 0 leaks; 15 120 cells varying the *preceding* token as well, 0. **No leak in `src/`.** Three
findings, one in the product and escalated.

### M11-R38 — the budget's unit is not a constant, and the worst legal body is the one that masks nothing

`ARCHITECTURE.md` published `units x ~3 µs => ~1.5 s`, taking its multiplicand from the cheapest
corner of `DOS-BUD`'s grid — while the same file says *a rejection is the expensive verdict*, because
`.any()` short-circuits on **accept**. Measured, the unit spans roughly **3.4 µs to 30 µs**, and a
zero-padded key column — `LPAD`-ed ids, which every ORM emits — is refused by the real binary only
after **14.6 s**. Fail-closed, not a regression, and *not* an M11 regression either: `v1.2.1` answers
the same body in 12.2 s.

All four remedies change product-visible behaviour, so the finding is **escalated**, and it does not
join M11-R18 so much as **re-price** it: the worst legal shape is no longer the lowercase-IBAN body
in term 3, it is this one in term 1.

### M11-R39 — the axis `DOS-BUD` never varied was the verdict

Every row of that grid was built from **valid** numbers. Adding the axis, on `--release`:

    3XX XXX XXXX (valid)     5 000 rows   40 000 spent    8 units/row   5 000 masked
    0NNNN NNNN id column     5 000 rows   60 000 spent   12 units/row       0 masked

The rejecting shape costs half again as much per row and **masks nothing**, so it reaches the
allowance on a *smaller* body — refused at 50 000 rows / 3.6 MB against the valid column's 100 000
rows / 7.5 MB. **The worst legal body is one whose candidates are all rejected.** That is the
opposite of what a grid of valid numbers encourages you to think, and it is why R38 re-prices R18.

**Two traps this file already names caught its author on the way.** My first version of the row
asserted *"rejected in all regions"* in a comment and measured **4 145 of 5 000 masked** — the shape
was mostly valid — on an odometer that repeated every 10 000 rows, so the memo served most of it free
and the cost read as sub-linear. Distinct now, and the label states the **shape** rather than a
verdict nobody had measured: the same correction M11-R34 made to prose, one file and one day over.

### M11-R40 — a guard scoped by the wrong registry

`UTF8-01` asks about **digits** and took its corpus from `CASE_ANSWERS`, which is scoped by
`pattern_can_match_a_letter` because letter case is *its* axis. So **12 of 24** recognizers were
outside it — the card, the universal phone, the SSN, the FR NIR, the 9- and 11-digit ids, the LV
dashed code, the bare P.IVA and all four domestic-phone families: precisely the patterns made of
nothing but the Unicode `\d` the guard exists for. No live defect behind it, and now derived from
`RENDERING_ANSWERS`: **932 substitutions became 2 168**, matching the round's own probe exactly, with
the floor moved from 500 to 1 500 so the widening cannot be undone silently.

M11-R31 one axis over, and the two together are the rule: **a guard takes its scope from the registry
whose question it asks, not from whichever registry is nearest.** There is now one registry per axis
and one all-recognizers registry underneath them.

### Numbers

**254 / 0 / 5** over 24 binaries; `fmt` and `clippy --all-targets -D warnings` clean. No test count
moved — both fixes widened existing guards rather than adding siblings.

**Tally**, by the ledger's severity cell: round 10 is **1 in the product** (R38 `hardening`, open) and
**2 on the net or the docs**. Running total: **27 on the net or the docs, 14 in the product**, 41
rows.

**Open: [M11-R18](reviews/M11.md#m11-r18) and [M11-R38](reviews/M11.md#m11-r38)** — one decision in
two rows, now with the coverage questions that accumulated beside them (R23, R27, R30, R35).

## 2026-09-05 — M11 round 9: my own fix leaked, and nothing could tell the two builds apart

Round 8 closed the card's 19-digit truncation by giving the `4-4-4-4` arm an optional 1-3-digit
tail. Round 9 found that the tail leaks.

### M11-R33 — the same mechanism as M11-R13, one recognizer over and one commit later

    sent : carta 4111 1111 1111 1111 123 e mail a.b@example.com
    seen : carta 4111 1111 1111 1111 123 e mail [EMAIL_1]      <- the email is the control
    sent : carta 4111 1111 1111 1111 e mail a.b@example.com
    seen : carta [CARD_1] e mail [EMAIL_1]

A card beside its CVV, a row index, a spreadsheet column. The tail is **greedy**, so the span matches
as 19 digits, `credit_card_valid` rejects it about nine times in ten, and `CreditCard` still carried
`shrink_on_reject: false` — so the rejected candidate was discarded and nothing shorter was proposed
at the same start. **152 of 200 measured cards in clear, 31 more truncated**, and the suite stayed
green at 254/0/5.

**What stopped me seeing it was the field's own doc**, which said a *checksum* recognizer must not
shrink because a shorter prefix of a non-Luhn-valid run can be Luhn-valid by coincidence. That was
written when a shrunk prefix could be any cut of the span. Since **M11-R22** it must be a full match
of the recognizer's own pattern, so the only prefixes the card can shrink to are complete card
renderings, Luhn-checked exactly as the un-shrunk span would be — the accepted rate, not a new one.
I read a rule I had rewritten two rounds earlier and did not notice it had stopped applying.

The form that generalises is now on the field: **a pattern whose reach exceeds what its validator
accepts must shrink.** It is a property of the *pair*, not of a recognizer — reading M11-R13 as "IBAN
needs this" is what let it recur.

**And round 9's sharpest observation is not the leak but that no test saw it.** Flipping the flag
took 152 leaking cells to 0 and nothing went red. A rendering is not detected in isolation; it is
detected in a sentence, and what follows it decides whether a greedy arm over-reaches into a
rejection. `RENDER-01` now drives every recorded rendering followed by six neighbours and asserts
**no six-character window** survives — M10-R1's predicate, *no byte of the value survives*, rather
than *one span equals the value*, since a legitimate coalesced over-mask covers more than the value
and must stay green. Six and not four because four produced a false positive on the guard's first
run: `" 12 "` is a window of `AB 12 34 56 C` and the neighbour `" 12"` rebuilds it unaided.

### M11-R34 — a field that asserts nothing while the thing it asserts lives in the sentence beside it

Three of the 24 `RENDERING_ANSWERS` rows named a published, undetected rendering in `why` while
`not_detected` was empty — in the commit that introduced the idea, with nothing to notice. The first
is worse than unasserted: `86 095 742 719` reaches the provider as `86 [PHONE_1]`, two digits in
clear and the remaining nine announced to the model as a **phone number**.

The audit gained the missing half, mechanically: every backtick-quoted token in `why` that looks like
a rendering must appear in one of the lists. It caught all three on its first run — and then caught
my first correction, which backticked the sentence `Steuer-ID 86 095 742 719` instead of the
rendering. Correctly: a quoted string it cannot match against a list is precisely the ambiguity the
assertion removes.

**Prose cannot be audited; a list can.**

### M11-R35 — and the one row that filled the field correctly gave a false reason

The `Ssn` row said the compact form is covered by the always-on 9-digit recognizer. That recognizer
is checksum-gated, so it accepts ~2 in 11 arbitrary nine-digit runs: a compact SSN is masked when it
happens to satisfy a Dutch or Portuguese checksum and forwarded otherwise. A real gap on a headline
PII type, now stated with its rate and escalated rather than papered over.

Worth more than the row: it was the **only** one of 24 that used `not_detected` correctly, and its
reason was still false. A filled field is not evidence that the sentence beside it is true, and
M11-R34's new assertion cannot catch this class either. Only reading it can, and that limit is
written on the guard.

### M11-R36 / M11-R37

The comment above the card recognizer described the **general** arm — the one measured and *not*
taken. Same class as M11-R32 one file over: a comment describing an intention rather than the code
beside it. And `[Unreleased]` said nothing about the card grouping change; it now does, with R33
folded into the same entry because it is the same release and the same value.

### Numbers

**254 / 0 / 5** over 24 binaries, twice, identical; `cargo test-onnx` **290 / 0 / 22**; `fmt` and
`clippy --all-targets -D warnings` clean. No test count moved — `RENDER-01` gained two assertions
rather than a sibling, which is the shape a guard should grow in when the axis is already registered.

**Tally**, by the ledger's severity cell: round 9 is **2 in the product** (R33 `leak`, R35
`precision`) and **3 on the net or the docs**. Running total: **25 on the net or the docs, 13 in the
product**, 38 rows.

**Two rounds in a row have found a leak in the previous round's fix** (R30 -> R33 after R10 -> R13),
and both times the mechanism was a widened pattern in front of a rejecting validator. That is not the
loop feeding itself: it is one class being met repeatedly because the fix for it kept being recorded
as a fact about a recognizer instead of about a pair. It is now on the field, in `ARCHITECTURE.md`,
and asserted by a guard that drives values in sentences rather than alone.

**Still open: [M11-R18](reviews/M11.md#m11-r18)**, with the coverage decisions that have accumulated
beside it — R23's glued IBAN, R27's `\p{Nd}` gap, R30's national-identifier groupings and the general
card arm, R35's compact SSN.

## 2026-09-04 — M11 round 8: a rendering has two coordinates, and round 7 closed one

Round 7 closed the separator axis by widening **which character** may sit between a value's groups.
Round 8 varied the other coordinate — **where the groups fall** — and found the card recognizer
forwarding values in clear.

### M11-R30 — Amex's own grouping, and a truncation that is worse than a miss

Through the real `.exe`, one request with its own control:

    client sends  : Amex 3782 822463 10005 and Visa 4111 1111 1111 1111
    provider gets : Amex 3782 822463 10005 and Visa [CARD_1]

    client sends  : Amex 378282246310005 compact
    provider gets : Amex [CARD_1] compact

The same Amex number compact **is** masked, so the value is recognised and Luhn passes; what is not
recognised is 4-6-5, the grouping Amex itself prints. And the 19-digit row is worse than a miss:
`4111 1111 1111 1111 110` had its first sixteen digits matched and the last three **forwarded** —
`[CARD_1] 110` — which is M10-R1's *"the mask ate it"* shape, the one the fixpoint cannot undo.

The card arm offered exactly one grouping while `credit_card_valid` already accepted any grouping of
13–19 Luhn-valid digits. That is `ARCHITECTURE.md`'s own sentence — *where the validator is more
permissive than the pattern, the difference is a set of renderings that reach the provider in clear*
— read on the coordinate the sentence did not name. Not a regression: the card's one grouped arm is
byte-identical at every tag since M1, and the whole card corpus has been two 16-digit Visa numbers in
4-4-4-4 for as long. *A corpus has a shape, and that shape is a blind spot.*

**I took the narrow fix, and the reason is a measurement the round could not have had.** The
chokepoint version — `\d{1,6}(?:[sep]\d{1,6}){1,4}`, letting the validator own the grouping, which
the round measured at +24 matches on 422.9 MB and which I preferred on reading it — comes back:

    both adjacent numbers must be detected separately,
    got [(CreditCard, "020 7946 0958 0161 496 0000")]

In digit-dense text the general arm finds several Luhn-valid sub-runs, `push_candidates` coalesces a
recognizer's overlapping hits into maximal runs, and two merge into one span that swallows both
phone numbers — breaking M8.1's promise that two adjacent numbers yield two placeholders. **A match
count cannot show that; it is a span-fidelity cost.** So what ships is the groupings issuers publish
(4-4-4-4 with an optional 1–3-digit tail, which closes the truncation; Amex/Diners 4-6-4/5; compact),
measured at **0 added matches**, and the general arm stays escalated with the new number attached.

This is knowingly the list rather than the chokepoint, and it is written down as such — because here
the class-wide fix breaks a shipped promise, and the alternative to saying so is a comment claiming
we thought about it.

### M11-R31 — and the guard that could not have caught it

`SEPARATOR-01` derives its scope from the pattern: *can this pattern match a gap?* For the separator
**character** that is the right question. For the **grouping** it is not — it exempted **16 of 24**
recognizers by construction, and a recognizer that offers no grouping is exactly the one whose
missing grouping is invisible that way.

`RENDER-01` asks the question of the **value** instead, so all 24 must answer, and the audit names
any that does not. It carries the `CaseRule::Fixed`-shaped way to say *no* that the separator
registry lacked: `not_detected` lists renderings the value **is** published in and this build
deliberately does not detect — the hyphenated ES DNI, the spaced FR NIR, HMRC's 3-4-2 GB VAT, PT's
3-3-3 — **asserted absent**, so widening coverage rewrites the answer instead of letting a limit
drift into folklore. Each carries its cost, and each is escalated: for `\d{9}`, admitting
`524 287 244` would make every three-column numeric table a candidate under a checksum that already
accepts ~2/11 of arbitrary values, and the honest answer may well be a documented refusal.

**Its span assertion is the half that separates a leak from a miss.** Each detected rendering must be
one span covering the **whole** value — a presence assertion calls `[CARD_1] 110` masked.

### M11-R32 — the third stale status in that paragraph, and the rule that ends it

`ARCHITECTURE.md` said *"The separator axis is the third, and it is not closed"* in the commit that
closed it — **inside the clause M11-R26 had added to prevent exactly that**: *"deliberately not
restated here so this paragraph cannot go stale."* The clause was right and the sentence next to it
asserted a status anyway.

So the fix is not a third correction. It is the rule: **a status is a fact about today; put it in the
ledger and link it, never in prose.** The paragraph now states the four axes as *questions to ask* —
case, digit script, separator, grouping — names the finding behind each, and asserts no status at
all, with all three stale-text findings cited beside it.

### Numbers

**254 / 0 / 5** over 24 binaries, twice, identical (round 8 measured 253; the +1 is `RENDER-01`);
`cargo test-onnx` **290 / 0 / 22**; `fmt` and `clippy --all-targets -D warnings` clean.

**Tally**, by the ledger's severity cell: round 8 is **1 in the product** (R30, `leak`) and **2 on
the net or the docs** (R31 `guard`, R32 `docs`). Running total: **22 on the net or the docs, 11 in
the product**, 33 rows.

**Still open: [M11-R18](reviews/M11.md#m11-r18)**, now with three neighbours wanting the same kind of
decision — the grouped glued-IBAN residue (R23), the `\p{Nd}` coverage gap (R27), and R30's
national-identifier groupings plus the general card arm.

## 2026-09-04 — M11 round 7: the separator, and the same sentence for the third time

Round 7 found a leak by varying the axis round 6 opened but did not finish: after the letters' case
and the digits' script, **the separator between a value's groups**.

### M11-R25 — a value separated by a non-ASCII space goes upstream in clear

Through the real `.exe` against a mock upstream:

    client sends : IBAN DE89⍽3704⍽0044⍽0532⍽0130⍽00, carta 4111⍽1111⍽1111⍽1111,
                   tel 020⍽7946⍽0958, mail a.b@example.com          (⍽ = U+00A0)
    provider gets: …the same three values byte for byte… mail [EMAIL_1]

The email is the control. **432 of 1 080 cells leaking**, predating M11 by ten milestones.

Every pattern spelled its gap as a literal ASCII space while `iban_mod97`, `luhn_valid` and
`nino_prefix_valid` all filter on `char::is_whitespace`, which accepts U+00A0, U+2007, U+202F,
U+205F and tab. **The validators were written for input the regexes could never deliver** — which is
M11-R10's sentence word for word, one axis over, and M11-R21's with the sign flipped. Three findings,
one rule: *a recognizer and its validator must agree on the alphabet of every axis, and where the
validator is the more permissive the difference is not slack — it is a set of renderings that reach
the provider in clear.*

**The fix is one class with three consumers**, so the two halves cannot drift apart again:
`GAP_CHARS` (Unicode `Zs` plus `\t`; `\r`/`\n` deliberately out, since a value must not span lines)
feeds the patterns' character class, the shrink's cut alphabet, and the phone validator's
normalisation. Patterns hold a `{GAP}` template expanded at construction, written `\x{..}`-escaped
so the pattern string stays ASCII and `CASE-01`'s ASCII assertion keeps speaking for it.

**And the second half is the one a pattern-only fix would have shipped without.** Widening the
patterns closed IBAN, the card, the universal phone and the NINO for every separator — and left the
**domestic phone tier at 2 of 5**, because `phonenumber`'s own normalisation accepts U+00A0 and
U+3000 and rejects U+202F, U+2007 and `\t`. So `020⍽7946⍽0958` matched the regex, reached the
validator, and was *refused*: a miss dressed as a validation, the same shape as the leak itself. With
`national_phone_valid` normalising through `GAP_CHARS` first: **5 of 5, all nine families**. Measured
cost of the whole widening, on the reference corpus: **0 added matches** — so unlike the case fold
(149 added, which needed `iban_case_gate`) this one needs no gate.

The finding offered two variants; the difference is whether `\t` is in the class. I took the one
that includes it — both measure 0, a TSV `tool_result` is ordinary agent traffic, every family
behind it is checksum- or plan-gated so the worst case is an over-mask, and it is reversible by
deleting one character literal.

**`SEPARATOR-01` carries all three halves M11-R11 established** — audit, matrix, reachability — and
its set is *derived*: the letter test became `pattern_can_match_any_of(pattern, wanted)`, called with
`ASCII_LETTERS` for one axis and `GAP_CHARS` for the other. One derivation, two axes. That is why
this took a morning where M11-R10 took a milestone, and it is the argument for building the
chokepoint rather than the list, made in numbers rather than in principle.

**Two guards went red on the way, and both were right to.** `CASE-01` unbound the moment the IBAN
and NINO patterns changed — exactly the forcing function M11-R11 built — so the answers now hold the
`{GAP}` template and the audit expands it, keeping "editing a pattern re-opens its answer" while not
demanding that widening `GAP_CHARS` re-paste four blobs. And `PHONE-NAT-08`'s generator compiled a
raw template; it fails loudly, so the fix is the const's **name**, `PHONE_SHAPE_TEMPLATES`, not a new
guard.

### The other four

- **M11-R26** — `ARCHITECTURE.md` still said *"Nothing enforces this yet"* about M11-R21 and linked
  it as **open**, a round after `UTF8-01` closed it. **Second time an M11 closure missed that file**,
  and the two misses have one cause: a closure edits the code, the guard and the record, and
  `ARCHITECTURE.md` is the one no test and no ledger row points at. Nothing mechanical can supply a
  trigger — the file states rules, not ids — so both invariants added this milestone now **link the
  ledger for their status** instead of restating it, which makes a stale *"open"* impossible rather
  than unlikely.
- **M11-R27** — *"matching `\p{Nd}` is what lets a value written in Arabic-Indic or fullwidth digits
  be **detected**"* was published in three files and my own round-6 DEVLOG entry. Measured over 3
  digit blocks × 17 values it is **false for all thirteen validated recognizers**: the pattern
  matches and the validator rejects, because every checksum here folds ASCII digits only. True of the
  universal `Phone` alone, which has no validator. So the reachable consequence of the Unicode `\d`
  was R21's panic and a **coverage gap** — a fullwidth-digit card is not masked. Corrected in all
  four places, and the gap is named beside M11-R18 rather than closed quietly, since normalising
  `\p{Nd}` before validation is a coverage decision.
- **M11-R28** — the CHANGELOG never mentioned that a 30-byte request returned `500` on every release
  ever cut. Added, with the fact an operator needs: *refused, not leaked*. The separator leak went in
  beside it.
- **M11-R29** — `iban_length_ok` compared an ISO 13616 **character** length against a **byte** count.
  Latent, held only by `iban_mod97`'s short-circuit — and M11-R18's option 3 proposes reordering
  exactly those two checks, which would have made it live. Fixed now, while it costs nothing, rather
  than as a surprise inside whichever option is picked. The slip is older than the rewrite that
  carried it: the pre-M11-R18 body compared `compact.len()`, also bytes.

### Numbers

**253 / 0 / 5** over 24 binaries, twice, identical (round 7 measured 252; the +1 is `SEPARATOR-01`);
`cargo test-onnx` **289 / 0 / 22**; `fmt` and `clippy --all-targets -D warnings` clean.

**Guard-vs-product tally**, by the ledger's severity cell: round 7 is **2 in the product** (R25
`leak`, R29 `hardening`) and **3 on the docs**. Running total: **20 on the net or the docs, 10 in the
product**, 30 rows.

**Still open, and still the only thing between here and the tag:
[M11-R18](reviews/M11.md#m11-r18)** — now with two neighbours that want the same decision: the
grouped glued-IBAN residue (M11-R23) and the `\p{Nd}` coverage gap (M11-R27).

## 2026-09-04 — M11 round 6: the alphabet, the axis M11 had never varied

Round 6 found **no leak in `src/`** and four findings, three of them in the product — the first round
since round 4 that is not mostly about guards. It got there by varying the one axis every earlier
round held constant: **the alphabet**. Rounds 1-5 all ran on ASCII.

### M11-R21 — a panic in every tag ever cut, fixed at HEAD by accident two days ago

`\d` is Unicode-aware. M4-R13 de-Unicoded the word *boundary* and correctly left `\d` alone —
matching `\p{Nd}` means such a value **matches the pattern**. *(Corrected 2026-09-04 by M11-R27:
this line said it is what lets such a value "be detected rather than forwarded in clear", and that
is false for all thirteen validated recognizers — the validator rejects what the pattern matched,
because every checksum here folds ASCII digits only. It is true of the universal `Phone` alone,
which has no validator.)* The consequence nobody drew is that a matched span is then a `&str`
whose byte length is not its character count, and `iban_mod97` byte-sliced it (`&compact[..4]`).
Through the **real v1.2.1 binary**, a 30-byte unauthenticated request —
`{"content":"Account AB𝟎𝟏ABCDEFGHIJK please"}` — returns **HTTP 500 with nothing forwarded**:
`panicked at 'byte index 4 is not a char boundary'`. The IBAN pattern is byte-identical at every tag
ever cut.

Fail-closed and never a leak, which is exactly why it survived. **And it is already fixed at HEAD —
by `831f916`'s allocation-free rewrite, which iterates `chars()` and never indexes, and whose
differential proof ran on ASCII groups and so could not have noticed.** Nothing pinned the property:
reverting that one function left the suite at 250/0/5.

`UTF8-01` pins it now, with its corpus **derived from `CASE_ANSWERS`** — one registry, two guards, so
a new letter-bearing recognizer is exercised on this axis the moment its case answer is recorded.
Four `\p{Nd}` digits of 2, 3, 3 and 4 bytes into every character position of every positive: **932
substitutions**, a number read off a run rather than estimated (I first wrote 896, which was a guess,
which is M11-R19 one round later and in my own hand). Reverting `iban_mod97` byte-for-byte from
`git show 831f916^` turns it red with the round's own panic message.

### M11-R22 — my own invariant, true about the wrong set

Three places carried this sentence, and I wrote it: *"the shrink cannot admit anything the
pre-M11-R10 build did not already admit."* It is false. `iban_case_gate` short-circuits `true` on any
span with no lowercase byte **without arithmetic**, so `Registers AB12 cafe babe dead beef are
clobbered` walks back one group at a time and stops at the bare **`AB12`** — masked at HEAD,
untouched by v1.2.1, verified through both real binaries.

The argument quantified over *the validator's verdict on a prefix*, and on that set it holds. But the
shrink also changes **which spans exist to be judged**: the emitted set went from *"the spans this
regex matches"* to that set **union their group-boundary prefixes**. *An invariant is only as strong
as the set it quantifies over* — M4's third lesson, landing on the fix for its second one.

**Closed by making the sentence true rather than by softening it**, which was the option I rejected:
a claim that the shrink is free is what the next person adding a rejecting validator will read, and
*"except sometimes"* leaves them no rule. `shrink_to_a_valid_prefix` now requires each prefix to be a
**full match of the recognizer's own pattern**. It touches the phone tier too, so that was measured
rather than assumed: **252 / 0 / 5**, every `PHONE-NAT` guard included.

`SHRINK-01` checks it differentially against `shipped_patterns()`. Its first version demanded a full
match and went red on `020 7946 0958 0161 496 0000` — a legitimate **coalesced maximal run** — so it
asks for a match *at the start*, and that limit is written on the guard rather than discovered again.
Rate of the original defect on real traffic: **0** over 319.7 MB and 0 over the M7 turn. The finding
was never the behaviour; it was that a false invariant is worse than an absent one.

### M11-R23 — the same sentence mis-scoped a second time, in the edit that fixed the first scoping

The glued residue handed to the maintainer read *"an IBAN glued to 1-4 alphanumerics yields no
candidate"*. R20 had fixed that sentence on the **case** axis one round earlier and left it wrong on
the **rendering** axis. The reason given is a property of the *continuous* arm; the claim is made
about *an IBAN*. For the **grouped** arm — the rendering banks print, and the guard's own helper is
called `in_groups_of_four` — a candidate **is** produced, stopping at the last complete group:

    in     : Please wire the deposit to ad87 0123 4567 8901 2345 6789abcd and confirm.
    masked : Please wire the deposit to ad87 [PHONE_1] 2345 6789abcd and confirm.

Country code, both check digits and two whole groups — **ten consecutive bytes** — in clear. That is
the output shape M11-R13 was filed as a **leak** for, word for word; the only difference is that
R13's trailing token is separated and this one is glued. **Not a regression** — the identical matrix
against a `v1.2.1` build is byte-identical in all 576 cells — but a pre-existing open residue whose
escalation read as *"nothing happens"*.

Restated with both arms, both axes and the numbers (**236 of 288** grouped rows leave 4+ bytes). And
`IBAN-05`'s window went from **eight** bytes to **four**: eight could not report a residue shorter
than eight while the doc claimed M10-R1's *no byte* predicate. Four is a group, and the threshold this
round measured with. One and two were tried and cannot pass — a single character or a pair
legitimately occurs in the carrier sentence — so the limit is measured and written down.

The lesson, since the same sentence was wrong twice in two rounds: **state a limit as a matrix over
its axes, not as a sentence with an example in it.**

### M11-R24 — and the tally was wrong too

Two DEVLOG entries reported guard-vs-product counts that did not add up to the ledger, and one called
M11-R18 — an open `hardening` row on `recognizers.rs` that blocks the tag — not-in-the-product. Both
corrected in place with a note saying what they used to read. The arithmetic was the smaller half:
**nothing said what counted**, so the fix is a stated rule carried in both entries — the split is read
off the **ledger's severity cell**, `leak`/`fidelity`/`hardening`/`precision` against
`build`/`guard`/`docs`, which anyone can check from `ROADMAP.md` in a minute.

### Numbers

**252 / 0 / 5** over 24 binaries (round 6 measured 250; the +2 are `SHRINK-01` and `UTF8-01`); `fmt`
and `clippy --all-targets -D warnings` clean.

**Guard-vs-product tally, under the rule above.** Round 6: **3 in the product** (R21 `hardening`, R22
and R23 `precision`), 1 on the docs. Running total: **17 on the net or the docs, 8 in the product**,
25 rows. The ratio has moved twice now — rounds 1 and 2 were 9 net findings to 1 product, rounds 4-6
are 8 net to 7 product — and the reason is legible: the early rounds were fixing a test net that could
not go red, and once it could, the rounds started finding things in the product instead. That is the
loop working rather than feeding itself.

**Still open, and still the only thing between here and the tag:
[M11-R18](reviews/M11.md#m11-r18)**, plus the pre-existing glued-IBAN residue R23 measured, which
wants the same decision.

## 2026-09-04 — M11 round 5: the fix held, what it cost had never been measured

Round 5 attacked M11-R13's fix on the axis its own guard holds constant and **could not break it**:
0 leaks over 3 360 separated renderings, including six leading tokens that can themselves start an
IBAN match. Three findings, none a leak. What the round is really about is that **the fix's price was
never measured** — not by me when I shipped it, and not by the round that prescribed it.

### M11-R18 — an unbudgeted term nobody had counted, and it is the maintainer's call

`shrink_on_reject: true` retries a rejected IBAN span at every interior separator — up to eight
`iban_case_gate` calls per match — and that validator is wrapped in `free()`, so **none of it is
charged to the request budget**. The wrapper's own doc justifies the exemption as *"a checksum over
at most 18 bytes"*, which was true while each validator ran once per match and is not true now: an
IBAN span is up to 44 characters and the shrink runs it eight times. On a body of *distinct* groups
the per-scan memo is inert, so every call is a miss.

Measured through the real binary: a legal 14.6 MiB request takes **8–10 s** where the same body
uppercased takes 2–3 s. `ARCHITECTURE.md` published *"across every shape measured, a request costs at
most about 3 s"* and a **two**-term CPU model. Both are now false — there is a third term, per
*candidate* rather than per byte, and the M11-R13 fix made it the largest of the three for a legal
body. This is a constant factor on a linear path, not a blow-up and not a leak, but availability is a
privacy property here in this repo's own words.

**All four options are product-visible — a new refusal on legal traffic, a retry cap that changes
what is detected, or republishing the ceiling — so the decision is the maintainer's and the finding
stays open.** What did not need an answer was done:

- **`iban_mod97` and `iban_length_ok` are now allocation-free.** The first built a compacted `String`
  and then a `format!`ed rearrangement — two allocations per call, a fair price while the validator
  ran once per match. Same arithmetic, same verdicts. Median of 9 on 4 MiB of distinct lowercase
  groups, with the uppercase control that proves it is the case-fold path and not the machine:
  **2 573 ms -> 2 108 ms** (min 2 012 -> 1 624), control flat at 694 -> 671 ms, **span counts
  byte-identical at 56 729**. ~18% off, and it does **not** close the finding — the lowercase path is
  still ~3x its own control.
- **Option 3 as the round framed it was measured and rejected.** Checking the ISO 13616 length before
  mod-97 buys nothing on the shape that matters: `iban_length_ok` answers `true` for a country code
  it does not know, and on random two-letter prefixes ~95% are unknown, so it rejects nothing
  earlier. A pre-filter that does not filter is M10-R13 wearing a different hat.
- **`ARCHITECTURE.md` was corrected regardless of the decision.** The model names three terms, states
  the measured numbers, says plainly that "at most about 3 s" no longer holds, and points at the open
  finding. Leaving a knowingly false availability guarantee standing is M11-R14's defect; correcting
  a number is not choosing an option.

**One number the round did not have, found while attributing the cost:** the shrink's price is not
only time. On the same body it takes the masked-span count from **7 943 to 56 729** — **7.1x more
over-masking** on an adversarial shape. All over-masks, restored byte-identically, so the direction
is safe — but it changes the balance between the options, because capping the retries would buy that
back as well as the time.

### M11-R19 — a number used to justify a design decision in five places did not reproduce

`iban_case_gate`'s residue was published as **0** in five places and promised to operators in the
CHANGELOG. Measured on 304.9 MB it is **1 of 936**: `ab22 ab23 ab44 ab45 ab66 ab67`, a matrix
kernel's SIMD register list, masked as `[IBAN_1]`.

The finding is not the behaviour — it is an over-mask, restored byte-identically, at ~1 per 300 MB.
It is that **the one place which reasoned about how the zero could fail concluded that it could
not.** `iban_length_ok` answers `true` for an unknown country code, so such a span is gated by mod-97
alone and one in 97 passes: the **expected** residue was `added / 97` ≈ 9.6. A measurement whose
expected value is ~10 should never have been published as 0 without asking why. All six sites now
carry the rate, its corpus, the survivor, and that expectation — so the number and the reasoning
cannot drift apart again.

### M11-R20 — the escalated limit was scoped to the wrong half

The residue handed to the maintainer said *"a **lowercase** IBAN glued to 1-4 alphanumerics"*.
A canonical **uppercase** IBAN glued to a lowercase token has the identical fate: one lowercase byte
anywhere in the glued token makes the gate demand verification of the over-long span, whatever case
the IBAN is written in. Restated without the scoping.

### Numbers

**250 / 0 / 5** over 24 binaries; `fmt` and `clippy --all-targets -D warnings` clean. No test count
moved this round — the work was measurement and documentation, plus one allocation-free rewrite whose
correctness is proved differentially (identical span counts) rather than by a new assertion.

**Guard-vs-product tally.** Round 5: 3 findings, and by the ledger's own severities **2 of them
are in the product** — R18 is `hardening` on the shipped binary's availability and R19 is
`precision` on what it masks; only R20 is docs. Running total across M11: **16 on guards or docs, 5
in the product**, 21 rows. *(Corrected 2026-09-04 by M11-R24: this entry originally said 16 + 3 =
19, and called M11-R18 — an open finding on `recognizers.rs` that blocks the tag — not-in-the-
product. The split is read off the **ledger's severity cell**, so it is checkable rather than a judgement call: *product* = `leak` / `fidelity` / `hardening` / `precision`; *net or docs* = `build` / `guard` / `docs`. Stating the rule is half the fix — M11-R24 was two wrong tallies, and they were wrong because nothing said what counted.)* The trend is still the loop doing what it is for. Rounds 3 and 4 each found a leak; round 5 found none, and spent itself on numbers that had
been published without being checked.

**Open, and the only thing between here and the tag: [M11-R18](reviews/M11.md#m11-r18).**

## 2026-09-04 — M11 round 4: the case-axis fix leaked, and the gate that caused it

Round 4 came back with five findings and the sharpest was in the product: **the M11-R10 fix
relocated the leak it closed.** The other four are the same fix's paperwork — a refuted rationale
still standing, a grammar helper invalidated by the very change its doc named as its invalidation
condition, an unpinned half of the fix, and a decision that never reached the two files a reader
outside the review loop would open.

### M11-R13 — a real IBAN dropped entirely, by the gate added to close a leak

```
client sends : Please wire the deposit to ES91 2100 0418 4502 0005 1332 for the invoice.
provider gets: Please wire the deposit to ES91 2100 [PHONE_1] 1332 for the invoice.
```

Country code, both check digits and the last group forwarded verbatim; the middle announced under
the wrong kind. Three lines that were never all true at once before M11: the grouped arm ends
`(?: [A-Za-z0-9]{1,4})?`, so an IBAN whose compact length is a **multiple of 4** leaves that tail
unspent and the match runs into the next short token; `iban_case_gate` — added by M11-R10 — then
*rejects* the over-long span for the lowercase byte the swallowed word contributed; and
`shrink_on_reject: false` meant no shorter prefix was tried. No candidate at all, so the resolver
never learned the value existed.

**Proved a regression by build, in three runs of the same probe.** At HEAD the sentence above yields
`[(Phone, "0418 4502 0005")]`. With `iban_case_gate` neutralised it yields `[(Iban, "ES91 … 1332
for")]` — the benign over-match, an over-mask. With the **exact** pre-M11-R10 recognizer restored
(uppercase-only pattern, `validate: None`) it yields `[(Iban, "ES91 … 1332")]`, because a pattern
that cannot match a lowercase byte can never swallow an English word. Folding case made the
over-reach possible; the gate turned it into a deletion.

**Fixed with `shrink_on_reject: true`**, the mechanism M10-R1 built for exactly this and which a doc
comment about *checksum* recognizers had excluded IBAN from. IBAN is not one: the gate accepts
unconditionally unless the span carries a lowercase byte, so every prefix the shrink tries either
carries no lowercase — accepted structurally, exactly what shipped for ten milestones — or must pass
mod-97 **and** the ISO 13616 length. **The shrink cannot admit anything the pre-M11-R10 build did
not already admit**, and that argument now lives on the field rather than in this entry.

**The guard is the predicate, not another example.** `IBAN-05` asserts M10-R1's rule — *no byte of
the value survives masking*, not *nothing detectable survives* — which the phone tier has had since
M10 and IBAN never got. Its corpus is **derived**: all 676 two-letter codes probed against
`iban_country_length`, every country it knows synthesised into a valid IBAN with real check digits
solved for and re-verified, driven in both renderings × three letter cases × six trailing contexts.
It carries a non-vacuity floor in the dimension that matters — at least ten countries whose length is
divisible by 4, of which there are 14 — because `IBAN-04`, the guard *named* for this behaviour, held
`IT60X…` (27) and `DE89…` (22): the two lengths that cannot over-reach. The repo's whole IBAN corpus
was DE/IT/FR/LV/NL/ZZ, not one vulnerable length in it.

Removing the fix turns `IBAN-05` red on a **worse** case than the one reported: with the carrier
`" and confirm."` the AD IBAN is not masked at all.

**Decided limit, and it is not a regression.** A lowercase IBAN glued to 1-4 alphanumerics
(`es9121000418450200051332abcd`) still yields no candidate — no separator to cut at. The pre-M11-R10
build produced no candidate for it either, its uppercase-only pattern being unable to match the
string. Closing it needs a length-derived cut whose false-positive cost is unmeasured, so it goes to
the maintainer rather than being decided here.

### The other four, and one of them was green on the first attempt

- **M11-R14** — the refuted uppercase-only rationale was still standing in three places, two of them
  describing the leak as **current behaviour** and the finding as *open*, and the third citing a test
  that no longer exists. These are not stale trivia: they are the argument for reverting the fix,
  written in the voice of a decision, where the next person to touch the patterns will read them.
- **M11-R15** — `vat_grammar_could_match` still asserted `[A-Z]{2}`, and its own doc named *"a
  pattern that stops being `[A-Z]{2}`-prefixed makes this function wrong"* as its invalidation
  condition. That happened in `33eb159`. Now `is_ascii_alphabetic`, with `es12345678z` moved into the
  **reachable** list where it belongs and a `1S12345678Z` row added so the weakened rule keeps a
  floor.
- **M11-R16** — `confidence_of`'s NL case fold, a named half of the M11-R10 fix, was pinned by
  nothing. Closed at the chokepoint: `CaseRule::Folds` now asserts kind, span **and `Confidence`**,
  so every recognizer with a confidence split is covered rather than NL alone. **The first attempt at
  that was green**, and that is the part worth keeping: with the only NL positive being one whose
  11-proef *passes*, both branches of `confidence_of` return the same thing and a broken fold is
  invisible. A second row carrying the `Structural` side makes it red. The rule, written beside the
  two rows: an answer must carry a positive from **each** branch of a confidence split.
- **M11-R17** — the case decision had reached the code and the review archive and neither
  `ARCHITECTURE.md` nor `CHANGELOG.md`. Waiting on R13 was right: the invariant it produced —
  ***a validator that rejects must not be able to delete a value*** — could not have been written a
  day earlier.

### Numbers

**250 / 0 / 5** over 24 binaries, re-run twice with identical counts (round 4 measured 249; the +1 is
`IBAN-05`). `cargo test-onnx` **286 / 0 / 22** (round 4: 285). `fmt` and `clippy --all-targets -D
warnings` clean.

**Guard-vs-product tally.** Round 4: **1 in the product** (R13, a leak) and **4 on the net or the
docs**. Running total across M11: **15 on the guards or docs, 3 in the product** — 18 rows, the
ledger's length at this point.

*(Corrected 2026-09-04 by M11-R24, which found this line reading 14 + 3 = 17 against an 18-row
ledger.* **The split is read off the ledger's severity cell**, so it is checkable rather than a
judgement call: *product* = `leak` / `fidelity` / `hardening` / `precision`; *net or docs* =
`build` / `guard` / `docs`. *Stating the rule is half the fix — two tallies were wrong, and they
were wrong because nothing said what counted.)*

The shape of the three product findings is worth naming, because it is one shape: R2 an unmeasured
labelling collision, R10 an axis nobody had decided, R13 the fix for R10 deleting what it was meant
to protect. Every product finding this milestone came from a decision that was never made rather
than from code that was written wrong.

## 2026-09-04 — M11-R11/R12: the chokepoint that was a list, and the record that was deleted

Two findings arrived before round 4 could start, and both are about the *loop* rather than the
product: the guard that closed round 3's leak did not have the property it documented, and the
commit that closed it deleted ten finding records. Both were registered before either was touched.

### M11-R11 — `CASE-01` was the instance-shaped fix wearing the chokepoint's words

Round 3 closed a real leak (a lower- or mixed-case IBAN or VAT number forwarded in clear) and built
`CASE-01` to keep it closed. Its doc comment promised that *"every letter-bearing recognizer has a
recorded answer on the case axis"* and that *"a new letter-bearing recognizer cannot ship without an
entry here"*, citing `shipped_tax_recognizer_count` and `pii_kinds!` as the moves it was making.

It was making neither. `MATRIX` was a nine-row hand-written `const` deriving nothing from the
recognizer registry — the exact move [M4's retrospective] is six rounds of warning against, with the
vocabulary of the chokepoint that finding had asked for. Derived from the registry rather than
counted by eye, the build ships **12** letter-bearing recognizers; the nine rows named eight.
**Four had no answer:** `Secret`, `Email`, the GB NINO and the CN resident id.

Reproduced by mutation, each paired with a temporary probe so a green could not be a mutation that
mutated nothing (at HEAD, with the four probes: **155 passed / 0 failed** — every rendering really
was detected, so each mutation removed real coverage):

    NINO   -> [A-Z]{2}(digits)[A-D]   probe red, CASE-01 GREEN, suite 154/1
    CN id  -> (17 digits)[0-9X]       probe red, CASE-01 GREEN, suite 154/1
    Secret -> sk-[a-z0-9_-]{6,}       probe red, CASE-01 GREEN, suite 154/1
    Email  -> lowercase-only          probe red, CASE-01 GREEN, suite 151/4

For three of the four the **whole library suite stayed green** while a recognizer was narrowed to
uppercase-only: `ab123456c` — a real NINO, and one `nino_prefix_valid` accepts, since it upper-cases
before checking — went from masked to forwarded in clear with nothing red. Round 3's own class,
alive inside the fix that closed it. `Email` was caught only *incidentally*, by three tests whose
fixtures happen to carry an uppercase letter; incidental coverage is not a recorded answer and moves
the moment a fixture is rewritten.

The finding's second half was smaller and sharper: the doc comment described a `MUST_MATCH` with
`Some(kind)` that does not exist in the file. The const was `(&str, PiiKind)` and every row meant
*folds* — so round 3's own **decision 3**, *"`Secret`'s `AKIA` stays uppercase-only, that one is a
format not a convention"*, was **unrepresentable in the matrix that existed to record it**. That is
why `Secret` was a missing row rather than a row saying *no*.

**The fix is a derivation, not four more rows.** `StructuredRecognizers::shipped_patterns()` exposes
every recognizer the scan is built from as `(kind, pattern)`; `pattern_can_match_a_letter` parses
each pattern to the same HIR `regex` compiles and asks every literal and class whether it covers
`A-Za-z`. A textual scan cannot answer this — a word boundary and a digit class are both spelled
with letters that match none, so every pattern "contains a letter" and none of that is the question.
Cost: a test-only `regex-syntax` dev-dependency, the crate `regex` already pulls, so no new crate
and nothing the default-tree footprint guard can see.

`CaseRule` is an enum precisely so decision 3 has somewhere to live: `Folds` checks lowercase,
uppercase and one-letter-flipped renderings; `Fixed { variant, why }` checks the canonical rendering
IS detected and a **named** case rendering is not — with `eq_ignore_ascii_case` asserted, so a row
cannot cheat by naming an unrelated undetectable string. `Secret`'s positive is mixed-case on
purpose (`sk-AbCdEf123456`): the prefix is case-fixed, the key body is opaque, and an all-lowercase
positive would have left the body unpinned — which was mutation D.

**Two things had to be proved, and the second is the one that gets skipped.** The matrix proves the
answers are *right*. A `Recognizer` **injected into `universal_recognizers()`** — not a synthetic
list handed to the pure function — proves the guard is *reached*: it came back red naming
`NationalId :: (?-u:\b)QQ\d{7}[A-Za-z](?-u:\b)` and refusing to pass until an answer was written. Removed
after being seen red. All four originally-green mutations are now red, as is forcing the derivation
itself to answer *no* — that last one caught by `not_letter_bearing`, the field that stops an empty
`unanswered` list being vacuous. It makes the two lists an **equality** — the letter-bearing
recognizers shipped are precisely the ones the answers name — with no count for anyone to keep
current, which is deliberately not the shape `CAT-01`'s floor had (M11-R4 / M11-R7).

One more mutation worth recording: updating the answer table's copy of the pattern *along with* the
product defeats the key check, and `CASE-01` is still red — on the behaviour, naming `ab123456c`.
The key and the assertion cover each other; neither alone is enough.

### M11-R12 — the record was deleted by the commit that closed the leak

`docs/reviews/M11.md` was **120 lines holding one anchor**. The ledger carried eleven rows, ten
linking to anchors that no longer existed. Measured over the file's history: `17ec2e5` 983 lines
(r0-r9), `c8b8aef` 1330 (r0-r10), `33eb159` **120** (r10). Round 3's fix commit rewrote the record
whole instead of appending R10's closure note, dropping **1210 lines and ten finding records** —
including the two closures round 2 had found did not hold, whose evidence lived only there.

Restored as the union of the two sides: R0-R9 from `c8b8aef` verbatim, R10 as HEAD had it (which is
`c8b8aef`'s entry **plus** the closure note), verified by diffing the two renderings of R10 against
each other — the closure lines are the only difference.

`CLAUDE.md` already says a finding is *"never copied, and never moved"* and that closing one
*appends*. The word that needed emphasis is **append**: on a 1400-line record, a fix commit that
regenerates the file destroys history silently, because no test and no lint can see it. `33eb159`
was otherwise a good commit, which is exactly why it went unnoticed for a full round.

### Numbers

**249 / 0 / 5** over 24 test binaries, re-run twice with identical counts (round 3 closed at
248/0/5; the +1 is `CASE-03`). `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings`
clean. `CAT-01` caught `CASE-03` as uncatalogued before it was written up — the guard working on its
own author for the second round running.

**Guard-vs-product tally, which is the number that says whether the loop is working:** R11 and R12
are both on the **net**, not the product — no shipped detection behaviour changed. Running total
across M11: **10 on the guards or docs, 2 in the product.**

## 2026-09-03 — M11 rounds 2 and 3: a guard that could not work, and a leak ten milestones old

**Round 2 broke a fix from round 1, and was right to.** `KIND-01` proved `PiiKind::ALL` complete by
walking a successor chain whose `match` the compiler checks. The compiler demands an **arm**, not a
place in the walk — so `Vin => return None` compiles, the walk never reaches it, `ALL` and the walk
still agree, and a twelfth kind ships green. My own closure mutation had wired the variant in
*properly*, which is the conscientious author's move, not the lazy one. Rust cannot enumerate an
enum's variants, so **no test could close this**: a guard was the wrong shape of answer. The chain
is deleted and a `pii_kinds!` macro generates the enum, `ALL`, `label` and `from_label` from one
list. `priority` and `is_structured` stay hand-written exhaustive matches on purpose — those are
*judgements* about a new kind, and the compiler already stops you there.

**And it caught an arithmetic error of mine that had a consequence.** `CAT-01`'s floor read `>= 70`
against a stated "measured 73" — but the extractor counts **(id, file) pairs** while my 73 counted
**distinct ids** from a probe. Two quantities, compared against each other. Re-read: **96 pairs, 79
distinct**. The consequence was exact: deleting the `//!` walk leaves **70**, so the mutation meant
to prove that walk alive passed by one. A bigger literal was not the fix — one total cannot notice
one of two mechanisms dying while the other grows — so each walk now asserts its own liveness.

**Round 3 found a leak, and it is the milestone's most serious finding.** A lower- or mixed-case
IBAN or VAT number was **forwarded to the provider in clear**: seven of thirteen renderings of
values already in this repo's own corpus, including `IT60x0542811101000000123456` — a canonical
IBAN with **one** lowercase letter. Reproduced before touching anything.

**Why it hid.** The patterns spelled their letters `[A-Z]`, and letters are ASCII word characters,
so there was no `(?-u:\b)` for a shorter recognizer to fall back on: the span did not shrink, it
**disappeared**. Every uppercase corpus test stayed green. Meanwhile `iban_mod97`'s doc comment has
promised to fold letters to uppercase since **M1** — the validator was written for input the regex
could never deliver, which is the clearest evidence available that nobody chose this. The IBAN half
predates M11 by ten milestones and survived every review this repo has run.

**The class is what to carry: an axis nobody decided.** The tier was inconsistent along it — Codice
Fiscale, ES DNI/NIE and the CN resident id all fold case; VAT and IBAN did not — and no test asked
the question, so the inconsistency was invisible. Worse, the docs asserted a *reason the code
refutes*: `VAT-05` argued that lowercase would let `"call it 12345678901"` swallow an English word,
while the next paragraph of the same comment forbids any space between prefix and digits, making
that case impossible under **any** rule. Making `IT` case-insensitive turned exactly one assertion
red in 149 — the one pinning the miss — and the assertion the rationale is attached to stayed green.

**Both halves closed, on the maintainer's decision, with both costs measured twice** (independently
of the review, over the same 341.1 MB / 16 380 files of third-party source). **VAT: folding case
adds 0 matches** for all five schemes, and each is checksum-gated besides. **IBAN needed a gate** —
an IBAN has no hard checksum (M4 masks a structurally valid one even when mod-97 fails), and folding
case takes it from 1 match to 150, sweeping in hex digests and base64. So `iban_case_gate` splits
the rule by rendering: canonical uppercase keeps M4's behaviour, any lowercase letter must be fully
verifiable. **Residue: 0 of the 149.** One correction to the round's arithmetic, in the safe
direction: `iban_length_ok` returns `true` for an unknown country code, so for 145 of them the gate
is mod-97 *alone* — the zero survives that weaker reading, which is the one the code implements.

**`CASE-01` is the chokepoint**, and the instance-shaped alternative is why it had to be: adding
`it00905811006` to a corpus would have closed IT and left DE, GB, PT, NL and IBAN open. It asks
every letter-bearing recognizer the same question, including the **single-letter-flipped**
rendering a lowercase corpus would miss.

**The loop's arithmetic so far:** ten findings closed — **eight on the test net or the docs, two in
the product** (R2's unmeasured labelling collision, R10's leak). The net is where the rounds keep
landing, which is the ratio `CLAUDE.md` asks to watch; but round 3 is also the one that went looking
at the product and found the only leak of the milestone.

## 2026-09-03 — M11 review round 1: five guards that could not go red

**The first independent round this milestone has ever had**, and what it found is one species, not
five defects: *guards that cannot fail* and *numbers quoted for something they do not measure*. No
leak, no fail-open, no over-mask regression. That the round found nothing in the masking path and
five things in the net around it is worth recording as a shape, because the net is what the next
milestone will inherit.

**The one with a product consequence — M11-R2, the mirror collision.** M11 found that ranking
`TaxId` above `Phone` relabelled compact GB/DE numbers, fixed the direction, and pinned two
instances. What nobody measured was the other direction, and it is much the larger: every
*issuable* P.IVA is 0-leading, which is precisely what the phone tier's separator-free `Trunk` arm
claims. Measured against the shipped default — `VAT-17` — **0.775 of issuable bare P.IVAs are named
`[PHONE_n]`, not `[TAXID_n]`**. The split by leading pair is the whole explanation: `00xx` **0.033**,
`0[1-9]xx` **0.859**, because libphonenumber reads a leading `00` as the international access code
and rejects it — **and all five real published P.IVAs this repo's corpus is built on are
`00…`-leading.** Every VAT guard sits inside the immune sub-shape. *A corpus has a shape, and that
shape is a blind spot* — M4's lesson 2, landing again in the milestone that quotes it. `VAT-10`'s
published 0.0998 cannot see this collision at all: its sweep is `1`-leading. Nothing leaks either
way; what the price buys is decision 1's whole purpose, a token that says *business* rather than
*person*, and for most of the bare form's issuable space it is not delivered. **Whether the ordering
should change for the separator-free arm is the maintainer's call and is left open** — the number it
needs now exists, which is what the closure was for.

**Two guards were structurally incapable of failing, and both had said otherwise in their own doc
comments.** `VAT-09` claimed a change to the over-mask cost "should have to edit this number to
land"; over a *contiguous* sweep any function of the first ten digits accepts exactly one value in
ten, so replacing the checksum with `d[10] == 7` printed `0.100` and passed. It now runs a second
sweep with the check digit **held constant**, where a correct mod-10 still accepts ~1/10 and a fixed
comparison accepts all or nothing. `VAT-04`'s ES arm asserted the absence of `"ES B12345678"` — with
a space, which the tier's grammar can never match — so a live checksum-less ES recognizer left it
green, and adding the one control needed to satisfy `VAT-16` left the whole suite at 239/239 with an
unmeasured Spanish scheme shipping. Its negatives are now canonical, and each is checked for
**reachability** against the grammar first.

**The two that were about lists, and both were closed at the chokepoint rather than the instance.**
A twelfth `PiiKind` shipped with the suite green, because `AUG-01` watched five string literals
instead of the enum — the exact failure its own doc comment describes. `PiiKind::ALL` is now the one
list, and `KIND-01` rebuilds it by walking a **compiler-checked exhaustive successor chain**, so a
new variant is a compile error and cannot reach `ALL` without being placed. The proof that matters is
the mutation that wires a new variant past *every* compiler demand and still leaves `ALL` short: red.
And `CAT-01` — the guard over the guards — recognised a declaration only if its family was in a
hand-kept `ID_PREFIXES` array, with a non-vacuity floor of **20 against 54**. The array is deleted
rather than extended: ids are recognised by **shape**, two structural exclusions replacing the
enumeration (a first segment under two characters drops all eight NER BIO tags; a mixed last segment
drops every `M<n>-R<k>` review reference). Measured on the way in, that brought **nine real guard
families under the check that had never been in it** — `CC`, `DBG`, `NER-EP`, `PERF`, `PHONE-COV`,
`REG`, `THREAD` and two more, all catalogued, none ever verified.

**A note on the loop's own arithmetic, per `CLAUDE.md`.** Round 1: **five findings, five on the test
net, zero on the product** — one of the five (R2) carries a product-visible *question*, but its
defect was a missing measurement and a false comment. Round 0 added one on the build (fmt). The ratio
says the product held and the net did not, which is the healthier direction; it also says the net is
where the next round should keep aiming.

**Numbers.** `cargo test` 243/0/5 (from 239), `cargo test-onnx` 279/0/22 (from 275), `cargo fmt
--check` and both clippy legs clean. Nine closure mutations, all red as predicted, each hitting a
distinct assertion. Record: `docs/reviews/M11.md` → Round 1.

## 2026-09-03 — M11 Track A: the over-mask bar cleared, and CI's fmt gate found red

**The real-traffic half of VAT-09, which the tier shipped without.** `VAT-09` measures the bare
Partita IVA against a uniform stream of 11-digit numbers and gets **0.100** — one in ten. That is
what a mod-10 does to arbitrary digits; it says nothing about how many 11-digit numbers real agent
traffic contains. M10 shipped the domestic phone tier with *both* halves and the VAT tier with
only the synthetic one, so the maintainer set the missing bar on 2026-09-03 **before the number
existed**: zero bare-form `TaxId` spans over the same real ~22 KiB Claude Code turn.

**Measured: 0 `TaxId` spans in 22 823 bytes. The bar is clear**, and `0.100` stands as a published
*synthetic worst case*. `tests/vat_overmask.rs` is the guard (`VAT-15`/`VAT-16`), modelled on
`tests/phone_overmask.rs` and running over the same shared fixture rather than a copy.

**Two assertions, because the obvious one can pass for the wrong reason.** `TaxId` ranks *below*
`NationalId` and `Phone` (that is `VAT-14`, the collision this track nearly shipped), so an
11-digit run the bare recognizer claims can be **named** by another tier and disappear from a
`kind == TaxId` filter while still being masked. Filtering on the label would measure the naming
rule; the second assertion filters on the **shape** — no masked span of exactly 11 ASCII digits —
and cannot be fooled that way. The mutation that proves it is worth keeping: splice Enel's P.IVA
`00811720580` (also a valid Latvian personal code) into the turn and only the second assertion
goes red.

**The residue, recorded because the guard cannot state it itself.** The denominator is **zero** —
the turn holds 11 all-digit tokens, longest run **5 digits**, and *no* 11-digit token at all. So
mod-10 was never invoked on this traffic. The zero means *this traffic offers the bare P.IVA
nothing to bite*, which is a real fact about agent turns and weaker than PHONE-OM's zero, whose
candidate set is every plausible digit group. What stays uncovered is traffic carrying 11-digit
database ids or order numbers, where `0.100` is the number that applies and nothing here measures
it. The guard **prints the denominator on every run** so it cannot quietly become a precision
claim, and `TESTING.md` → `VAT-15` says the same thing where it gets read.

**`shipped_tax_recognizer_count()` — a four-line addition, and the point is the chokepoint.**
`VAT-16` holds one live control per shipped scheme, and a hand-kept list of six stays six when a
seventh recognizer lands. That is M10-R7 verbatim one tier over, where a control set covering one
shape family of three passed with the other two deleted. The count is now read from the recognizer
set, so deleting the NL recognizer turns `VAT-16` red.

**Found while verifying: `cargo fmt --check` was red on `main`, and CI gates on it.** Four files
the M11 feature commits touched — `src/pii/onnx.rs`, `src/pii/recognizers.rs`,
`src/pipeline/privacy.rs`, `tests/m7_latency.rs` — were left unformatted by `c830784`/`d2fc694`.
Confirmed pre-existing by running `rustfmt --check` against the `HEAD` blobs, not by inference.
`ci.yml`'s `fmt` job runs `cargo fmt --check`, so M11 as committed would have failed CI on push:
the milestone had had **zero review rounds**, and this is the first thing a round finds.

## 2026-09-02 — M11 Track A: VAT numbers, a new placeholder, and the collision that nearly shipped

**`PiiKind::TaxId` → `[TAXID_n]`, five countries, always on.** The Italian Partita IVA (bare and
VIES form) plus VIES-form DE, GB, NL and PT. Both maintainer decisions were already taken and
recorded in the ROADMAP, so the work was the corpus and the wiring, exactly as the track predicted.

**The corpus is the deliverable, and the anchor is six real published P.IVAs** — ENI, Ferrari, TIM,
Luxottica, Enel, Stellantis Italy — plus the German tax administration's own documented vector
`136695976`, a second real DE number, two GB numbers and two PT ones. Six independent real check
digits is what separates *an implementation of a scheme* from *a plausible transcription of it*; a
corpus generated from the validator would have proved nothing at all, and one hand-picked number
barely more.

**Three countries did not ship, and that is the milestone working rather than failing.** ES, FR and
LV VAT numbers are real and well-formed. The ES legal-entity CIF control character, the FR key over
the SIREN and the LV legal-entity checksum could not be confirmed against trustworthy real pairs
here — and *an unmeasured recognizer does not ship* is the rule that produced nine phone regions
instead of a table of guesses. VAT-04 asserts their absence so the gap stays a decision a reader can
find, rather than a bug a reader discovers. GB **did** ship despite not being a VIES country since
Brexit: it is in the national-ID tier this track takes its country list from, and its number is
checksum-verifiable, which is the tier's actual criterion.

**The defect this track nearly shipped, and the shape of it is worth more than the fix.** A bare
Partita IVA is `\d{11}`. So is the compact domestic phone form M10 measured — and `02079460958`, a
real London number, satisfies the P.IVA mod-10. With `TaxId` ranked above `Phone` (the placement
that looks right if you only think about tax identifiers), **every compact GB and DE number M10
measured silently became `[TAXID_n]`**. Not a leak: the bytes are masked either way, and the round
trip is exact. But a fidelity regression on a shipped, *measured* capability — the proxy telling the
model a phone number is a tax identifier. PHONE-NAT-01 caught it on the first run, which is the
whole argument for keeping measured guards around after their milestone closes.

The fix is one line of `PiiKind::priority`, and the reasoning is two principles that happen to
agree:
- **Under `NationalId`** — conservatism about personhood. Between a label implying *a person* and
  one implying *a business*, the person-implying label is the conservative one: mislabelling a
  person's ID as a company's under-states its sensitivity, and over-stating is the side this
  project errs on everywhere else.
- **Under `Phone`** — strength of evidence. The domestic-phone tier does not match a shape; it asks
  `phonenumber` whether the candidate is a real **assigned** number in that region's plan. That
  beats a mod-10 check one arbitrary number in ten satisfies.

Both collisions are now pinned from the VAT side by VAT-14, which is where a change that
reintroduces them would actually be written.

**Two rates measured rather than asserted.** The bare P.IVA accepts **10 000 / 100 000 = 0.100** of
arbitrary 11-digit numbers — the published over-mask cost, on the same M4-R6 grounds as the numeric
national IDs. And **1 997 / 20 000 = 0.0998** of valid P.IVAs also satisfy a DE Steuer-ID or LV
check, so ~10% are named `[NATID_n]` and ~90% `[TAXID_n]`. That second number is the one that says
this recognizer adds real coverage rather than relabelling what the national-ID tier already caught.
(Enel's real P.IVA is in the colliding 10% — which is why two of the first test expectations were
wrong and had to be re-picked. A small real sample reproducing the measured rate is a good sign, not
a nuisance.)

**NL is the one shipped scheme with nothing to check**, and it says so through `Confidence` rather
than quietly claiming a verification: the 2020 sole-trader `btw-id` is randomized by design, so the
recognizer accepts on format (14 chars, mandatory `NL`, a literal `B` at position 11) and tags
`Structural` unless the body passes the 11-proef. The same honesty an IBAN whose mod-97 fails gets.

**Anti-false-positive decisions worth naming.** Country prefixes are **uppercase only** — lowercase
would make `it` a matchable prefix, and `"call it <11 digits>"` would then produce a span swallowing
one of the commonest words in English. And **no space** between prefix and digits: VIES does not use
one, while allowing it would put `DE`, `IT` and `GB` — all live English abbreviations — one space
away from a digit run.

**Item 6 of the brief, the system-prompt cache, checked and found structurally safe.** The worry is
real in shape: add a placeholder kind, and a cached instruction that never mentions it degrades the
model's handling of exactly the token you just added, silently. It cannot happen here, and AUG-02
pins *why* rather than asserting the conclusion — the instruction is appended **after** masking has
run, so the detector is never asked about it and the cache (which keys on the texts the detector
sees) has nothing to serve. A recording detector proves no submitted text contains the prompt. If
the injection ever moved before masking, that test fails, and that reordering is the only way the
bug becomes real.

**CAT-01 passed vacuously at first, and that is its own finding.** The new guards declare `VAT-…`
and `AUG-…` ids, neither of which was in `ID_PREFIXES` — so the guard that catalogues guards could
not see them and reported green over sixteen uncatalogued ids. The array is CAT-01's own blind spot,
one level up from the drift it was built to end. Adding the two prefixes made it fail with all
sixteen listed, which is what it should have done from the start.

## 2026-09-02 — M11 Track B: the intra-op thread base moves to physical cores

**Shipped the decision the ROADMAP recorded, and nothing more.** `NER_INTRA_THREADS` still derives
as `max(1, base / NER_POOL_SIZE)`; what moved is the base — from `available_parallelism()` (logical
threads) to `min(physical_cores, available_parallelism())`. On the 6-core / 12-thread reference box
the shipped default goes from `1×12` to `1×6`, and `NER_POOL_SIZE=2` from `2×6` to `2×3`. The
mechanism is that both SMT siblings of a core were running the same int8 GEMM, contending for one
core's L1d/L2 and one set of vector units — the derivation's own purpose (the product fits the box)
read against the wrong count of "box".

**The spike came first, and it was the only thing that could have stopped this.** The plan's single
unverified assumption was `num_cpus::get_physical()`. Measured on Windows/MSVC before a line of the
real change was written: `num_cpus` 1.17.0 builds; it reports **6** where `available_parallelism()`
reports 12; DEP-01 and DEP-02 stay green across the full release matrix; and `cargo tree` finds it
in the `onnx` tree only — **0 hits in the default tree**. The fallback (hand-rolled per-OS calls) was
a differently-sized job and never had to be opened.

**Three things the change deliberately did *not* do.**
- **It did not add a second path.** `resolve_pool_and_intra` keeps its single home and its
  `0`-is-unset symmetry; only the base it divides moved. `GLINER_POOL_SIZE`/`GLINER_INTRA_THREADS`
  inherited the new base for free — the payoff M7-R1 bought and the reason not to special-case
  GLiNER here.
- **It did not narrow NER-THREAD-01.** That guard still sweeps `intra` up to the *logical* count
  even though nothing ships there any more. It is a detection-inertness guard, not a
  shape-of-the-default guard: more partitions is strictly more coverage, and the logical count is
  the largest partitioning an operator can actually request. Re-run: **194 entities, identical at
  intra 1 / 2 / 4 / 6 / 12.**
- **It did not claim to have resolved SMT.** [M7-R2](reviews/M7.md#m7-r2) left that unresolved after
  four runs whose sign flipped under a ~40% same-configuration spread, and this milestone says so in
  the code, in ARCHITECTURE and in the ROADMAP. The base is the *conventional* one for GEMM-bound
  inference, adopted by decision to stop paying for a knob no measurement on this hardware can read.
  **The sweep records the change; it never justified it** — and M11's own numbers are consistent
  with that: `1×6` 2.48× vs `1×12` 2.42×, still inside the noise, still not a result.

**What the re-run did resolve is throughput, and it went the other way from the worry.** 4
concurrent turns: `1×6` **0.485** turns/s vs `1×12` 0.419 (**+16%**), and `2×3` **0.664** vs `2×6`
0.558 (**+19%**) — the new centralized shape is the fastest row the harness has measured. Sublinear
intra-op scaling again: fewer, less-contended threads per session aggregate better.

**The one consequence the plan did not foresee: PERF-M7-05's floor had to split per shape.** M7's
bar asserts every shipped shape is ≥1.5× the pre-M7 `2×1` shape. The centralized shape lost half its
per-session threads and measured **1.46 / 1.56 / 1.56 / 1.69×** across four isolated runs — the floor
sitting *inside* its run-to-run band, i.e. a guard that fails intermittently on correct code, which
is the worst kind because it trains the reader to re-run until green. Put to the maintainer with the
throughput numbers beside it; the call was **per-shape floors** — default ≥1.5× (it measures
~1.9–2.1×), centralized ≥1.3×. Dropping the centralized shape from the assertion would have re-opened
M7-R1's class, a harness watching one shipped configuration while another ships unwatched; lowering
both to 1.3 would have discarded the default's real headroom. The floor now travels *with* the shape
in `bar_shapes()`, so a third shipped shape cannot silently inherit a floor nobody measured for it.

**The startup log grew four fields, and that is M7-R5 rather than decoration.** M7-R5 rejected
`pool_size=0, intra_threads=12` because no arithmetic reconciles it. Once the base stopped being the
core count an operator's task manager shows, a bare `intra_threads` became just as unreconcilable —
on this box it reads 6 where the machine advertises 12. The line now carries `thread_base`,
`thread_base_source` (`physical` / `parallelism-cap` / `physical-unknown`), `logical_cores` and
`physical_cores`, so `intra = max(1, base / pool)` can be redone from the line that reports it. The
`--bench-providers` header got the same treatment for the same reason.

**The `min` is the part to not get wrong, and THREAD-01 pins it without owning the hardware.**
`available_parallelism()` honours cgroup quota, affinity masks and job objects; a physical-core count
does not — it reports the silicon. Taking it bare would derive `intra = 32` for a proxy granted 2
CPUs on a 32-core host: the oversubscription this derivation exists to prevent, arriving through the
fix for it. `derive_thread_base` is pure in both arguments, so the grid — SMT box `(12,6)→6`, SMT off
`(6,6)→6`, hybrid P+E `(20,14)→14`, container `(2,32)→2`, unavailable `(12,None)→12` — is asserted on
a machine that is none of them. The invariant is restated on the new base **still split by regime**,
never widened inside a `max()`, and its bases are now *derived* from `(logical, physical)` pairs
rather than plausible integers: the composition is what actually runs.

Scope was Track B only; A and C carry open maintainer decisions and were not started.

## 2026-07-31 — `v1.2.1` cut, and what the release page actually contains

**Tagged from `cee931d`, published green: ten binaries, three guards satisfied, curated body.** The
order is the part worth keeping, because it is what stopped this tag from being broken: push to
`main` → let `ci.yml` run → run **Manual build** by hand (the ten-target cross-compile, which
otherwise happens for the first time *after* the tag is public) → only then tag. `ci.yml`
deliberately does not cross-compile, so without that middle step the `DEP-02` defect found today —
real, and invisible on Windows — would have surfaced with the release already out.

**The `CHANGELOG.md` guard shipped with this release** and is now the third precondition beside the
ROADMAP row and the manifest: a tag this file does not describe is refused before anything is
published. All three were simulated locally against the real files before the tag went up, using
the same `grep`/`awk` the workflow runs.

**One prediction in that design was wrong, and the correction is worth more than the prediction
was.** `generate_release_notes: true` was described — in the workflow comment, in the commit
message, and in the options put to the maintainer — as appending *the commit list*, with "45
commits" named as what the auto-generated half would show. It does not: GitHub generates notes from
**pull requests**. This repo pushes straight to `main`, so there were none, and the generated half
of the v1.2.1 body reduced to a single `**Full Changelog**: …/compare/v1.2.0...v1.2.1` line.

The outcome is the one we wanted, reached for the opposite reason — a curated section plus a
compare link, with no noise. But the option the maintainer *declined* (auto-notes only) would not
have produced a noisy page as described; it would have produced an **almost empty one**: a title
and a link. A choice offered on a wrong description is a choice that was not really offered, which
is why this is recorded rather than quietly enjoyed. The comment in `release-build-publish.yml` now
states the measured behaviour.

## 2026-07-31 — CI's first look at M10, and the guard that was green because of the OS it ran on

**The 31 M10 commits reached CI for the first time today, and it went red immediately.** Both test
legs, on the same test: `the_default_build_compiles_no_native_code` — **DEP-02, the guard M10-R9
added** to replace a six-name denylist with the *property*. Not a regression from any of today's
work; `openssl-sys` sits in `Cargo.lock` at `9e31f36` too, the last green run. What was new was a
guard finally able to see it, running somewhere other than this laptop.

**The mechanism, and it is this milestone's signature error one more time.** `reqwest` is pinned to
`native-tls` on purpose — reqwest 0.13 flipped its default to rustls + `aws-lc-rs`, which compiles
C. The `Cargo.toml` comment justifying the pin read *"keeps the default build native-dep-free"*. But
`native-tls` **is** the platform's TLS: schannel on Windows (pure-Rust declarations), **OpenSSL on
Linux** (`openssl-sys` + `cc`), Security.framework on macOS (three `*-sys`). Both DEP guards asked
`cargo tree` about **the machine running them**, with `windows-sys` as the single allowance — so on
Windows they were green and said nothing, and the claim they were cited as enforcing was a fact
about one point of a grid. Twelfth instance in M10, and the first one in a *guard's own query*
rather than in a measurement.

**Fixed as the property, not as the symptom.** Both guards now iterate the five released targets
(`release-build.yml`'s matrix), each with an explicit allowance naming that platform's TLS crates
and the reason beside them; everything else stays forbidden everywhere, so a genuinely new native
dependency is still caught on every target. **Verified by mutation**: deleting the Linux allowance
reds the guard **on Windows**, naming `x86_64-unknown-linux-gnu` and its two offenders — which is
exactly the property that was missing. `DEP_GUARD_HOST_ONLY=1` checks the host alone for offline
work and says so in the output; CI never sets it. `--locked` (not `--offline`) is what keeps
`Cargo.lock` untouched, confirmed after the run.

**There is no way to make the strong claim true, so it was reformulated instead.** rustls' crypto
providers — `aws-lc-rs`, `ring` — compile C as well, so no reqwest-reachable backend is pure Rust.
The rule now reads: *the default build reaches no native dependency except the TLS its operating
system already provides*, corrected in `Cargo.toml`, `ARCHITECTURE.md`, `TESTING.md` and `ci.yml`.
The operational consequence is named where a packager will meet it: a Linux binary links the system
`libssl`, which is fine on any ordinary distribution and not fine in a `scratch` image.

**Worth stating plainly: the tag would have shipped with this.** The release pipeline builds ten
targets but only at tag push, and `ci.yml` deliberately does not cross-compile — so a defect visible
only on another platform had no earlier gate. It was caught because the release was pushed to `main`
first and CI was allowed to run before the tag, which is now the reason to keep doing it in that
order.

## 2026-07-31 — The CC battery, M10's subset: four scenarios × two postures, and the numerics nobody masked

**The last box M10 owed, and the one no review round could produce.** CC-01 / CC-03 / CC-04 / CC-09,
each run twice (`PII_DEBUG_SKIP_DEMASK` unset, then `=1`), against real Anthropic through the proxy,
with the owner driving a real Claude Code session in `tools/claude-code-session/`. Hybrid confirmed
before trusting anything: `cargo build-onnx` immediately before launch, `NER_REQUIRED=1`, both green
lines (`ONNX NER detector loaded … pool_size=1 intra_threads=12 provider="cpu"`, `listening on
http://127.0.0.1:8787`), `upstream_api_key: None` — the client's own credential, forwarded verbatim,
200 on the first request as in M6.

**All four pass both postures.** 47 forwarded requests across the two logs, **zero** fixpoint 400,
zero `ERROR`, one `WARN` (the `PII_DEBUG_SKIP_DEMASK` banner, on the run that set it). **DBG-02 = 0**
on all 16 raw values — three emails, four phones, three SSNs, two IBANs, the card, the secret in
full, and the three names — across **both** logs.

**What the pair proves that neither half does.** On Run ON the client read `[PERSON_4]`, `[EMAIL_2]`,
`[IBAN_1]`; on Run OFF *those same three tokens* are what the trace shows leaving, and the client got
`Jane Doe` / `jane.doe@example.com` / `IT60X…123456` back. Same input, same provider, same
assignment — one round-trip seen from both ends, not two plausible halves. The masked bodies are
**byte-identical between postures** (request-side masking does not depend on the flag), which is the
comparison that would have been a finding had it failed. CC-04 isolates the de-mask to a two-line
diff: identical `tool_use.input` (`{"content":"[EMAIL_2]", …}`) in both runs, while `first-contact.txt`
on disk holds `bob@test.com` after OFF and **`[EMAIL_2]` after ON** — the placeholder on disk, the
clearest artifact the battery produces.

**The M10 question — over-masking real agent traffic — is answered, and the answer is clean.** This
is what the four were chosen for: no corpus test can show a masked line number corrupting a tool
argument, because a corpus holds only the false positives we imagined. On live traffic: the `Read`
tool result's line numbers `1`…`5` passed **untouched** while the phone column beside them became
`[PHONE_1..3]`; the SQL result's `"id":1` and `"row_count":1` passed untouched beside six masked
fields; and a 199 KB real turn yielded **zero** `Phone` spans. (The single `[PHONE_2]` in that body
is literal text of our own augmentation prompt — *"for example [EMAIL_1], [PHONE_2], …"* — not a
detection. Worth checking rather than assuming, since a grep for `[PHONE_` alone would have read as
an over-mask.) PHONE-OM's offline result now has its live twin, with tool *results* included.

**Five scenarios deliberately not run — CC-02, CC-05, CC-06, CC-07, CC-08.** M10 touches the
deterministic layer only; the NER, determinism across turns, secrets, thinking blocks and streaming
are all code this milestone never edited, and re-running them would buy confidence in the wrong
place. Saying so is the result; silence would read as a full battery that passed. Two of them were
nonetheless visible in passing: the NER masked `Jane Doe` as `[PERSON_4]` on every turn (so the
hybrid was demonstrably running, which is CC-02's *concern* though not its assertions), and a
`thinking` block was replayed with its signature and accepted (CC-07's invariant — but the block's
content was empty, so it is an indication, not a run).

**A setup defect in the runbook, found the only way it could be.** `MANUAL_VERIFICATION.md` and
`TESTING.md` both said CC-09's throwaway DB is reached because *"the `python-sql` MCP server takes an
absolute-path `sqlite` connection"*. It does not: it resolves connection **names** out of a
connections file, and passing a path returns `Unknown connection`. The 2026-07-18 run worked because
the owner had added such a connection by hand; nothing recorded that, and by today it was gone —
`list-connections` returned **only real corporate Oracle schemas**. Adding the fixture to that file
would have been the obvious fix and the wrong one: it holds live credentials, and it would leave a
session whose whole job is to emit PII one tool call away from a production table. Instead the
fixture workspace now carries its own **`.mcp.json`** (server `cc09-sqlite`) pointed at a connections
file of its own, holding exactly one read-only SQLite entry. The rule *"never point this at a real
table"* stops being a warning and becomes a property of the wiring. `.mcp.json` is gitignored
(absolute machine paths); `fixtures/cc09-mcp.example.json` is the committed template, and the runbook
now says how to build both.

**One artifact, recorded because it is visible to a user and is not a defect.** On Run OFF the model
sometimes explains the masking it was told about — *"i valori sono placeholder tipizzati … non
modificarli"* — while the values around that sentence have already been restored to the real ones.
The de-mask substitutes tokens; it cannot rewrite meta-commentary about them, and it should not try.
Harmless to privacy, mildly confusing to read. (As in M6, the raw SQL row also appears in Claude
Code's **local** tool-output pane: that result never traverses the proxy, and the masked copy is what
goes upstream.)

## 2026-07-31 — M10 round 9: the axis was added, the claim was not emitted, and the number is wrong again

All four closures checked hold as **edits**. R47's guard clause is load-bearing by mutation — deleting
`Err(err) if err.is_budget_exhausted() => Err(err)` reds FAILOPEN-BUD *and* DOS-08 and nothing else.
R54's odometer is genuinely distinct at 16 MiB in both renderings (219,096 of 219,096 phone strings
distinct, counted rather than assumed), and its grouped REFUSED verdict is a real legal-body refusal,
not an R49-style rejection cost: the same column masks 50,000 of 50,000 at a flat 8.00 units/row.
R55's three sites are filled. Six new findings, **none needing a source change**.

**M10-R56 — the replacement band is wrong by 3.3×, and it is the fifth version of these numbers.**
Four documents say the reachable band starts *"around 2.6 MB (a dense grouped column)"*, and both
READMEs name the rendering: `320 123 4567`-style grouping. Measured with DOS-BUD's **own** generator,
that column costs 8.00 units per row, so a one-column export is refused at **62,500 rows / 793 KB** —
exactly (499,992 units at 62,499 rows, 500,000 at 62,500) — and a `SELECT name, phone` export at
**1.88 MB**. Confirmed through the real `.exe`: 799 KB → `400`, upstream never contacted; 761 KB →
`200`, forwarded and restored. The published 2.6 MB came from an ad-hoc probe measuring **2.5**
units/row, which is the per-scan memo being served by a generator whose candidates repeat: this
milestone's oldest trap, seventh appearance, and the first to land directly in a published number
rather than in a draft.

**M10-R57 — *"1 to 29 units"* has R53's own defect inside R53's own fix.** The table takes one sample
per shape family, and the French sample (`01 23 45 67 89`) is the cheapest number of its family. Six
ordinary French renderings — `06 12 34 56 78`, `02 40 12 34 56`, `04 91 12 34 56`, `05 56 12 34 56`,
`08 92 70 12 34`, `09 70 12 34 56` — cost **46** units and every one masks whole; the overall maximum
found is **65**. *A conclusion drawn from one point of a grid is a fact about that point* — written
into ARCHITECTURE by R53's closure, eleven lines above a table that is thirteen points of a grid.

**M10-R58 is why neither was caught, and it is the tenth of its class.** R54's fix had three items;
item 2 was *"the one-column density row, **which is where the 2.6 MB / 6.0 MB refusal line lives**"*.
Items 1 and 3 landed; item 2 did not, and the closure note does not mention it. So the band is still
an **off-harness number** that nothing re-runs — which is exactly the mechanism that produced the
previous three wrong versions. Tenth *"a closure skipped a step its own finding named"*, in the commit
that closed the ninth and named the mechanism (*the interesting half of a fix crowds out the checklist
half*).

**M10-R59 — the stopping rule's premise does not hold.** *"Both classes now have a mechanical guard
(DOS-BUD's rendering axis; FAILOPEN-BUD and the catalogue entries), which is what makes them stop"* is
false in both halves: FAILOPEN-BUD guards fail-open behaviour, not catalogue drift, and R55's own
~15-line id-extraction guard was never written. DOS-BUD's axis, meanwhile, has the shape DOS-08's
first version had — aimed at the instance (`MAX_BODY_BYTES`, refused yes/no) rather than the shape the
claim takes (a refusal line in bytes, at any size), which is why R56 exists.

**Nothing here is a live defect in `src/`.** Rounds 5–9 have now found zero. The refusals are the
fail-closed path working: nothing forwarded, nothing leaked, an actionable message carrying no
input-derived bytes, and the positive control masked upstream (`[PHONE_1]`, `[EMAIL_1]`) and restored
to the client. The release workflow's two guards were **simulated** rather than assumed — `v1.2.1` is
correctly blocked today by the Status table's `(planned)` and by the `1.2.0` manifest, and the
table-only scoping holds.

**One decision is owed.** R53's closure kept 500,000 on *"an ordinary 367 KB tool result spends ~1%"*
and *"what gets refused is a multi-megabyte contact export — rare"*. Both are properties of the
cheapest rendering: at the grouped one, a 354 KB tool result spends **8%**, DOS-BUD's own 3.6 MB row
spends **80%** (the table publishes 10%), and the refused export is **793 KB**. Eight percent is
comfortable and the threshold is still defensible; *rare* is the word that no longer fits.

**And this round published no wall clock at all.** The box was never idle — a `copilot` process held
~1.3 cores throughout and a `java` process later added ~3 more (19% → 35% of twelve cores), neither
mine to stop. By M10-R50's own rule (*a published number is measured on an idle box or it is not
measured*) every timing measured was discarded rather than filed. Every number in round 9's findings
is a **unit count**, which is deterministic and load-independent — which is also why the refutation of
the band is exact rather than arguable.

**220 default / 253 onnx green**, `fmt`, `clippy -D warnings` on both feature sets and the 15-warning
`cargo doc` baseline all clean.

## 2026-07-30 — M10 round 9: the limit was never a number of bytes, and the review is done

**No live defect in `src/`. Fifth consecutive round.** Round 9 drove the real `.exe` end to end
(masked → forwarded → restored; refused → upstream never contacted), mutated the tree to re-verify
FAILOPEN-BUD and DOS-08, counted the odometer's distinct substrings instead of assuming them, and
simulated the release workflow's two guards. Six findings, none needing a source change.

**M10-R56 — the band was wrong a fifth time, and the fix was to stop publishing it in megabytes.**
The Italian grouped rendering `320 123 4567` — the ordinary way to write a mobile — costs **8.00 units
per row in a column**, exactly and flatly: 62,499 rows spend 499,992 and are masked, 62,500 spend
500,000 and are refused. So the allowance is ≈**62,500 phone numbers per request** at the worst column
rendering and ≈500,000 at the cheapest. The **bytes** are a property of the payload's layout, not of
the limit — the same 62,500 numbers are refused at **793 KB** as a bare column, **2.0 MB** as
`name,phone` and **4.45 MB** as a six-column export. Publishing one of those as *the* refusal line is
what went wrong four times running.

**And R57 explains why the isolated cost table was misleading even after it was correct.** In
isolation an FR pair-separated number costs 46 units and a `+CC` form costs **0** (that recognizer has
no validator at all); in a *column* FR collapses to 3.27, because the per-scan memo absorbs the
repeated sub-candidate prefixes. *Isolation measures how a rendering behaves; a column measures what a
body costs* — and only the second answers "can real traffic reach this?".

**The threshold stays at 500,000, re-confirmed by the maintainer on the corrected numbers.** What
changed in its justification is the word *"rare"*: an ordinary 5,000-row export spends **8%**, not 1%,
and the refused body can be sub-megabyte. M10-R27's rule now has a real counterexample, written into
ARCHITECTURE rather than smoothed over.

**Two mechanical guards, which is what actually ends the loop.** `DOS-BUD` now runs the SQL sweep in
**both** renderings, three layouts at the refusal line, and a per-rendering column table — so the
sixth version of these numbers was *read off the harness* rather than typed in, which the first five
were (R58). And **CAT-01** extracts every guard id a `#[test]` declares and asserts it is named in
`TESTING.md` — the check M10-R55 prescribed, M10-R59 found unbuilt (the tenth instance of "a closure
skipped a site its own finding named"), and which **caught four uncatalogued ids on its first run**:
`LOG-03` and `PROP-01a/b/c`, hidden inside the compressed entries `LOG-01/02/03` and `PROP-01`. That
is how a missing entry hides, and it is the best evidence the guard is real.

**Where the milestone stands: the review is done.** Rounds 5–9 found zero live defects in `src/`. What
kept producing findings was the loop between measurement and prose, and both halves of it now have a
guard instead of a promise. **61 findings across nine rounds, all closed. 221 default / 254 onnx
green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc` baseline all clean. What is owed
before the tag needs the maintainer: the CC battery, and the `1.2.1` bump.

## 2026-07-30 — M10 round 8: the availability claim was a property of one rendering

All six of round 7's closures hold, verified by mutation and by recompiling the cases they describe.
Three new findings, none needing a source change — and one of them is the sharpest thing this milestone
has produced that is not a defect.

**M10-R53.** Four live documents said *"a legal phone-bearing body cannot reach the allowance at all"*.
It is false, and it was published on the strength of one measurement: DOS-BUD's `347 XXXXXXX` column.
That rendering is the **global minimum** — `LongBlock` is the only single-region family and the only
shape no other family's regex matches inside — so it costs **1** unit, while `320 123 4567` costs 12,
`612 34 56 78` costs 26 and `01 23 45 67 89` costs **29**. `Scan::Overlapping` resumes one `char` past
each match's *start*, so every other rendering also proposes sub-candidates from inside itself, each
rejected and each paying its family's whole region list. Measured through the real `.exe`: 1.2 MB of
IT mobiles → `200`; **2.6 MB grouped → `400`**; 6.0 MB compact → `400`. Five of six legal 16 MiB
payloads are refused.

Nothing leaks and nothing is forwarded — the refusal is the fail-closed path working, with an
actionable message. What was wrong is the answer to M10-R27's question, *can real traffic hit this?*,
and it is the first time in this milestone a published availability figure was wrong in the
**optimistic** direction. *A conclusion drawn from one point of a grid is a fact about that point* —
and the tell was there all along: the harness had one column, the conclusion was about all columns.

**The threshold stays at 500,000** (the maintainer's call). An ordinary 367 KB tool result spends ~1%,
the M7 turn spends 0, and what gets refused is a multi-megabyte contact export. M10-R27's rule — *a
refusal that is a routine event is the wrong threshold* — now has a concrete counterexample instead of
a hypothetical one, and it is written down rather than smoothed over.

**R54** gives DOS-BUD the axis it held constant for three rounds: a per-rendering unit table and the
`MAX_BODY_BYTES` row in **two** legal renderings. **R55** closed the 7th–9th instance of *"a closure
skipped a site its own finding named"* — nine is enough to name the mechanism rather than the lapse:
**the interesting half of a fix crowds out the checklist half**, and a closure note written while the
interesting half is fresh reads as complete because it is complete *about the part its author was
thinking in*.

**And the oldest trap arrived a sixth time, on me, writing R54's second rendering.** The first
generator had a 10,000-row period, so the memo served 95% of a 16 MiB dump for free and the grouped
rendering measured **cheaper** than the cheapest — the exact opposite of what the row exists to show,
and green. *A modular generator looks varied at every call site; only the aggregate shows it is not.*

**Where the milestone stands:** rounds 5–8 found **zero live defects in `src/`**. The code has
converged; the loop between measurement and prose had not, and the two mechanical guards this round
added are what should end it. A reasonable stopping rule for the tag: **a round that produces only doc
findings for a second consecutive time.** Round 8 was the first.

**220 default / 253 onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc` baseline
all clean.

## 2026-07-30 — M10 round 7: the harness that answers "can real traffic hit this?" was measuring non-numbers

All five of round 6's closures hold — verified by **mutation** and by the **compiler**, which is the
bar worth keeping: a `detect`-only implementor is `error[E0046]`, and putting the budget-dropping
defect back into `CachingDetector::redetect` reds DOS-08 and DOS-09 and *nothing else in the suite*.
Six new findings, none a live leak, none a behaviour change, none needing a decision.

**The one that reframes the milestone is R49.** `DOS-BUD`'s "phone column" emitted **eleven digits**,
and no Italian plan accepts eleven — so the harness whose entire job is answering *"can real traffic
reach the budget?"* masked **nothing at any size**, and every unit it published for two rounds was the
cost of *rejecting non-numbers*. It could not have told anyone: its verdict column printed `0 left`,
which is exactly what a correctly-masked column prints. *A measurement harness needs its own
non-vacuity assertion* — the M4-R13 bar for corpora, applied to the thing doing the measuring.
Reporting `N masked` is the whole fix and would have caught it on the first run.

**With a real column the answer inverts, and it is a better answer.** `national_phone_valid` is
`.any()` over the enabled regions, and `.any()` short-circuits on **accept** — so a real number costs
~1 unit while a candidate every plan rejects pays for all nine. The largest phone-bearing request the
proxy accepts, **16 MiB / 221,941 numbers**, spends **221,941 of 500,000** in 2.56 s. So *a legal
phone-bearing body cannot reach the allowance at all*; `MAX_BODY_BYTES` binds first. What reaches the
budget is text that **fails** validation — the adversarial shape, by construction, which is the right
thing for a fail-closed bound to be reachable by. Every "refusal line in rows" this project published
is retired: for that payload shape there isn't one.

**R47 is the one that mattered for safety.** `FailOpen`'s *"never swallow a budget refusal"* — the
line M10-R41 added — was asserted by **nothing**. Delete it and the whole suite stayed green while a
body the request path must refuse was forwarded through `Caching(Composite([FailOpen(Structured)])))`.
M10-R41's own *"test that would have caught it"* had prescribed exactly the missing case. **A closure
that takes the fix and leaves the prescribed test has closed half the finding** — second time here.
`FAILOPEN-BUD` now asserts both sides of the distinction, verified by mutation.

Also: R48 put the three `FailOpen` positions into `shipped_chains()` and, where the list still cannot
be derived from the wiring, **says so** rather than implying a completeness the code cannot deliver.
R50 retired the **5.2 s** worst case — measured under load *and* on the broken column; idle it is
2.56 s, and refusals land at 1.4–1.9 s. *A published number is measured on an idle box or it is not
measured*, and the contention is usually our own build. R51 fixed two sites round 6's closures had
themselves named (fourth and fifth of that class). R52 recorded that a private field reaches a
module's **descendants**, so the `DetectError` literal still compiles inside `pii::*` — the claim was
weaker than it looked, and this milestone has now been wrong about what the compiler enforces twice,
both times in the reassuring direction.

**220 default / 253 onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc` baseline
all clean.

## 2026-07-30 — M10 round 6: both of round 5's closures claimed more than they delivered

Round 5's fix **holds** — verified on the real `.exe` at the shipped default, a legal 15.63 MiB body
is a `400` in **1.0 s** where it was `200` in 17.2 s, upstream untouched — and six of its seven
closures with it. What did not hold were the two sentences written *about* the fix, and both failed
in the same direction: **they described a property of the shape and were implemented against an
instance.**

**M10-R42 — the guard.** Three documents said DOS-08 was *"phrased over the trait, so a seventh
detector that drops the allowance fails here"*. What was phrased over the trait were the method names
it called; what it quantified over was **one concrete type — the leaf, the one already fixed**.
Reintroducing the identical defect in `CachingDetector::redetect`, a shipped wrapper on the default
request path, left all 218 tests green while the request under-charged **21×** and forwarded a body
it must refuse. *A guard aimed at the instance relocates the blind spot* — which is M4-R7 → R9's *"a
fix that only re-ranks relocates the leak"* in its testing form. DOS-08 now loops over every chain
`AppState::new` can build, and **DOS-09** adds the end-to-end half: 20 **identical** fields through
the cached chain, the complement of DOS-07's distinct-fields-no-cache attack. Identical fields make
pass 0 free from the cache, so the whole cost lands on `redetect` — which is deliberately never
cached, and is therefore invisible to any guard without a cache in it. Both verified by **mutation**:
red with the one-line defect, green without.

**M10-R44 — the claim.** Four places said *"no method remains that a default could route to which
would mint another allowance"*. True of `redetect`; false of `try_detect`, whose own default routed
to `detect` — and every production `detect` **mints** an allowance *and* `unwrap_or_default()`s the
refusal. A five-line wrapper implementing only `detect` compiled, never mentioned `Budget`, and
forwarded DOS-07's body with the refusal not ignored but **erased**. So `try_detect` is now the
**required** method and `detect` the derived one: `detect` is the convenience view, `try_detect` is
the contract. The four detect-only test doubles that stopped compiling are this fix's entire
regression suite, and they are worth more than a test would be — *the guarantee comes from the
compiler, not from a case somebody remembered to write.*

Also closed: the constant's own doc still carried the pre-round-5 table and *"≈50,000 phone numbers
per request"*, 2× optimistic in the reassuring direction, because round 5's re-measurement updated
ARCHITECTURE and never touched the constant the finding had named (R43 — *a closure is checked
against the finding's own locations*, for the third time in this milestone). `DetectError`'s
`budget_exhausted` is private now with an accessor, so the two constructors are the only way to build
one outside its module — *"a new error site has to choose"* was a convention while the field was
`pub` (R45). And two comments still refused a field *"by `try_detect_within`"*, deleted by the commit
that wrote them (R46).

**DOS-09's first draft repeated one literal group** — so the per-scan memo collapsed a 20 KB field to
a single validated candidate and the request was forwarded, which reads as *"the budget is not
charged"* when it is the generator saying there was nothing to charge for. Fifth time in this
milestone. The fields have to be identical to **each other** and distinct **within**.

**219 default / 252 onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc` baseline
all clean.

## 2026-07-30 — M10 round 5: the fix for the wrong unit had a hole shaped exactly like itself

**Round 4's fix threaded the per-request allowance through a *new pair* of trait methods** —
`try_detect_within` / `redetect_within` — whose **defaults delegated to the budget-less originals**.
So every implementor carried an obligation to override *both*, and the penalty for missing one was
invisible: the call fell through to a method that **minted a fresh allowance**. Round 4 saw that
hazard, wrote it into the trait's own doc comment, and closed it for all three **wrappers**.

It missed the **leaf**. `StructuredRecognizers` is the only detector whose cost the budget bounds and
the only place in the tree that mints one; it overrode `try_detect_within` and not
`redetect_within`. Every fixpoint pass after the first therefore started from a full 500,000, and a
legal **15.63 MiB body answered `200` in 17.2 s** against a ceiling this project had just published as
~1.4 s. The one-line difference left **the entire suite green on both sides**.

*An obligation a trait default can satisfy is not carried by the type system — and "every test
passes" is the signature of that, not evidence against it.*

**So the seam was deleted rather than filled** (the maintainer's call, over the one-line override).
`try_detect` and `redetect` now **take** a `&Budget`; `redetect`'s default forwards the same one;
`Vault::mask_all` lost its budget-less convenience for the identical reason — two entry points where
one mints and the other accepts is precisely the shape of the finding. Every implementor and call site
moved with the signature. **When forgetting to override is silently valid, the API is the defect.**

*(That turned over one half of the trait. Round 6 found the other: `try_detect`'s own default routed
to `detect`, which mints an allowance **and** swallows the refusal — see the round-6 entry above.)*

**The threshold survived the correction; the numbers around it did not.** Charging the later passes
roughly **doubles** what a masking body spends, so the refusal line for a phone-bearing database
result moved from ~90,000 rows to ~25,000–35,000. 500,000 stays — the ordinary 5,000-row tool result
spends 100,010 of it, and a real 22 KiB Claude Code turn still spends **0**. Everything else was
re-measured and republished from DOS-BUD's own rows: the 15.6 MiB / 78-field body is **refused in
2.24 s**, and the honest ceiling is *~1.6 s of validation **plus** work linear in the body* — the
slowest legal body measured is **5.2 s**. Publishing the validation term alone as "the ceiling" would
have been M10-R30 in a new place, one round after promoting the rule against it, so DOS-BUD gained a
row for the unbudgeted floor (16 MiB, phone tier off: 228 ms) and the claim is stated as two terms.

**Also closed:** `FailOpen` decided what to swallow by asking `budget.is_exhausted()` — a property of
the *request*, asked about an error that may belong to the *detector*. Correct only through an
unstated invariant, and it turned a genuine GPU or tokenizer failure arriving at a spent budget into a
`400` on a proxy configured to degrade to structured-only. `DetectError` carries the distinction now,
built by `budget_exhausted(..)` vs `unavailable(..)`, and one `fail_open` helper serves both entry
points so they cannot drift apart. E2E-05's digit-run check was re-phrased from *"nothing forbidden
appears"* to *"only these two integers appear"* — its exemption for runs under four digits disabled
the assertion exactly where a truncation leaks (M10-R1's orphaned `912`).

**The new guard is DOS-08**, the one whose absence let this through, and it is written against the
**trait** rather than the type. Its second half failed first on correct code, which is the part worth
keeping: the obvious two-pass input exposes a *card*, and card validation is deliberately free
(M10-R29), so the fixpoint's spend equalled pass 0's. Masking has to expose a **phone** — and an ASCII
word boundary is what creates one. *A guard for "the later passes cost something" must be built from
work the budget actually charges.*

Five findings, five complexity guards, and each written by the blind spot of the one before it: field
size → entity count → alphabet → periodicity → field count → **fixpoint pass**. **218 default / 251
onnx green**, `fmt`, `clippy -D warnings` and the 15-warning `cargo doc` baseline all clean.

## 2026-07-30 — M10 round 4: two closures fell, and the budget turned out to bound the wrong unit

**Round 4 verified the round-3 closures and six of eight hold.** The headline is that the two that
did not are the two that mattered. The full record is in [reviews/M10.md](reviews/M10.md); what
belongs here is what the round *changed about the milestone's own claims*.

**The fail-open hunt came back empty, and that is the good news.** The one thing round 4 was
launched to answer — can an exhausted validation budget ever become a silent *"no PII found"* — is
**no**, on every path walked: `detect()`, `FailOpen` (the structured recognizers are never wrapped in
it), `CompositeDetector`, `CachingDetector` (an error is never cached), `Vault::mask_all`'s fixpoint,
`PrivacyStage`, and the response path. The budget is a call-local `Cell`, so `try_detect` stays a
pure function of its input and `pii::cache`'s soundness argument survives untouched.

**But the budget bounds a *field*, and the client chooses how many fields a body has.** M10-R28:
the same 15.6 MiB that M10-R20 refuses in one field answers **200 in 57 s** split across 78 legal
`messages[].content` fields — and the pre-fix binary is *indistinguishable* on that body, three warm
runs each. So M10-R20's repro really is fixed and its **conclusion** is not: *"the work is bounded"*
is true of a field, and the number that made it a BLOCKER is a property of a request. This is M4's
retrospective lesson 6 — *ask what a guard holds constant* — arriving a **third** time, and each time
the un-varied quantity was one level above the one the guard thinks in. Every complexity guard this
project has written measures one string. The masking path takes a body.

**And the budget is spent by validators it was never sized for.** M10-R29: the counter lives in
`push_candidates` and decrements for *any* recognizer with a validator — the nine always-on
national-ID checksums included, which are nanoseconds of arithmetic rather than the ~6.5 µs
`phonenumber` call the number was derived from. Measured consequence: with `PII_LOCALES=` (phone tier
**not loaded**), 800 KB of bare 9-digit tokens is a 400 in 45 ms where `1.2.0` masked and forwarded
it in 150 ms. A live behaviour regression, and M10-R27's own rule landing on M10-R27's own fix — the
refusal it made actionable is now *confidently wrong*, prescribing a SQL `LIMIT` for a cost that came
from somewhere else.

**Closed in the first pass: R31, R32, R34.** R32 added **E2E-05**, the test M10-R27 shipped without: the
refusal asserted where the client reads it, including that no digit run in the message is drawn from
the body. R34 was filed as a doc comment naming five countries over an array of four — the array was
the smaller half, because the line indexing it said `% 4`, so the fifth was unreachable by
construction. Fixing both moved a measured floor (slot 3: 17 → 15 of 2400), which is the only reason
anyone would know it had been broken. *A collection and the modulus that indexes it are one fact.*

**R33 was half-closed on purpose and held open until R28 landed.** Both READMEs list the budget refusal
among the causes of a 400 — the one an otherwise completely legal body can trigger. Their *"a large body
can't stall the proxy for everyone"* bullet was left **untouched** in the first pass: it claims a
per-request CPU bound, and until R28 set one, any rewording would either restate the false conclusion or
invent a number. It now says what is measured — *linear is a shape, not a budget* — with the 57 s → 0.19 s
figure and the ~1.4 s per-request ceiling. Following the finding's own ordering instruction was worth it.

**R28, R29 and R30 went to the maintainer before any code was written**, which is the process working
rather than stalling: each needed a threshold with functional consequences or a change to visible
behaviour. The decisions taken, recorded here because the reasoning is the part that does not survive
in a diff:

- **What the budget counts → only the phone validator, per `parse()`.** Restores `1.2.0`'s behaviour
  on every non-phone body and makes one unit mean one thing (~2.7 µs), which is what lets a number
  encode a CPU ceiling at all.
- **Where the bound lives → one allowance per request**, threaded through a new `try_detect_within`
  seam on `PiiDetector`. Chosen over a cumulative-bytes cap in the stage, which would bound bytes
  rather than work, and over lowering `MAX_BODY_BYTES`, which changes what the proxy *accepts*
  instead of what it costs.
- **The tag → close everything first**, then round 5, then `1.2.1`.

**Then the measurement moved the threshold, and this is the part worth remembering.** Charging per
`parse()` shrank the effective allowance ~9×, so before publishing anything I measured the case the
maintainer had flagged sessions earlier — an MCP SQL tool pulling a table with a phone column. At
50,000 units an entirely ordinary **367 KB / 5,000-row** result came back a `400`. Raised to
**500,000** (~1.4 s of CPU, ≈50,000 numbers per request). *A fail-closed threshold whose refusal is a
routine event is the wrong threshold* — every refusal costs an agent a turn, and a bound that fires on
legal traffic teaches its operator to raise it rather than to trust it. It is deliberately **not** an
environment variable: a CPU bound an operator can raise is not a bound.

Measured after: the 15.6 MiB body that answered `200` in 57 s is **refused in 0.19 s**; a 6.1 MB /
80,000-row SQL result is still masked (445,005 units, 1.23 s); a real 22 KiB Claude Code turn spends
**0**. Suite green throughout: **217 default / 250 onnx**, `fmt` and `clippy -D warnings` clean on both.

**One incidental fix, because a false red is worse than no guard.** The complexity wall clock was 10 s
against a slowest linear case of 3.3 s, and under `cargo test --features onnx` — every test binary at
once — a 1.9 s case was seen crossing it. Widened to 30 s, with every measured figure written beside
the constant and the ceiling set by the fastest *quadratic* case on record (52 s), so it is a real
separation rather than "big enough to never fail".

**And the milestone's own trap landed a fourth time, on me, while measuring the fix for it.** DOS-BUD's
first SQL generator built its phone column with `(r * 7) % 9000`, which repeats after its period: 20,000
rows measured the *same* 30,781 units as 10,000. Caught only because the two rows were printed next to
each other. Four times now — DOS-05, DOS-06's first draft, PHONE-NAT-10, and this — the same shape:
*a generator that looks varied and silently repeats reports the product's cost as lower than it is.*

## 2026-07-29 — "the CC battery needs a live API key" was never true, and it kept costing decisions

**Corrected on the maintainer's report, and the interesting part is *why* it survived.** The CC
battery does **not** need a credential configured on the proxy. The proxy holds none: it forwards
the client's own (`src/proxy.rs::messages_auth`), which is precisely why M6's live run returned 200
on the first try with **nothing configured** — a fact recorded in this file and in the ROADMAP's own
M6 section. What the battery needs is a human with a working Claude Code pointed at the proxy.

**The claim was a summary drifting from its source.** M5's constraint was real and correctly
written: *a live provider*, at a time when the proxy was OpenAI-compat-only and Claude Code
**could not route through it at all**. M6 shipped the native route and made that false — but the
compressed form, "needs a live `ANTHROPIC_API_KEY`", had already been copied into the ROADMAP and
`TESTING.md`, and it outlived the milestone that falsified it. `MANUAL_VERIFICATION.md`, the
runbook, said the right thing the whole time (*"the recommended mode is the proxy holds no
credential at all"*) — nobody read it, because the summaries were closer to hand.

**What it cost, concretely:** M10 recorded its own open box as blocked on a credential nobody had,
when it was blocked on an hour of the maintainer's time. I repeated the claim to a review agent
across three rounds, so it was never challenged. *A wrong summary of a correct document is worse
than no summary — it is the version people act on.*

Fixed at all three points that state it, each carrying the correction rather than a silent edit,
plus a rule that generalizes: **when the ROADMAP, `TESTING.md` and `MANUAL_VERIFICATION.md` disagree
about how to run something, the runbook wins** — it is the one written while doing it.

Also in this pass: `v1.2.1` comes out of M10's *name* (heading and Status-table link). The tag still
appears in the Status **cell** as `tag `v1.2.1` (planned)`, which is the documented convention and
what `release-build-publish.yml` actually parses — the guard scopes itself to `^\| \[M` rows, so
renaming the milestone cannot move it. And the ROADMAP now carries a **What is left before the tag**
section: round-4 verification, the four CC scenarios, and the `1.2.1` manifest bump at tag time.

## 2026-07-29 — M10 review round 3: every DoS number this milestone published measured the wrong thing

**The blind spot all three rounds shared: every one of them used `unit.repeat(n)`.** M10-R2's
table, its closure, round 2's verification, M10-R13's "the gate bought nothing" — and DOS-05
itself. The per-scan memoization that carries the whole fix is keyed on the matched bytes, so **its
benefit is a property of the input repeating, not of the code**. Same shape, same 4 MiB, varying
only how many candidates are distinct: **207 ms → 17,049 ms**. A legal **15 MiB** body — inside the
default 16 MiB limit — answered `200` after **64.5 s** of CPU at the completely default
configuration.

*A quantity a test never varies is a quantity the test cannot see.* That is M4-R24's lesson and
then M10-R2's, arriving a third time on the same file. The un-varied quantity was not the field
size, not the entity count, not even the alphabet — it was **how often the same bytes come round
again**.

**Closed by bounding the work rather than accelerating it**, because there is nothing to
accelerate: `phonenumber::parse().is_valid()` is ~6.5 µs per region however it is asked, and the one
cheap pre-filter was round 2's leak. So a field's cache-missing validator calls are capped at
50,000, and exceeding the cap is an **`Err` on the `try_detect` channel** — the request is blocked,
never forwarded with a partially scanned field. Same call M5-R7 settled: *a detector may degrade its
own recall, but it may never decide for the caller that degraded output is acceptable.*

| body (distinct candidates) | before | after |
|---|---|---|
| 15 MiB | **64.5 s**, `Ok` | **0.99 s**, refused |
| 4 MiB | ~17 s, `Ok` | 0.70 s, refused |
| 1 MiB | 3.3 s | 0.81 s, `Ok` |
| 4 MiB **repeated** (DOS-05's shape) | 0.38 s | 0.42 s, unchanged |

**The budget is sized from the shipped build, not from the profile the guard runs in** — 50,000
calls is ~0.5 s in release and ~25 s in debug, which is why DOS-06's refusal case is deliberately
not wrapped in a wall clock. Letting the test profile set the product's bound is backwards.

> **Superseded on 2026-07-30 — every number in the two paragraphs above is measured in the wrong
> unit.** The allowance was minted per *call*, so the ceiling was `budget × fields × passes` and none
> of it was a property of a request; the *"0.5 s"* was a third figure again. See the round-4 entry at
> the top of this file, [M10-R28](reviews/M10.md#m10-r28) and
> [M10-R30](reviews/M10.md#m10-r30). Kept as written because this is what was believed on 2026-07-29
> and the belief is the point of a log.

**And writing DOS-06 reproduced the finding one more time.** Its first generator used
`(i * 7) % 9000`, looked distinct, silently repeated after its period, produced a 4 MiB body the memo
absorbed, and reported "the budget was never reached" as though that were the product's doing. It is
an odometer now, with the reason in a comment, because the next person will reach for the modular
hash too.

Two more worth keeping. **PHONE-NAT-10's non-vacuity floor was aggregate and therefore blind where
it mattered**: `Trunk` alone contributed 705 of 1,286 acceptances while the band M10-R13 lived in
yielded **2**, and zero in seven of twenty seeds. The floor is per-shape now, and the generator
*aims* at that band with a real country calling code (2 → 17 acceptances); slot 3 keeps its own,
lower, **measured** floor with the reason beside it, rather than a tidy uniform number that would
mean either a red suite or a slower test. And **the invariant round 1 promoted was overstated**: a
trunk anchor *constrains* where a candidate may begin, it does not forbid a mid-value start —
`0958` inside `020 7946 0958` is a `0` on a word boundary. The consequence differs, and that is the
real point: the shifted span **overlaps** and the resolver unions it, so it is an over-mask, never a
truncation. That is why the trunk families need no shrink.

## 2026-07-29 — M10 review round 2: the fix for the DoS had relocated the leak

**All twelve round-1 closures hold** — verified against the pre-fix tree rather than the diff, so
every "it's fixed" has a matching "it was broken there". Seven new findings, and the sharp one is
the M4 retrospective's signature move committed one more time, by me, on the guard that was
supposed to make it impossible.

**M10-R2's digit-count gate refused numbers our own validator calls real.** It was derived from
`possible_length` plus one character for the trunk prefix. But `phonenumber::parse` normalizes
before it validates: it also strips an **international prefix** and a **bare country calling code**,
so a candidate can carry several more digits than any `possible_length`. `39 3332 2673 8858` — a
real Italian number written with its country code — was rejected before any region saw it, and for
the `Groups` family (regex reaching 15 digits, mask stopping at 13) recall in that band was **0**,
two thirds of them *truncated* rather than merely missed. That is M10-R1's shape, re-entered from
the other side, by the fix for M10-R2.

**It is closed by deleting the gate, and that decision came from a measurement rather than an
argument.** Before rewriting the derivation to cover country codes, I forced the gate fully open
and re-ran the very inputs it was introduced for: **382 → 384 ms** on 4 MiB of arbitrary digit
groups, **258 → 257** on repeated real numbers, **145 → 145** on 781 KiB of *distinct* candidates —
the case memoization cannot help with. It bought **nothing**. The per-scan memoization was already
carrying all of M10-R2. A filter that costs recall and buys no speed has no defence, so it is gone
rather than repaired.

> **The rule, and it generalizes past phones:** *a cheap filter in front of a validator must be
> derived from what the **validator** accepts, not from what the **metadata** describes — and it
> must be proved by a differential test against generated inputs, never by a list the author
> expected it to allow.* Its own guard asserted the superset property over 30 hand-written
> literals — all domestic renderings, so all ≤ 13 digits — while the defect lived at 14–15.
> **An assertion made only where it cannot fail is not an assertion.**

Writing the replacement proved the same point twice more. PHONE-NAT-10 now generates from each
family's grammar; its first version emitted a **four-pair** French form where the family requires
five and reported the resulting miss as a detector bug, so it asserts each generated string matches
its family *whole* before drawing any conclusion from it. And its premise had to become **per
family**: "valid for any enabled region" folds in the deliberate per-region shape restriction and
reports it as a miss — a 14-digit un-anchored run that only Germany's plan accepts is *not* offered
to Germany, on purpose.

The other six were docs and naming, and one is worth keeping: the same latency measurement had been
published as two different pairs of numbers (0.30/0.32 and 0.55/0.57). Settled by re-measuring three
times — 0.30/0.32, identical each run — and the note now records that the outlier existed, so the
next reader knows the spread is real instead of assuming the box is deterministic.

## 2026-07-29 — M10 review round 1: twelve findings, and the two that mattered were both invisible

**The review found a partial leak and an availability blocker, and the same sentence caused the
first one.** `national_phone_recognizers` copied M8.1's fixpoint argument forward verbatim — *"an
over-long span `is_valid` rejects … the next pass masks it"* — into two families the trunk anchor
no longer protects. The anchor was doing more than cutting false positives: it guaranteed a
candidate could only **begin where a number begins**. Un-anchored, a rejected greedy match is
replaced by a *shifted* accepted one, and masking **truncates** the neighbour:

```text
912 345 678 913 456 789   →  912 [PHONE_1]     ← three digits of a real number, upstream in clear
138 0013 8000 139 0013 8001 → 138 0013 8[PHONE_1] 8001
```

**The fixpoint recovers a value it did not touch; it can never recover one the mask ate.** Fixed by
retrying a rejected match one digit group shorter until the validator accepts a prefix at the same
start — precisely what the trunk anchor gave for free. Applied to the un-anchored families only:
doing it uniformly also accepts mid-number prefixes that are valid *somewhere*, which bridged two
adjacent GB numbers into one coalesced span and broke the M8.1 behaviour this milestone promised
to preserve.

**Every existing guard was structurally blind, and that is the durable half.** The M8-R8 test
asserts `detect(masked).is_empty()` — a predicate the leak *satisfies*, since an orphaned `912` is
not detectable, which is exactly why it survived. PROP-03 quantifies over **accepted** candidates
and those bytes belonged to a rejected one, so it passed vacuously (M4-R17's lesson, on a new
candidate generator). The property that matters is **"no byte of a real value survives"**, not
"nothing detectable survives"; PHONE-NAT-09 now asserts it.

**The blocker: a legal 12 MiB body cost 105 s of CPU on an unauthenticated path.** Every candidate
paid up to five `phonenumber::parse()` calls, and `.any()` short-circuits only on *accept* — so a
**rejection is the expensive verdict**, and on adversarial input rejection is the entire workload.
The code comment claimed the opposite ("cheap-to-be-wrong"), which is what would have stopped the
next reader looking here. Two fixes: validator results **memoized per scan** (call-local, no lock;
the validator is a pure function of the matched bytes), and a **digit-count gate read out of
libphonenumber's metadata** before any parse. 4 MiB of digit groups: 45–50 s → **0.70 s**; a 1 MiB
CSV-shaped `tool_result`: 1.94 s → **0.06 s**.

The gate's first cut masked **nothing at all** — `possible_length` on the *general* descriptor is
empty for most regions, so the mask came out 0. Now it unions every descriptor and **fails open**
if the metadata yields nothing: an optimisation may never be the thing that decides a value is not
PII. PHONE-NAT-10 asserts the gate is a superset of what `is_valid` accepts, including the
one-trunk-character assumption its +1 allowance rests on.

**Why `complexity.rs` missed it, one level past M4-R24.** DOS-01…04 vary field *size* and entity
*count* and hold the **character class** constant — `"a"*1M`, `"sk-"*350k`, `"4111 1111…"`,
`"a@b.co "*n`. **Not one produces a phone candidate**, so the whole change was invisible to the DoS
guard. *A quantity a test never varies is a quantity the test cannot see* — and here the un-varied
quantity was not size or count but the **alphabet**. DOS-05 varies it.

**And the measurement itself was wrong, which is the finding I'd least like to have shipped.** The
published per-category zeros (`ports`, `money`, `refs`, `sizes` 0.000) were a property of the pool,
not the detector: an un-anchored candidate needs a **2–3-digit leading token**, and the pool's
non-date entries almost never had one (`chunk 8192 bytes`, `order 2026 1042`, `port 8080` — 4-digit
leads, not candidates at all), while the `LongBlock` family had essentially no representative.
*A corpus has a shape, and that shape is a blind spot* (M4-R13) — landing on the milestone's own
deliverable. Regenerating the pool **from the families' own structure** put CN at offsets 0.250 and
sizes 0.156.

**That drove a real design change rather than just a re-publication.** A single trunk/non-trunk flag
had handed China the Italian-mobile `LongBlock` rendering; regions now declare **which renderings
their numbers actually take** (`Trunk` · `TrunkPairs` · `Groups` · `LongBlock`), which takes CN's
offsets and sizes to 0.000/0.031 with Chinese mobiles still covered, and is held honest by a test
that fails if a declared shape is not needed by any corpus rendering of that country. The corrected
union: dates 0.180, **tables 0.375**, codes 0.091, offsets 0.050, sizes 0.031, ports/money/refs
0.000 — recall still 1.000, curated negatives still 0/20, and still **zero `Phone` spans on the
real 22 KiB turn**.

One curated negative stopped being one under the corrected pool (`512 1024 2048` is the shape of a
real Suzhou landline). It is **pinned as a known over-mask** rather than deleted — dropping a corpus
negative that stopped passing is how a measured cost quietly becomes an unmeasured one.

Also closed: "union-only FPs = 0" was a **structural identity** of the dispatch, not a discovered
fact, and was being cited as the evidence that made all-on safe — it is now an assertion with the
right label. `PII_LOCALES=` (empty) turned all nine regions **on** while ARCHITECTURE named it as
the way to turn the tier off; empty now means none. CLI-06 was green only because the harness shell
exported `NO_COLOR` (it strips ANSI now). CLI-05 scanned a hand-written three-file list and was
already missing `HF_HOME`/`HF_HUB_CACHE`; it walks `src/` now. DEP-01 was a six-name denylist being
cited as a native-dep-free *guarantee* — DEP-02 asserts the property. `--version --bogus` exited 0.

## 2026-07-29 — M10 built: nine domestic-phone regions on by default, measured

**The default now detects something.** `PII_LOCALES` shipped for two milestones with the value
`it,us` — a placeholder M4 chose while the FP-prone tier was *empty*, never revisited when M8.1
filled it with `gb`/`de`. Both codes mapped to no recognizer, so `06 69821234` reached the provider
in clear. Nine regions (`de es fr gb it lv nl pt cn`) are now on out of the box, bounded by the
step-5 principle — exactly the countries the tool already claims (the ten national-ID packs plus the
NER's languages), with US excluded because it has no trunk-`0` domestic form and `ar` excluded
because there is no single Arabic numbering plan. Landing **(b)** from the ROADMAP's ladder:
`PII_LOCALES` survives as an *override* that replaces the default set, so a patch release does not
break an operator who set it.

**The dispatch shape held up exactly as the planning probe predicted.** Latency over the M7 22 KiB
turn: **0.30 ms with no region enabled, 0.31–0.32 ms with all nine.** Adding a region costs
validations on candidates only, never another O(n·L) scan of every field — which is the whole
argument for putting the region loop inside a boxed-closure validator instead of shipping one
recognizer per region.

> **Conditions, recorded because this repo has been burned by omitting them** (M7-R12): reference
> box, `--release`, `--test-threads=1`, **not idle** — there was other load on the machine
> throughout. So these are a busy box's milliseconds, not the product's floor. The claim they
> support is not an absolute anyway — it is that the curve is **flat in the region count**, and
> background load raises a flat line without sloping it. A clean-box re-measure would move the
> numbers, not the conclusion.

**Generalizing the candidate anchor was the real work, and the first attempt was wrong in an
instructive way.** A country that dropped the trunk prefix (ES, PT, IT mobiles, LV, CN mobiles)
proposes *no candidate at all* under a `0`-anchored regex, so a match arm alone would have been a
silent no-op for it. Adding an un-anchored shape family and validating it against **all** enabled
regions produced 8 false positives out of 24 hand-written digit-shaped non-phones: `512 1024 2048
4096`, `30 60 120`, `20 30 40`, `123 456 789`, `100 200 300`…

The cause is not the regex. **libphonenumber's `parse` accepts a national number with *or without*
its trunk prefix** — because in a trunk-prefix country you really can dial a local number that way
— so handing un-anchored digit groups to Germany asks *"could this be a same-area local dial in
Berlin?"*, which is true of an enormous slice of ordinary numeric text. Blamed per region: **DE 7,
FR 4, NL 3, PT 1, and ES/GB/IT/LV/CN 0.** The fix is structural rather than a heuristic: each region
declares whether its numbers are really written without a leading `0`, and the un-anchored family is
validated **only** against those. Same 24 negatives afterwards: DE/FR/NL/GB → 0.

**Two shape decisions were also settled by measurement, and both cost recall on purpose.**
- The un-anchored family **requires separators**. A bare `3471234567` is indistinguishable from an
  order number or a unix timestamp, and bare 9-/11-digit runs are *already* over-masked by the
  national-ID tier under M4-R6.
- Its leading group stays **2–3 digits**. Widening to 4 buys Latvia's `6712 3456` rendering and
  costs four new false positives — every `YYYY NNNN` pair becomes an 8-digit candidate, and
  Latvia's plan (8 digits, `2…`/`6…`) accepts essentially all of them. LV stays covered by
  `67 22 33 44` and `67 123 456`.

**The measurement, which is what M10 was actually for** (`tests/phone_eval.rs`, release; 35 corpus
positives, 20 curated negatives, 385 generated digit-shaped non-phones):

| | recall | curated FP | dates | codes | offsets | sizes | ports · money · refs |
|---|---|---|---|---|---|---|---|
| de · es · fr · gb · nl | 1.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| it | 1.000 | 0.000 | 0.083 | 0.000 | 0.000 | 0.000 | 0.000 |
| lv | 1.000 | 0.000 | 0.125 | 0.000 | 0.000 | 0.000 | 0.000 |
| pt | 1.000 | 0.000 | 0.000 | 0.091 | 0.083 | 0.000 | 0.000 |
| cn | 1.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.042 | 0.000 |
| **union (shipped)** | **1.000** | 0.000 | **0.188** | 0.091 | 0.083 | 0.042 | 0.000 |

> ⚠️ **This table is superseded — the pool that produced it could not reach half the shapes M10
> added, so its zeros mean "unmeasured", not "clean".** Kept as written because the correction is
> the more instructive record: see the review-round-1 entry above, and
> [ARCHITECTURE → *Domestic phone coverage*](ARCHITECTURE.md) for the numbers that hold.

**Reported per category on purpose.** A single blended rate over a pool whose composition you chose
is a number about the pool, not about the product. That principle survived the correction; what did
not survive was the conclusion drawn from *this* pool ("the entire exposure is dates"), which the
regenerated pool refutes.

**Union-only false positives: 0.** Enabling N regions unions their accepted sets, and the worry
going in was that the compound rate would exceed the sum of its parts. It doesn't — nothing is
masked that no single region already masked — so a region's measured cost is also its marginal cost.
*(Also corrected in review: this is a **structural identity** of shape (b), true for every possible
pool, so it is now asserted rather than reported — and it was never the evidence that made (b) safe.
Recall 1.000, the per-category table and PHONE-OM are.)*

**And the check that decided it: on real agent traffic the cost is zero.** Over the M7 fixture — a
genuine 22 KiB Claude Code turn already in the repo, written for a different milestone and therefore
not curated for this one — the shipped default yields **no `Phone` spans at all**. Pinned as PHONE-OM
with a positive control, because "found nothing" and "detector is dead" produce the same empty
vector (M9-R28). The fixture moved to `tests/common/m7_turn.rs`: the guard has to run in the
**default** build and `m7_latency.rs` is `onnx`-only, and two copies of a fixture whose whole value
is *text nobody curated* would have drifted apart.

**A corpus negative stopped being one, and it is the clearest illustration of the compound effect.**
M8.1's `ref 0123456789 rejected` was a GB look-alike; the moment France was enabled it became a real
number — `01 23 45 67 89`, Paris. Replaced with an all-zero run and the story kept in the corpus,
because over-masking another country's real phone number is the *expected* shape of this trade, not
a defect.

**Also shipped, and unrelated to detection:** `--version` (version + target triple + whether the ML
layer is compiled in), a `--help` that lists every environment variable with a test that fails if
one is added without documenting it, an `info` default log level, and **local timestamps with an
explicit offset**. That last one is not the one-liner it looks: `tracing_subscriber`'s `LocalTime`
calls `time`'s local-offset lookup, which refuses to answer once the process is multi-threaded
(CVE-2020-26235) — and `#[tokio::main]` builds the workers *before* `main`'s body runs. The failure
is platform-split (Windows usually answers, Linux/macOS do not), so it ships as "it looked right on
my box". `main` is now a plain `fn` that reads the offset first and builds the runtime itself; an
indeterminate offset falls back to UTC and says so once. **And `time` turned out to be a genuinely
new default-build dependency** — the plan assumed it was free because `Cargo.lock` already had it,
and `cargo tree` showed it was there only under `--features onnx`. Pure Rust, so the native-dep-free
guarantee holds; recorded because that is exactly the class of assumption this project keeps getting
wrong.

## 2026-07-29 — M10 planning: the validator dispatch shape, measured

**The open design call in M10 is closed, and the number came out against the intuition.**
`Recognizer.validate` is a bare `fn(&str) -> bool` that cannot carry a region, so covering N
countries is either (a) one recognizer per region — N regexes scanned over every field — or (b) one
shared regex whose validator tries the enabled regions, which needs `validate` to become a boxed
closure. A throwaway probe (deleted) measured both in the **release** profile on a 22 KiB payload,
252 candidates, 9 regions, with a third shape (b′) added specifically to attribute the difference:

| shape | per pass |
|---|---|
| (a) one recognizer per region | 12.28 ms |
| (b′) shared regex, validate every region | 6.02 ms |
| (b) shared regex, short-circuit | 2.40 ms |

**(a) vs (b′) is the clean comparison**: identical validation work (11,880 calls each), so the 2.04×
gap is *only* the 8 extra passes over the text — ~0.78 ms per scan per 22 KiB, and that term grows
linearly with every region added. Nine regions under (a) would put ~7 ms per field onto a
structured-only path that costs ~20 ms for an entire turn. The short-circuit is worth a further
2.51×, 5.12× in total.

**Conditions, recorded because omitting them is a mistake this repo has already made once** (the M7
"AC versus battery" spread that turned out to separate nothing): the box was **under load** — a
Teams call in progress — with **mismatched memory banks** (16 GB + 8 GB, not a clean dual-channel
pair). The absolute milliseconds are this machine's on a bad day and should not be quoted as the
product's. **The decision survives that, because it does not rest on the timings:** (a) runs the
same validations as (b′) *plus* N−1 extra scans of the text, so it is strictly more work — the
measurement quantifies the win, arithmetic guarantees its direction. A quieter box would change the
size, not the winner.

**Worth recording because the guess was wrong:** debug measured 2.91× and release 5.12×. The
expectation was the opposite — that optimizing would make `phonenumber` validation cheap and leave
scan count looking relatively worse under (a) — so a decision taken from the debug number, or from
argument alone, would have understated the winner by nearly half.

Two clarifications folded into the plan while writing it up. **Region granularity is not what (b)
trades away** — regions are still enabled individually and still bounded by the step-5 principle;
only dispatch changes. And **neither shape "solves" the M4-R19 DoS**: bounded match length is a
constraint on the *regex*, so it binds both equally — what (b) buys is one pattern family to keep
bounded instead of nine, i.e. one place to get it wrong.

**Also considered and rejected: defaulting to the host's locale.** It contradicts M4-R1's rule that
coverage follows *the data that arrives*, not the deployment — a Frankfurt proxy carries Italian
data, and an Italian developer on a US-locale Windows box would silently lose IT coverage, which is
the exact failure M10 exists to end. It would also make masking machine-dependent: same request, two
boxes, two results, nothing in the logs to explain it. Under (b) an extra region is cheap enough
that guessing buys nothing over covering what we already claim to cover.

## 2026-07-29 — Version in the artifact filename: built, then rolled back for `--version`

**The ask was "can the release tag be in the artifact name?" — yes, and it was implemented**: a
`version` input on the reusable `release-build.yml` (needed because its other caller,
`manual-build.yml`, runs on `workflow_dispatch` where no tag exists and fell back to
`<Cargo.toml version>-dev-<short sha>`), producing `llm-proxy-pii-rust-v1.2.0-<target>[-variant]`.
Both branches were simulated locally and the YAML validated.

**It was then reverted, on the maintainer's call, in favour of `--version` — the better answer, and
the reasoning is worth keeping.** A filename is a *convention*: it identifies the artifact only
until someone renames it, and this project's own README tells operators to rename it (to
`llm-proxy-pii-rust`) as step one. A binary reporting `CARGO_PKG_VERSION` cannot be made to lie by
a rename. The filename approach also forfeits GitHub's stable `releases/latest/download/<name>`
redirect, which needs a fixed name. So the pipelines are unchanged and the capability moves into
the binary, tracked in [M10](ROADMAP.md#m10).

**And measuring that led to the sharper finding: with `RUST_LOG` unset the proxy prints nothing.**
`EnvFilter::from_default_env()` falls back to ERROR-only, so a healthy start is **completely
silent** — no `listening on`, no `ONNX NER detector loaded`. Confirmed by running the binary both
ways, not inferred. That quietly broke the check the previous DEVLOG entry had just told operators
to make ("no NER line ⇒ structured-only"), because with no `RUST_LOG` *every* line is missing and a
silent process looks the same whether it is healthy or wedged. Both READMEs now set `RUST_LOG=info`
in the Quick start and say plainly that this is a defect we own; M10 changes the default, keeping
`RUST_LOG` as the override. Checked before proposing it: `Config`'s manual `Debug`
(`src/config.rs:251`) redacts `upstream_api_key`, so an `info` default does not start printing a
credential.

M10 also picked up **`--help`**: it exists, but lists only the two flags and defers configuration to
the repo — for a tool with no config file and ~30 environment variables, that leaves the shipped
binary unable to say what it accepts. Pinned against drift by a test that scans the source for every
env key and fails if one is missing from the help text.

## 2026-07-29 — Docs: how to run the *released binary* (the gap nobody had noticed)

**The hole.** Every "how do I start it?" path in this repo assumed you were building from source:
README's Quick start ran `cargo build-onnx --release` out of `target/onnx/`, `SETUP.md` is titled
*Development setup* and uses `cargo run-onnx` throughout, and `MANUAL_VERIFICATION.md` uses the
alias **on purpose** (staleness). The only place a release artifact was ever mentioned was the
per-backend download table — which lives inside the *GPU acceleration* section and exists to
explain execution providers, not installation. So an operator who downloaded the binary this
project ships had **no documented path at all**: not how to configure it, not how to keep it
running. `release-build.yml` packages a bare executable — no README, no env sample — so the
artifact didn't carry the answer either.

**Two things it had to say that were nowhere in writing.** First: *there is no config file*, and
that is a decision, not an omission (`src/main.rs` — no arg parser, no dotenv; nothing to leave a
credential in, no file-vs-env precedence to reason about). Second, and this is the one that
matters: **a released binary with no model configured is a structured-only proxy, silently.**
Every asset is built `--features onnx`, but the model isn't bundled, and with neither
`NER_MODEL_PATH` nor `NER_MODEL_REPO` set the proxy starts happily and forwards names in clear.
That is the exact failure that burned the first live verification run (2026-07-16) — where it was
caught by `NER_REQUIRED=1`, a flag the release-binary user had never been told about. It is now a
warning box in the Quick start, with the two-variable fix and the pair of startup lines that tell
you which proxy you are actually running.

**Also stated for the first time: the Linux assets are plain ELF executables, not installers.**
`release-build.yml`'s Package step `cp`s the binary and stops — no `.deb`, no `.rpm`, no archive.
The one thing a downloaded asset lacks on Unix is the execute bit, which is now in the snippet.

**The shape it landed in: commands, not tooling.** The first pass built a `deploy/` directory — an
annotated `proxy.env` plus a `run-proxy.sh` / `run-proxy.ps1` pair that applied it — and a hardened
systemd unit. All of it was **deleted before commit**, on the maintainer's call, and the reasoning
is the part worth keeping:

- **A launcher script is a second thing to maintain and a second thing to trust.** What the reader
  actually needed was four `export` lines. A script that reads an env file is machinery *around* a
  configuration model that is already one line per variable — it adds a file format, a parser and
  its failure modes, to save nothing.
- **A unit file per init system is the wrong shape for "run it as a service."** Once that is the
  goal, a **published OCI image** is the answer that generalizes; it stays Backlog, and the README
  no longer half-implies otherwise. Same for a `launchd` plist.
- **Prose was the other thing cut.** The section had grown a warning box, a rationale paragraph and
  a table of launchers before it got to "here is how you start it". It is now: download, four
  variables, run — then a three-row table for the only variables whose *absence* changes what gets
  masked, and the two startup lines that tell you which proxy you got.

**Then the doc pass turned into a measurement, and found a hole. `PII_LOCALES` does less than
anyone assumed — including its own default.** `fp_prone_recognizers` (`src/pii/recognizers.rs:313`)
matches **only** `gb` and `de`; `it` and `us` return an empty vec, so the shipped default `it,us` is
inert. That is not pedantry about a knob: a throwaway probe (deleted after reading) drove
`StructuredRecognizers::with_locales` over real domestic numbers from nine countries, and

**with the default configuration, no Italian domestic phone number is masked** — `06 69821234`,
`011 5627111`, `347 1234567` all go upstream in clear. `+39 347 1234567` is caught by the universal
`+CC` arm, and `320 123 4567` only because its 3-3-4 grouping collides with the universal *US* arm.
For a tool whose stated locales are IT + US, that is the finding, not a footnote.

The other half of the probe: **`de` masks Rome, Milan, Naples and Florence landlines — but not
Turin** — because a locale code names a *numbering plan*, and plans overlap per number rather than
per country. FR/BE/AT/CH/IE numbers are untouched by either code. So "enabling `de` covers other
countries too" is true by accident and false by design, and a libphonenumber metadata update could
move it with no change of ours. The measured matrix is promoted into `ARCHITECTURE.md` → *Locale
coverage* (the design file, per the promote rule); the work is **[M10](ROADMAP.md#m10)**.

**M10 opened, scoped to every missing country rather than IT alone, and targeted at `v1.2.1`** — a
patch, because it closes a gap between what the tool claims and what it does rather than adding a
capability. History checked while writing it: nothing regressed. M4 introduced `PII_LOCALES` when
the FP-prone tier was **empty**, picking `it,us` as a placeholder naming the project's own locales;
M8.1 added the tier's first two entries without revisiting that default. The mismatch is a leftover
of "the tier had nothing in it".

**The library is not the blocker — we are.** `phonenumber` 0.3.10 ships **245 regions**; every
country worth adding is already in `country::Id`. What actually gates the work is two things of our
own. First, the candidate regex is **`0`-trunk only** (`recognizers.rs:346`), so countries that
dropped the trunk prefix — ES `91 123 45 67`, and Italian **mobiles** `347 1234567` — propose no
candidate at all and a match arm alone would be a no-op; generalizing that anchor widens the
candidate set sharply and leaves `is_valid()` as the only filter, which is where the FP risk moves.
Second, cost scales with N: a candidate is validated per region until one accepts, on the
deterministic path that is today the *fast* one. So "turn on all 245" is not a design.

**Which is why M10 is not scoped as "delete the variable".** The gate exists because M4-R1 called
the un-anchored phone FP-prone — an objection M8.1 defused by measurement, which does argue the
opt-in has outlived its reason. But precision was measured **per region**, and enabling N regions
unions their accepted sets: the union's FP-rate is ≥ the worst single region's and grows with N.
That number doesn't exist yet, so the deliverable is the measurement and the default follows from
it. `PII_LOCALES` stays *accepted* whatever is decided — silently dropping a documented variable
would break every operator who set it, which a patch release must not do.

Docs corrected in the same pass: both READMEs (Quick start row + Configuration table),
`ARCHITECTURE.md` (the matrix), and `SETUP.md` — the worst of them, still claiming the FP-prone tier
was empty and `PII_LOCALES` a no-op, which M8.1 had made false.

## 2026-07-20 — Dual licensing (AGPL + commercial)

**Decision.** Sole-contributor project, sole copyright holder — no CLA backlog to clear, so
dual-licensing is a documentation change, not a legal migration. Kept `AGPL-3.0-or-later`
(`Cargo.toml`, `LICENSE`) as the open-source track unchanged; added a commercial track for use
cases the AGPL's network-copyleft doesn't cover (embedding a modified version, or linking the
crate into a closed-source product). AGPL itself stays free for everyone, companies included —
running the proxy unmodified never triggers anything to release.

**Added `CONTRIBUTING.md` / `CONTRIBUTING.it.md`** with a copyright-assignment clause: an
external contribution assigns copyright to the maintainer (with a grant-back of the same rights
any AGPL user gets), which is what keeps "sell a commercial license on the whole codebase"
possible once contributions aren't 100% solo-authored. Agreement is indicated via a signed-off
commit (`git commit -s`) plus a checkbox in the new `.github/PULL_REQUEST_TEMPLATE.md` — silence
isn't treated as consent.

**Added `.github/ISSUE_TEMPLATE/commercial-license.md`** as the inbound channel for commercial
inquiries (chosen over a public email address). README / README.it's License section now states
the dual-license model and links both.

## 2026-07-19 — M9: execution-provider selector (all backends wired) + the DirectML spike

**Opened M9 (GPU optimization).** The box has an **AMD Radeon DX12 iGPU** — which settles the
backend choice by elimination: CUDA is NVIDIA-only, so on this hardware **DirectML is the only GPU
path** (and, being D3D12, the only vendor-agnostic one on Windows). ONNX Runtime has **no Vulkan
EP**, so the "most compatible" intuition maps to DirectML (Windows) / WebGPU (cross-OS), not Vulkan.

**Spike first, per M9's own gate (`don't spend until the GPU actually beats the CPU here`).** A
throwaway crate (`scratchpad/gpuspike`) ran the shipped XLM-R **int8** model on CPU vs DirectML:
- **Stage A (power-independent): green.** The `directml` feature compiles, the ORT-with-DML binary
  downloads and loads, the AMD DX12 device is accepted, int8 runs. Toolchain + hardware de-risked;
  the make-or-break unknown (does `download-binaries` ship the DML EP? → **no, needs `ort/directml`**,
  confirmed in `ep/directml.rs:103`) is answered.
- **Stage B (on battery, indicative only): int8-on-DML is 2–5× *slower* than CPU** (0.20–0.46× at
  seq 128/256/512). Expected and **not the verdict**: int8 is a CPU-optimized format, and int8 ops
  fall back / partition badly on DirectML. The fair test is **fp16 on DML, on AC power** (Stage B').

**Stage B' — the fair test, on AC power. Verdict: NO-GO for DirectML on this hardware.** fp32 XLM-R
(`onnx/model.onnx`, 1.11 GB) was converted to fp16 (555 MB, `onnxruntime.transformers.float16`,
`keep_io_types=True` so `logits` stays fp32) and the full matrix measured (ms/inference):

| config | seq 128 | seq 256 | seq 512 |
|---|---:|---:|---:|
| **CPU int8** *(shipped)* | **26.5** | **47.1** | **121.3** |
| CPU fp16 | 108.9 | 166.3 | 328.9 |
| DML int8 | 104.8 | 344.3 | 607.6 |
| DML fp16 | 18.3 | 104.7 | 322.9 |

**DML-fp16 vs CPU-int8: 1.45× / 0.45× / 0.38×.** Three conclusions. (1) **fp16 was indeed the right
test** — DML-fp16 crushes DML-int8 (18 vs 105 ms at seq 128), confirming the GPU really runs and that
Stage B was a false negative. (2) **CPU-fp16 ≫ slower than CPU-int8** re-confirms ORT up-casts fp16→fp32
on CPU (DEVLOG M8). (3) **But the iGPU wins only at short sequences and loses 2–2.6× where it matters:**
fields are chunked to `MAX_WINDOW_TOKENS` = 480, so the latency-dominant inferences run at seq ~480–512,
exactly where DML is ~2.6× slower. The seq-128 win is +8 ms on a case already at 26 ms — invisible.
**Mechanism:** a shared-memory iGPU is bandwidth-bound, and attention cost grows quadratically with
sequence; 12 CPU threads on int8 beat it. A hardware ceiling, not a code one — dimension-overrides or
`HighPerformance` don't close 2.6×, and this box has no discrete GPU. **So CPU-int8 stays the default.**
The verdict is about *this iGPU*: a discrete GPU (10–20× the bandwidth) would very likely win, and with
the selector that is a config flip.

**Landed the EP selector regardless — the multi-backend scaffolding is wanted independently of
which GPU wins.** `NER_EXECUTION_PROVIDER` (default `cpu`) selects the backend for **both** the
XLM-R NER and GLiNER, resolved once in `server.rs` and passed to `OnnxNerDetector::load` /
`GLiNerDetector::load`. The selection + **CPU-fallback** policy has one home, `onnx::build_session_pool`,
which both detectors' pools now build through (this also deleted GLiNER's duplicate session-builder).

- **All seven accelerators reachable, only DirectML tested** — an accepted trade-off (we run
  DirectML). Every EP *type* compiles on every platform (only `register()` is feature-gated in
  `ort`), so an absent provider just fails registration and **falls back to CPU, loudly**
  (`.error_on_failure()` → explicit `warn!`, not ORT's silent drop). *(Superseded the next day: the
  platform's accelerator moved into a per-target `ort` feature, so `--features onnx` carries it and
  the `ep-*` features became escape hatches — see the 2026-07-20 entry.)* The measured/trusted table
  lives in `ARCHITECTURE.md` → *Execution providers*.
- **Fail-closed reasoning (why untested EPs are safe):** the fallback covers *initialization*, not
  *numerical correctness* — but an EP that loads-and-computes-wrong can only cost **NER recall**
  (best-effort); the deterministic structured layer runs on CPU regex, independent of the EP, so it
  is untouched by any backend. Blast radius bounded to the layer that already tolerates misses.
- **A typo fails startup, an absent-but-real GPU falls back** — two different failures, kept
  distinct (`parse` rejects `vulkan`/`gpu`; `build_session_reporting` falls back for a real provider). M8-R5's
  rule applied to the accelerator knob.
- Tests: `onnx::ep_tests` (parse round-trips, case/alias handling, unknown→Err incl. `vulkan`).
  Default build green; `clippy-onnx -D warnings` clean; 143 lib tests green; `cargo build-directml`
  (new `.cargo` alias, own `target/directml` dir) compiles the DirectML build.

**`--bench-providers`: ship the measurement, not the number (`src/pii/bench.rs`).** The M9 verdict is
**hardware-specific** — it is true for this iGPU and probably false for a discrete GPU — so publishing
our number would mislead every operator whose box differs. Instead the binary can measure the
**model × provider matrix** on the machine it is running on and name the winner. Both axes are needed:
backend and quantization are *coupled* (CPU wants int8, a GPU wants fp16), so a single-model sweep
across providers — the first version of this — reproduces exactly the false negative that cost this
milestone a round. The configured model plus `NER_BENCH_MODELS` (comma-separated extra variants) form
the matrix; the report picks the winner at seq 512 (the chunked-field operating point).

The report encodes the two things we got wrong before catching them: it **always** warns that an int8
model makes a good GPU look 2–5× slow (and flags loudly when the run has int8 but no fp16, i.e. cannot
answer "is the GPU worth it?"), and it warns to measure on an **idle** machine — the same box measured
~3× slower right after a 13-minute LTO build, with the *ranking* intact but the absolute numbers not.

**The review tail — nine rounds, 29 findings, none a leak.** Worth recording because the *shape* was
consistent and the code was rarely the problem. M9's own scope was review-clean by round 7; rounds 1–6
were mostly **incomplete sweeps**: a claim would be corrected in the file the finding cited while the same
sentence stood in three others, twice leaving two documents giving **opposite verdicts on the same
feature**. That produced the rule now in ARCHITECTURE — *fix every site that makes a claim, not the site
the finding cited; grep the claim, and the category* — and its sibling in TESTING: *when a fix changes
mechanism, re-read the surviving prose against the new code, not against the old defect.* Three
measurement errors of the same family (int8-on-GPU, the ~3× inflated absolutes, the seven-feature WebGPU
link failure) produced the other: **a result measured with several `ep-*` features says nothing about any
one of them.**

**The one real bug the review surfaced was not M9's** (M9-R28). Running both suites back-to-back under
load exposed a pre-existing flake in `anonymizer`'s log-capture helper — and behind it something worse:
`a_clean_convergence_stays_silent` asserts only an *absence*, and a broken capture also yields an empty
buffer, so it could never distinguish "correctly silent" from "capture dead". It reported `ok` in a run
where its sibling proved the capture had failed. Fixing it took two attempts: a `Mutex` serializing the
two callers **did not work (7/30)** because the interference comes from the other ~114 tests, not the
sibling — `tracing` caches per-callsite interest off a **process-global** max level. A global subscriber
installed once with per-thread routing did (0/30, then 0/10 full-suite; the reviewer independently ran
178 clean against a validated 7.5% pre-fix baseline). The absence-test now carries a liveness probe.
**Any assert-absence test needs a positive control** — promoted to TESTING.

**Validated against the spike it replaces.** Run on the same box, the shipped tool reproduced the
throwaway harness's verdict — CPU-int8 fastest at seq 512 (296.9 ms) vs DML-fp16 (714.3 ms, 0.42×),
DML-fp16 ahead at seq 128 (39.8 vs 66.1 ms, 1.66×), CPU-fp16 slow throughout, DML-int8 worst. Both
runs also landed ~2–3× above the idle-machine numbers because each followed a long compile — the
*ranking* was identical every time, which is exactly why the report leads with ranking and warns
about absolutes rather than publishing a millisecond figure.

**"Can we just compile in every backend?" — measured, and no (2026-07-20).** The obvious wish, and
worth an experiment rather than an opinion. `cargo check` with all seven `ep-*` features passes;
`cargo build` then fails at **link** with `LNK1181: cannot open input file 'webgpu_dawn.lib'`, which
I read at the time as "`ep-webgpu` does not link on Windows". **That inference was wrong** — see the
2026-07-20 correction below: `resolve_dist` keys on the *combination*, the seven-feature key matches
no row and falls back to the plain tarball while `static_link` still emits the WebGPU link directive,
and `ep-webgpu` **alone** resolves to a real prebuilt. Drop WebGPU and the other **six link fine** —
which looked like a yes, until
the binary ran: `cpu` and `directml` report `ok`, while **`cuda` / `tensorrt` / `coreml` / `rocm` /
`openvino` all report `unavailable — fell back to cpu`**. The reason is that `download-binaries`
fetches **one** ONNX Runtime distribution and *that* decides which EPs exist; a cargo feature only
decides whether our `register()` is compiled. Cross-platform it is impossible regardless — CoreML
ships only in macOS builds, DirectML only in Windows ones. (Pleasing side note: the fallback
reporting built earlier in this milestone caught all five, so not one CPU row was presented as a
GPU. The honesty machinery worked in exactly the condition it was built for.)

**So: one natural accelerator per platform, set per-target rather than per-command.** Each platform
declares its own `ort` feature in `Cargo.toml`, so **the plain `--features onnx` build carries the
GPU** — no special build, and no way for a developer build and the release pipeline to disagree
about the accelerator (they read the same manifest). The now-redundant `*-directml` cargo aliases
were deleted: they implied a special build was needed to touch the GPU, the same misreading the
READMEs had to be rewritten to kill.

**Which platforms could be wired *without* a CI run turned out to be answerable from `ort-sys`'s
source, not from guessing (M9-R13).** `resolve_dist` keys the downloaded prebuilt on `training`,
`webgpu`, `cuda|tensorrt`, `nvrtx`, `rocm` — and nothing else. So:

- **DirectML is not a key**, and `build/static_link/mod.rs` links it on *every* Windows target
  unconditionally ("pyke libs always ship compiled with DirectML on Windows"). Wiring it is a
  **no-op on the artifact** — provably safe on both Windows arches, no CI run needed.
- **CoreML is not a key** either: macOS fetches the same tarball as before. Same reasoning.
- **CUDA *is* a key.** On Linux the key becomes `cu12`, so `x86_64-unknown-linux-gnu` genuinely
  downloads a different, larger prebuilt on every build — NVIDIA hardware or not. And `dist.txt`
  has **no `cu12` row for `aarch64-unknown-linux-gnu`**, a shipped release target: requesting it
  there resolves to nothing and **silently falls back** to the plain distribution, so the build is
  green and the accelerator is simply absent. Hence CUDA is wired for **x86_64 Linux only** —
  better to claim nothing than to claim an accelerator that isn't in the binary.

The general rule now lives next to the provider table in ARCHITECTURE: **wiring an EP per-target is
free only when `ort-sys` does not key its distribution on it — check `resolve_dist` first.** A green
CI run would not have caught the arm64 case; both Linux legs compile either way.

**That forced the provider list to become a runtime question.** With the accelerator arriving via a
per-target dependency, `#[cfg(feature = "ep-directml")]` is *false* on the very machine that has
DirectML — so `available_providers()` now asks `ort::ep::ExecutionProvider::is_available()` what the
linked distribution actually contains. A feature-derived list was wrong in both directions: it
showed five GPU rows that were CPU in the six-feature experiment, and it would miss the per-target
accelerator entirely. Asking the binary cannot lie either way.

**It works in every build, and never advises a rebuild.** Which providers exist is a build-time
choice, but the platform's accelerator is already wired per-target — so when none was measured the
report explains *this machine's* situation rather than naming a cargo feature the operator already
has (that mistake shipped twice, in both branches; `BENCH-01` and `CLI-03` together pin it shut —
one per branch, because neither can reach the other's). A session that
falls back is reported as **`unavailable — fell back to cpu`** rather than silently logged as a GPU
measurement (`build_session_reporting` exposes the *effective* provider for exactly this). Without
the `onnx` feature the flag still runs and explains that there is no ML layer to accelerate,
instead of erroring — the command behaves the same everywhere.

## 2026-07-19 — M8.1: national phone recognizer (GB/DE), the deterministic path that beat GLiNER

**M8 pointed the un-anchored national-phone gap at GLiNER's context. A post-merge feasibility study found
a deterministic path that is better on every axis, so M8.1 takes it.** The gap: a domestic phone with no
`+CC` (GB `020 7946 0958`, DE `030 12345678`) collides with order numbers / sort codes / national IDs, so
the deterministic layer never matched it and XLM-R doesn't cover phone at all — it went upstream in clear.

**The study (throwaway probe `scratchpad/phonestudy`, then reproduced in-tree).** Two questions:

1. **Footprint — does `phonenumber` breach the native-dep-free default-build bar?** No. `cargo tree` on
   `phonenumber v0.3.10+9.0.33`: **zero native/C deps** — no `*-sys`, no `cc`/bindgen, nothing on
   `tests/dependency_footprint.rs`'s forbidden list (hf-hub / ort / tokenizers / aws-lc). Pure Rust, so it
   ships in the **default** build without breaking the guarantee (guard stays green). Accepted costs, real
   and recorded: ~15 net-new crates incl. a few **unmaintained** transitive ones (`oncemutex` 2016,
   `regex-cache` 0.2.1, old `regex-syntax` 0.6.29 alongside 0.8); **+~3 MB** binary from the embedded
   worldwide metadata (1.5 MB XML → postcard blob at *build* time via `quick-xml`, `include_bytes!`'d; at
   runtime a one-time `Lazy` postcard deserialize on first validation — no XML parsing on the hot path).

2. **Precision — is `is_valid()` precise enough on the FP-prone tier?** Yes, decisively. Faithful
   two-stage model (loose `0`-anchored regex → `is_valid`) on an adversarial corpus of 22 real GB/DE
   nationals + 22 phone-shaped non-phones:

   | validator | precision | recall | FP-rate |
   |---|---|---|---|
   | `parse().is_ok` (loose, length-only) | 0.647 | 1.000 | 0.545 |
   | **`is_valid()` (strict, assigned range)** | **0.955** | 0.955 | **0.045** |

   Per-locale under strict: **GB precision 1.000 / recall 0.917 / FP-rate 0.000** (one FN = the Ofcom
   fiction range `07700 900123`, which libphonenumber correctly rejects); **DE 0.909 / 1.000 / 0.100**
   (one "FP" = `0049301234` = `00 49 30 1234`, a real international dial to Berlin). The loose→strict jump
   is the whole point: `is_valid` checks the candidate against the region's **real numbering plan**
   (assigned prefixes + lengths), the check a hand-written regex can't do — which is exactly why M4-R1
   called the un-anchored form FP-prone and shelved it. That objection is now defused.

**Why deterministic, not GLiNER:** no second ML model, no inference latency, no recall gap, runs in the
default build. GLiNER keeps its real role (names / orgs / **free-form address** — genuinely contextual);
only its *phone* motivation moves here. M8 is not wasted — the decision (addition, not successor) stands.

**Design.** `fp_prone_recognizers("gb"|"de")` now returns a `PiiKind::Phone` recognizer sharing one
bounded-group `0`-trunk regex — compact `0\d{6,11}` plus 2- and 3-group forms
(`0\d{1,4}[ -]\d{3,4}[ -]\d{3,4}` …) — gated by a per-locale `is_valid()` validator. **Bounded groups on
purpose:** an open `(?:[ -]?\d)+` would grab `020 7946 0958 0161 496 0000` as one over-long span that
`is_valid` then rejects — a *leak*; enumerating a fixed group count means a match can't run across two
adjacent numbers (the same guard the universal phone / IBAN patterns use). Max ~15 chars → length-bounded,
so `Scan::Overlapping` stays linear (M4-R19). ASCII `(?-u:\b)` (M4-R13) keeps a `0`-run inside a longer
ASCII token (`user0207946095@…`) from being a candidate. `validate` is `fn(&str) -> bool` (can't carry the
region), so `gb_phone_valid` / `de_phone_valid` are thin wrappers over `national_phone_valid(Id, s)`.

**Two things the in-tree tests surfaced that the isolated probe didn't:**
- **The universal 3-3-4 phone arm already masks any 3-3-4 shape *unvalidated*.** So an adversarial "must
  not mask" case has to be a **compact** `0`-run (no separators) — those reach the `phonenumber` validator
  and are correctly rejected; a 3-3-4-shaped junk number would be masked by the pre-existing universal arm
  regardless of M8.1. The tests use compact look-alikes (`0000000000`, `0123456789`, `0999999999`).
- **The validator is not a locale *discriminator*.** National numbering plans overlap: `07911 123456`
  (a GB mobile) also validates as DE. That is **privacy-safe** — over-masking a real phone is never a leak —
  but it means "enable GB only" does not reject every DE number. Documented in ARCHITECTURE; a London
  geographic number *is* rejected by the DE validator, so the plans aren't identical, just overlapping.

**Tests (all in `recognizers.rs`):** detection across GB/DE shapes, `PII_LOCALES` gating (GB number not
masked with `gb` off — using the 3-4-4 form the universal arm can't catch, so the gate is what's under
test), validator rejects compact look-alikes, no adjacent-number swallowing, direct validator unit tests.
**115 default lib green, clippy + fmt clean.** Version stays **1.1.0** (M8.1 is part of M8). Reviewer loop
next.

## 2026-07-19 — M8 GLiNER implemented, measured, and shipped **opt-in** (not a successor on int8)

**Built the whole M8 slice and validated it end-to-end against the real model**
(`onnx-community/gliner_multi_pii-v1`, int8 `model_quantized.onnx`, 349 MB, pinned). New:
`src/pii/gliner_decode.rs` (model-independent: the regex word splitter, the span decode, the
label→`PiiKind` map — 12 unit tests), `src/pii/gliner.rs` (`GLiNerDetector`: prompt + tensor build,
the session pool, word-window chunking — 5 unit tests), the `load_gliner` opt-in wiring in
`server.rs`, and `tests/gliner_eval.rs` (smoke + eval + inertness canary, `#[ignore]`d, gated on
`GLINER_MODEL_PATH`). 132 onnx / 108 default lib tests green, `clippy` clean on both feature sets.

**S0 — the ONNX I/O contract, verified from the real export** (not guessed). GLiNER span-mode
`markerV0`: inputs `input_ids` / `attention_mask` / `words_mask` (i64 `[1,L]`), `text_lengths`
(i64 `[1,1]`), `span_idx` (i64 `[1,S,2]`), `span_mask` (**bool** `[1,S]`); output `logits`
(f32 `[1, num_words, max_width, num_types]`). `S = num_words·max_width`, word-major, so the flat
logit index is `(word·max_width + width)·num_types + type` — the decode reads it directly.

**Two bugs the smoke test caught on the real model** (the reason S0 must be *run*, not reasoned):
1. **Word splitter.** GLiNER's `whitespace` splitter is not "split on spaces" — it is the regex
   `\w+(?:[-_]\w+)*|\S` (verified against the reference `gliner` lib), which **separates trailing
   punctuation**: `"Milano."` → `["Milano", "."]`. A pure-whitespace split kept `"Milano."` whole
   and measurably lowered scores. Fixed in `split_words`.
2. **Threshold.** The int8 model's confidences run low; correct entities cluster at **0.15–0.6**, so
   the nominal 0.5 missed most of them. Default set to **0.15** by a measured sweep (below).

**The measured decision (S2) — score through the hybrid on `ner_cases.json`, int8, threshold sweep:**

| threshold | Person R/P | Org R/P | Location R/P |
|---|---|---|---|
| 0.30 | 0.58 / 0.88 | 1.00 / 1.00 | 0.27 / 1.00 |
| 0.20 | 0.58 / 0.78 | 1.00 / 1.00 | 0.64 / 1.00 |
| **0.15** | **0.58 / 0.78** | **1.00 / 1.00** | **0.91 / 1.00** |
| 0.10 | 0.58 / 0.64 | 1.00 / 1.00 | 0.91 / 1.00 |

At the 0.15 optimum GLiNER **matches XLM-R on Location (0.91) and Organization (1.00)** but its
**Person recall is stuck at 0.583 at every threshold** — single-word / CJK / Arabic names (Tizio,
Caia, 张伟, محمد أحمد) it never scores, where XLM-R's floor is **0.83**. **So int8 GLiNER is *not* a
successor** — replacing XLM-R would regress name recall, i.e. more leaks, on the most important kind.
**Decision: ship it opt-in, off by default.** XLM-R stays the default NER; GLiNER is enabled via
`GLINER_MODEL_PATH` (+ `_TOKENIZER_PATH` / `_CONFIG_PATH`), adding what XLM-R *can't* do — contextual,
open-label kinds like a **bare national phone** (`"020 7946 0958"`, no `+CC`, the M8 recall gap) and a
free-form address. This is the "measure first, the milestone may say no [to successor]" gate doing its
job, exactly like Piiranha at M2 and the stop-at-the-bar call at M7.

**Quantization sweep (2026-07-19, follow-up) — is int8 the reason, or the model?** The low int8
confidences prompted the obvious question: does *less aggressive* quantization clear the successor bar?
Downloaded and scored all three variants through the hybrid on `ner_cases.json`:

| variant | size | Person R/P | Org | Location R/P | note |
|---|---|---|---|---|---|
| **int8** | 349 MB | 0.583 / 0.778 (@0.15) | 1.00 | 0.909 / 1.00 | the lean default; highest precision |
| **fp16** | 580 MB | **0.667** / 0.667 (@0.3) | 1.00 | 0.909 / 0.909 | +Caia, +Amsterdam; confidences run higher (Location already 0.909 at the nominal 0.5) |
| **fp32** | 1.16 GB | **0.667** / 0.667 | 1.00 | 0.909 | **identical to fp16** |

**Two conclusions.** (1) Less aggressive quantization *does* help — Person recall **0.58 → 0.67** and the
scores calibrate better — so int8 was hiding some of GLiNER's ability. (2) But **the verdict holds at full
precision**: even fp32's Person recall (0.667) is below XLM-R's 0.83. Quantization explains ~half the int8
gap; the rest is the **model** — single-word / CJK / Arabic names (Tizio, 张伟, محمد أحمد) it doesn't score
at *any* precision. The gain also costs ~0.11 precision (pronoun false positives — `She`/`I`/`me` → Person).
**fp32 ≡ fp16 because ORT up-casts fp16→fp32 on CPU**, so fp16 already delivers fp32 accuracy: **fp16 is the
higher-recall GLiNER option, and fp32 is pointless on CPU** (2× the RAM for the same result). int8 stays the
lean default; an operator wanting max GLiNER recall uses **fp16** (`GLINER_MODEL_PATH=…/model_fp16.onnx`,
~580 MB). "Not a successor" is now measured across the whole quantization spread, not just int8.

**The inertness canary — "GLiNER especially" (M5-R4) confirmed, and safe.** Run directly on
placeholder-dense text, GLiNER **does** tag our own `[PERSON_1]` / `[ORG_1]` tokens as entities (int8
XLM-R does not — that is why the docs singled GLiNER out). It is safe regardless: `keep_maskable`
drops an *exact* `[KIND_N]` hit by construction (CC-08) and **S4** keeps GLiNER off every pass after
the first, so `mask_all` on placeholder-dense input **converges with the text unchanged** (verified —
`gliner_placeholder_inertness_canary`), never a fixpoint 400. `redetect → empty` (idempotent after
pass 0) carries to GLiNER for the same reason it does to XLM-R.

**Integration shape.** `GLiNerDetector` implements `PiiDetector`, so it drops into `CompositeDetector`
next to the structured recognizers and XLM-R; the overlap resolver dedups a GLiNER guess against a
deterministic match (checksum wins), and a GLiNER false positive is an over-mask, never a leak. GLiNER
maps `"phone number" → Phone` / `"address" → Location` deliberately (email stays with the deterministic
layer). `NER_REQUIRED` now means "≥1 ML detector (XLM-R and/or GLiNER) must load and run unwrapped";
`GLINER_LABELS` / `GLINER_THRESHOLD` / `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` tune it. Explicit
local paths only for now (the airtight-privacy path); an `hf-hub` auto-download parity with the NER is
a documented future addition.

**Reviewer round 1 (2026-07-19) — 5 findings, none a leak; all closed. One of them was load-bearing.**
The reviewer independently reproduced the eval to the digit and confirmed no leak / no fail-open, then
flagged that the **chunking path had never run against the real model** (M8-R2) and lacked a `max_len`
choke-point guard (M8-R1). Acting on that pair exposed a real recall bug the pure-function unit test had
hidden: a window filled to the `max_len` budget makes the model return **all-low logits at seq ≈ 384** (a
313-word window scored *zero* on a clear name), and GLiNER int8's confidence **dilutes with context** well
before that (a name at a window's start keeps ≳0.2 while the window stays ≲100 text tokens, ~0.15 by ~130).
**Fix:** cap the window at `MAX_WINDOW_TEXT_TOKENS = 100` (far below the `max_len` budget), which bounds
the context every span is scored against — the dilution is a function of window *size*, not the entity's
position in it (M8-R7), so a small window scores an entity at any offset; an 8-word overlap keeps a
boundary-crossing entity whole; plus the M5-R7 choke-point guard as the hard safety net. Long-field recall stays weaker than short-field (a documented model property;
the default XLM-R covers long system prompts). The other three: the overlap invariant for a GLiNER `Phone`
(`is_structured` → union-merged, not "loses") promoted to ARCHITECTURE (M8-R3), a 12th decode test
(determinism, M8-R4), and `load_gliner` hardened to **fail loud** on partial config / a bad threshold
rather than silently disabling an opt-in feature (M8-R5). Full record: `docs/reviews/M8.md`.

## 2026-07-18 — M8/M9 promoted from Backlog; the M8 GLiNER implementation plan

**Post-`1.0.0` planning.** With `1.0.0` tagged and the ROADMAP's scheduled work all closed (the only open
checkboxes left were the two GPU-optimization backlog items), the next two directions are promoted to
numbered milestones: **M8 — GLiNER** (contextual / open-label PII) and **M9 — GPU optimization**. M9 keeps
its existing rationale (EP-agnostic model choice, DirectML on this box); M8 gets the plan below. The ROADMAP
M8 section carries the scope checkboxes — this entry carries the *how*.

### Why GLiNER is not a drop-in (the thing the plan is shaped around)

The current NER (`OnnxNerDetector`) is **token-classification**: `input_ids` + `attention_mask` → per-token
logits → argmax → BIO decode (`ner_decode`). GLiNER is a different architecture, and every stage below
exists because of one of these differences:
- **Open-label input.** The entity types are fed to the model *as text* — prepended to the input, framed by
  special entity/separator tokens — so the label set is chosen at inference (`"person"`, `"phone number"`,
  `"address"`, …), not baked into the head. This is the whole reason to want it: contextual, anchor-less PII
  (a bare national phone, a free-form address) the deterministic layer can't disambiguate.
- **Span output, not per-token.** It emits scores per *(span, label)* pair, decoded by threshold + greedy
  non-overlapping selection — a `gliner_decode` module, the span×label analogue of BIO `ner_decode`.
- **Word-level spans.** It scores spans over *words*, so the detector must map words → sub-word token ranges
  (via the tokenizer's word ids / offsets) — more bookkeeping than argmax-per-token.
- **The labels share the sequence budget.** Because the labels are prepended to *every* window, the usable
  text budget is `model_max − prefix − specials − drift`, so M5's chunking math is recomputed against a
  budget the labels eat into.

Artifacts: `onnx-community/gliner_multi_pii-v1` (base `urchade/gliner_multi-v2.1`, PII-tuned, 6 languages
incl. IT), int8 `model_quantized.onnx` (~349 MB) + `tokenizer.json` (mDeBERTa-v3 SentencePiece) + config,
**revision-pinned**. Community conversion → scoring against the corpus *is* the trust check.

### Stages

- **S0 — Verify the ONNX I/O contract from the real export.** Download the pinned model; inspect the
  graph's actual input/output names and shapes. The key unknown: does the export **enumerate spans inside
  the graph**, or expect `span_idx` / `span_mask` as inputs? GLiNER ONNX exports differ here, and the decode
  design forks on the answer. Deliverable: the documented contract. **Everything downstream is built against
  this, not a guess.**
- **S1 — `GLiNerDetector` (`src/pii/gliner.rs`) + `gliner_decode`.** The detector: tokenize with the label
  prefix, build the word→token map, enumerate candidate spans (or feed span inputs per S0), run the session,
  pull span logits. The decode (model-independent, unit-tested without a model like `ner_decode`):
  (span, label, score) → per-label threshold → greedy non-overlap → `Vec<PiiEntity>`. Behind `onnx`; slots
  into `CompositeDetector` behind the `PiiDetector` trait, so the pipeline is untouched. `GLINER_LABELS`
  (natural-language types) → `PiiKind` map; start mapping to **existing** kinds (phone number → `Phone`,
  address → `Location`) so placeholders and de-mask are unchanged — a new `PiiKind::Address` is a
  deliberate, eval-justified addition (it ripples through `label`/`from_label`/`priority`/`is_structured`).
- **S2 — The eval harness + the measured decision (the gate).** `tests/gliner_eval.rs` (`#[ignore]`,
  `--features onnx`) scores GLiNER int8 **through the hybrid resolver** on an extended `ner_cases.json`
  (add bare national phones, free-form addresses, single-word names) — P/R/F1 per type + CPU latency / RAM /
  size vs XLM-R int8. **Recall is metric #1.** The decision: **successor** (GLiNER replaces XLM-R in the
  composite — the clean win, one model; requires ≥ XLM-R's M4 floor Person 0.83 / Org 1.00 / Loc 0.91),
  **addition** (both NERs run — GLiNER only for the contextual kinds; 2× model RAM + latency, only if it
  underperforms XLM-R on PER/ORG/LOC but adds real contextual value), or **rejected** (doesn't clear the
  lean bar — a legitimate outcome, exactly as M7 stopped at its bar and Piiranha was rejected at M2).
  Numbers → DEVLOG. **Recommended: evaluate as a successor first.**
- **S3 — Chunking against the shared label-prefix budget.** Port M5's discipline — the compile-time headroom
  invariant (M5-R2/M5-R10), enforcement at the single choke point, the posture-is-the-caller's rule (M5-R7)
  — recomputed so the window budget subtracts the label prefix, which rides every window. Guard: every
  re-tokenized window **plus its prefix** stays under GLiNER's usable max.
- **S4 — Wire into load + `redetect` idempotence + the model-swap canaries.** Extend `load_onnx_ner`/`hf.rs`
  (or a parallel `load_gliner`) for the pinned repo. Fail closed under `NER_REQUIRED`, `FailOpen`-wrapped
  otherwise. Override `redetect` → empty **with the 0-loss recall measurement re-run for GLiNER** (the S4
  argument is per-model: masking a name never reveals a new one). **Re-run the `m5_r4` placeholder-inertness
  canary against the real GLiNER model** — inertness is enforced by construction since S4's `keep_maskable`,
  but the canary is how a *filter-leaning idempotent* model (the GLiNER case the docs flag, M5-R4 / M7-R23)
  is caught. Wire successor-vs-addition per S2.
- **S5 — Docs + builder→reviewer.** ARCHITECTURE (span decode, shared-budget chunking, label config),
  TESTING (eval + corpus cases + re-run canaries), READMEs (env + detection matrix), DEVLOG. Reviewer loop
  until clean.

### Invariants any NER swap must re-check (carried from the reviews)
- **Placeholder inertness (M5-R4 → the `m5_r4` canary).** Enforced by construction since S4, but the docs
  single out "GLiNER especially" — re-run the canary against the real model.
- **The fixpoint / `redetect` (S4).** GLiNER overrides `redetect` → empty on the same argument as XLM-R; the
  0-loss recall claim is per-model and must be re-measured.
- **Sequence-budget headroom (M5-R2/R10).** Recomputed for GLiNER's label-shared budget; a tokenizer swap
  re-opens the drift number.
- **Fail-closed posture (M5-R7).** Any GLiNER threshold may degrade its own recall but must never decide the
  caller's posture — errors flow through `try_detect` / `FailOpen` exactly as the NER's do.

### Cross-link to M9
If GLiNER int8's CPU latency misses the lean bar, that is the trigger to pull **M9 (GPU)** forward, not to
ship slow: the escalation path in `M2-NER-EVALUATION.md` is explicit that a GPU EP + `model_fp16.onnx` is
how a heavier model earns its place (on CPU, fp16 is up-cast to fp32 — no speedup, so fp16 only pays off on
GPU).

## 2026-07-18 — CC battery CLOSED on the S4 binary: the two 400s converge, zero leak

**The last box on the road to `1.0.0`.** Re-ran the CC battery through the proxy against real Anthropic on
the **S4 binary** (cache S3 on), with the owner driving Claude Code:
- **CC-05** (chat "what's the email?") — 400'd pre-S4; now **converges**, client gets the real email (OFF).
- **CC-09** (MCP SQL `SELECT *`) — 400'd pre-S4; now **converges**, all six real values restored (OFF).
- **CC-08** (the original 400 — reminder list ×30) — **both postures**: OFF restores the three real emails
  across all 30 lines; ON shows `[EMAIL_2/3/4]` (exactly what the provider saw). 30 restorations of values
  the model never held.

**Zero fixpoint 400 across the entire S4 run; DBG-02 = 0 on every value, every scenario.** S4 is validated
live, end-to-end: the fragmentation non-convergence that blocked three scenarios is gone. CC-03/04/06/07
were leak-clean pre-S4 and S4 only ever masks *more* (it runs the full NER on pass 0 unchanged; it drops the
NER only on *later* passes), and the de-mask code S4 doesn't touch, so their earlier OFF results carry.
CC-01/02 were already both postures. **The battery is closed; the privacy property held on every turn,
including the fail-closed blocks.** `1.0.0` is unblocked — all that's left is the mechanical PR → merge → tag.

## 2026-07-18 — S3: content-keyed detection cache (M7.1 complete)

**The other M7.1 lead, landed.** Claude Code re-sends 20–40 KB of **byte-identical** system prompt + tool
schemas every turn; detecting PII in it (the NER above all) dominates the masking latency. `CachingDetector`
(`src/pii/cache.rs`) wraps the composite and memoizes `try_detect` **keyed on the exact field bytes**, so
turn 2+ skips the scan. The per-request vault still mints the placeholders, so numbering is unchanged — the
cache stores *what/where*, never a mask.

**The threat argument the ROADMAP demanded, discharged.** A cache hit must never mask *less* than a fresh
scan. `try_detect` is a pure function of its input (stateless regex; NER inference on the input alone) and
the key is the *whole* input, so a hit returns exactly what a fresh scan would — it cannot mask less. Only
`Ok` results are cached (an error still fails closed); the cache is bounded (a dependency-free two-generation
map, ~`2 × PII_CACHE_ENTRIES` live entries, only fields 256 B–128 KiB, hot keys promoted on read); and
`redetect` (S4's later passes, on per-request masked text) is never cached. `PII_CACHE_ENTRIES` (default 16,
`0` disables) is the one knob. Default **on** — it is sound, and the latency win is the whole point.

Tested: 5 unit tests (hit==fresh, error-not-cached, redetect-uncached, small-skipped, bounded-LRU) + an e2e
(`proxy_e2e.rs::e2e_cache_on_a_repeated_large_field_still_masks_both_times`, a repeated PII-bearing large
field masks on both requests). 116 onnx lib tests green, clippy clean. **M7.1 is complete** (S3 + S4);
ARCHITECTURE has both invariants. Left before `1.0.0`: the CC battery re-run on the S4 binary, with the user.

## 2026-07-18 — the *real* non-convergence cause, found live: NER sub-word fragmentation (CC-05) → S4 is the fix

**The instrumentation paid off.** Running the CC battery's **Run OFF** half on the diagnostic binary, **CC-05
(a plain "what's the email?" turn) hit the fail-closed 400** — and this time the value-free diagnostic named
the cause: `per_pass=[[ORG 6, PER 2],[ORG 2],[PER 1],[PER 1]]`, `remaining=[ORG 2]`,
**`placeholder_tags_suppressed=0`**. So it is **not** placeholder re-tagging (the earlier hardening was valid
but not the cause) — it is the **NER tagging sub-word fragments** of dense product/org names. The masked
bodies show it plainly: `S[ORG_6]` (Slack → "lack" tagged, "S" left), `Git[ORG_4]` (GitHub), `[PERSON_2] Code`
(Claude). Each mask splits the word; the next pass re-tags the leftover; the Claude Code **system prompt** is
dense with these, so it needs **> `MAX_MASK_PASSES`** and 400s. This is [M7-R7](reviews/M7.md#m7-r7)'s
fragment over-mask — which M7-R7 called "a latency cost, not a correctness one." **Live CC-05 proves that
wrong:** past 4 passes it is a fail-closed availability failure on real Claude Code traffic. It never
reproduced synthetically before because no synthetic input carried a real system prompt's density.

**Investigated three fixes offline (production composite, real XLM-R):**
- **Word-boundary snap** (extend a fragment span to the whole word) — **rejected**: it makes convergence
  *worse* (8 passes vs 4 on the same dense text). Counterintuitive but measured.
- **Bump `MAX_MASK_PASSES`** — **rejected**: PLAIN passes **grow with the dense text**, 6 (5 KB) → 11 (15 KB)
  → 13 (30 KB), unbounded; no safe fixed value, and each pass is a full NER scan.
- **S4 (NER on pass 0 only, structured recognizers after)** — **converges in 1 pass at every size.** This is
  the M7.1 "stop paying twice for the fixpoint" lead, now promoted from optimisation to *the fix*.

**S4's recall risk — the argument M7-R7 demanded — is answered.** Dropping the NER after pass 0 could in
principle miss a name a later pass would expose at a seam (`[PERSON_1]-Jones`). Measured: **0** losses — S4
masks every expected entity PLAIN does across the labelled corpus (25 NER entities / 15 cases), and **0** raw
PII survives when real names/emails/IBAN are injected into fragmenting dense text. Masking only ever *reduces*
NER context, so later passes surface no genuinely-new names — only the fragments they created.

**Decision (owner):** record all of this in M7.1's S4 spec (done, ROADMAP), keep running the OFF battery on
the current binary to collect any further 400s, and do the S4 change as a proper builder→reviewer cycle. S4
now conditionally gates `1.0.0`: fail-closed never leaks, but a proxy that 400s a fraction of real turns is
not "usable". Investigation harness was throwaway (`tests/cc05_investigate.rs`, deleted); the numbers live
here and the regression tests ship with S4.

**Implemented the same day.** A `redetect` method on `PiiDetector` (default `try_detect`; `OnnxNerDetector`
overrides it to return nothing — idempotent after pass 0; `CompositeDetector`/`FailOpen` delegate).
`Vault::mask_all` runs the whole detector on pass 0 and `redetect` on every later pass *and the fixpoint
confirm*, so the NER's fragments can't chain. Verified live: `ner_perf.rs::
m7_s4_dense_org_names_converge_instead_of_400` — dense system-prompt text that 400'd now converges — plus
deterministic `Fragmenter` unit tests (FC-09, bug→fix). 111 onnx lib tests green, `clippy` clean. Invariant
in `ARCHITECTURE.md` (*Masking must run to a fixpoint*); S4 closed in M7.1. S3 (the cache) is next.

## 2026-07-18 — CC-08 resolved: placeholder inertness *by construction* + a value-free block diagnostic

**The finding, recapped.** CC-08 (a long reminder-list turn) returned a fail-closed **400** — masking
could not confirm a fixpoint in `MAX_MASK_PASSES=4`. The guard fired **correctly**: blocked before
forwarding, **zero leak**. But a 400 on ordinary work is a real *availability* defect, and the 400 carried
no *reason* — the failing content isn't logged (by design, fail-closed blocks before the forward-trace).

**Diagnosis: the prime suspect was wrong.** The suspected mechanism was the NER tagging one of our own
`[KIND_N]` placeholders, so each pass re-masks it and the text never shrinks (`anonymizer.rs:75-80`, the
latent path the code documents). Rebuilt an instrumented proxy and had the owner re-run CC-08: it
**converged** (fresh session, less accumulated context). Then reproduced offline against the **production
composite** (structured it+us + the real ONNX NER) over every CC-08-like shape — placeholder-dense fields
with two-digit indices and mixed kinds in Italian context, the raw `contacts.csv`, and a chunk-triggering
long field. **All converge in ≤1 pass; the official `m5_r4` inertness test passes.** Across the live re-run
and ~10 synthetic reconstructions, the 400 **never reproduced**. Conclusion: placeholders are **empirically
inert for this model** — the suspected cause is *not* what fired. The real trigger is content-specific to
that one session (an unseen system-prompt/context field, or deep structured nesting > 4 passes), which
fail-closed handles correctly by design.

**Resolution (owner's call: instrument + harden, don't chase an unpinnable trigger).**
- **Placeholder inertness is now enforced *by construction*, for every engine.** `Vault::mask_all` runs
  detection through a new `detect_maskable`, which **drops any detection that is exactly one of our own
  `[KIND_N]` tokens** (`is_placeholder_token`) before masking — a real value can never take that shape, so
  this never drops genuine PII. Every surviving detection is real PII, masking real PII strictly shrinks the
  raw text, so the fixpoint converges **regardless of the NER**. This upgrades M5-R4 from an *empirical
  model property* to an *algorithm property*; the `m5_r4` NER test stays as **belt-and-braces + a model-swap
  canary** (it still tells us *whether* a future model — e.g. GLiNER — leans on the filter).
- **The fail-closed branch now explains itself, value-free.** On non-convergence it logs the **per-pass kind
  tally** (shrinking = a deep nest that would clear with more passes; stalled = genuine), the **residue's
  kinds**, and `placeholder_tags_suppressed` (the canary). Kinds and counts only — never the text, which
  fail-closed never forwards or logs. This turns any *future* recurrence into a pinned cause, which is worth
  more than a synthetic guess at this one.
- **Tests:** `mask_all_converges_even_if_a_detector_tags_its_own_placeholders` (a `TagsPlaceholders`
  detector that re-tags every placeholder — the exact pathology — and `mask_all` still converges),
  `is_placeholder_token_matches_only_our_own_tokens` (the filter's boundary: our tokens incl. tolerant
  corruptions in, foreign `[TODO_1]` / partials / real PII out), and `kind_histogram_…is_value_free`.
  107 onnx lib tests green, `clippy` clean. Invariant promoted to `ARCHITECTURE.md` → *Masking must run to a
  fixpoint*; catalog in `TESTING.md` (FC-07, NER-INERT-01 reworded).

**Why not a live re-run to confirm?** The fix only *adds* a filter that can never make convergence worse,
and CC-08 already converged live before it. The privacy property is untouched; the change is covered by
tests + the reviewer. A live re-run would only re-show convergence.

## 2026-07-18 — CC battery live run: 8/9 leak-clean, CC-08 the one finding

**Manual verification, the M7 → `1.0.0` gate.** Ran a real Claude Code session through the proxy against
real Anthropic — hybrid, `NER_REQUIRED=1`, at the new **`pool=1` default** (exercised live throughout,
startup logs `pool_size=1 intra_threads=12`). Each turn verified from the trace: only placeholders
leave, and a DBG-02 grep of every fixture's raw values returns **0**. Across ~41 forwarded requests,
DBG-02 stayed 0 for every raw value and every PII pattern.

**Leak-clean (7):**
- **CC-01** (contacts.csv → JSON) — *both* postures. Run ON: client saw `[EMAIL_2]`/`[PERSON_4]`; Run
  OFF: the restored real values. (Index ≠ 1 is *correct*, not drift: the NER numbers entities across
  the whole body in encounter order, and the boilerplate's entities precede the user's — so the user's
  email is `[EMAIL_2]`, not `_1`. Confirmed by the owner running from a fresh `/clear`.)
- **CC-02** (release note thanking a Person at an Org in a City) — *both* postures; Mario Rossi / Acme /
  Milano → `[PERSON]/[ORG]/[LOCATION]`, all NER.
- **CC-03** (read the whole CSV) — every category masked: `[EMAIL]/[PERSON]/[IBAN]/[PHONE]/[SSN]`.
- **CC-04** (write the first email to a scratch file) — the file on disk holds **`[EMAIL_2]`**, not the
  real email: proof the client acted only on the placeholder.
- **CC-05** (asked point-blank "what's the email?") — the model answered **`[EMAIL_2]`**. It genuinely
  does not hold the real value; masking is not just in/out, the model itself operates on placeholders.
- **CC-06** (deploy-config.env) — the SECRET test the old ML-only proxy failed: all three keys
  (`sk-ant-…`, `sk-…`, `AKIA…`) → `[SECRET_1/2/3]`, plus email/phone. Zero secret-shaped tokens out.
- **CC-07** ("which IBAN is German?") — the model **cannot tell**, because the `DE` prefix was masked
  before it arrived: the privacy/utility trade working as designed. Also showed vault consistency — the
  Italian IBAN repeated in two rows got the **same** `[IBAN_1]` both times.

**CC-08 — a real finding (availability, not privacy).** The long-reminder-list scenario returned
**HTTP 400 "vault detector failed: masking did not reach a fixpoint in 4 passes"**. This is the
**fail-closed guard firing correctly** — the request was **blocked before forwarding** (no `forwarding`
trace line, zero leak), exactly the posture a privacy proxy must have when it cannot confirm a field is
clean (`anonymizer.rs::mask_all`, `MAX_MASK_PASSES=4`, M4-R20). But masking that *cannot converge* on an
ordinary task is a real defect: the code itself flagged this as a **latent path** ("no input has ever
been shown to need more than 2 passes"), and CC-08 is the first input to trigger it. Prime suspect
(`anonymizer.rs:75-80`): the NER tagging a placeholder as an entity in the pathological
repeated-placeholder context, so each pass re-masks it and the text never shrinks.
- **Not yet reproduced.** The failing content isn't logged (fail-closed blocks *before* the forward-log,
  and we never log raw PII). Three synthetic reproductions — the placeholder reminder-list, the raw CSV,
  and the CSV×3 — all **converged** (reached upstream). So the trigger is more specific than "repeated
  placeholders"; pinning it needs instrumentation.
- **Next:** add a *value-free* per-pass kind/count log to `mask_all` (kinds only, never values), re-run
  CC-08 to capture what stays detectable on pass 4, then fix (likely: protect existing placeholders from
  re-detection) + a regression test. Tracked in ROADMAP.

**CC-09 — leak-clean, after fixing a self-sabotaging fixture.** The original `customer-lookup.sql`
carried the PII as **literals in the query text** (`SELECT 'bob@test.com'… FROM DUAL`): to run it the
agent reads the file → the proxy masks the literals **on read** → the agent runs a query that already
says `[EMAIL_1]` → the result has nothing new to mask, and the `tool_result` path the scenario exists
for is never exercised. **Fix (2026-07-18):** put the PII in a **table**, not the text. A synthetic
`cc09_customers` (one row: email/phone/ssn/card/iban/secret) is created out-of-band by
`fixtures/cc09-setup.sql`, and the agent runs the **PII-free** `SELECT * FROM cc09_customers`
(`customer-lookup.sql` is now exactly that). The available SQL MCP servers pointed only at real
corporate Oracle DBs (we did **not** query real tables — real-PII risk), so the owner stood up a
throwaway **SQLite** DB for the `python-sql` server (absolute-path connection, so the setup session and
the CC session hit the same file). **Run ON, verified:** the client saw
`[EMAIL_2]/[PHONE_1]/[SSN_1]/[CARD_1]/[IBAN_1]/[SECRET_1]`, and DBG-02 on the outbound log returned
**0** for all six raw values and every PII pattern. So an **MCP tool result** — a path the proxy never
sees coming — is masked like any other. (The raw row does appear in Claude Code's *local* tool-output
pane; that is the MCP tool's local return, which never transits the proxy — only the re-send to the
model does, and that left masked.) TESTING / MANUAL_VERIFICATION / the two fixtures updated to match.

**Bottom line for `1.0.0`:** the privacy property held on every turn that ran, including the fail-closed
block and the MCP tool-result path. **8/9 leak-clean; CC-08's non-convergence resolved the same day** —
see the entry above (placeholder inertness by construction + a value-free block diagnostic).

## 2026-07-17 — `NER_POOL_SIZE` default flips 2 → 1 (the personal shape becomes the default)

**Source + docs.** `DEFAULT_POOL_SIZE` is now **1**. The dominant deployment is a personal proxy in
front of a single client (Claude Code, concurrency ≈ 1), and a single request only ever occupies one
session (S1a — the field walk holds `&mut Vault`, `infer_chunked` loops its windows, only then does
the session run), so a second pooled session buys a lone request **nothing** while holding a second
copy of the model. So the lean default is one session: the whole box for the in-flight request, and
**less RAM** (how much — measured — is its own paragraph below; the earlier "half" was arithmetic and
wrong). That is `CLAUDE.md`'s *low-RAM* bar applied to the case almost everyone runs. M7 had already
identified `(1, 12)` as the personal shape and documented it — it just left the
*default* on the pooled `(2, 6)`; this flips which of the two documented shapes an operator gets by
setting nothing.

**What it is NOT: a latency win.** `intra = cores` vs `cores/2` is inside this box's noise — the SMT
question (`1×6` vs `1×12`) is UNRESOLVED (M7-R2/S1), its sign flips run to run. Latency between the
two shapes is a wash. Anyone reading this flip as "~2× faster single request" is re-reading the noise
M7 spent three rounds learning not to.

**The cost, named (not papered over).** `pool=1` measured **−23% throughput** under concurrent load
— two independent measurements plus a mechanism: intra-op scaling is sublinear, so N sessions ×
cores/N threads aggregate better than one × cores (2026-07-16 entry below, PERF-M7-04). The flip is
scoped around exactly that: it targets the **personal** case, which has no concurrency to lose the
23% on, while the RAM it saves is real. A **centralizing** operator serving concurrent clients sets
`NER_POOL_SIZE=N` to reclaim the throughput — which is why the pool stays an override rather than the
default's job.

**RAM, measured — and the "half" was wrong.** The pre-flip docs said `pool=1` "halves the RAM",
reasoning `834 MB / 2 sessions ≈ 400 MB each`. Measured properly (idle resident, same debug build,
2026-07-17): **`pool=1` = 563 MB private (585 MB working set), `pool=2` = 834 MB** — and the `pool=2`
figure reproduces the README's prior 834 MB exactly, so the method matches. So it is **~290 MB of
shared base + ~270 MB per session** (`pool=N` ≈ 290 + N×270 MB) — *not* a clean doubling, because the
ONNX runtime and the first session's arenas don't duplicate. Dropping the second session saves
~270 MB (**about a third**, not half); `pool=6` is ~1.9 GB, not the ~2.5 GB the `834/2` split
projected. That `834/2 ≈ 400-per-session` arithmetic assumed **zero base** — the same class of error
this milestone keeps naming, a number never checked against what the product does. README /
ARCHITECTURE / the config knobs now carry the measured scaling; this is the number to quote.

**Semantics that moved with it (M7-R13 / M7.1).** Under `pool=2`, `intra` floored at 1 below 4 cores,
so the derived default *was* `PRE_M7_SHAPE (2,1)` and M7's ratio was 1.0 by construction — nothing to
deliver on a small box. Under `pool=1` the derivation is `intra = cores`, so that identity now holds
**only at 1 core**; from two cores up the default already adds threads a lone request can use. The
latency harness still skips its ratio guard below 4 cores, but for a **different** reason now — the
few-thread shapes there are too thread-poor to clear the 1.5× floor reliably (untested on a small
box), not because the ratio is 1.0 by construction. The unit test pinning the derivation is renamed
`the_default_gives_one_session_the_whole_box`; `bar_shapes` in `m7_latency.rs` now guards the default
`(1,12)` **and** the centralized `(2,6)`.

**Exercised live the same day, leak-clean in both postures.** The hybrid ran a real Claude Code
session through the proxy against real Anthropic. **Run OFF** (old default `pool=2`) on a
name/org/location turn: the client saw the **restored real values**, the outbound trace carried only
`[PERSON_1]/[ORG_1]/[LOCATION_1]`, and a DBG-02 grep for the three raw values returned **0 hits**.
**Run ON** (`PII_DEBUG_SKIP_DEMASK=1`, new default `pool=1` → `intra=12`) on a JSON-extraction turn:
the client saw the **placeholders** (`[PERSON_4]/[EMAIL_2]/[IBAN_1]`), the outbound body carried only
placeholders, and a pattern scan found **no** real email or IBAN. Both postures held independently.
The one thing still open for a *formal* CC-battery closure is the strict **same-prompt** OFF/ON
pairing (the runbook's "same round-trip, two halves" proof) — here the two runs used different
prompts.

Files: `src/pii/onnx.rs` (default + `thread_tests`), `tests/m7_latency.rs` (`bar_shapes`,
`MIN_CORES_FOR_A_MEANINGFUL_RATIO` rationale, S1 docs), `docs/ARCHITECTURE.md`, `docs/TESTING.md`,
`docs/MANUAL_VERIFICATION.md`, `README.md` + `README.it.md`.

## 2026-07-16 — The onnx build gets its own target dir (the clobber footgun, closed)

**Infra, no source change.** The default and `onnx` builds both wrote
`target/debug/llm-proxy-pii-rust.exe`, so *any* default-features command — `cargo test`, `cargo
clippy --all-targets`, a plain `cargo build` — silently replaced the hybrid binary with a
structured-only one. Not hypothetical: it is what made the first live M6 run test half the product
(entry below), and `MANUAL_VERIFICATION.md` had been carrying it as a **warning box** — i.e. as a
discipline to remember, which is the weakest kind of guard this repo accepts anywhere else.

**Why not just rename the binary** (the first instinct, and the ask): **Cargo cannot name a binary
per feature.** Features are additive and crate-wide; `required-features` gates *whether* a bin
builds, never what it is called; and "not onnx" is inexpressible. A second `[[bin]]` with
`required-features = ["onnx"]` *would* give a distinct name — but `cargo build --features onnx`
would still also build the ambiguous `llm-proxy-pii-rust.exe` (a wasted link, ~5 min under fat
LTO), so the trap binary survives next to the safe one and you must now *know which to point at*.
That relocates the confusion instead of removing it.

**So the split is by directory, not by name** — `.cargo/config.toml` aliases add `--features onnx
--target-dir target/onnx`:

| Path | Contains |
|---|---|
| `target/onnx/debug/llm-proxy-pii-rust.exe` | **the hybrid — always.** Only the aliases write here |
| `target/debug/llm-proxy-pii-rust.exe` | whatever the last default-features command left; structured-only *by convention* |

**Only the first row is a guarantee** — an explicit `cargo build --features onnx` still writes a
hybrid to `target/debug/`, and seven milestones of docs trained exactly that habit. The asymmetry
is fine because it runs in the safe direction: the *dangerous* case (structured-only masquerading
as the hybrid) is the one `NER_REQUIRED=1` makes fatal. The shipped artifact name is untouched.
`cargo build-onnx` / `run-onnx` / `test-onnx` / `clippy-onnx`; extra args append
(`cargo build-onnx --release`). Run them from the repo root — `--target-dir` is cwd-relative, so
from a subdirectory you get a stray `<subdir>/target/onnx/` that a root `cargo clean` won't find.

**Verified, not assumed** (this box exists because the last one wasn't): both suites run green
through the aliases — **85 lib tests default, 97 with `cargo test-onnx`**, zero warnings, `fmt`
clean, `clippy-onnx -- -D warnings` clean, and all five workflow YAMLs parse. `cargo build-onnx`
then `cargo build` left **both** binaries alive — 54.9 MB (links ONNX Runtime) vs 13.2 MB, the
default command finishing in 2.1 s without touching the hybrid. `cargo run-onnx` rebuilds and
launches `target/onnx/debug/`. And the two binaries are genuinely different builds, proved by the
backstop they each hit under `NER_REQUIRED=1`:

```text
target\debug\…       Error: NER_REQUIRED is set but this binary was built without the `onnx` feature
target\onnx\debug\…  Error: NER_REQUIRED is set but the NER is not configured (set NER_MODEL_PATH / …)
```

**`NER_REQUIRED=1` stays mandatory for the CC battery.** The aliases make the right build
automatic; the flag makes the wrong one fatal. Two guards, and the cheap one is not a reason to
drop the other — the flag is what *caught* this in the first place.

**What the split does NOT close — and the review caught me claiming otherwise.** The clobber was
**loud**: it destroyed the binary, so `NER_REQUIRED` turned it into a fatal error. A binary in its
own directory is never destroyed — it goes **stale**, and a stale hybrid loads the NER and prints
both green startup lines. `NER_REQUIRED` sees a *missing feature*, never *old code*. My first draft
dropped MANUAL_VERIFICATION's old imperative ("always rebuild immediately before starting") on the
grounds that the trap was now closed by construction — which is the M4-retrospective move exactly:
*the failure relocated, and the doc stopped warning about it*. Fixed structurally rather than by
restoring the imperative: the recipe now runs **`cargo run-onnx`**, so cargo rebuilds before
launching and staleness is impossible.

**Bonus, and not free:** two target dirs are two caches, so flipping between default and `onnx`
stops invalidating the ort/tokenizers/hf-hub tree each time (that round trip was a ~7.5 min
recompile). Paid in disk — `target/onnx/` measures 8.9 GiB of a 36.1 GiB `target/`.

**The pipelines were already correct and are unchanged** (checked, since the question was asked):
`release-build.yml` is `--features onnx` throughout, and its packaged artifact comes from
`--target <triple>` — its own directory — with the build running *after* the tests, so a
structured-only binary cannot ship. A comment now says why it doesn't use the aliases (rust-cache
would stop covering a nested target dir), so nobody "fixes" it later.

**`ci.yml`'s default leg stays — but my recorded reason for it was wrong**, which the reviewer
rightly called the more dangerous half. I wrote that it guards the native-dep-free invariant
(DEP-01, M2.5-R1). It does not: `tests/dependency_footprint.rs` shells `cargo tree` with **no
`--features` flag**, so it asserts the same thing in the onnx leg — DEP-01 needs no default leg at
all, and a future builder who notices that deletes the leg as redundant *and is being reasonable*.
What the leg actually guards is what nothing else in CI compiles: the default feature set linking
and passing green without the native stack, and `src/server.rs`'s `#[cfg(not(feature = "onnx"))]`
block — the **only** not-onnx path in the tree, and the source of the `NER_REQUIRED` backstop error
this entire change leans on. That reason now lives in `ci.yml` next to the leg, not only here.

Docs realigned to the onnx variant: both READMEs' quick start, `SETUP.md`,
`MANUAL_VERIFICATION.md` (the warning box now *describes the split* and its stale-binary limit),
`TESTING.md` → *Running* (it taught only `cargo test` while the READMEs taught the pair), and the
`ner_eval` / `ner_perf` header commands.

**Not promoted to ARCHITECTURE/TESTING, deliberately** (the reviewer's call, and it's the right
one). The durable rule here is *"a live verification must prove which build it ran"* — and
TESTING.md already carries it, next to the thing it governs: the hybrid-with-`NER_REQUIRED=1` bar
and CC-02, "the one that catches a half-running product". The build-dir split is the *ergonomic
implementation* of that rule, with no runtime surface — implementations belong in SETUP, so
ARCHITECTURE would only be noise.

**Stale workflow comments fixed while verifying the pipeline claims** (in scope, since the entry
above asserts the pipelines were checked): three files still said per-push CI no longer exists and
pointed at a `ci.yml.disabled` that isn't there — `ci.yml` was brought back trimmed when Dependabot
was enabled. The cross-compile really is tag/manual-only; *CI* isn't gone, so the comments now say
which half is true. `release-build.yml`'s `upload-artifacts` input is also documented honestly: no
caller passes it, and the caller its rationale cited (the retired per-push `ci.yml`) no longer
exists.

## 2026-07-16 — M7 built: S0 measured the plan wrong, S1 met the bar, S3/S4 deliberately not done

**The plan said start at S0 and re-measure because the headline number was suspect. It was — by
6×. And the *reasoning* was wrong in a more interesting way than the number.**

### S0 — the fixture is the experiment

`tests/m7_latency.rs`: one realistic Claude Code turn — 112 fields, 22,823 B (22.3 KiB) — as the walk actually
sees it (one big `system`, 10 medium `tools[].description`, 100 tiny `input_schema` descriptions,
one ~130 B user message holding all the PII). The shape is **asserted**, not hoped for: the first
draft came out at 13.5 KiB because I wrote 350-byte tool descriptions when the real ones are 1–4 KB,
and the guard rejected it. That guard is the whole point of the file.

**A realistic turn masks in ~4.2–4.7 s — not 27 s.** The 903 ms/KB headline came from a blob densely
packed with Italian names; a realistic turn runs at ~190–210 ms/KiB. *(The `ms/KB` there is the
pre-M7 entry's own unit, hand-computed from a live trace, and nothing records whether its `29.4 KB`
was ÷1024 or ÷1000 — so it stays as written. The M7 fixture's figures are KiB because the harness
divides by 1024 and prints it. Relabelling the old number would be asserting a unit nobody measured
— M7-R10.)* Per field, before any change (one sample — see the S1 note on this harness's noise):

| part | fields | bytes | ms | ms/KiB | passes |
|---|---|---|---|---|---|
| `system` | 1 | 6,151 | 1,881 | 313 | **2** |
| `tools[].description` | 10 | 9,482 | 1,157 | 125 | 10 |
| `input_schema` descriptions | 100 | 7,060 | 1,094 | 159 | 100 |
| user message | 1 | 130 | 46 | 362 | 2 |

### …and S0's *mechanism* was wrong, which matters more than its number

The plan asserts the boilerplate carries **~zero PII**, therefore costs **one** fixpoint pass —
and demotes the fixpoint lead on that basis. **False.** `m7_s0_what_the_ner_finds_in_boilerplate_that_has_no_pii`
prints exactly one hit in text that contains no PII by construction:

```text
system  →  [(Organization, "An")]
```

A **two-character fragment of "Anthropic's"**, tagged `Organization`. So the largest field in the
turn pays a second full NER scan — ~940 ms of the original 4.24 s. And a real Claude Code system
prompt names Anthropic and GitHub constantly, so this is the **normal** case. Two consequences:

- **S4 (skip the NER on later fixpoint passes) is *more* relevant than the plan concluded, not
  less.** It is not done anyway — see S2 — but the reason is the bar, not irrelevance.
- **It is also an over-mask**: `"Anthropic's"` becomes `"[ORG_1]thropic's"` in the system prompt
  the model receives. Not a leak (it fails toward masking) and squarely the accepted M4-R6 class,
  but it is *boilerplate corruption* nobody had seen, because nobody had run the NER over a real
  system prompt. **Logged as a finding for the review, not fixed here** — precision work is not
  M7's scope and would need its own recall argument.

### S1 — measured on both axes, and the numbers refuted the intuition (once, not twice)

> **⚠ SUPERSEDED BY M11 Track B (2026-09-02) — the divisor, not the derivation. Left as written
> because it is the record of what M7 shipped and why.** The formula below is still the formula; the
> base it divides moved from `available_parallelism()` (**logical**) to
> `min(physical_cores, available_parallelism())`. So every `intra` in these tables is the shape M7
> shipped, not the shape that ships now: on this box the default is `1×6` (was `1×12`) and
> `NER_POOL_SIZE=2` gives `2×3` (was `2×6`). The SMT question item 3 below records as *unresolved*
> stayed unresolved — M11 closed it **by decision**, not by measuring it. Current numbers: the
> M11 entry at the top of this file; current rule: ARCHITECTURE → *NER threading*.

`NER_INTRA_THREADS`, explicit-wins-else-derived (`max(1, available_parallelism() / NER_POOL_SIZE)`).
The two knobs multiply, so the **product** must fit the box; `0` is unset for **both** knobs, never
ONNX Runtime's "pick for me", which would put `pool × all-cores` threads on the machine. Both
resolve in one place — `onnx::resolve_pool_and_intra`, which the harness calls too, so it cannot
measure a config the server doesn't ship (M7-R1).

**Latency — one request, 22.3 KiB turn, 12 logical cores, best of 3:**

| pool | intra | ms | ms/KiB | vs pre-M7 |
|---|---|---|---|---|
| **2** | **1** | **~4,700** | 213 | 1.00× ← pre-M7 |
| 1 | 1 | ~4,500–6,000 | — | ~1× |
| 1 | 2 | 4,111 | 184 | 1.15× |
| 1 | 4 | 2,845 | 128 | 1.67× |
| 1 | 6 | 2,651 | 119 | 1.79× |
| 1 | 12 | 2,966 | 133 | 1.60× |
| **2** | **6** | **2,547** | 114 | **1.86×** ← the new default |
| 4 | 3 | 3,192 | 143 | 1.49× |

**Read that table with its noise, which is the whole of [M7-R2](reviews/M7.md#m7-r2).** The same
configuration drifts **~40% between runs** on this box (`1×12` measured at 2.1 / 2.5 / 3.0 s on
different days). The sweep now takes 3 reps and prints min/median/spread; the bar (PERF-M7-05) takes
the **minimum**, which is the closest thing to the interference-free cost. Believe a row only when a
mechanism backs it.

**Throughput — 4 concurrent turns:**

| pool | intra | turns/s |
|---|---|---|
| 2 | 1 | 0.288 ← pre-M7 |
| **2** | **6** | **0.609** ← the new default |
| 1 | 12 | 0.472 |
| 4 | 3 | **0.731** |

**What the measurement settled — and what it did not:**

1. **`pool=1` is *not* free.** I had written "it is not a trade at all". It is: **−23% throughput**
   (0.472 vs 0.609; the reviewer independently measured −21%). Intra-op scaling is sublinear, so 4
   sessions × 3 threads aggregate better than 1 × 12. The deployment-shape argument in ROADMAP → M7
   stands exactly as written — which is why the default is derived and overridable rather than
   repointed at the personal case. **This one is real: two independent measurements, and a
   mechanism.**
2. **Scaling is sublinear** — ~2×, never 12×. Replicates everywhere.
3. **The pool is inert at concurrency 1** — true, and believe it from the **code**, not the table:
   one request occupies one session (the walk holds `&mut Vault`; `infer_chunked` loops its
   windows), so `pool` cannot help it. `2×1` ≈ `1×1` in most runs and diverges in others — that is
   the box, not a mechanism.
4. **SMT — UNRESOLVED, and the first cut claimed otherwise.** I wrote *"12 logical threads beat 6
   (2.09 vs 2.46 s); hyperthreading helps this int8 model"*, and made it the write-up's high point:
   *both* "measure, don't reason" items resolved against intuition. **It was one sample.** Re-run,
   the sign flips (`1×6` by 11%, then `1×12` by 8%, then `1×6` by 3%, then `1×6` by 11%) — an 18%
   effect read off a 40% band. It is load-bearing, too: `default_intra_threads` divides the
   **logical** core count, which is right only if SMT helps. That divisor is now an open question the
   milestone briefly believed it had closed.

**The derived default improves both axes over what shipped** — ~4.7 → ~2.5 s latency *and*
0.288 → 0.609 turns/s — so it is not a trade against the shared proxy at all. The single-client shape
(`NER_POOL_SIZE=1` → intra 12) gets **~2.1 s and half the RAM**, since each session holds its own
copy of the weights. **The RAM half is arithmetic; the latency half is inside the noise on this box** —
so the READMEs lead that recommendation with RAM.

> **The S1 mistake is the S0 mistake, one level up — and I made it inside the milestone that exists
> to name it.** S0 says: *a corpus has a shape, and the shape is the blind spot.* S1's blind spot
> wasn't the corpus, it was the **measurement design**: n=1 on a noisy box. Getting the fixture right
> does not save you from reading a conclusion off a single sample. That is why the reps and the
> spread column are now in the harness rather than in a reviewer's head.

### S2 — the bar was MISSED, and we stopped anyway (which is the stronger sentence)

**~4.7 s at the shipped default** — reproduced independently at **4,724** and **4,757 ms**, isolated,
on its balanced/energy-efficiency power plan. That is **~60% over the ~3 s bar**. The `2.46 s` this
entry once led with was the fastest of
seven observations and has never reproduced; it should not have been the headline, and the READMEs no
longer carry it.

**`S3` (content-keyed cache) and `S4` (skip the NER on later passes) are still not implemented, and the
reason changed.** It is no longer "the bar was met". It is: **what M7 could deliver, it delivered** — a
reproducible **~2×**, checkable on any box — and the rest of the gap is the machine, not the code. Both
leads put real risk on the masking path (state; lost detection), and that risk should be bought
deliberately when something demands it, not spent because we were already in here. The bar was declared
*before* the numbers so this decision couldn't be rationalised afterwards — and the discipline held in
the direction that actually costs something: **the bar came back missed, and the honest move was to say
so rather than to re-describe the number until it fit.**

**The bar guards both shapes** (M7-R1). The first cut asserted it on `pool=1` only — while `server.rs`
defaults to `pool=2` — so it had ~28% headroom on a configuration nobody runs and **none** on the one
they do. Both now resolve through the server's own function.

> **The absolute number here has an un-named variable bigger than every knob in S1 — and I named it
> wrong twice (M7-R9, then M7-R12).** The *same* code, fixture and box, across two people:
> **2,462 / 3,943 / 4,724 / 4,757 / 4,841 / 4,933 / 7,142 ms** — each run internally tight (spread
> under 7%). So min-of-3, my own fix for M7-R2's noise, was **precise and wrong**: it removes jitter,
> and this is not jitter. All reps sit inside the regime and agree confidently on the wrong number.
>
> **Then I explained the regime, and the explanation was fiction.** I wrote *"power and thermal
> regime, nothing else"*. The data refute it: a **battery** run (3,943) beat **three AC** runs
> (4,757 / 4,841 / 4,933). No power model orders that. And "throttled AC" — the third category I
> introduced — was assigned **post hoc from the number itself**; I never measured a thermal state.
> *A run was called slow because it was slow, then cited as evidence that slowness was throttling.*
> **That is the exact move this milestone exists to name**, committed in the entry naming it.
>
> **Then the machine's owner ended the argument in one sentence: the runs I called "AC" were on the
> *same energy-efficiency plan* as the ones I called "battery".** The charger was plugged in; the
> power profile never changed. So the variable was not mis-modelled — **it did not vary.** Every
> theory I built on it was theorising about a constant, which is why no amount of re-measuring could
> have rescued it, and why the "battery beat AC" contradiction was an artefact of the label rather
> than a fact about the box. **The person who owned the hardware knew in one line what four rounds of
> benchmarking could not tell me.** *When a variable you never controlled is doing the explaining, ask
> whoever controls it before you write the mechanism down.* What a **performance** plan does here
> remains unmeasured, and the published figure is the ordinary-laptop case deliberately.
>
> **The variable that is actually measured:** test **concurrency**. Cargo runs the perf tests in
> parallel, so the documented command measured the product **against four other copies of itself** —
> **1.50×** at constant power (4,757 isolated → 7,142 contended). `--test-threads=1` is now part of
> the contract, in the harness doc and in TESTING's recipe.
>
> **M7-R1 taught this milestone to name a number's shape; R9 named its power state; R12 showed the
> power state cannot order the data — and the box's owner showed it was never a state at all.** Four
> rounds, the same shape each time. *Naming one variable does not make a measurement reproducible —
> it makes the un-named ones harder to notice, because the number now looks qualified.*
>
> **So the assert is a ratio, and the ~3 s figure is a reported claim, not a guard.** The bar test
> measures the **pre-M7 shape** (`2×1`) as a calibration leg *in the same run*, seconds from the
> shapes under test, and asserts `pre_m7 / shape > 1.5`. The box's power state divides out:
> **~1.7 – 2.3×** across every one of the seven runs above, while the absolute moved **2.9×**. A
> ratio catches a real regression on any box and cannot go red because the box is slow — which the
> 3 s assert did on five of seven runs, while staying blind to a genuine 20% regression. *(The band
> is quoted loosely and the docs lead with the asserted **≥1.5× floor**, not a tight range: the ratio
> cancels power but not raw box speed, so a faster box compresses it toward the floor — 2.19×
> reference, 1.74× a quicker box — and every tight band we published got undercut by the next clean
> run; M7-R18.)*
>
> **What the ratio does NOT buy, said plainly because an honest guard states its blind spot
> (M7-R14):** at a 1.5 floor against a ~1.7 worst case, it tolerates a **~13% regression** — the same
> blindness the wall-clock bar had. It answers the **false positive**, not the false negative, and
> the floor cannot be tightened without false-firing against the observed spread.
>
> **And it has a domain (M7-R13):** below **4 cores** the derived default *is* `PRE_M7_SHAPE`, so the
> ratio is 1.0 by construction — the guard skips and says so, rather than reporting "a regression in
> the thread work" on a box where M7 simply has nothing to deliver. *The speedup scales with the box.*
>
> A **15 s** sanity ceiling remains for the order-of-magnitude case. It was 8 s and that was not loose
> at all: the reviewer's documented-command run hit a **median of 10,391 ms**, so the ceiling fired on
> the harness's own recipe — and blamed the power state for what was test concurrency.

**Where that leaves the milestone, stated without the flattering framing.**
- **The bar is missed.** ~4.7 s at the shipped default, ~60% over. It is missed at the *fixture's*
  22.3 KiB; real Claude Code turns run **20–40 KB**, so the top of the range is worse again (~8 s).
- **The ~2× is real**, reproducible on any box, and it is what M7 set out to buy: `with_intra_threads(1)`
  was leaving 11 of 12 cores idle on every request. That part is done and guarded.
- **The remaining gap is not a threads problem.** No arrangement of `pool × intra` closes a 4.7 s turn
  to 3 s on this hardware — the sweep's whole surface tops out around 2×, because intra-op scaling is
  sublinear and one request occupies one session.

So: **we missed the bar and stopped anyway**, because the next move is not more of this one. **S3 is
the named lead**, and the reason it is the right one is that it does not fight the box at all: the
boilerplate is byte-identical every turn, so a content-keyed cache makes turn 2+ nearly free
regardless of the machine, the power state, or the core count. It carries a real risk — **state on the
masking path** — and its threat argument is already written (S3, above). If the CC battery re-run (S6)
says the latency still bites in practice, that is where to go, and it should be bought on purpose.

### An asymmetry worth recording for whoever picks this up

At `intra=12`, the **100 tiny schema descriptions became the single biggest tier** — 909 ms of a
~2.2 s turn, for only 7 KB. They barely improved (1,094 → 909 ms, 1.2×) while the tool descriptions
improved 2.5×: a ~20-token sequence cannot use 12 threads, so it is nearly all per-call overhead
(~9 ms × 100). **Threads are done here; batching those calls is the next lead, not more threads.**

## 2026-07-16 — M7 implementation plan (design only — not built)

The technical blueprint for M7 (NER latency), to hand to the builder. **No source written.** Scope
and the measurements that opened it are in ROADMAP → M7 and the entry below.

### S0 — Fix the measurement first, because mine is probably wrong

**Start here, and do not skip it.** The entry below reports **903 ms/KB** and concludes the
fixpoint's second pass triples the cost. That number came from 29 KB **densely packed with names**
(`"Il cliente Mario Rossi di Acme SpA a Milano…"` ×450). **A real Claude Code request has the
opposite shape**: ~30 KB of boilerplate with **almost no PII**, plus a ~100-byte user message that
has it all.

And `Vault::mask_all` runs **per field**, not over the whole body. So on real traffic:

| field | size | entities | fixpoint passes | cost @ 286 ms/KB |
|---|---|---|---|---|
| `system` + tool schemas | ~30 KB | ~0 | **1** (detect → empty → return) | ~8.6 s |
| the user's message | ~100 B | 2–3 | 2 | negligible |

**Which changes the whole priority order.** A real turn is likely **~9 s, not 27 s**, and **lead 2
(the fixpoint) is worth almost nothing** — it only doubles fields that *contain* PII, and those are
tiny. Meanwhile **lead 3 (the cache) gets bigger**: ~8.6 s per turn spent re-scanning byte-identical
boilerplate, every turn, forever.

> **This is the M5 mistake, one level down, and it is mine.** The entry below scolds PERF-01 for
> measuring *a repeated synthetic sentence instead of a real payload* — and then measures *a dense
> synthetic blob instead of a real payload*. The corpus had a shape; the shape was a blind spot.
> **The fixture is the experiment.** Build it right before trusting a single number, including the
> ones I wrote down.

**Do:**
- Add a **realistic** payload fixture: ~30 KB of system prompt + ~10 tool `input_schema`s + a short
  user message carrying a few PII values. Sparse entities, exactly like real traffic. *(A captured
  real body is tempting — the trace log has one — but it is **masked**, so its NER pass finds
  nothing and the measurement lies in the other direction. Synthesize the shape, not the content.)*
- Re-measure per-field, not per-body: it is the field distribution that decides everything.
- **Then re-read the leads.** If a real turn is ~9 s and threads buy 3×, it is ~3 s and M7 may be
  done without touching the fixpoint or adding a cache.

### S1 — Lead 1: use more than one core (`src/pii/onnx.rs`)

> **⚠ The sketch below is M7's plan as written on 2026-07-16. M11 Track B (2026-09-02) changed the
> divisor** — `available_parallelism()` became `min(physical_cores, available_parallelism())`. Don't
> lift the snippet; see ARCHITECTURE → *NER threading*.

`with_intra_threads(1)` + a pool of sessions optimizes **concurrent throughput**; we need
**single-request latency**. Replace the constant with a **derived, overridable** value:

```rust
// NER_INTRA_THREADS, else derive. The two knobs MULTIPLY: pool × intra is the thread count,
// and it must fit the machine — `intra = 12` with `pool = 2` puts 24 threads on 12 cores.
let intra = env("NER_INTRA_THREADS")
    .unwrap_or_else(|| max(1, available_parallelism() / pool_size));
```

**Measure both axes** (this is the trade-off, not a tuning knob):
- **latency**: one request, wall clock — the Claude Code case (concurrency ≈ 1);
- **throughput**: N concurrent requests, req/s — the shared-proxy case, which the current design
  was built for and which must not regress silently.

**Two things to measure rather than assume:** SMT (`available_parallelism()` = 12 logical = 6
physical × HT; dense math often prefers **6**), and sublinear scaling (expect ~3× from 6 threads,
less on an **int8** model whose kernels are memory-bandwidth-bound).

#### S1a — …and the derived formula is wrong at concurrency 1, because the pool does nothing there

**Read the code before believing the formula above — I didn't, and it divides by the wrong thing.**
A single request is sequential at **three nested levels**, and only the innermost is `intra_threads`:

| level | where | today |
|---|---|---|
| fields | `privacy.rs:92` — `mask` is a closure holding `&mut vault` | sequential **by construction** |
| chunks | `onnx.rs:219` — `infer_chunked`'s plain `for` loop | sequential |
| the model call | `onnx.rs:154` — `with_intra_threads(1)` | 1 thread |

So **one request uses exactly one core, whatever `NER_POOL_SIZE` is.** The pool buys *concurrent*
capacity: a second session only ever runs when a second request is in flight. `pool × intra` is
therefore the thread count **under saturated load**, not the count a single request can reach —
that one is `intra` alone.

Which makes `intra = available_parallelism() / pool_size` a **pessimization in the case M7 exists
for**: at the default `pool = 2` it yields `12 / 2 = 6`, and the Claude Code case (concurrency ≈ 1)
then runs 6 threads while **6 cores sit idle** waiting for a second request that never comes. The
divisor is right for the shared proxy and wrong for the personal one.

**Two ways out — and they are not equivalent:**
- **Chunk-level parallelism** (`infer_chunked` fans its ~18 *independent* chunks across the pool).
  Near-linear — chunks are embarrassingly parallel — where intra-op scaling is sublinear (~3×). It
  is also what makes the divisor honest: `pool = 2 × intra = 6` would then really be 12 threads on
  one request. **Costs RAM, and this is the trade:** each session holds its own copy of the weights
  — measured **~834 MB at `pool = 2`** (README), i.e. ~400 MB per session, so `pool = 6` ≈ **2.5 GB**
  against a *lean-RAM* bar. Latency bought in gigabytes.
- **`pool = 1, intra = all`** — free, no RAM change, and already the ROADMAP's recommendation for the
  personal case. It gets the sublinear ~3×, not the ~6×.

> **The boundary that decides which parallelism is even legal: parallelize *detection*, never
> *minting*.** Chunk fan-out is safe because chunks are read-only w.r.t. the `Vault` — they only
> detect, and `infer_chunked` already merges by a deterministic `sort` + `dedup`. The **field** walk
> is not safe to parallelize, and `&mut vault` is not merely why it's hard, it's why it's *wrong*:
> placeholder **numbering follows encounter order**, so racing two fields makes `[EMAIL_1]` vs
> `[EMAIL_2]` a coin flip and breaks the determinism M1 Part B pins. Whoever picks this up: the
> `&mut` is load-bearing, not an obstacle to route around.

**So S1's real question is not "what value for `intra_threads`" — it is "does a single request get
to use the box at all?"** Measure `pool=1, intra=6` and `pool=1, intra=12` first (free, no RAM),
and only reach for chunk fan-out if the sublinear ceiling isn't enough — with S2's bar, not to
exhaustion.

### S2 — The stop criterion, declared *before* the numbers

**If a realistic turn drops below ~3 s, stop.** Ship it, re-run the battery, tag. Do **not** do S3 or
S4. Adding state to the masking path or removing a detector pass are both real risks, and buying
them "because we were already in there" is how a privacy tool grows a leak. **Optimize to a bar, not
to exhaustion.**

### S3 — Lead 3: don't re-scan an unchanged system prompt *(only if S2 says so)*

The boilerplate is byte-identical every turn. A **content-keyed cache** (hash of the field text →
the `Vec<PiiEntity>` found) makes turn 2+ nearly free; the per-request `Vault` still mints the
placeholders, so determinism and the per-request round-trip are untouched.

**It needs its own threat argument before it ships**, because it puts **state on the masking path**:
- what is the key, and can two different texts collide into one entity set? (hash choice is a
  *security* decision here, not a perf one);
- bounded size + eviction — an unbounded cache on an **unauthenticated** path is the M4-R19 shape
  again, in memory instead of CPU;
- what happens on a cache **miss vs a wrong hit**? A miss costs time; a wrong hit **leaks**. Fail
  closed: on any doubt, re-scan.

### S4 — Lead 2: the fixpoint's second pass *(probably unnecessary — see S0)*

Only if S0's realistic numbers still show it matters. The idea: passes ≥2 exist to catch masking
**exposing** PII (a masked phone splits a digit run, revealing a card) — a **deterministic**
phenomenon. So re-run only the structured layer on later passes and skip the NER.

**The correctness argument must be made and tested first**, on paper and in the corpus: masking
`John Smith` inside `John Smith-Jones` yields `[PERSON_1]-Jones` — **does the NER then tag `Jones`,
and do we lose it if pass 2 skips the NER?** Decide it with a test, not an opinion. This is the one
lead that can **lose detection**; the other two cannot.

### S5 — Rewrite the CC prompts as natural agent tasks *(independent — do it anytime)*

No measurement blocks this. See ROADMAP → M7 for the design question (do **not** fix it with a
`CLAUDE.md` telling the agent to comply).

### S6 — Re-run the battery, then publish the numbers

CC-01…CC-09 × {OFF, ON} with `NER_REQUIRED=1`, and record per-turn latency next to the RAM figures
in both READMEs — measured, like the RAM ones, not claimed.

## 2026-07-16 — The live gate closed, then re-opened: the hybrid is unusable with Claude Code

The first real Claude Code session through the proxy **worked** — and then the same afternoon of
measurement turned the celebration into the most useful set of numbers this project has. Nothing
here is a leak; all of it is the kind of thing only a real client on a real provider can show.

**What held (measured, 2026-07-16).** A subscription-logged-in Claude Code, pointed at the proxy
with **no credential configured anywhere**, got **200 on the first request — no 401, no retry**.
It forwards its own credential; the proxy passed it verbatim while `UPSTREAM_API_KEY` was unset.
The request left masked (`contatta [EMAIL_1], IBAN [IBAN_1]`), the reply came back with the real
values restored, and a grep of the whole trace for the raw email and IBAN found **zero** — DBG-02
on real traffic, not the synthetic case `tests/log_safety.rs` pins.

> **The auth doc was backwards, and only the test could say so.** The previous
> `MANUAL_VERIFICATION.md` asserted that routing Claude Code through the proxy was impossible on
> **two** counts: schema *and* auth ("an OAuth token scoped to Claude Code's own flow… not a plain
> Bearer usable against the raw API"). M6 disproved the schema half by construction. The live run
> disproved the auth half by observation. Third-party write-ups still claim a custom
> `ANTHROPIC_BASE_URL` *requires* setting `ANTHROPIC_AUTH_TOKEN`; for 2.1.211 it does not.

**Then: the first run was testing half the product, silently.** No model was configured, so
`load_onnx_ner` returned `Ok(None)` and the proxy ran **structured-only** — the deliberate
fail-*open* posture for names. Email and IBAN masked perfectly *because they are deterministic
recognizers*, so everything looked green while **the NER never ran**; a `Person` would have gone
upstream in clear. `NER_REQUIRED=1` is now mandatory for the battery: it makes that silent
downgrade a fatal startup error. It proved itself within minutes — `cargo test` (default features)
overwrites the onnx binary at the **same path**, with no warning, and the flag caught exactly that.

**And with the NER actually on, the proxy is not usable with Claude Code.** Measured on this box
(Ryzen 5 PRO 8540U, 6 cores / 12 threads), timing the *masking alone* (a credential-less request
masks fully, then 401s — so the time to the 401 **is** the masking cost):

| what | 29 KB field | per KB |
|---|---|---|
| structured only | **20 ms** | ~0.7 ms |
| hybrid, debug | 27,728 ms | 956 ms |
| hybrid, **release** (fat LTO) | 26,863 ms | 926 ms |

- **The masking is linear** — ~0.96 s/KB, constant across 2 / 10 / 29 KB. M4's DoS guards hold;
  this is not an algorithmic bug.
- **The NER is ~100% of the cost**: 20 ms vs 27 s on the same 29 KB — a factor of **~1,400×**.
  The deterministic layer is free, exactly as the README claims.
- **Release buys 3%.** The cost is inside ONNX Runtime — a prebuilt, already-optimized native
  library. Compiling *our* Rust faster changes nothing. (Worth knowing before anyone "fixes" this
  with a profile flag.)
- **Claude Code sends 20–40 KB every turn** (its system prompt plus every tool's `input_schema`),
  which we re-scan from scratch each time → **20–40 s per message**.

**Why nobody saw it.** M5's PERF-01 measured the NER on a *repeated synthetic sentence* and
concluded "linear" — correctly. It never measured a **real client's payload**, which is dominated
by boilerplate re-sent every turn. "Linear" was true and beside the point: the constant is what
makes it unusable, and only a live client exposes the constant.

**Three leads, measured not guessed:**

1. **We use 1 core of 12.** `OnnxNerDetector::load` sets `with_intra_threads(1)` deliberately — the
   design holds a *pool* of single-threaded sessions, which optimizes **concurrent throughput** and
   is exactly wrong for **single-request latency**: an oversized field's ~18 chunks run one after
   another, each on one core.
2. **The fixpoint's second pass costs more than the first.** A 34.7 KB field with **no** PII (one
   detector pass) masks in **9,923 ms** = 286 ms/KB. A *smaller* 29.4 KB field **with** PII (two
   passes) takes **26,567 ms** = 903 ms/KB — **2.7× slower while being 15% shorter**. M4-R21
   accepted that "~2×" when the NER saw small fields; on a 30 KB system prompt it is ~13 s of
   re-scanning per turn. And the phenomenon the second pass exists for — masking *exposing* PII by
   splitting a digit run — is a **structured-recognizer** effect. Masking a name to `[PERSON_1]`
   does not reveal a new name. Re-running only the *deterministic* layer on passes ≥2 is the
   obvious lead; it needs a real argument about NER recall at the seams before it ships.
3. **The static prompt is re-scanned every turn.** The system prompt and tool schemas are identical
   across turns. Nothing exploits that today.

**Two flaws were in the test, not the product.** The battery's prompts said *"reply with exactly
this sentence: contact jane.doe@example.com, IBAN …"* — and the model **refused**, correctly reading
it as an injection attempt ("it has nothing to do with this repository"). Claude Code is an *agent
with a repo context*, not a completion endpoint, and it inherited that context precisely because we
moved the fixture **inside** the repo. The scenarios must be **natural agent tasks** (read this
file, format this contact), which is also what real usage looks like. Rewriting them is the next
step.

**Status: the `1.0.0` tag waits.** Not for a leak — for an honest answer to "is the product we
advertise actually usable?". Investigating leads 1–3 before tagging.

## 2026-07-16 — M6 review round 1 closed (5/5): the leak was a source *inside* a known block

Independent reviewer pass over the M6 landing (`0cdd251` + `98d9f55`). **Five findings, all closed.** Full
record: [`reviews/M6.md`](reviews/M6.md). Post-fix: **85** lib tests default / **97** with `--features onnx`,
**10** e2e; `fmt` + `clippy` clean on both.

**M6-R1 — the one real leak, and it hid one level down from where the guard looks.** The block-type dispatch
is strict (unknown *block* → 400), but inside the known `document` block the **`source.type`** dispatch was
fail-*open*: the first cut masked only a `text` source and skipped every other source type. Anthropic has a
**`content`** document source — `{type:document, source:{type:content, content:[{type:text, text:"…"}]}}` —
whose blocks carry plaintext PII, and the reviewer reproduced it through the real router: a raw email + IBAN
reached the mock upstream **in clear**. The fix mirrors the block-level rule one level down —
`mask_anthropic_document` dispatches `source.type` (`text`→data, `content`→recurse the nested array,
`base64`/`url`→skip, **unknown→fail closed**) and also masks the `title` / `context` metadata the first cut
skipped. **The lesson: "unknown → fail closed" has to hold at *every* dispatch, not just the outermost one —
a fail-open branch inside a known block is exactly as much a leak as an unmodelled block.** R2 is the same
class one level up (the `system` array skipped a no-`text` object open) — closed the same way.

**M6-R3/R4/R5 — the docs/test-quality trio.** R3: the test counts were wrong (a "96 lib default" that was
really the onnx count, "14 unit" that was 12) — corrected and restated in the "N default / M onnx" form the
M4 miscount lesson prescribes. R4: three e2e placeholder-*presence* asserts were satisfiable by the
augmentation prompt (which literally contains `[EMAIL_1]`/`[IBAN_1]` as examples) — not vacuous (the
`!contains(raw)` checks are the real guard), but weaker than they read; now they assert the **specific masked
field** or a token absent from the prompt (`[PHONE_1]` / `[EMAIL_6]`). R5: a "Prepend" comment that appends.

## 2026-07-16 — M6 built: native Anthropic `/v1/messages` (Claude Code passthrough)

Implemented M6 end to end on `feat/m6-anthropic-messages`, following the 7-stage plan below. The proxy now
accepts a **native** Anthropic Messages body, masks it **in place** (no OpenAI translation), forwards
native→native, and de-anonymizes both the buffered reply and the SSE stream. The masking engine
(`Vault::mask_all`, the fixpoint, `spawn_blocking` + fail-closed) carried over untouched — M6 is *only* the
Anthropic-schema walks plus the native forward/auth. **Green on both feature sets** (after review round 1:
**85** lib tests default / **97** with `--features onnx`, **10** new e2e; `fmt` + `clippy` clean).

**What landed, per stage:**
- **T0 — `WireSchema` tag** (`pipeline/mod.rs`): `enum WireSchema { OpenAi, Anthropic }` on `RequestContext`,
  defaulting to `OpenAi`, so the existing path is untouched. `PrivacyStage` and `SseDemasker` dispatch on it.
- **T1 — route + handler** (`server.rs`): `/v1/messages` registered **only when `provider == "anthropic"`**.
  The mask + fail-closed + streaming-detect flow is factored into `run_privacy_stages` / `finish_buffered` /
  `finish_streaming`, shared by both handlers — the two differ only in the schema tag, the forward, and the
  SSE schema.
- **T2 — request mask** (`pipeline/privacy.rs`, `mask_anthropic_request`): `system` (string / text-block
  array), `messages[].content` blocks dispatched on `type`, `tools[].description` + `input_schema`. Unknown
  block type → **fail closed, 400**, with a reason that carries **no** client-controlled value.
- **T3 — augmentation** into the top-level `system` (`inject_augmentation_anthropic`): absent → created;
  string → appended; block array → pushed as a trailing text block. Only when something was masked.
- **T4 — buffered demask** (`demask_anthropic_response`): top-level `content[]` — `text` and `tool_use.input`
  string leaves — mirroring T2.
- **T5 — SSE demask** (`stream.rs`): `SseDemasker` factored into the shared split-placeholder hold-back core
  + a per-schema rewriter. Anthropic `content_block_delta` (`text_delta` / `input_json_delta`), held back per
  block `index`.
- **T6 — native forward/auth** (`proxy.rs`, `send_messages` / `forward_messages` / `messages_auth`): path
  `/v1/messages` (`config.upstream_messages_path`, default `/v1/messages`); credential resolution; default
  `anthropic-version`.
- **T7 — tests:** 13 unit (`privacy.rs` ×9: mask coverage / document sources / fail-closed / augmentation /
  demask; `stream.rs` ×4: SSE split / `input_json_delta` / pre-stop flush / pass-through) + 10 e2e
  (`tests/anthropic_messages_e2e.rs`). *(Counts are post-review-round-1.)*

**Four design calls made while building, recorded honestly:**

- **Auth: client credential wins, proxy key is the fallback — a reconciliation.** The ROADMAP scope says
  *"inject the proxy's own key as `x-api-key` only when the client sent none"* (client wins); the terser
  DEVLOG-plan **T6** line wrote it the other way (*"proxy-key → `x-api-key`, else client `Authorization`"*).
  These conflict. I took the ROADMAP ordering — it matches the existing **chat-path** posture (ARCHITECTURE:
  *"the client's own `Authorization` wins, else the configured key"*) and the M5 note that the proxy *"forwards
  a client `Authorization` verbatim in preference to `UPSTREAM_API_KEY`"*, and it is the whole point of the
  feature (Claude Code's OAuth token must not be overridden by a configured key). Final order:
  client `Authorization` (verbatim) → client `x-api-key` → proxy key as `x-api-key` → **401**. An OAuth token
  only ever rides `Authorization`, never `x-api-key` (Anthropic 401).
- **`thinking` is masked on the way up but never de-masked on the way down.** A thinking block is generated by
  the model over already-masked input, so it naturally contains only placeholders, and its `signature` signs
  *that* placeholder text. Leaving the placeholders intact means the bytes never change across a multi-turn
  replay (re-masking an inert placeholder is a no-op), so the signature stays valid — robustly, even if
  placeholder *numbering* shifts elsewhere in the conversation. De-masking thinking would either break the
  signature or make correctness depend on reproducing identical numbering. So the request walk masks `thinking`
  (safety, in case a client injects fresh PII) and the response walk deliberately skips it. Promoted to
  ARCHITECTURE → *Native Anthropic Messages*.
- **A text-source `document` is masked, improving on the plan's "skip document".** The plan said skip `document`
  as non-text — but an Anthropic `{type:document, source:{type:text, data:"…"}}` carries **plaintext** that can
  hold PII, so skipping it would leak. The walk masks `source.data` when the source is text and skips the
  base64 file case. Never-leak beats the simplification.
- **The SSE demasker holds the `event:` line to fix frame ordering.** Anthropic frames each event as an
  `event:` line + a `data:` line. A block's held-back tail must flush **before** its `content_block_stop`
  frame (a `content_block_delta` after the stop is protocol-invalid). But the `event: content_block_stop` line
  arrives *first*, so flushing while processing the `data:` line would scramble the frame. Fix: the demasker
  holds each `event:` line until its `data:` line, and at a `content_block_stop` it flushes the block's tail
  ahead of the held `event:` line. OpenAI streams have no `event:` lines, so the mechanism is inert there. This
  is exactly the streaming-demask piece the prior proxy left as a TODO.

**One scope boundary:** **server-side result blocks** (`server_tool_use` / `web_search_tool_result`, sent only
when server-side tools are enabled — not the default Claude Code path) currently **fail closed** rather than
being modelled. This is safe (no leak) and a conscious future addition; the core client-side flow
(`text` / `tool_use` / `tool_result` / `thinking`) is covered.

**Still open — the `1.0.0` gate is unchanged:** the opt-in live verification (point a real Claude Code session
at the proxy, run the M5 dual-run against live Anthropic). It needs a human with Claude Code + credentials and
is not runnable in this environment. Everything testable without a live provider is done and green.

## 2026-07-15 — M6 implementation plan (design only — not built)

The technical blueprint for M6 (native Anthropic `/v1/messages`, Claude Code passthrough), to hand to the
builder. **No source written** — this is the plan; scope + the design decisions are pinned in ROADMAP → M6.

**Reuse, don't rebuild.** The masking engine is already schema-agnostic: `Vault::mask_all` (the M4-R17
fixpoint), the `spawn_blocking` + fail-closed handling in `server.rs`, the split-placeholder hold-back in
`stream.rs`, and `AUGMENTATION_PROMPT` all carry over untouched. What M6 adds is *only* the Anthropic-schema
walks (mask / demask / SSE) and the native forward/auth.

**The 7 stages, with files:**
- **T0 — schema tag.** `enum WireSchema { OpenAi, Anthropic }`; a field on `RequestContext` (`pipeline/mod.rs`);
  `PrivacyStage` dispatches on it. Zero impact on the OpenAI path.
- **T1 — route + handler** (`server.rs`). `/v1/messages` registered **only when `UPSTREAM_PROVIDER=anthropic`**;
  a `messages` handler sharing the mask + fail-closed + streaming-detect flow with `chat_completions`
  (factored), differing only in schema, forward, and SSE demasker.
- **T2 — request mask** (`pipeline/privacy.rs`, `mask_anthropic_request`). `system` (in place),
  `messages[].content` blocks dispatched on `type` (`text` / `tool_use.input` object leaves /
  `tool_result.content` recursive / `thinking`), `tools[].description` + `input_schema`. **Unknown block-type
  → fail closed, 400**; the known set is exhaustive for Claude Code and pinned by a guard.
- **T3 — augmentation** into the top-level `system` (string or block array), only if something was masked.
- **T4 — buffered response demask** (`demask_anthropic_response`): top-level `content[]` (`text` +
  `tool_use.input`), mirroring T2.
- **T5 — SSE demask** (`stream.rs`): factor `SseDemasker` into the shared split-placeholder core + a per-schema
  rewriter; handle Anthropic `content_block_delta` (`text_delta` / `input_json_delta`), held back per block
  `index`. *The privacy-critical piece the prior proxy punted.*
- **T6 — native forward/auth** (`proxy.rs`, `send_messages`): path `/v1/messages`; proxy-key → `x-api-key`,
  else client `Authorization: Bearer` (OAuth) verbatim, else client `x-api-key`, else 401; never OAuth in
  `x-api-key`; forward `anthropic-version` (default `2023-06-01`) + `anthropic-beta`.
- **T7 — tests** (adversarial-first): native coverage / fail-closed, buffered + SSE demask, e2e mock native
  upstream, opt-in Claude Code smoke, and the M5 manual dual-run finally runnable via Claude Code → the proxy.

**Delivery:** one branch → PR (`feat/m6-anthropic-messages`), streaming included, gated by the per-PR CI.

## 2026-07-15 — reqwest 0.12→0.13 (TLS backend pinned) + tower-http 0.6→0.7

Took the two Dependabot cargo bumps deliberately, by hand, rather than merging the auto-PRs — because
reqwest 0.13 hid a trap.

- **tower-http 0.6 → 0.7**: a clean bump. No `http`/`hyper`/`tower` major change, and the only thing this
  crate uses — `TraceLayer::new_for_http()` — is unchanged. Nothing to do but the version.
- **reqwest 0.12 → 0.13**: the source APIs this crate uses (`Client`, `.json()`, `.bytes_stream()`,
  `.headers()`, `header::*`) are all unchanged — but reqwest 0.13 **silently flipped its default TLS backend**
  from native-tls to **rustls + aws-lc-rs**. Dependabot's PR kept the default features, so it would have
  pulled `aws-lc-rs` into the **default** build — breaking the native-dep-free guarantee that
  `tests/dependency_footprint.rs` (M2.5-R1) exists to hold, and adding a cmake/NASM build requirement on the
  Windows/arm64 release. So the bump **pins the TLS backend explicitly**:
  `default-features = false, features = ["native-tls", "json", "stream", "http2", "charset", "system-proxy"]`
  — same runtime behaviour as 0.12, native-dep-free default preserved, and the crypto backend is now a
  deliberate choice rather than a dependency's shifting default (the lesson is recorded next to the dep in
  `Cargo.toml`).

Verified locally on the default build: `cargo build`, the full default `cargo test` (incl. the footprint
guard — green, no `aws-lc-rs`), and `cargo clippy -D warnings` all pass. reqwest deduplicates to a single
`0.13.4` (the onnx/hf-hub stack already used it). One benign new duplicate: reqwest 0.13 is now built on
tower, so it pulls its own `tower-http 0.6.11` alongside our `0.7.0` — cargo-deny only **warns** on that
(`multiple-versions = "warn"`). The onnx leg / MSRV / fmt are left to CI (`ci.yml`), which now gates this
through a PR. Landed on a branch so the gate runs before merge.

## 2026-07-15 — CI reinstated (trimmed) as a per-PR gate; Dependabot tuned down

Enabling Dependabot (below) exposed a gap: version-update PRs had **no automated build/test**. The
tag-driven pipeline only builds at tag/manual time, and cargo-deny / CodeQL check vulnerabilities and
patterns — not "does it still compile". So a bump like reqwest 0.12→0.13 (a *breaking* 0.x bump) could read
"Ready to merge" in the UI yet break `main` silently. Fixed on two sides:

- **`ci.yml` brought back, trimmed** (was `ci.yml.disabled`). On push-to-main + PR: `fmt`, `clippy` + `test`
  on both feature sets (`default` and `--features onnx`), and an MSRV `cargo check`. It **drops** the
  all-targets release build — that stays in `manual-build.yml` / the tag release (`release-build.yml`). So
  every PR, Dependabot's especially, is now self-verifying. Ships with `permissions: contents: read` (no
  repeat of the missing-workflow-permissions alert).
  - The MSRV leg **reads the floor from `Cargo.toml`** (`cargo metadata --no-deps | jq .rust_version`)
    instead of hardcoding it, so CI and the manifest can't drift — the M5-R5 lesson, applied.
- **`dependabot.yml` tuned down** — `weekly`→`monthly`; all non-major cargo updates **grouped** into one PR;
  true `semver-major` (1.x→2.x) bumps **ignored** (done by hand). Caveat recorded in the file: Dependabot
  classifies 0.x breaking bumps (reqwest 0.12→0.13) as *minor*, so `ignore semver-major` does **not** suppress
  them — `ci.yml` is what makes those safe.

**This partly reverses the same-day pipeline change** (which had moved fmt/clippy/test off push/PR). The
reversal is deliberate and narrow: the tag-driven *release* is unchanged, and the all-target cross-compile is
still tag/manual-only — what came back is only the lightweight *correctness* gate, because Dependabot needs
one. The 3 Dependabot PRs already open (reqwest 0.13, tower-http 0.7, a github-actions group) are unaffected by
the config change (it applies to future runs); once `ci.yml` lands on `main`, Dependabot rebases them and the
gate runs on each — so their check status shows directly whether the breaking bumps build.

## 2026-07-15 — Least-privilege GITHUB_TOKEN across all workflows (CodeQL alert)

First finding from the just-enabled CodeQL default setup — `actions/missing-workflow-permissions`, on **all
three** build workflows. A true positive: a workflow with no `permissions:` block runs on the repo's default
token, which can be read-write; that violates least privilege for no reason.

Added a top-level `permissions: contents: read` to `release-build.yml` (reusable), `manual-build.yml`, and
`release-build-publish.yml`. The release **publish** job keeps its own `contents: write` — the one privilege
that creates the Release (`write` subsumes `read`); everything else (checkout, build, test, artifact upload)
needs only read (artifact upload uses the Actions runtime token, not `GITHUB_TOKEN`). `security.yml` already
ran read-only. `ci.yml.disabled` is parked — not scanned by Actions/CodeQL — and gets the same block if it is
ever revived. No release behaviour changes.

## 2026-07-15 — Automated dependency-security scanning (Dependabot + cargo-deny)

Added free, public-repo supply-chain scanning — a privacy proxy inherits the CVEs of everything it links, so
the dependency surface is now scanned automatically instead of by hope. All three additions are infra/docs;
no source changed.

- **`.github/workflows/security.yml`** — runs **`cargo deny check advisories bans sources`** via
  `EmbarkStudios/cargo-deny-action@v2`. `advisories` = the **RustSec** CVE DB (the security core); `sources`
  pins crates to crates.io (an unknown git/registry host is the shape a supply-chain attack takes); `bans`
  flags duplicate versions. Triggers: PR + push-to-main **on dependency-manifest changes only** (paths-filtered,
  so doc commits don't spin it up), a **weekly `cron`** (the important one — a CVE can be disclosed against a
  dep you already have, with no code change to trigger a run), and `workflow_dispatch`.
  - **Set `rust-version: stable`.** The action defaults to cargo 1.71, which can't even run `cargo metadata`
    on this tree — it pulls crates needing `edition2024` (cargo ≥ 1.85). cargo-deny only needs a cargo new
    enough to *read* the dependency graph, **not** the project MSRV, so `stable` (always ≥ 1.85) is the lean
    choice — no hardcoded version to drift from `Cargo.toml`. (Left at the default it would have failed on the
    first run.)
- **`deny.toml`** — config for the above. `advisories.unmaintained = "workspace"` (flag OUR deps going
  unmaintained, not the dormant transitive world = noise); `sources` fail-closed; `bans` warn on dup versions,
  deny wildcards. **`licenses` is deliberately OFF the CI command** — compliance, not security, and noisy
  against the `onnx` crypto stack (`ring` / `aws-lc-sys` carry non-SPDX license refs). A ready-to-enable
  allow-list is commented in the file (this crate is AGPL-3.0-or-later).
- **`.github/dependabot.yml`** — version updates for the `cargo` and `github-actions` ecosystems (grouped
  weekly to keep PR noise down). Dependabot **alerts + security updates** are a separate repo toggle the
  maintainer enables in *Settings → Code security & analysis* (same for native CodeQL / secret scanning).

**On the push/PR posture.** The pipeline change below (same day) disabled the *build* CI — fmt/clippy/test
moved to build/tag time. This adds back a **security-only** workflow on push/PR/schedule, deliberately narrow:
it does not compile the crate (cargo-deny reads `Cargo.lock`), so it does not reintroduce per-push build cost,
and fmt/clippy/test stay exactly as that change left them. The design posture is recorded in ARCHITECTURE →
*Supply-chain & dependency security*.

## 2026-07-15 — Pipeline change: CI disabled, tag-driven release only, + Windows-arm64 target

A deliberate change of approach to the GitHub Actions setup (maintainer's call), reversing part of the M5
CI story:

- **`ci.yml` disabled** (renamed `ci.yml.disabled`). Nothing runs on push/PR anymore. **`cargo test` was
  moved into `release-build.yml`** (see below), so the suite still runs on GitHub — on *every* target's native
  runner, at `manual-build` / tag time — just not per push; a release that fails its tests never publishes.
  **`fmt` / `clippy` / `msrv` are now local-only** gates (the CLAUDE.md "green before done" bar). A lint break
  surfaces at build time or locally, not on a PR. (Tradeoff accepted deliberately; recorded so it isn't a
  surprise.)
- **`release-publish.yml` → `release-build-publish.yml`** (`name: Release build & publish`). Same tag-only
  trigger, same structural "no manual run can publish". The other workflows' cross-references were updated to
  match.
- **READMEs now badge the release, not CI**: the `release-build-publish` workflow status ("did the last tag
  build?") plus a `github/v/release` version badge, replacing the old `ci.yml` badge. A tag is needed before
  either shows anything — hence the interim `v0.4.0` below.
- **Added Windows-arm64** (`aarch64-pc-windows-msvc`) to the matrix, built natively on GitHub's free
  `windows-11-arm` runner. **Not yet validated.** A local `cargo check --target aarch64-pc-windows-msvc
  --features onnx` failed — but on a *local-toolchain* gap, not the target: `aws-lc-sys` (onnx TLS stack)
  compiles its ARMv8 crypto from source and needs the ARM64 MSVC toolchain this x86_64 box lacks (`CC=None`,
  `LNK1181 chacha-armv8.o`). aws-lc-sys *does* support the triple but has had win-arm64 friction upstream, so
  it is plausible-but-unproven on the native runner. **Now validated GREEN** — a `manual-build` run built all
  5 targets, win-arm64 included (run 29437007077, 2026-07-15). The native `windows-11-arm` runner has the
  ARM64 toolchain the local x86_64 box lacked, so aws-lc-sys built its ARMv8 crypto with no friction. The
  caution was worth taking; the outcome is positive.
- **Then `cargo test --features onnx` was added to `release-build.yml`** (per the maintainer) — so the tag
  build and every manual build now test on each target's native runner (the old CI only tested x86_64 Linux).
  This still needs one more `manual-build` to confirm the test step is green on all arches before the tag.
- **Interim tag `v0.4.0`** (the current version) to populate the release badge — explicitly *not* the `1.0.0`
  release, which still waits on M6. Cut only after a `manual-build` (with the test step) is green on all targets.

No source changed — workflows + docs only.

## 2026-07-15 — M6 opened: native Anthropic `/v1/messages` (Claude Code passthrough); `1.0.0` gate moved behind it

Promoted the Claude-Code slice of Option B into a scheduled milestone, **M6**, after establishing that the
tool cannot front the LLM we actually use: Claude Code speaks **only** the native Anthropic Messages API
(`POST /v1/messages`), and the proxy is OpenAI-compat-only (in *and* out), so a Claude Code session pointed
at it just 404s.

**Grounded in the prior proxy, not guessed.** Studied `francesco-stimola/llmproxy-extended` — specifically
the maintainer's own commit `d9962a4` *"add Anthropic-native /v1/messages route for Claude Code passthrough"*
(stabilized in `c658e6c`). Its design: accept `/v1/messages`, run the masking pipeline **on the
Anthropic-native body in place** (inject the top-level `system` field as a temporary message so the masker's
`messages[]` walk covers it, then restore), and forward **native→native** to Anthropic — no OpenAI
translation. Auth is a verbatim passthrough of the client's `Authorization: Bearer` (the OAuth
`sk-ant-oat01-*` token) + `anthropic-beta`; the proxy never holds a key. The commit's own note pins the
gotcha: **an OAuth token in `x-api-key` → Anthropic 401** — Bearer for OAuth, x-api-key for API keys.

**Two corrections to my earlier read, recorded honestly:**
- *Auth is not a blocker.* I had said Claude Code was blocked on both schema *and* auth. The prior proxy
  proves auth works via verbatim Bearer + `anthropic-beta` passthrough on a native route. The **only** real
  blocker is the missing native schema handling.
- *Translation is the wrong tool here.* The fork's base (Fabrizio Salmi) ships a full OpenAI↔native
  translation adapter for ~25 providers. Considered and **rejected** for the masking path: two lossy schema
  boundaries = two leak surfaces, against fail-closed. Adopted the maintainer's **mask-in-place** route
  instead; the translation adapter is kept only as a *field map* for the Anthropic schema.

**Where M6 improves on the prior proxy:** that route left **streaming demask as a TODO** — its streamed
replies were forwarded un-demasked, i.e. placeholders could reach the client. M6 makes the Anthropic SSE
demask (`content_block_delta` → `delta.text`) a scope item, reusing the hold-back `SseDemasker` we already
have for the OpenAI shape. A placeholder reaching the client is the exact failure this tool exists to
prevent, so it cannot be a TODO here.

Also: the prior proxy's PII *detection* had real gaps (SECRET unreliable, IBAN misclassified, names missed —
its own README's "Known Issues") — all already solved in this Rust proxy. M6 reuses the old **transport /
schema** design, never its detection.

**Release plan changed (per the maintainer):** the `1.0.0` tag now waits on M6 — a release ships only once
Claude Code works end-to-end against real Anthropic. Fronting the LLM we actually use is a `1.0` requirement,
not a follow-on. This also finally makes the M5 manual dual-run executable for real (a Claude Code
subscription drives the native route). README + README.it now state the OpenAI-compat-only scope explicitly
and point native-client support at M6. No source changed yet — M6 is scoped, not built.

## 2026-07-15 — M5's manual verification: dry-run-validated against a mock; the live-provider bar redefined to opt-in

Closed M5's last open box — the manual dual-run of `docs/MANUAL_VERIFICATION.md` — by **redefining its
bar**, transparently. The box asked for a run against the *real* provider, which needs a live
`ANTHROPIC_API_KEY` this environment does not hold.

**The obvious workaround was investigated and ruled out.** "Use this Claude Code session's own Anthropic
access as the credential" fails twice over: Claude Code speaks **only** the native Messages API
(`POST /v1/messages`, content blocks / `tool_use`), while the proxy proxies **only** the OpenAI-compat
`/v1/chat/completions` — a Claude Code session pointed at the proxy just 404s, nothing masked; and the
subscription credential is an **OAuth token scoped to Claude Code's flow** (the `anthropic-beta` header
carries an OAuth capability the upstream requires), not a plain `Bearer` usable against the raw API. So a
real-provider run cannot be surrogated from inside a session. Routing Claude Code *through* the proxy is
exactly **Option B** (native `/v1/messages` masking) — Backlog, out of M5.

**What was actually done:** ran the procedure's Run A / Run B through the **real compiled binary** against a
throwaway Node mock upstream that echoes the masked text it receives.
- **Run A** (`PII_DEBUG_SKIP_DEMASK=1`) → client received `[EMAIL_1]`; the `forwarding masked request body
  upstream` trace carried the placeholder; the mock saw only the placeholder; the raw email appeared in
  **neither** log stream (DBG-02).
- **Run B** (normal) → client received the restored `jane.doe@example.com`; the trace still showed only the
  placeholder, and the de-masked client output was **never** logged.
- **A vs B on the same input → a byte-identical masked body upstream**: the chain holds end-to-end.

**Why this is enough to close the box (a conscious call, not a silent one):** the *permanent* guarantee
already lives in CI — DBG-01 (`e2e_debug_skip_demask_returns_placeholders_to_client`) and DBG-02
(`tests/log_safety.rs`) — plus the three-preset mock e2e (`tests/proxy_e2e.rs`). The manual dry-run only
confirmed the **written procedure** itself works as documented. The real-provider dual-run is therefore
reclassified from a close-gate to an **opt-in** extra, ready in `docs/MANUAL_VERIFICATION.md` and
`tests/anthropic_smoke.rs` (E2E-INT-01) for whoever next holds a key. The `1.0.0` release gate is unchanged
and still stands on its own (a real green CI run).

Incidental finding folded into the guide: `tracing` writes to **stdout**, so the DBG-02 grep must include
stdout (a `2>`-only redirect would miss the trace). No source changed; no tests added or removed.

## 2026-07-15 — CI builds every release target; workflows renamed; the Node-24 bump done right

Three follow-ups after the first manual run went **green on all four targets** (including the new
`aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm` — so that runner and `ort`'s linux-arm64 prebuilt both
exist):

- **The Node-20 warning, fixed *properly* this time.** The previous bump to `@v5` was wrong: reading the
  actions' own `action.yml`, **`upload-artifact@v5` still declares `using: node20`** (v6 was the first on
  node24), and **`download-artifact@v6` is node20** too (v7 was its first node24). Bumped to the current
  node24 majors — **`upload-artifact@v7`, `download-artifact@v8`** (they differ because they are separate
  repos); `checkout@v5` was already node24. Lesson: don't infer an action's Node runtime from its version
  number — read `runs.using` in its `action.yml`.
- **CI now builds every release target** (reverses the "CI stays lean" call in the entry below). That call
  was made on *cost* grounds — and GitHub Actions is **free for public repos**, so the premise was wrong.
  `ci.yml` gained a `release-targets` job that calls the reusable `release-build.yml` with
  `upload-artifacts: false`: every push/PR now cross-compiles all four targets (a compile check, no
  artifacts), so a target-specific break like the Intel-mac one surfaces on a PR instead of at tag time.
  The reuse means "what CI checks" and "what a release builds" are the *same definition*, byte for byte.
- **Workflows renamed for clarity** (no more "build" vs "release" ambiguity): `build.yml` → `release-build.yml`
  (`name: Release build`), `release.yml` → `release-publish.yml` (`name: Release publish`). `manual-build.yml`
  keeps its name. The reusable workflow grew an `upload-artifacts` boolean input (default true; CI passes
  false) so the one definition serves both "build and keep" and "build to check".

All four workflows re-validated as YAML; no source changed.

## 2026-07-15 — release pipeline restructured: one build definition, two entry points, tag-only publish

Follow-up to the first-run fixes below. Reworked the packaging pipeline from one `release.yml` into
three files so the *manual* build and the *release* build share one definition and can't drift, and so
"a manual run can never publish" stops being an `if`-gate and becomes structural:

- **`build.yml`** — a **reusable** workflow (`on: workflow_call`) holding the whole cross-compile matrix
  and the package/upload steps, with a `retention-days` input. It has **no publish step at all**.
- **`release.yml`** — triggers **only on a `v*.*.*` tag**, calls `build.yml`, then publishes. It no longer
  has a `workflow_dispatch` trigger or a publish `if`-gate: with no manual trigger on the *publishing*
  workflow, a manual run structurally cannot cut a release. This **retires [M5-R11](reviews/M5.md#m5-r11)**
  (the event-vs-ref gate) — there is nothing left to get wrong. *(A note the first-run entry got slightly
  wrong: that run didn't publish primarily because a build failed; it didn't publish because it was a
  **manual event**. Even four green builds would not have published. This restructure makes that guarantee
  obvious instead of subtle.)*
- **`manual-build.yml`** — `workflow_dispatch` only, calls the same `build.yml` with **30-day** retention
  (manual builds are throwaway checks), and has **no publish job**. This is the pre-tag "do all targets
  still compile?" button.

**Added `aarch64-unknown-linux-gnu`** to the matrix — ARM Linux servers (Graviton et al.) are common and
`ort` *does* ship a prebuilt for it (unlike Intel macOS). Built **natively** on GitHub's `ubuntu-24.04-arm`
runner, so there's no cross-linker to maintain. Matrix is now Linux x86_64 + arm64, macOS arm64, Windows
x86_64-msvc. Comments are kept **count-agnostic** so adding/removing a target doesn't need a prose edit.

**On aligning CI with the release targets:** deliberately *not* done. A fat-LTO cross-build of four targets
on every PR is slow and costly for marginal benefit; `manual-build.yml` *is* the on-demand all-targets
check, run before a tag. (A scheduled weekly run is the cheap automation if drift-detection is ever wanted
— noted, not built.) `ci.yml` stays the fast per-push correctness gate (test/clippy/msrv on ubuntu).

All four workflows re-validated as YAML. No source changed. **`1.0.0` gate unchanged**: a real, fully-green
run — now easiest to get by running `manual-build.yml` across all targets first, then pushing the tag.

## 2026-07-15 — the release workflow's first *real* run: two things a green local build can't show

`release.yml` had never actually run — ROADMAP flagged exactly that as the open gate for `1.0.0`. A
manual `workflow_dispatch` run finally exercised it, and (as the whole point of running it was to find
out) it surfaced two things no amount of local `cargo build` could:

- **`x86_64-apple-darwin` fails to build.** `ort`'s `download-binaries` ships **no prebuilt ONNX Runtime**
  for Intel macOS at the pinned `=2.0.0-rc.12`; the build then falls back to compiling ONNX Runtime *from
  source*, which needs a cmake toolchain we don't set up in CI → exit 101. The other three targets
  (Linux x86_64, **macOS aarch64**, Windows x86_64-msvc) built clean and the publish step **correctly
  skipped** (manual run, not a tag — the [M5-R11](reviews/M5.md#m5-r11) event-gate working as designed).
  **Decision: drop the target, don't build ORT from source.** Intel Macs are legacy (Apple finished the
  arm64 transition in 2023), `aarch64-apple-darwin` covers every current Mac, and a from-source ORT build
  is a long, fragile, cmake-dependent step to maintain for a shrinking audience — the *over-engineering*
  the project's bar rules out. An Intel-Mac user builds from source (`docs/SETUP.md`); we don't ship a
  binary no one else here can reproduce.
- **Node-20 deprecation warnings.** `actions/checkout@v4` and `actions/upload-artifact@v4` target Node 20,
  which GitHub is retiring (the runner force-ran them on Node 24 and warned). Bumped the JS actions to the
  current majors on Node 24 — `checkout@v5`, `upload-artifact@v5`, `download-artifact@v5` (verified those
  majors exist and are the matched artifact generation). `Swatinem/rust-cache@v2` and
  `softprops/action-gh-release@v2` were **not** flagged, so left as-is; `action-gh-release` sits in the
  publish job the manual run skipped, so if it warns on the first *tagged* run we bump it then.

No source changed — workflow + docs only. Both workflow files re-validated as YAML, the matrix now lists
exactly the three buildable targets, and ROADMAP's target list is corrected (the DEVLOG entry that first
recorded `release.yml` still reads "x86_64 + aarch64" — that was true when written; this entry supersedes
it rather than rewriting history). **The `1.0.0` gate is unchanged: a real, fully-green run of these
workflows, ideally after the manual live-provider check.**

## 2026-07-14 — M5 review round 3 closed: a guard's own guard, and the ledger goes clean (12/12)

Round 3 reviewed the *product* changes (single MSRV, release profile, the manual release trigger, the
optional provider token, the README rewrite), re-verified the M5-R7 fail-closed fix **through the real
binary**, and opened three findings — **all now closed. M5's ledger is 12/12 and clean.** Build + `fmt`
+ `clippy` clean on both feature sets. Record: [`reviews/M5.md`](reviews/M5.md).

**M5-R10 — the guard the M5-R7 closure rests on guarded the wrong thing.** M5-R7 argued the token-overflow
error path is unreachable, on the strength of 32 tokens of headroom. But the compile-time invariant that
was supposed to *hold* that headroom asserted only `MAX_WINDOW_TOKENS < MODEL_MAX_TOKENS` — satisfied by
**any** headroom ≥ 1. The reviewer measured it: a 511-token window passes the assert and re-tokenizes to
~514, and the PERF-01 `Expand` error is back — reintroduced by a one-character edit that every CI-runnable
guard approves. Fix: make the **headroom** the invariant —
`MODEL_MAX_TOKENS - MAX_WINDOW_TOKENS >= MIN_DRIFT_HEADROOM_TOKENS` (16). The const subtraction underflows
on a window over the ceiling, so it subsumes the old assert rather than sitting beside it. A 510-token
window now **fails to compile** where it used to build green and overflow at runtime.

> **The lesson, promoted to ARCHITECTURE:** *a compile-time invariant must encode the constraint the code
> relies on, not the weakest one that happens to hold at today's values.* `A < B` is not "A leaves room
> for drift" — and when that invariant is the **only** guard a modelless CI can run, the gap between the
> two is the whole exposure. This is the M5-R2 lesson (*"a bound you do not check is not a bound"*)
> recurring one level up: the *check itself* was bounded by a constant nothing checked.

**M5-R11 / M5-R12 — the small ones, both "one home for a fact."** The release workflow's publish gate keyed
on the *ref* (`startsWith(github.ref, 'refs/tags/v')`), but GitHub's "Run workflow" picker lists tags too,
so a manual run *on a tag ref* would have cut a real release — the one thing the gate's comment promises
can't happen. Now gated on the **event** as well (`github.event_name == 'push' && …`). And the
`panic = "unwind"` rationale in `Cargo.toml` cited a **400** for the caught-panic path that is actually a
**500**; rephrased to "blocks the request (fail-closed)", the load-bearing part that can't go stale (the
profile's conclusion was always correct). Both are the *fourth and fifth* M5 items where a fact was right
in one file and stale in a second that repeated it.

Also, unrelated to the review: the README banner now spells **`llm-proxy-pii`** (was `llm-proxy`) — the
`-PII` glyphs appended in the same ANSI Shadow font, EN + IT in step.

## 2026-07-14 — M5 review round 2: my own fix committed the M4 retrospective's signature move

Round 2 verified round 1's six closures (five hold outright) and found three more. **All closed. M5's
ledger is 9/9.** 132 tests green (default) / 145 + 6 `#[ignore]`d (`--features onnx`); `fmt` + `clippy`
clean on both. Record: [`reviews/M5.md`](reviews/M5.md).

**M5-R7 — the fail-closed regression, and it was mine.** My M5-R2 fix enforced the token ceiling by
**clamping** an over-long NER sequence and returning `Ok(partial)` — reasoning that losing a window's tail
beats losing the whole field. The reasoning is fine. **Making the call there is not.** Under `NER_REQUIRED`
the detector goes into the composite *unwrapped*, so a `try_detect` `Err` is what produces the 400. Before
the fix an over-budget sequence errored → **400, nothing forwarded**. After it, the same condition returned
`Ok` → the request is **forwarded with a window's tail unscanned**, to an operator who had explicitly asked
to be blocked. Two claims in my own doc comment propped it up and neither survives: the overlap does **not**
re-cover the *last* window (it has no successor), and — the tell — I had written the 400 as the *bad*
outcome, when under `NER_REQUIRED` **the 400 is the product**.

> **The rule, now in ARCHITECTURE:** *a detector may degrade its own recall, but it may never decide **for
> the caller** that degraded output is acceptable.* Fail-open vs fail-closed is `FailOpen`'s decision and
> the only road to it is the `try_detect` error channel. The clamp was **right for the default posture and
> fatal to the other**, and a component that cannot see the posture must not choose between them. **When in
> doubt, a detector returns `Err` and lets the wrapper decide.**

The fix is the *minimum*, deliberately: `run_and_decode` returns `Err`, naming the cause. I skipped the
reviewer's "better" (re-split the window until it fits) because it buys a retry loop, a termination
argument and a coverage-gap argument to guard a path that **cannot currently be reached** — 32 tokens of
headroom against a measured +1…+3 drift. And the suggested test (make the consts injectable so the valve is
forceable) is *no longer needed*, which is the point: there is no valve. The overflow is now an ordinary
detector error, and "a required detector's error blocks / a `FailOpen`-wrapped one is swallowed" is already
pinned by FC-04 and CMP-02. **The fix didn't just remove the wrong behaviour — it removed the category of
behaviour that needed a bespoke guard.**

**M5-R8 / M5-R9 — the same drift, twice more.** ARCHITECTURE still named `MAX_SEQUENCE_TOKENS` (a constant
that no longer exists) and still called the window *"conservatively under `max_position_embeddings`"* — the
exact hope M5-R2 refuted — **in the file M5-R1 had just promoted to sole home**. Rewritten around what
ships, and the refuted framing is *kept as a warning* rather than deleted: **a bound you do not check is not
a bound.** Also written down at last: the chunker's unstated assumption that the `(0, 0)` offset sentinel
appears **only at sequence ends** (verified over 17 adversarial inputs; a tokenizer swap must re-check it).
M5-R9: the M5-R2 guard hand-copied `32` for `CHUNK_OVERLAP_TOKENS` — the one constant `chunk_char_ranges`
was made `pub` to *avoid* hand-copying. Now `pub` too. **A guard that shares only *some* of its subject's
constants is measuring a program that doesn't exist.**

## 2026-07-14 — Product pass: single MSRV, manual release trigger, max-opt profile, README rewrite

Four changes driven by how the product will actually be run and shipped.

- **One MSRV: `1.89`.** M5-R5 measured two floors (1.86 default, 1.89 `onnx`) and declared 1.86, keeping
  1.89 as documentation. But the product **always runs with `onnx` on** — that is what the hybrid *is* — so
  a "default-build MSRV" is a promise about a configuration nobody deploys: **a second number to keep
  honest, buying nothing**, which is precisely the drift shape this whole review round is about. Manifest
  and CI now carry the single real floor. **It paid for itself immediately:** raising `rust-version`
  un-gated an MSRV-aware clippy lint that `1.82` had been suppressing
  (`clippy::manual_is_multiple_of`, stable since 1.87) — on `luhn_valid`, the card checksum. Semantically
  identical, corpus tests unchanged, but a neat proof of the finding's own thesis: **an under-declared MSRV
  doesn't just fail to protect you, it hides the tooling that would.**
- **The release pipeline does not fire on every push to `main`** (it never did — it was tag-only). Added
  **`workflow_dispatch`**: a "Run workflow" button that builds all four targets from any branch and
  attaches them as **artifacts only**. The publish step is now **tag-gated** (`if: startsWith(github.ref,
  'refs/tags/v')`), so a manual run cannot accidentally cut a release.
- **`[profile.release]` is now the max-optimization one** — `opt-level = 3`, fat LTO, `codegen-units = 1`,
  symbols stripped (~5 min build; the masking path *is* the product's latency). **`panic` stays `unwind`,
  and that is a correctness bar, not a preference:** `abort` would turn a caught masking panic — which
  today blocks **one** request, fail-closed (M4-R19) — into a **process abort**. Converting a contained
  fail-closed into an outage is not an optimization; availability is a privacy property here. Verified the
  stripped LTO binary boots, answers `/healthz`, and still 404s an unproxied path.
- **README rewritten** (EN + IT) as a product README — problem, an ASCII flow diagram, a `curl` showing the
  *actual* masked body the provider receives, the detection matrix, the bars it holds itself to. **The
  internal development status is gone from it**: that belongs in ROADMAP, and the README now defers there.
  Also **`MANUAL_VERIFICATION.md` + `anthropic_smoke.rs`: the API key is optional.** The proxy already
  forwards a client `Authorization` verbatim in preference to `UPSTREAM_API_KEY`, so the runbook now leads
  with the mode where **the proxy never holds the credential at all** — and the smoke test drives that
  path, which is both the recommended posture and the stricter thing to prove.

## 2026-07-14 — M5 review round 1 closed: five of six findings were a *claim that had stopped being true*

All six M5 findings closed. **132 tests green (default) / 145 + 6 `#[ignore]`d (`--features onnx`);
`fmt` + `clippy` clean on both.** Full record: [`reviews/M5.md`](reviews/M5.md). No leak, no fail-open,
no over-mask regression — the reviewer reproduced the pre-fix `Expand` error from the parent commit and
drove the real binary end-to-end with NER on an oversized field before finding anything.

**Two findings were bigger than they were filed as, and for the same reason: the finding said "this
claim is unverified", and verifying it showed the claim was *false*.**

**M5-R5 — the declared MSRV was fiction (filed `low`; really the round's most load-bearing).** The
finding was "CI pins `stable`, so `rust-version` is never exercised". It offered two fixes — add the
job, or drop the claim. I measured instead, and **`1.82` cannot even parse the dependency tree**
(`idna_adapter` needs `edition2024`). The real floors:

| build | declared | true floor |
|---|---|---|
| default | 1.82 | **1.86** (`icu_*` / `idna_adapter`) |
| `--features onnx` | 1.82 | **1.89** (`redb` ← `hf-xet`) |

They **differ per feature set**, which one `rust-version` field cannot express: it now declares **1.86**
(the shipped, native-dep-free default build) and documents 1.89 for onnx, where cargo's own
`redb@3.1.3 requires rustc 1.89` is self-explanatory. A new `msrv` CI matrix **builds** both.

> **This is the M4-R22 lesson finally landing.** M4-R22 added `rust-version` *to prevent exactly this*,
> and it structurally cannot: the field makes cargo refuse a too-**old** toolchain; nothing makes the
> crate stay compatible with it. **A declared MSRV with no job building on it is not a floor — it is a
> comment shaped like a guarantee.** Only a job that builds on the MSRV can hold the MSRV.

**M5-R2 — the constant that bounded nothing.** `MAX_SEQUENCE_TOKENS = 480` claimed to bound the sequence
fed to the model. It bounded the *planning window*: `infer_chunked` **re-tokenizes** each window from its
own text (it must — a middle window needs its own `<s>…</s>` framing), which adds the specials and drifts
at the cut edges, so the real sequence was **always over** the "bound" — measured 481–483 against a
usable ceiling of 512 (XLM-R's 514 `max_position_embeddings` minus RoBERTa's position-id offset of 2).
29 tokens of headroom, held by nothing. Now: the constants are split (**`MAX_WINDOW_TOKENS`** vs
**`MODEL_MAX_TOKENS`**), the ceiling is **enforced** in `run_and_decode` (the single choke point every
path into the session goes through) with a clamp + kind-free `warn!`, their relationships are
**compile-time** invariants (`const _: () = assert!(…)` — get one wrong and the crate doesn't build), and
the drift is **asserted** by a live guard over six adversarial scripts (CJK, Cyrillic, zalgo, a
4 000-char run of `あ` with no spaces), which independently reproduced the reviewer's 481–483.

**M5-R3 — closed with the guard the codebase already had.** The chunk slice `&input[a..b]` hard-indexed
tokenizer offsets — the one spot on the masking path that could panic on attacker input, while
`decode_entities` (M2-R6), `Vault::mask` and `overlap::materialize` all refuse to. The fix is to *apply
the existing rule*, not invent a parallel one: `chunk_char_ranges` now widens every window through
**`overlap::widen_to_char_boundaries`** (promoted to `pub(crate)`), making the ranges sliceable **by
construction**; `infer_chunked` still uses `.get()` + `debug_assert!` + skip. Its unit test carries **its
own non-vacuity assertion** — it checks the offsets table really does cut a multi-byte char, because a
guard that quietly stops exercising its hazard is exactly how M4-R13 and M4-R24 stayed invisible.

**M5-R4 — the fixpoint's proof has a hole, and the *next* model is the one likely to fall in it.**
"A placeholder is inert" is proved **by construction** for the regex recognizers (`[KIND_N]` has no `@`,
no `sk-`, not enough digits). But `mask_all` runs the **composite**, and an ML model is under no such
constraint. If a model tagged `[PERSON_1]`, masking would never shrink the text, `MAX_MASK_PASSES` would
exhaust, and the request would **400** — fail-*closed*, never a leak, but a hard availability failure on
ordinary input. It holds for XLM-R (0 entities on placeholder-only text — now a live guard), so it is an
**empirical property of the chosen model**, not a theorem. Written down next to the invariant in
ARCHITECTURE *and* on `mask_all`, because the Backlog's designated successor is **GLiNER** — a zero-shot,
open-label, **context-driven** extractor, i.e. precisely the kind of model that would read
`Contact [PERSON_1] at [ORG_1]` and tag both. **M5 is also what made this reachable**: before chunking, a
field that large never reached the NER at all.

**M5-R1 / M5-R6 — the docs.** TESTING.md still asserted the NER was unchunked and quadratic — the claim
M5 both *disproved* and *fixed* — because that claim lived in **two** files and only ARCHITECTURE was
updated; it is now a **pointer**, not a duplicate. Both READMEs named, as a future trigger for `1.0.0`,
the very commit that contained the sentence; they now state the real gate (CI has never actually run; the
live-provider check has never been performed) and defer to ROADMAP.

> **The through-line.** Five of six findings are one shape: *a claim that was true when written, was
> never re-checked, and had quietly stopped being true.* None leaked. All were **documentation of a
> guarantee that had drifted from the guarantee.** The project's answer to that is already written down
> for review findings — **one home for a fact** — and this round is what applied it to **design claims**,
> which is where the drift actually lives.

## 2026-07-14 — M5: README + CI/release workflows — code-complete, one box left

Closed the two remaining M5 items that don't need a live provider.

- **README.md + README.it.md rewritten** from the "early development" placeholder to describe
  the shipped product: what it does (three-tier structured detection, optional NER, streaming,
  multi-provider), a quick-start, and a full env-var reference table for both the core proxy and
  the `onnx` feature.
- **`.github/workflows/ci.yml`** — `fmt` (once, feature-independent) + `clippy` + `cargo test`,
  matrixed one job for the default build and one for `--features onnx`, on push to `main` and on
  every PR. Model-dependent NER tests are `#[ignore]`d, so the `onnx` job needs no model file.
- **`.github/workflows/release.yml`** — on a `v*.*.*` tag, cross-compiles the full
  `--features onnx` product for Linux (x86_64-unknown-linux-gnu), macOS (x86_64 + aarch64
  -apple-darwin), and Windows (x86_64-pc-windows-msvc) and attaches the binaries to a GitHub
  Release.
- **Neither has been exercised live yet** — no PR has run the new CI, and no tag has been
  pushed. Both are standard, unremarkable GitHub Actions shapes, but "the YAML parses" is not
  "it's green"; that only gets proven the first time a real push/PR/tag runs them.

**M5 is now code-complete except one box that cannot be closed from inside a session:** the
manual dual-run verification (`docs/MANUAL_VERIFICATION.md`, E2E-INT-02) needs a human with a
real `ANTHROPIC_API_KEY` to actually run it and read the trace output — see the entry below for
what's already in place around it. The first tagged release (`1.0.0`) should wait for a real
green CI run at minimum, and ideally that manual check having been run once.

## 2026-07-14 — M5: integration tests, a performance harness, and a real NER bug found via testing

Picked up M5 (integration & performance testing) end to end. Four threads, in the order they
landed:

**1. Real integration tests (`tests/proxy_e2e.rs`).** Implemented the two cataloged-but-missing
old-proxy scenarios — **E2E-02** (PII in a CSV `tool_result`) and **E2E-04** (all six structured
categories in a `SELECT … FROM DUAL`-style result) — against a mock upstream, both asserting
the masked-upstream / restored-to-client pair like the existing E2E-01/03. Added three more: a
**full-HTTP tool-call round-trip** (the real-server companion to the pipeline-level INT-03: a
mock's `tool_calls[].function.arguments` referencing a placeholder is de-anonymized before the
client sees it), a **multi-turn determinism** e2e (two real HTTP round-trips resending
conversation history, proving a repeated value keeps its placeholder token across two
independent per-request vaults — the stateless-client shape this proxy actually serves), and
extended the provider-agnostic test (LOC-11) to compare **all three** mock upstream shapes M5
asks for (OpenAI / Copilot / Anthropic), not just two. Also added `tests/anthropic_smoke.rs`
(E2E-INT-01): a real-provider smoke test against Anthropic's OpenAI-compat endpoint —
`#[ignore]`d, gated on a real `ANTHROPIC_API_KEY`, never run in CI. **Written and compiling, not
yet run against a live key** (no credentials in this environment) — the same posture the project
already uses for `ner_eval.rs`.

**2. The manual dual-run runbook (`docs/MANUAL_VERIFICATION.md`, E2E-INT-02).** A step-by-step
procedure for the check that can't be a `#[test]`: run the same PII prompt twice against a real
provider, once with `PII_DEBUG_SKIP_DEMASK=1` (proves the request left masked) and once normal
(proves the client gets the restored value), with `RUST_LOG=…=trace` so DBG-02 (never-log-raw-PII)
gets re-checked on real data. Written, not yet executed — same reason as above.

**3. A performance/load harness (`tests/perf.rs`).** The system-level companion to
`tests/complexity.rs` (which pins the masking *algorithm's* complexity, no HTTP):
`healthz_stays_responsive_under_concurrent_masking_load` fires 8 concurrent ~350 KB / 50 K-entity
requests and polls `/healthz` while they're in flight, turning the M4-R19 "masking runs on
`spawn_blocking` so the executor never starves" architecture claim (previously only
hand-measured) into a repeatable guard — measured **~40 ms** for `/healthz` under load (budget
2 s). `streaming_throughput_of_repeated_placeholder_restoration_stays_within_budget` streams
~150 KB containing ~6000 placeholder occurrences through the real SSE de-anonymizer in small
fragments — measured **~0.75 s** (budget 20 s). Both are generous wall-clock budgets in the
`tests/complexity.rs` style: they catch a regression back to seconds-to-minutes, not a
micro-benchmark.

**4. NER field-size measurement — and a real, live bug (`tests/ner_perf.rs`, `src/pii/onnx.rs`).**
The ROADMAP's own suspicion was that `OnnxNerDetector` feeding a field to the model as one
sequence would be *slow* on large fields (quadratic self-attention). Measuring it found
something worse: past the model's `max_position_embeddings` (514 for the picked XLM-R int8), the
ONNX graph's position-embedding lookup goes **out of range**, and the run call fails outright
(`Non-zero status code returned while running Expand node … invalid expand shape`) — not a
slowdown, a hard error, on any field over roughly 500 tokens (~2 KB of prose). Fails open by
default (silently drops to structured-only for that request) but is a **hard 400 block** under
`NER_REQUIRED` — an availability gap in the same family as M4-R19/R24, though opt-in and off by
default so never the unauthenticated DoS those were.

**Fix: overlapping-window chunking.** `OnnxNerDetector::infer` now tokenizes once; if the
sequence fits the budget it runs exactly as before (M2, unchanged), otherwise `infer_chunked`
splits it into overlapping token windows (`MAX_SEQUENCE_TOKENS = 480`, `CHUNK_OVERLAP_TOKENS =
32`), each **re-tokenized independently** — a middle window needs its own `<s>…</s>` framing, so
it can't be a raw slice of the whole field's token ids — run through the same single-window
path, and merged with exact duplicates from the overlap deduped.

**And chunking had its own bug, caught only by testing at a size that exercised the last
window.** The first version computed a window's char end from `offsets[token_end - 1].1`. That's
correct *unless* `token_end == seq`, in which case `token_end - 1` is the closing `</s>` token —
whose offset is the sentinel `(0, 0)`, not the real text end. The bug collapsed the **entire
final window** to a zero-length slice: measured on a 64-sentence input, this silently dropped 61
of 192 entities (32%) with **no error at all** — worse than the original bug in one way, because
it degrades silently instead of failing loudly. Caught by `tests/ner_perf.rs` at reps=64 (the
first size in the sweep that needed two windows) — reps=16 had (correctly) only ever needed one.
Fixed: a window reaching the sequence end uses `input.len()` for its char end. Extracted the
window math into a pure `chunk_char_ranges` function (offsets + lengths in, char ranges out — no
tokenizer or model needed) specifically so this exact bug gets a **real unit test**
(`the_last_window_reaches_the_true_text_end_not_the_closing_token_sentinel`,
`src/pii/onnx.rs::chunk_tests`) rather than living only in an `#[ignore]`d, model-dependent check.

**Measured after the fix:** linear scaling and full recall — 64/256/1024× a repeated sentence run
in 448 ms / 2.07 s / 7.53 s (debug profile), 192/192, 771/768, and 3084/3072 entities found (the
small excess above 100% at larger sizes is an occasional un-deduped near-boundary double-detection
— a precision nit, not a recall loss). Re-measured M4-R21's ~2× fixpoint-confirmation cost live
too: ~1.8–3× on a short field, consistent with the 64 ms → 127 ms (1.99×) recorded when M4-R21 was
closed — confirmed as the deliberate correctness cost it always was, not a regression.

**132 tests green (default) / 144 + 4 `#[ignore]`d (`--features onnx`), no warnings; `fmt` +
`clippy` clean on both feature sets.**

## 2026-07-14 — Review 11: M4-R24 closure verified — DoS class closed on both axes

Independent reviewer pass over the builder's M4-R24 fix (`eed9949`). **Holds.** Read the rewritten
`Vault::mask` (one L→R copy into a capacity-reserved buffer, O(n + k)) and reproduced both splice strategies
in a standalone probe: old `replace_range` R→L is **×4 per doubling** of entity count (O(n²), 18 s at 400 K —
past DOS-04's budget), new forward copy is **×2** (linear), and the two produce **byte-identical** output, so
it is a pure speedup. Confirmed no third quadratic hides behind it (`demask` = linear `replace_all`,
`placeholder_for` = O(1)), DOS-04 is non-vacuous (`entities.len() == reps`, 600 K entities → 214 ms), the
malformed-span guard is drop-never-leak-never-panic, and MSRV `1.82` is untouched. Both feature sets green
(126 default / 134 + 1 ignored onnx), `fmt` + `clippy` clean. No new finding — **M4's ledger is genuinely
clean, 24/24**. Full round: [`reviews/M4.md#review-11`](reviews/M4.md#review-11).

## 2026-07-14 — M4-R24: the *other* quadratic — ask what a guard holds constant, not what it varies

**M4 is done — all 24 findings closed, and M5 is unblocked.** (This supersedes the "M4 is NOT done" line in
the entry below.)

**The bug.** `Vault::mask` spliced placeholders in **right-to-left** with `String::replace_range`. Correct,
and quadratic: every splice memmoves the whole tail of the string, so *k* entities in *n* bytes shift Θ(n·k)
bytes — and a field of many **small** values (`a@b.co `, an SSN, a phone) has *k* growing with *n*, so it is
**Θ(n²)**. A 13.4 MiB body of repeated emails — under the 16 MiB limit, on the **unauthenticated** masking
path — burned **~7 minutes** of CPU. The fix is one **left-to-right copy** into a fresh, capacity-reserved
buffer: O(n + k), each byte touched once. Placeholder numbering is untouched, because it always followed the
entities in *start order*; splice **direction** never determined it. Measured (debug, splice isolated):
800 K entities go from **91,049 ms → 272 ms**, and ×3.9-per-doubling becomes ×2.0 — **335×**, and linear.
Over HTTP, the reviewer's own payload: **13.4 MiB → 1.8 s, 200 OK**; eight concurrently in 4.3 s with
`/healthz` answering in 48 ms.

**Why it survived M4-R19, which was the *same* bug.** Because the masking path has **two** size axes, and
we had only closed one. M4-R19 made detection linear in the **field size**; this one is quadratic in the
**entity count**. They are independent, and *closing one says nothing about the other* — linear detection
does not bound the splice, which is why the M4-R19 pass sailed straight past it.

**And the guards could not see it — that is the lesson.** DOS-01…03 scale the field to megabytes, and every
single one of them pins the entity count at **one** (DOS-03's card row coalesces to k≈1). So they varied *n*
and silently held *k* constant, and a per-entity quadratic lived directly underneath them. The smoking gun is
exact: 13.4 MiB as **one** email masks in 219 ms; the **same** 13.4 MiB as many small emails took **421 s**.
Identical bytes — the only variable is *k*.

> **A quantity a test never varies is a quantity the test cannot see.** This is the M4-R13 lesson — *"a
> corpus has a shape, and that shape is a blind spot"* — arriving a second time, now on the **guards
> themselves**. A guard is a corpus too, and it has a shape. Ask what it holds *constant*, not only what it
> varies.

**DOS-04** is the guard that varies *k*: 600 K entities, timing `Vault::mask` **alone** (the code under test
— that makes it decisive, ~0.2 s linear vs ~52 s quadratic, where an end-to-end debug timing would have left
a flaky 2× margin). It asserts `entities.len() == reps`, so if the corpus ever coalesces back to k≈1 the
guard *says so* rather than quietly going blind again. **Non-vacuity, and it reproduces the blind spot
exactly: on the pre-fix splice DOS-04 times out while DOS-01/02/03 all pass.**

**One hardening the finding didn't ask for.** The new loop slices `text[cursor..start]`, so a malformed span
(overlapping / out of bounds / off a `char` boundary) would **panic** — and this is a proxy on
attacker-influenced input. The precondition (guaranteed by `resolve_overlaps`, the only production caller) is
now stated, `debug_assert!`ed, and in release handled by advancing the cursor **past** the bad span, widened
to a `char` boundary: **drop, never leak**, never a panic.

**One thing found while closing it, left open on purpose.** `OnnxNerDetector` feeds the **whole field as one
sequence** — no chunking — so the NER path is quadratic in field size from self-attention. Opt-in and off by
default, so it is *not* the unauthenticated DoS these two were, and not a leak. But it is the third
appearance of the same lesson, so the docs now say "the **structured** path is linear" rather than "the path
is linear", and PERF-01 must measure it before NER is recommended for large bodies.

**126 tests green (default) / 134 + 1 `#[ignore]`d (`--features onnx`); `fmt` + `clippy` clean on both.**

## 2026-07-13 — Review 10: the DoS pass verified, and a *second* O(n²) found (M4-R24, reopens M4)

**M4 is NOT done — [M4-R24](reviews/M4.md#m4-r24) (BLOCKER) is open; M5 is blocked again.** (Supersedes the
"M4 is done" line in the entry below, which was true only of M4-R19…R23.)

Independent verification of M4-R19…R23. **They hold:** M4-R19 (the candidate *rescan* quadratic) is
genuinely closed — the two unbounded shapes are linear by measurement (email/secret ~15–30 ms at 1–2 MB),
DOS-01…03 are non-vacuous, and the `Scan::Sequential` + fixpoint argument is sound. M4-R20 fails closed and
is non-vacuous; **the R19↔R20 worry that Sequential-scanned chains could exhaust the passes into a 400 does
not happen** — every chain converges in ≤ 2 passes. R21/R22/R23 all hold (MSRV `1.82` is the true floor; no
stale ROADMAP pointers remain).

**But the DoS class was not fully closed.** M4-R19 fixed the quadratic in *candidate generation*; masking
itself — `Vault::mask`'s right-to-left `replace_range` splice (`anonymizer.rs:138-143`) — is **still
O(n²)**, now in the **entity count**: each splice shifts the string tail, so *k* entities in an *n*-byte
field cost Θ(n·k). Measured through the real `mask_all`: a 13 MiB `content` field of small emails ≈ **7
minutes** of CPU, while the *same* 13 MiB as **one** email masks in **0.2 s** — that gap is why DOS-01…03,
which each pin a single entity, never saw it. Same unauthenticated path as M4-R19; `spawn_blocking` keeps
the async executor alive but the shared blocking pool still saturates. Fix: single-pass splice (O(n)) +
**DOS-04** (a many-entity guard). Full record: [`reviews/M4.md`](reviews/M4.md#m4-r24).

## 2026-07-13 — M4-R19/R20/R22/R23: a safety fix has a cost, and the cost is part of the fix

**M4-R19…R23 closed** *(but see Review 10 above — M4-R24 later reopened M4).*

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
