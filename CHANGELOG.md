# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

> **Versions track milestones, not the SemVer contract — stated plainly rather than claimed and
> then broken.** A tag is cut from the milestone it completes ([`docs/ROADMAP.md`](docs/ROADMAP.md)
> maps every one), and the number reflects how far that milestone moved the product, judged by the
> maintainer. `1.2.1` is the honest example: under strict [SemVer](https://semver.org/) a
> default-on capability would be a minor and a configuration variable that changed meaning would be
> a major, while M10 is read here as broadening a tier that has shipped since M8.1. **So do not
> infer risk from the number** — the `### Changed` section of each entry is where a change that can
> surprise you is named, and it is the part to read before upgrading.

> **This file is what the GitHub release page shows.** `release-build-publish.yml` extracts the
> section matching the tag and publishes it as the release body, with GitHub's auto-generated
> commit list appended below it — and **refuses to publish a tag this file does not describe**,
> the same way it already refuses one the ROADMAP or `Cargo.toml` does not name. A release whose
> changes nobody wrote down does not happen.
>
> Entries answer *"what changed for someone running this?"* — the reasoning behind each change
> lives in [`docs/DEVLOG.md`](docs/DEVLOG.md), and the milestone it came from in
> [`docs/ROADMAP.md`](docs/ROADMAP.md).

> **While a version is still being built, entries accumulate under `## [Unreleased]`, renamed to
> `## [<version>] — <date>` when the tag is cut.** Writing the section at release time from memory
> is how the detail that mattered gets lost; this file is meant to be written as the work lands.
> Forgetting the rename is **safe**: the guard matches the heading `## [<version>]` by *literal
> prefix*, so an `Unreleased` section satisfies no tag and the release is **refused** — never
> published wrong. **What is not safe is `## [1.3.0] — unreleased`**: that heading *does* match the
> prefix, so the guard would extract it and publish a section labelled unreleased as the release
> body. The placeholder goes in the version slot, never beside it.

## [Unreleased]

Milestone [M11](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m11) — in progress.

### Added

- **VAT numbers are masked, with a new placeholder — `[TAXID_1]`.** The Italian **Partita IVA** in
  both its bare 11-digit form (`P.IVA 00159560366`) and its VIES form, plus VIES-form 🇩🇪 🇬🇧 🇳🇱 🇵🇹
  numbers (`DE136695976`, `GB220430231`, `NL111222333B01`, `PT524287244`). Each is accepted only
  when that country's VAT checksum passes.
  - **It is a new token, not a reused `[NATID_1]`, and that matters if you consume placeholders.**
    A VAT number identifies a *business* and is personal data only when that business is a sole
    trader; a Codice Fiscale identifies a person. Anything downstream that pattern-matches
    placeholder labels will see `TAXID` for the first time.
  - **Always on**, like the national IDs it joins — `PII_LOCALES` does not gate it, and there is
    no new variable to set.
  - **One cost, stated plainly: a bare 11-digit Partita IVA is a mod-10 check, so about 1 in 10
    arbitrary 11-digit numbers is now masked** — a long order reference or id can come back as
    `[TAXID_1]`. It is restored byte-identically on the response path, so the round trip is exact;
    what changes is that the model sees a placeholder where it might have wanted the number. The
    prefixed forms (`DE…`/`GB…`/`NL…`/`PT…`) have no such cost, since they need the literal country
    code as well as the checksum. This is the same trade already accepted for the 9- and 11-digit
    national IDs.
  - 🇪🇸 🇫🇷 🇱🇻 VAT numbers are **not** recognised. Their checksums are not measured here, and an
    unmeasured recogniser does not ship — the rule that decided the nine phone regions.
  - Phone numbers are unaffected: a compact domestic number that also satisfies the VAT checksum
    stays `[PHONE_1]`, because a numbering-plan lookup is better evidence than a mod-10 check.

### Changed

- **The NER's default thread count halves on every SMT machine — no config change needed, and this
  is the one to read before upgrading.** `NER_INTRA_THREADS` is derived, and the number it divides
  moved from *logical threads* to **physical cores**: `max(1, base / NER_POOL_SIZE)` where
  `base = min(physical cores, available parallelism)`. On a 6-core / 12-thread box the default goes
  from 12 threads to 6, and a `NER_POOL_SIZE=2` deployment from 6 per session to 3. The reason is
  that both SMT siblings of a core were running the same int8 GEMM, contending for one core's cache
  and one set of vector units — work that does not benefit from SMT the way the runtime's own
  latency-bound work (tokio, TLS, JSON) does.
  - **Measured on the reference box:** single-request latency is a wash at the default (within the
    harness's noise), and **throughput improved** — +16% at the default shape, +19% at
    `NER_POOL_SIZE=2`, which became the fastest shape measured.
  - **The old shape is one variable away.** `NER_INTRA_THREADS` is an explicit override and still
    wins over the derivation; set it to your logical core count for pre-1.3.0 behaviour.
  - **The startup log now prints the arithmetic**: `thread_base`, `thread_base_source`
    (`physical` / `parallelism-cap` / `physical-unknown`), `logical_cores` and `physical_cores`
    alongside `pool_size` and `intra_threads`, so the derived value can be reproduced from the line
    that reports it.
  - Containers and pinned processes are safe: the base is a `min`, so a proxy granted 2 CPUs on a
    32-core host still derives from 2. Where a platform will not report physical cores, the base
    stays the logical count — exactly the previous behaviour.
  - `GLINER_INTRA_THREADS` derives from the same base, the same way.

### Fixed

- **A lower- or mixed-case IBAN or VAT number was forwarded in clear. It is now masked.** This is a
  leak, and one half of it has been present since 1.0.0. The recognizers spelled their letters
  uppercase-only, so `it00905811006`, `de136695976`, `nl111222333b01`,
  `it60x0542811101000000123456` — and, sharpest, `IT60x0542811101000000123456`, an otherwise
  canonical IBAN with **one** lowercase letter — matched nothing at all and reached the provider
  byte for byte. Seven of thirteen renderings of real published values were affected. The Codice
  Fiscale, ES DNI/NIE and CN resident-id recognizers always folded case, so the behaviour was also
  inconsistent between tiers.
  - **The five VAT prefixes and the Dutch literal `B` now fold case.** Measured cost over 341.1 MB
    of third-party source: **no additional matches** for any scheme, and each is checksum-gated
    besides. Nothing to configure.
  - **IBAN folds too, but only for a rendering it can verify — and this asymmetry is worth
    knowing.** A canonical all-uppercase IBAN behaves exactly as before: structurally valid is
    masked even when mod-97 fails. A rendering carrying **any** lowercase letter is masked only if
    it passes mod-97 *and* its country's ISO 13616 length. The reason is measured, not cautious:
    folding without that gate would mask **931 additional spans** on the same corpus — hex digests,
    base64 blobs, `Ed25519PublicKey` strings — and masking one of those inside a `tool_use.input`
    is a real functional harm. So `de89370400440532013000` is masked; a lowercase IBAN-shaped
    string that verifies as nothing almost never is. **Almost**, and the honest number is worth
    having: a country code the ISO 13616 length table does not know is checked by mod-97 alone, so
    roughly one such string in 97 is still masked — measured at **1 in 936** over 304.9 MB of
    third-party source. It is masked, not leaked, and restored byte-identically on the way back.
- **A phone number written the way every API stores it — `+393471234567` — is now masked.** The
  `+CC` form was detected only when it was written with spaces (`+39 347 1234567`); the compact
  **E.164** rendering, which is what an address book, a CRM export or a JSON payload contains,
  matched nothing and went to the provider **in clear**. Measured over 13 000 numbers confirmed
  valid: 0.918 of the E.164 renderings were missed. Some were worse than missed —
  `+55 11 91234 5678` came back as `[PHONE_1] 5678`, four digits of a real mobile forwarded. The
  compact form is now checked against the country's real numbering plan, which the country code in
  the value itself makes possible: measured over 358.4 MB of third-party source, it adds **83
  masked spans, all 83 real phone numbers**.
- **Credit cards written the way the issuer prints them are now masked.** Only two renderings were
  detected — compact, and four groups of four — so an American Express number in its own 4-6-5
  grouping (`3782 822463 10005`) and a Diners 4-6-4 went to the provider **in clear**, while the
  same digits compact were masked. Worse, a 19-digit card written in groups had its first sixteen
  digits masked and **the last three forwarded**. Both are fixed: the groupings issuers publish are
  detected, and a card followed by a short token — its CVV, a row index, a spreadsheet column — is
  masked whole rather than dropped.
- **A 30-byte request could return `500` instead of being processed — on every release ever cut.**
  A value whose digits are written in a non-ASCII script (`Account AB𝟎𝟏ABCDEFGHIJK`) made the IBAN
  checksum slice a multi-byte character in half and panic. The proxy is **fail-closed**, so nothing
  was ever forwarded: the request was refused, not leaked. But it is a refusal an unauthenticated
  caller could trigger with one short string, and it is fixed.
- **Values whose groups are separated by a non-ASCII space are now masked.** An IBAN, card, phone
  number or UK NINO written with a no-break space, a narrow no-break space, a figure space, an
  ideographic space or a tab between its groups — which is what a word processor, a web page or a
  TSV tool result produces — matched nothing at all and was forwarded **in clear**, while an email
  address in the same sentence was masked. Every pattern spelled its separator as a plain ASCII
  space; every checksum behind it already accepted the others. Widening cost **no additional
  matches** over 341.7 MB of third-party source, so there is nothing to trade and nothing to
  configure.
- **A real IBAN could be dropped entirely when followed by a short word.** Found and fixed inside
  this same release, so no shipped version carries it — but it is the shape to know. An IBAN whose
  compact length is a multiple of 4 (🇦🇩 🇦🇹 🇧🇪 🇨🇾 🇨🇿 🇪🇪 🇪🇸 🇭🇺 🇱🇹 🇱🇺 🇵🇱 🇷🇴 🇸🇪 🇸🇰) is
  written in groups that leave the pattern's optional trailing group unused, so the match ran on
  into the next 1–4-character token. `Please wire to ES91 2100 0418 4502 0005 1332 for the invoice`
  produced no IBAN candidate at all, and the provider saw the country code, both check digits and
  the final group in clear. A rejected span is now retried one separator shorter instead of being
  discarded.

## [1.2.1] — 2026-07-31

Milestone [M10](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m10) — national phone coverage + release hygiene.

### Added

- **Domestic phone numbers for nine regions, detected by default** — 🇩🇪 🇪🇸 🇫🇷 🇬🇧 🇮🇹 🇱🇻 🇳🇱 🇵🇹 🇨🇳
  numbers written with no `+CC` (`020 7946 0958`, `347 1234567`, `91 123 45 67`). Each candidate is
  checked against that country's **real numbering plan**, not just a digit pattern: measured recall
  **1.000** over 35 real renderings, zero false positives across 453 digit-shaped non-phones, and
  **zero `Phone` spans** on a real 22 KiB Claude Code turn.
- **`--version` and `--help`** — a downloaded binary can now say what it is and list every
  environment variable it honors, without the repository.

### Changed

- **`PII_LOCALES` now narrows rather than enables — read this one if you set it.** All nine regions
  are on out of the box, and the variable **replaces** that set rather than adding to it. So a
  config carried over from 1.2.0 that reads `PII_LOCALES=it` now yields **less** domestic-phone
  coverage than leaving the variable unset entirely. Setting it *empty* switches the tier off,
  which is a different thing again from leaving it unset. National IDs remain always-on regardless
  of what you set.
- **Logs carry local time with an explicit offset**, and the default level is `info`, so a freshly
  downloaded binary reports what it is doing with no configuration.
- **The project is dual-licensed** — AGPL-3.0-or-later, plus a separate commercial license for
  cases the AGPL does not cover for you. Running it unmodified carries no obligation, for anyone.

### Fixed

- **A partial leak.** A real domestic number could be left **partly in clear** when a candidate was
  rejected and its remainder was not re-examined. Now a rejected match is retried one group shorter.
- **A CPU exhaustion reachable by a legal request body.** Per-candidate phone validation could hold
  a worker for ~57 s on a digit-dense 15.6 MiB body. Validation now draws on an allowance scoped to
  the **whole request** — so splitting the same content across more fields buys no extra CPU — and
  exhausting it **fails closed with a 400**, never a partial scan reported as clean.
- **The refusal is actionable by the client that meets it.** An agent retrying an identical request
  would wedge, so the 400 names the cause and the remedy (send less digit-dense text — a `LIMIT` on
  the query behind an oversized tool result, fewer rows per call) and carries **no input-derived
  bytes**.

### Notes for operators

The allowance counts *phone numbers*, not bytes: roughly **62,500** per request at the common
grouped rendering, ~500,000 at the cheapest. Which byte size that lands on depends on the payload's
layout — the same 62,500 numbers are refused at 793 KB as a bare column, 2 MB as `name,phone`,
4.4 MB as a six-column export. An ordinary 5,000-row export spends **8%**; a typical chat turn
spends none.

**Packaging, corrected rather than newly true:** the default build has always used the platform's own
TLS, so a **Linux** binary links the system OpenSSL — self-contained on any ordinary distribution,
but not in a `scratch`/distroless image. The project described that build as *native-dependency-free*,
which held only on Windows; the dependency guard now checks every released target instead of the one
it happens to run on, and the documents say what is true on all of them. No dependency changed.

Verified before the tag rather than after: 61 review findings across nine independent rounds all
closed, 221 default / 254 `onnx` tests green, and the manual [CC
battery](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/TESTING.md#cc-battery) run against real Anthropic through the proxy — four scenarios
in both postures, zero leaks, zero fail-closed 400s.

## [1.2.0] — 2026-07-20

Milestone [M9.1](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m91) — **one release binary per backend.** A single ONNX Runtime
distribution carries a single set of execution providers, so the accelerator choice moved to *which
artifact you download*: standard (DirectML on Windows, CoreML on macOS), `-cuda`, or `-webgpu`.

## [1.1.0] — 2026-07-19

Milestone [M8.1](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m81) — the national phone recognizer, **opt-in** at this version
(M10 turned it on by default), plus GLiNER as an optional second, additive ML engine.

## [1.0.0] — 2026-07-18

Milestone [M7.1](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m71) — the first tagged release. Native Anthropic `/v1/messages`
support (M6), the NER latency work (M7), the system-prompt detection cache and the fixpoint fix
that together removed the fail-closed 400 a real Claude Code session used to hit.

## [0.4.0] — 2026-07-15

Milestone [M5](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m5) — interim badge release: integration and performance testing.

[1.2.1]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v0.4.0...v1.0.0
[0.4.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/tag/v0.4.0
