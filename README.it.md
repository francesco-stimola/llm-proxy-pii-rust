<div align="center">

```
██╗     ██╗     ███╗   ███╗      ██████╗ ██████╗  ██████╗ ██╗  ██╗██╗   ██╗      ██████╗ ██╗██╗
██║     ██║     ████╗ ████║      ██╔══██╗██╔══██╗██╔═══██╗╚██╗██╔╝╚██╗ ██╔╝      ██╔══██╗██║██║
██║     ██║     ██╔████╔██║█████╗██████╔╝██████╔╝██║   ██║ ╚███╔╝  ╚████╔╝ █████╗██████╔╝██║██║
██║     ██║     ██║╚██╔╝██║╚════╝██╔═══╝ ██╔══██╗██║   ██║ ██╔██╗   ╚██╔╝  ╚════╝██╔═══╝ ██║██║
███████╗███████╗██║ ╚═╝ ██║      ██║     ██║  ██║╚██████╔╝██╔╝ ██╗   ██║         ██║     ██║██║
╚══════╝╚══════╝╚═╝     ╚═╝      ╚═╝     ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝   ╚═╝         ╚═╝     ╚═╝╚═╝
```

### Il firewall PII per il tuo traffico LLM

**I tuoi prompt escono mascherati. La tua app continua a vedere i dati reali.**

Rilevamento local-first · segnaposto reversibili · fail-closed · streaming · compatibile OpenAI

