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

## [1.3.0] — 2026-09-06

Milestone [M11](https://github.com/francesco-stimola/llm-proxy-pii-rust/blob/main/docs/ROADMAP.md#m11) — complete.

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

- **Digit-dense text is masked more often — IP addresses and aligned numeric columns now come back
  as `[PHONE_n]`. This is the second one to read before upgrading.** Closing the separator leaks
  below meant accepting `.` `/` `(` `)` between a phone number's groups and separator **runs** of up
  to four characters, and both are how ordinary command output is written. So a dotted-decimal IPv4
  address, a `dd.mm.yyyy` or `dd/mm/yyyy` date, and a `psql` / `df` / `ls -l` column aligned with two
  to four spaces are now candidates — and a fair share of them are a real assigned number in one of
  the nine enabled plans. `170.75.154.131` is `17075154131`, a valid Chinese mobile.
  - **The rates are measured, and they live in one place because that is the only place a guard
    keeps them true:** `docs/ARCHITECTURE.md` → *Domestic phone coverage*, where the per-category
    matrix is printed by `tests/phone_eval.rs` and asserted against that file on every test run.
    Read it before enabling this on command-output traffic — the aligned-column and IPv4 rates are
    the two largest in the table, and `cn` is the single biggest contributor to them.
  - **Nothing leaks, and the round trip is exact.** Every masked value is restored byte-identically
    on the response path, so the client sees what it sent. The cost is that the **model** sees
    `[PHONE_1]` where it needed the value — an IP address inside an `ssh` argument, a row of a query
    result inside a `tool_result`. If your traffic is command output and your agent acts on the
    numbers in it, that is a functional cost worth weighing.
  - **The trade, in one line:** the alternative was leaving `(020) 7946 0958` and `+39  347
    1234567` reaching the provider **in clear** — measured at 0.923 and 1.000 of their renderings.
    An over-mask is recoverable; a miss is not.
  - **Two ways to bound it.** `PII_LOCALES` narrows the enabled plans (the `cn` plan is the largest
    single contributor to the IP rates), and setting it **empty** switches the domestic tier off
    entirely. Neither affects `+CC` numbers, the always-on national IDs, or any other tier.

- **A body dense in lowercase alphanumeric groups can now be refused where it used to be masked, and
  the published CPU ceiling has been rewritten upward.** The request's validation allowance
  (500,000 units, not configurable) used to charge only the phone validator. It missed a second
  expensive check entirely: the IBAN case gate, whose mod-97 arithmetic runs once per candidate *and
  once per retry* — on one 4 MiB field it ran hundreds of thousands of times and was charged
  nothing. It is charged now.
  - **What changes for you: a large hex dump can now come back `400`.** `xxd`, `od -x` and every
    debugger emit uniform hex per group, and about one group in eighteen reads as two letters
    followed by two digits — which is what an IBAN starts with. Measured, an `xxd` body is masked
    and forwarded up to about **11 MiB** and refused above it, where before it went through at the
    full 16 MiB body limit. Fail-closed, as always: nothing is sent to the provider, and the
    refusal now says which tier spent the allowance instead of blaming the phone one. Ordinary
    prose, JSON, base64 and source code are nowhere near the line, and a 22 KiB Claude Code turn
    spends nothing at all.
  - **The honest reading is that the line was never where it looked.** With this work uncounted, a
    16 MiB hex dump already sat at **98.6 %** of the allowance — so a third of its validation cost
    was simply not being measured. Charging it makes the bound true; it does not make the body
    slower.
  - **The ceiling itself was understated and the correction is upward.** It was published as *"about
    3 s"*, read off the cheapest corner of a grid that never varied the candidate's shape. One unit
    turns out to span more than an order of magnitude depending on what the candidate looks like,
    and the dearest shape is *legal* traffic (a zero-padded three-group key column), not an attack.
    The old figure is withdrawn rather than adjusted. The measured band, and the harness that
    re-derives it, are in `docs/ARCHITECTURE.md` → *What the budget costs*.
  - **The bound stays a count, not a time limit**, deliberately: a wall-clock limit would make the
    same body pass on an idle machine and fail on a busy one, and a fail-closed layer whose verdict
    depends on load is not one an operator can reason about.

- **A malformed `UPSTREAM_BASE_URL` now refuses the startup instead of failing every request.** It
  was the one configuration value read with no validation at all, sitting between two
  (`LISTEN_ADDR`, `MAX_BODY_BYTES`) that have always refused a bad value at startup. So
  `api.openai.com` with the scheme forgotten, an exported-but-blank value, or `htp://…` bound the
  listener, logged `listening on`, accepted a client's PII and only then failed per request inside
  the HTTP client. **What changes for you: those spellings now exit non-zero with
  `invalid UPSTREAM_BASE_URL: …` and never bind a port.** The rule is an `http`/`https` URL with a
  host; the value is otherwise used exactly as written, never rewritten or normalised — so a base
  URL that worked keeps working, including the one-slash `https:/host` spelling the URL standard
  accepts.

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
- **Values separated the way people actually write them are now masked.** A phone number, IBAN,
  card or UK NINO written with a **no-break space**, a narrow no-break space, a figure space, an
  ideographic space or a tab went upstream in clear; so did a `+CC` number written with `-`, `.`,
  `/` or parentheses — `tel:+1-201-555-0123` is RFC 3966's own example, and `+49 (0)30 12345678` is
  how a German number is printed. So did **any** of them written with more than one separator
  character between groups: `+39  347  1234567` with two spaces, or `030 / 12345678`. Each of those
  renderings was masked in some other form, so the value was recognised — what was not recognised
  was how it was written.
  - **What it costs is a first-class entry under *Changed* above**, not a footnote here: some
    things that are not phone numbers are now masked too.
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

### Security

- **`h2` is updated to 0.4.19, closing RUSTSEC-2026-0258 — unbounded memory use, or a panic, from a
  stream of empty HTTP/2 DATA frames.** The advisory is against `h2` 0.4.15, which reaches this
  binary through `reqwest`/`hyper` on the **upstream** connection, and the fix landed in 0.4.16;
  the lockfile now carries 0.4.19. **The listener you expose is not the exposed side:** `axum`'s
  default features are HTTP/1 only, so `axum::serve` refuses an HTTP/2 preface and a client cannot
  reach `h2` at all — `reqwest`'s `http2` feature, which this project sets deliberately, is what
  puts it in the graph. Frames from the *provider* were the reachable path. It is the only
  vulnerability in the dependency graph — all 427 locked crates were scanned; the other two
  findings are `paste` and `atomic-polyfill`, both *unmaintained* rather than vulnerable, and
  `deny.toml` gates unmaintained crates on this project's own direct dependencies only.
- **A `user:password@` in `UPSTREAM_BASE_URL` no longer appears in the startup log line.** A URL
  may carry credentials, and the proxy accepts and forwards them — but it also logs its whole
  configuration when it starts, and that line printed the base URL whole, immediately above the
  `upstream_api_key: Some("<redacted>")` that exists so a secret never reaches a log. **If you
  configure credentials that way, they were written to your logs by every release ever cut.** The
  startup line now reads `https://<redacted>@api.openai.com/` — marked rather than dropped, so you
  can still see that a credential is set and where the proxy points. **Nothing else changes:** the
  value sent upstream is untouched, and a base URL without credentials is still printed exactly as
  you configured it. The redaction covers the URL's credentials, not a secret written into its
  query string.
- **`UPSTREAM_BASE_URL` is validated, which is also what the two Critical CodeQL
  *server-side request forgery* alerts on `src/proxy.rs` were pointing at.** There is no
  exploitable SSRF: no request data ever reaches that URL — it is the process environment plus a
  path resolved once at startup — but the value did arrive from `env::var` with nothing between it
  and the outbound request, and an unvalidated environment source is what the query flags. The
  refusal it now performs is described under *Changed*.

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

[1.3.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.2.1...v1.3.0
[1.2.1]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/compare/v0.4.0...v1.0.0
[0.4.0]: https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/tag/v0.4.0
