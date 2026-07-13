# Development setup (Windows, no admin)

This project targets Windows with the **MSVC** toolchain. Every step here installs
**per-user, without administrator rights**.

## 1. Rust toolchain

Rust installs under your user profile (`%USERPROFILE%\.cargo`, `.rustup`) — no admin.

Download and run `rustup-init.exe` from <https://win.rustup.rs/x86_64>:

```powershell
& "$env:TEMP\rustup-init.exe" -y --default-toolchain stable --default-host x86_64-pc-windows-msvc --profile default
```

Verify (open a **new** shell so `%USERPROFILE%\.cargo\bin` is on `PATH`):

```powershell
cargo --version
rustc --version
```

`rustc` compiles, but producing an `.exe` needs a **linker**. For the MSVC target
that linker is `link.exe`, from the MSVC Build Tools (next step). Without it you
get `error: linker 'link.exe' not found`.

## 2. MSVC build tools — portable, no admin

The official VS Build Tools installer needs admin. Instead use
[**PortableBuildTools**](https://github.com/Data-Oriented-House/PortableBuildTools)
(open-source), which downloads the MSVC compiler + Windows SDK from Microsoft and
extracts them to a user folder, setting user-scope environment variables.

1. Download `PortableBuildTools.exe` (v2.10.2) from the project's GitHub releases.
2. Install non-interactively into a **user-writable** folder (under your profile
   avoids needing admin for the `C:\` root):

   ```powershell
   & .\PortableBuildTools.exe accept_license env=user target=x64 host=x64 path="%USERPROFILE%\BuildTools"
   ```

   - `accept_license` — no prompts (fully headless).
   - `env=user` — writes `INCLUDE`, `LIB`, and `Path` to `HKEY_CURRENT_USER`
     (no admin). Persistent, effective in new sessions after you log out/in.
   - `msvc=` / `sdk=` default to the latest versions.
   - `list` (instead of the flags) prints the available MSVC/SDK versions.

3. For the **current** shell (before logging out), load the environment from the
   generated script, then build:

   ```powershell
   . "$env:USERPROFILE\BuildTools\devcmd.ps1"
   cargo build
   ```

## 3. Verify

```powershell
cargo build      # links successfully now
cargo test       # runs the suite — see docs/TESTING.md
```

## 4. The NER model (M2 / M2.5, feature `onnx`)

The default build is native-dep-free and structured-PII only. The unstructured-entity
NER (names / orgs / locations) needs the `onnx` feature **and** a model. Two ways to
provide it — pick one:

**(A) Opt-in auto-download (M2.5, recommended).** Let `hf-hub` fetch the picked,
revision-pinned model into the standard HF cache (`%USERPROFILE%\.cache\huggingface\hub`,
shared/deduped with other tools). Only the repo is required; the rest default to the
XLM-R int8 pick:

```powershell
$env:NER_MODEL_REPO = "jiting/xlm-roberta-base-ner-hrl_onnx"
# optional overrides (these are the defaults):
#   $env:NER_MODEL_REVISION = "478a2a3"
#   $env:NER_MODEL_FILE     = "onnx/model_quantized.onnx"
#   $env:NER_TOKENIZER_FILE = "tokenizer.json"
cargo run --features onnx
```

The fetch is **one-time** (cached afterwards), **opt-in** (nothing downloads unless
`NER_MODEL_REPO` is set), and pulls **model artifacts only — never user data**.
`NER_LABELS` is derived from the model's `config.json`. Honors `HF_HOME` / `HF_HUB_CACHE`.

**(B) Explicit local files (zero outbound calls).** Point the proxy at files you
already have; nothing is fetched:

```powershell
$env:NER_MODEL_PATH      = "C:\path\model_quantized.onnx"
$env:NER_TOKENIZER_PATH  = "C:\path\tokenizer.json"
$env:NER_LABELS          = "O,B-DATE,I-DATE,B-PER,I-PER,B-ORG,I-ORG,B-LOC,I-LOC"  # class-id order
cargo run --features onnx
```

Common knobs (both modes): `NER_POOL_SIZE` (session pool for concurrency),
`NER_TOKEN_TYPE_IDS=1` (BERT-family models), `NER_REQUIRED=1` (fail closed if the model
can't load — a missing NER then blocks startup instead of silently downgrading).

## 5. Debug & observability (M2.6, off by default)

Opt-in tools to eyeball that masking holds — never enable in production:

```powershell
# See the placeholders the provider saw (skips response de-mask; loud startup warning):
$env:PII_DEBUG_SKIP_DEMASK = "1"
# Dump the exact masked body sent upstream (placeholders only — safe):
$env:RUST_LOG = "llm_proxy_pii_rust=trace"
cargo run
```

Request-side masking always runs, so neither flag ever sends raw PII upstream. The final
de-masked client output (real values) is **never** logged.

## 6. Provider selection & streaming (M3)

The proxy fronts any provider's **OpenAI-compatible** endpoint. Pick one with a preset
that sets the right path + client-header passthrough (all overridable):

```powershell
# OpenAI (default) — nothing extra needed beyond the key:
$env:UPSTREAM_BASE_URL = "https://api.openai.com"
$env:UPSTREAM_API_KEY  = "sk-…"

# GitHub Copilot (no /v1; passes editor headers through):
$env:UPSTREAM_PROVIDER = "copilot"
$env:UPSTREAM_BASE_URL = "https://api.githubcopilot.com"

# Anthropic via its OpenAI-compat layer (passes anthropic-version through):
$env:UPSTREAM_PROVIDER = "anthropic"
$env:UPSTREAM_BASE_URL = "https://api.anthropic.com"
```

Overrides: `UPSTREAM_CHAT_PATH` (e.g. `/chat/completions`), `UPSTREAM_FORWARD_HEADERS`
(comma list of client headers to pass through), `UPSTREAM_EXTRA_HEADERS` (`Key=Value`
pairs separated by `;` added to every upstream request).

**One provider per instance (today).** `UPSTREAM_PROVIDER` is chosen at startup, so a single
proxy fronts one provider at a time. To front **several at once** (e.g. Copilot *and* Anthropic),
run **one instance per provider** on different ports (set `LISTEN_ADDR`) and point each client at
the right one — no code needed. Per-request routing from a *single* instance is a Backlog item
(see `docs/ROADMAP.md`).

**Streaming** works automatically: a request with `"stream": true` is forwarded as SSE and
de-anonymized incrementally on the way back (placeholders split across token chunks are
reassembled). Request-side masking always runs first, so the provider only ever sees
placeholders.

**PII locales (M4).** Detection runs in three tiers: **universal** (email, IBAN, credit card,
phone) and **national IDs** (US SSN, IT Codice Fiscale, GB NINO, ES DNI/NIE, FR NIR) are
**always on** — a national ID is masked regardless of configuration (privacy-first). `PII_LOCALES`
(comma-separated, default `it,us`) gates only the **FP-prone** tier — ambiguous recognizers like
national *phone* formats — of which there are none yet, so `PII_LOCALES` is a no-op today (the seam
is kept for future opt-in recognizers).

## Fallback: GNU toolchain (no MSVC, no admin)

If MSVC is unavailable, the GNU toolchain bundles its own linker and builds M1
(pure Rust, no ONNX) immediately:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
cargo build
```

Note: ONNX Runtime (`ort`, milestone M2) links best against MSVC — switch back to
`stable-x86_64-pc-windows-msvc` before M2.

## Notes

- No component here requires administrator rights.
- `git` must be available for cloning and committing.

## Optional: auto-load the MSVC env in Claude Code

Each Claude Code tool call runs in a fresh shell that does not inherit the MSVC
env until you log out/in. To avoid dot-sourcing `devcmd.ps1` on every command,
capture the env once into `.claude/settings.local.json` (gitignored,
machine-local) under an `"env"` object — `INCLUDE`, `LIB`, the full `PATH` (with
the MSVC bins and `%USERPROFILE%\.cargo\bin`), plus the `VCToolsInstallDir` /
`WindowsSDKDir` / `WindowsSDKVersion` helpers. Every tool shell then resolves
`link.exe` and `cargo` automatically. Regenerate it if the MSVC or SDK version
changes.