[![CI](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![Locales](https://img.shields.io/badge/documenti%20nazionali-10%20paesi-green.svg)](#cosa-rileva)

[Avvio rapido](#avvio-rapido) · [Cosa rileva](#cosa-rileva) · [Come funziona](#come-funziona) · [Configurazione](#configurazione) · [Architettura](docs/ARCHITECTURE.md)

</div>

---

## Il problema

Ogni prompt che invii a un LLM ospitato è una copia dei dati dei tuoi utenti sul server di
qualcun altro — nomi, email, numeri di telefono, documenti d'identità, IBAN, chiavi API. Non
puoi "de-inviarlo", e *"lo oscuriamo dopo"* non è un controllo.

`llm-proxy-pii-rust` sposta l'oscuramento **dalla tua parte del filo**, e lo rende reversibile
così che nulla a valle si rompa.

```
   la tua app                  IL PROXY                         provider
  ┌──────────┐          ┌────────────────────┐              ┌────────────┐
  │          │  dati    │  rileva  ──► masc. │  solo dati   │            │
  │  client  │─────────►│                    │─────────────►│  OpenAI /  │
  │          │  reali   │  [EMAIL_1] [IBAN_1]│  mascherati  │  Copilot / │
  │          │          │                    │              │  Anthropic │
  │          │◄─────────│  ripristina ◄ vault│◄─────────────│            │
  └──────────┘  dati    └────────────────────┘  segnaposto  └────────────┘
                reali        in locale, on-box              non vede mai PII in chiaro
```

Fai puntare il tuo client compatibile con OpenAI al proxy. Nient'altro nel tuo stack cambia.

---

## Avvio rapido

Richiede Rust **1.89+**.

```sh
cargo build --release --features onnx
UPSTREAM_API_KEY=sk-... ./target/release/llm-proxy-pii-rust
```

Poi parlaci esattamente come faresti col provider reale:

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user",
       "content":"scrivi a jane@example.com per la fattura IT60X0542811101000000123456"}]}'
```

**Cosa ha ricevuto davvero il provider:**

```json
{"role":"user","content":"scrivi a [EMAIL_1] per la fattura [IBAN_1]"}
```

**Cosa ha ricevuto il tuo client:** la risposta, con `jane@example.com` e l'IBAN ripristinati.

> Il proxy non ha bisogno di custodire la tua chiave: un header `Authorization` inviato dal
> client ha sempre la precedenza su `UPSTREAM_API_KEY`, quindi puoi passare il tuo token a
> ogni richiesta e lasciare il proxy privo di credenziali.

---

## Cosa rileva

**PII strutturate — deterministiche, sempre attive, senza modello.** Ogni match è validato con
checksum o regole specifiche, così il tasso di falsi positivi resta quasi nullo.

| | |
|---|---|
| **Universali** | email · telefono (US + `+CC`) · carta di credito (Luhn) · IBAN (mod-97 + lunghezza per paese) · chiavi API e segreti (`sk-…`, `sk-ant-…`, `AKIA…`) |
| **Documenti nazionali** *(10 paesi)* | 🇺🇸 SSN · 🇮🇹 Codice Fiscale · 🇬🇧 NINO · 🇪🇸 DNI/NIE · 🇫🇷 NIR · 🇩🇪 Steuer-ID · 🇳🇱 BSN · 🇵🇹 NIF · 🇱🇻 codice personale · 🇨🇳 documento di residenza |

I documenti nazionali vengono mascherati **indipendentemente dalla configurazione dei locale** —
privacy-first: un documento che raggiunge il proxy viene mascherato anche se il suo paese non è
quello configurato.

**Entità non strutturate — NER ONNX locale (XLM-R int8, CPU).** Persone, organizzazioni e luoghi
in **ar · de · en · es · fr · it · lv · nl · pt · zh**. Gira sul tuo hardware; i campi grandi
vengono suddivisi in finestre, così funzionano anche i documenti lunghi.

---

## Come funziona

1. **Rileva** — i riconoscitori deterministici scansionano ogni campo testuale dello schema chat
   (`content`, `name`, argomenti delle tool call, descrizioni degli strumenti e dei parametri
   annidati), più il NER per nomi/organizzazioni/luoghi.
2. **Maschera** — ogni valore diventa un segnaposto tipizzato (`[EMAIL_1]`, `[PERSON_2]`),
   registrato in un vault per-richiesta. Il mascheramento gira **fino a un punto fisso**, perché
   sostituire un valore può esporne un altro.
3. **Istruisce il modello** — viene iniettata un'istruzione di sistema che spiega che i segnaposto
   rappresentano dati reali e vanno usati alla lettera, anche negli argomenti delle tool call.
4. **Inoltra** — il provider vede i segnaposto e nient'altro.
5. **Ripristina** — il vault rimette i valori reali: nelle risposte bufferizzate, in
   `tool_calls[].function.arguments`, e **incrementalmente negli stream SSE** (un segnaposto
   diviso tra due chunk, `[EMA` + `IL_1]`, si risolve comunque).

### Lo standard che si impone

- **Fail closed.** Una forma di richiesta illeggibile, un rilevatore obbligatorio che fallisce, o
  un mascheramento che non raggiunge un punto fisso stabile **bloccano la richiesta (400)**
  invece di inoltrare qualcosa dallo stato PII sconosciuto. Solo `POST /v1/chat/completions` è
  proxato — tutto il resto è `404`, mai inoltrato.
- **Mai loggare PII in chiaro.** I log riportano categorie, conteggi e segnaposto — mai i valori.
  Garantito da un test, non da una convenzione.
- **Lineare sotto carico.** Il percorso di mascheramento è dimostrabilmente lineare sia nella
  *dimensione* del campo che nel *numero* di entità, e il lavoro CPU-bound gira fuori
  dall'executor asincrono: un corpo grande non può bloccare il proxy per tutti. *Un proxy giù non
  protegge nulla.*
- **Deterministico.** Lo stesso valore ottiene sempre lo stesso segnaposto all'interno di una
  richiesta, così le conversazioni multi-turno stateless restano coerenti.

---

## Provider

Una sola impostazione instrada verso qualsiasi endpoint compatibile con OpenAI. Il mascheramento
è **identico** in ogni caso — il preset cambia solo l'instradamento (percorso, header), mai cosa
viene oscurato.

```sh
UPSTREAM_PROVIDER=openai      # default
UPSTREAM_PROVIDER=copilot     # GitHub Copilot
UPSTREAM_PROVIDER=anthropic   # endpoint compatibile OpenAI di Anthropic
```

> **Ambito — la forma API compatibile con OpenAI, su entrambi i lati.** Il proxy parla lo schema OpenAI Chat
> Completions (`/v1/chat/completions`) *sia* verso il tuo client *sia* verso l'upstream; Anthropic e Copilot
> sono raggiunti tramite i loro endpoint **compatibili con OpenAI**, non le loro API native. Un client che
> parla il protocollo **nativo** di un provider non è quindi ancora supportato — in particolare **Claude
> Code**, che usa l'API nativa `/v1/messages` di Anthropic. Il supporto nativo per Anthropic, così che Claude
> Code possa passare dal proxy, è il prossimo traguardo — vedi la [roadmap](docs/ROADMAP.md).

---

## Configurazione

Tutto è pilotato da variabili d'ambiente.

| Variabile | Default | Scopo |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Indirizzo di ascolto del proxy |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | URL base del provider a monte |
| `UPSTREAM_API_KEY` | *(non impostata)* | Iniettata come `Authorization: Bearer …` **solo** se il client non ne invia una propria |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — preset di instradamento |
| `UPSTREAM_CHAT_PATH` | *(preset)* | Sovrascrive il percorso delle chat completions |
| `UPSTREAM_FORWARD_HEADERS` | *(preset)* | Header del client da inoltrare, separati da virgola |
| `UPSTREAM_EXTRA_HEADERS` | *(nessuno)* | `Chiave=Valore;Chiave2=Valore2` header statici per ogni richiesta a monte |
| `MAX_BODY_BYTES` | `16777216` | Limite del corpo della richiesta (16 MiB) |
| `PII_LOCALES` | `it,us` | Governa **solo** il livello di riconoscitori inclini a falsi positivi. **I documenti nazionali sono sempre attivi** |
| `RUST_LOG` | *(non impostata)* | es. `llm_proxy_pii_rust=debug` |

<details>
<summary><b>NER (entità non strutturate) — <code>--features onnx</code></b></summary>

<br>

| Variabile | Default | Scopo |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(non impostate)* | File di modello locali espliciti — **zero chiamate in uscita**, hanno sempre la precedenza |
| `NER_MODEL_REPO` | *(non impostata)* | Download automatico opzionale (`owner/name`) di un modello con revisione fissata nella cache HuggingFace standard. È l'unica chiamata in uscita dell'intero strumento, fatta una volta all'avvio, e scarica **artefatti del modello, non dati utente** |
| `NER_MODEL_REVISION` | `478a2a3` | Revisione fissata per il download automatico |
| `NER_POOL_SIZE` | `2` | Dimensione del pool di sessioni ONNX concorrenti |
| `NER_REQUIRED` | disattivato | **Fail closed per i nomi**: un NER mancante o fallito blocca la richiesta (400) invece di degradare silenziosamente al solo strutturato |

Senza né `NER_MODEL_PATH` né `NER_MODEL_REPO`, la build esegue semplicemente il solo rilevamento
strutturato.

</details>

<details>
<summary><b>Debug — vedi il mascheramento con i tuoi occhi</b></summary>

<br>

| Variabile | Scopo |
|---|---|
| `PII_DEBUG_SKIP_DEMASK` | Salta il de-mascheramento della risposta, così il tuo client riceve **i segnaposto che ha visto il provider** — prova diretta che il round-trip è collegato. Emette un avviso rumoroso all'avvio. Mai attivarlo in produzione |
| `RUST_LOG=llm_proxy_pii_rust=trace` | Logga i byte esatti (mascherati) inviati a monte. La risposta de-mascherata non viene **mai** loggata |

Esegui lo stesso prompt due volte — una con `PII_DEBUG_SKIP_DEMASK=1`, una senza — e confronta.
Procedura completa: [`docs/MANUAL_VERIFICATION.md`](docs/MANUAL_VERIFICATION.md).

</details>

---

## Documentazione

*(I documenti di progetto sono in inglese.)*

| | |
|---|---|
| [Architettura & invarianti](docs/ARCHITECTURE.md) | Come funziona, e **cosa non deve mai rompersi** |
| [Strategia di test](docs/TESTING.md) | Le garanzie, e perché ognuna esiste |
| [Verifica manuale](docs/MANUAL_VERIFICATION.md) | Dimostra la catena end-to-end contro un provider reale |
| [Setup di sviluppo](docs/SETUP.md) | Toolchain (incl. Windows, senza permessi di admin) |

```sh
cargo test                    # suite solo strutturato
cargo test --features onnx    # + il percorso NER
```

---

## Licenza

Copyright (C) 2026 Francesco Stimola.

**GNU Affero General Public License v3.0 o successiva** (`AGPL-3.0-or-later`) — vedi
[LICENSE](LICENSE). Trattandosi di un proxy privacy servito in rete, l'AGPL garantisce che chi
esegue una versione **modificata** come servizio debba condividerne le modifiche. Eseguirlo non
modificato non comporta alcun obbligo.

---

<div align="center">

🇬🇧 [Read in English](README.md)

</div>
