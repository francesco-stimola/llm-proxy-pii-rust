# Contributing

Thanks for your interest in `llm-proxy-pii-rust`.

## Why this file exists

This project is **dual-licensed**: open source under [AGPL-3.0-or-later](LICENSE), and
available under a separate commercial license for organizations that need terms the AGPL
doesn't offer (see [README.md → License](README.md#license)). Offering that commercial license
depends on one thing staying true: the maintainer holds full rights to *every* line in the
repository, including yours. That's what the terms below secure.

## Copyright assignment

By submitting a contribution to this repository — a pull request, a patch, or any other
form — you agree that:

1. You have the right to submit it: it's your original work, or you already hold the rights
   necessary to submit it under these terms.
2. You assign to Francesco Stimola all right, title, and interest, including all copyright, in
   and to your contribution, effective upon submission.
3. Francesco Stimola may use, modify, relicense, and sublicense your contribution — including
   under proprietary or commercial terms — without further consent from you.
4. In return, you're granted back the same rights to your own contribution that any other user
   of the project gets under its then-current open source license (currently
   `AGPL-3.0-or-later`).

If you're contributing on behalf of an employer, make sure you have their permission first —
this assignment can only convey rights you actually hold.

*(This is a standard mechanism for single-copyright-holder dual-licensed projects — it's what
lets the project sell commercial licenses on the whole codebase, community contributions
included, without re-clearing rights from every past contributor. It isn't a substitute for
your own legal advice if you're contributing something non-trivial.)*

## How to indicate agreement

- Sign off your commits: `git commit -s` (adds a `Signed-off-by` trailer — the same lightweight
  convention as the Linux kernel's DCO).
- Check the acknowledgment box in the pull request template.

A PR without both is assumed **not** to carry this agreement, and won't be merged.

## Development workflow

Toolchain and build/test commands: [docs/SETUP.md](docs/SETUP.md). Before opening a PR:

- `cargo test` (and `cargo test-onnx` if you touched the `onnx` feature) green, no warnings.
- New tests for any behavior change; **adversarial** cases for detection changes — a miss is a
  leak (see [docs/TESTING.md](docs/TESTING.md)).
- `docs/ROADMAP.md` checkboxes and `docs/DEVLOG.md` updated if the change lands a milestone
  item.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the invariants the detection layer rests
on — read it before changing detection.
