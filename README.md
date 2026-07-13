# llm-proxy-pii-rust

A fast, privacy-preserving LLM proxy written in Rust.

`llm-proxy-pii-rust` sits between your application and any LLM provider. It
detects and anonymizes personal information (PII) locally, before requests ever
leave your network, and can restore the original values in the provider's
response so your application sees coherent, un-redacted output.

The goal is to be a drop-in privacy layer: point your existing OpenAI-compatible
client at the proxy's URL and nothing else in your stack has to change.

## Why

Sending prompts to a hosted LLM means trusting a third party with whatever your
users type — names, emails, phone numbers, addresses, account numbers. This
proxy keeps the detection and redaction step under your control, on your own
infrastructure, instead of relying on the provider to handle it for you.

## Goals

- **Local-first detection** — PII is identified on your own hardware; nothing is
  sent elsewhere for the filtering step.
- **Reversible anonymization** — placeholders replace sensitive values on the
  way out and are restored on the way back, keeping responses usable.
- **Provider-agnostic** — works as a transparent proxy in front of
  OpenAI-compatible APIs.
- **Stability and performance** — built in Rust to handle concurrency and
  streaming reliably under load.

## Status

Early development. The architecture and detection engine are still being
designed and may change.

## Development

Living documents that track the whole build so nothing gets lost between
sessions:

- [Development setup (Windows, no admin)](docs/SETUP.md)
- [Architecture & design decisions](docs/ARCHITECTURE.md)
- [Roadmap & milestones](docs/ROADMAP.md)
- [Testing strategy](docs/TESTING.md)
- [Development log](docs/DEVLOG.md)

## License

Copyright (C) 2026 Francesco Stimola.

Licensed under the **GNU Affero General Public License v3.0 or later**
(`AGPL-3.0-or-later`) — see [LICENSE](LICENSE). As a network-served privacy proxy,
AGPL ensures anyone who runs a **modified** version as a service shares their
changes; running it unmodified carries no such obligation.

---

🇮🇹 Una traduzione italiana è disponibile in [README.it.md](README.it.md).
