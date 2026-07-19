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

[![Release build & publish](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml/badge.svg)](https://github.com/francesco-stimola/llm-proxy-pii-rust/actions/workflows/release-build-publish.yml)
[![Ultima release](https://img.shields.io/github/v/release/francesco-stimola/llm-proxy-pii-rust?sort=semver&label=release)](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest)
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
cargo build-onnx --release
UPSTREAM_API_KEY=sk-... ./target/onnx/release/llm-proxy-pii-rust
```

> `build-onnx` è un alias del repo per `build --features onnx --target-dir target/onnx`
> (`.cargo/config.toml`). L'ibrido compila in una **directory propria**, così un semplice
> `cargo build`/`cargo test` — che scrive lo stesso nome di file sotto `target/` — non può
> sovrascriverlo con un binario solo-strutturato. Anche la build di default funziona, ma
> rinuncia alla NER: solo PII strutturata.

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
| **Telefono nazionale** *(opt-in via `PII_LOCALES`)* | 🇬🇧 GB · 🇩🇪 DE, numeri domestici senza `+CC` (`020 7946 0958`, `030 12345678`) — validati contro il piano di numerazione reale, così numeri d'ordine e documenti non vengono sovra-mascherati |

I documenti nazionali vengono mascherati **indipendentemente dalla configurazione dei locale** —
privacy-first: un documento che raggiunge il proxy viene mascherato anche se il suo paese non è
quello configurato. Il livello **telefono nazionale** è l'eccezione: un numero domestico nudo si
confonde con normali sequenze di cifre, quindi è opt-in per locale via `PII_LOCALES` e protetto da
un controllo di numero realmente assegnato (la libreria Rust puro `phonenumber` — nessuna dipendenza
nativa).

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

- **Fail closed.** Una forma di richiesta illeggibile, un tipo di blocco di contenuto
  sconosciuto, un rilevatore obbligatorio che fallisce, o un mascheramento che non raggiunge un
  punto fisso stabile **bloccano la richiesta (400)** invece di inoltrare qualcosa dallo stato
  PII sconosciuto. Solo `POST /v1/chat/completions` (e `POST /v1/messages` quando
  `UPSTREAM_PROVIDER=anthropic`) è proxato — tutto il resto è `404`, mai inoltrato.
- **Mai loggare PII in chiaro.** I log riportano categorie, conteggi e segnaposto — mai i valori.
  Garantito da un test, non da una convenzione.
- **Lineare sotto carico.** Il percorso di mascheramento è dimostrabilmente lineare sia nella
  *dimensione* del campo che nel *numero* di entità, e il lavoro CPU-bound gira fuori
  dall'executor asincrono: un corpo grande non può bloccare il proxy per tutti. *Un proxy giù non
  protegge nulla.*
- **Deterministico.** Lo stesso valore ottiene sempre lo stesso segnaposto all'interno di una
  richiesta, così le conversazioni multi-turno stateless restano coerenti.

### Impronta — misurata, non dichiarata

Memoria residente (Windows, build debug, a riposo dopo l'avvio; solo-strutturato misurato il
2026-07-16, le righe ibride per pool il 2026-07-17):

| build | private | working set | cosa domina |
|---|---|---|---|
| **solo strutturato** (feature di default) | **~10 MB** | ~36 MB | niente — sono regex |
| **ibrido**, default `NER_POOL_SIZE=1` (`--features onnx`, XLM-R int8) | **~563 MB** | ~585 MB | il NER: **una** sessione ONNX |
| **ibrido**, `NER_POOL_SIZE=2` | ~834 MB | ~856 MB | **due** sessioni — il modello tenuto due volte; ogni sessione aggiunge **~270 MB** |
| **ibrido + GLiNER int8** solo (opt-in, `GLINER_MODEL_PATH`, XLM-R spento) | ~537 MB | ~560 MB | la singola sessione di GLiNER — paragonabile a XLM-R da solo (M8) |
| **ibrido, XLM-R + GLiNER int8** (entrambi, opt-in) | **~1073 MB** | ~1094 MB | **due modelli caricati** — GLiNER aggiunge **~510 MB** sopra XLM-R (M8) |

> **GLiNER (M8) è opt-in e *additivo*** — **non** sostituisce il NER (misurato: su int8 la sua recall
> sui nomi è sotto quella di XLM-R — vedi [ROADMAP M8](docs/ROADMAP.md#m8)). Abilitato insieme a XLM-R
> carica un **secondo** modello, quindi considera la RAM: **~1,07 GB** per entrambi contro ~563 MB per
> il solo XLM-R (misurato). Esegui GLiNER *al posto* di XLM-R (~537 MB) solo se vuoi specificamente le
> sue categorie contestuali e accetti la recall più debole sui nomi.

Il livello deterministico è sostanzialmente gratis; **il modello è tutto il costo**, e scala col
pool: **~290 MB di base condivisa più ~270 MB per sessione** (misurato — 563 MB a `pool=1`, 834 MB a
`pool=2`, quindi `pool=N` ≈ 290 + N×270 MB — *non* un raddoppio pulito, perché il runtime e le arene
della prima sessione sono condivisi). `NER_POOL_SIZE` (**default `1` dal 2026-07-17**) è una sola
sessione — la forma lean per singolo client; un proxy **centralizzato** lo alza a `N` per throughput
concorrente a quella RAM. Scegli la build in base alla minaccia che hai davvero: il solo strutturato
copre già email, IBAN, carte, secret e 10 schemi di identificativo nazionale, ed è indipendente dalla
lingua. Il NER ti compra nomi, organizzazioni e
luoghi — nient'altro.

### Latenza — anch'essa misurata, su un payload realistico

Mascheramento di **un turno realistico di Claude Code** (22,3 KiB: system prompt + 10 schemi di
tool + un messaggio utente), 2026-07-17, macchina di riferimento (Ryzen 5 PRO 8540U, 6 core / 12
thread) con il suo **piano di alimentazione bilanciato / a risparmio energetico** — un portatile
normale nel suo stato normale, che è il caso per cui questo proxy esiste — build debug, eseguito in
isolamento, migliore di 3:

| rilevamento | per turno | per KiB |
|---|---|---|
| **solo strutturato** | **~20 ms** | ~0,9 ms |
| **ibrido** (entrambe le forme — vedi sotto) | **~4,7 s** | ~210 ms |

**Il NER è ~100% del costo** — il livello deterministico è oltre 100× più veloce sugli stessi byte.

**Considera quel numero come proprio di questa macchina, non del prodotto.** Su sette misurazioni
fatte da due persone, lo stesso codice e la stessa fixture hanno dato **da 2,5 a 7,1 s**. Due
variabili sono note: eseguire il benchmark insieme ad altri test costa **1,5×** (misurato), e il resto
non sappiamo spiegarlo. Quello che possiamo dire è cosa *non* è: abbiamo attribuito a lungo la
dispersione a "rete contro batteria", finché il proprietario della macchina non ha fatto notare che
entrambi i gruppi di run giravano sullo **stesso profilo di risparmio energetico** — l'alimentatore
era attaccato, il piano non è mai cambiato. Quell'etichetta non separava nulla, ed è per questo che
una run "a batteria" ha battuto tre run "a rete". Quindi `~4,7 s` è la cifra che si è riprodotta due
volte, in modo indipendente, in isolamento; i numeri più veloci che avevamo pubblicato erano il
migliore di un insieme rumoroso — che non è la stessa cosa della verità.

> Una macchina su un piano di alimentazione **prestazionale** dovrebbe fare meglio di così, e non ne
> abbiamo misurata una. La cifra sopra è deliberatamente il caso pessimistico del portatile ordinario,
> non un caso migliore che avremmo potuto allestire.

**Ciò che resta stabile è il *miglioramento*, ed è la parte che riguarda il codice:** il default
spedito è **almeno 1,5× più veloce** della versione pre-threading — il floor che il test asserisce
davvero — e **tipicamente ~1,7–2,3×** a seconda della macchina. (Citiamo il floor, non il numero
migliore visto: l'accelerazione annulla lo stato di alimentazione della macchina ma non la sua
velocità grezza, quindi una macchina più veloce comprime il rapporto *verso* il floor invece che
lontano — ed è per questo che ogni banda più stretta che pubblicavamo veniva puntualmente smentita
dalla run pulita successiva.) Se vuoi verificare l'affermazione di questo repo sulla tua macchina, è
quel rapporto da guardare — l'harness lo stampa, calcolato contro una gamba di calibrazione misurata
pochi secondi prima nella stessa run.

**Quale forma usare?** Il default — **`NER_POOL_SIZE=1`** (dal 2026-07-17) — è la forma per singolo
client (un agente di coding, un IDE): tiene **una** sessione ONNX, quindi usa **~270 MB di RAM in
meno** della forma a pool (**misurato**: 563 MB contro 834 MB — togliere la seconda sessione libera
la sua copia da ~270 MB dei pesi, circa un terzo del totale) e dà alla tua unica richiesta in volo
tutto il box. Al caso personale non costa **nulla**, perché una singola richiesta occupa comunque una sola
sessione, e **sulla latenza le due forme si equivalgono** (abbiamo misurato ciascuna vincere del
10–15% in run diverse: la differenza è dentro il rumore di questa macchina). Se invece gestisci un
proxy **condiviso / centralizzato** davanti a client concorrenti, imposta **`NER_POOL_SIZE=N`**: il
pool vale circa il **30% di throughput in più** (equivalentemente, `pool=1` è ~−23% sotto carico
concorrente) — e *quello* è un trade reale, misurato, pagato a `N×` la RAM del modello.

> **Misura sulla tua macchina prima di credere a questi numeri:** `cargo test-onnx --test
> m7_latency -- --ignored --nocapture --test-threads=1`. **Il `--test-threads=1` è essenziale**:
> senza, cargo esegue i benchmark in parallelo e si misurano a vicenda (1,5×). L'harness stampa il
> dettaglio per campo, uno sweep sui thread con min/mediana/**spread**, le cifre di concorrenza e —
> dato che i millisecondi assoluti non sono confrontabili tra macchine — **un rapporto di
> accelerazione contro una gamba di calibrazione misurata nella stessa run**, più quanto la tua
> macchina disti da quella citata qui. Una build *release* è irrilevante (misurato: 3%): il costo è
> dentro ONNX Runtime, una libreria nativa precompilata, quindi compilare meglio il *nostro* Rust
> non cambia niente.

---

## Provider e client

Il proxy parla **un solo** formato — lo schema OpenAI Chat Completions
(`/v1/chat/completions`) — su *entrambi* i lati. Quindi "funziona con X?" sono in realtà due
domande: verso cosa **inoltra** (upstream) e cosa può **parlargli** (client).

**Upstream — verso cosa inoltra.** Qualsiasi endpoint compatibile con OpenAI. Un preset
`UPSTREAM_PROVIDER` sceglie l'*involucro* giusto per un provider noto (il mascheramento è **identico**
in ogni caso — il preset cambia solo l'instradamento, mai cosa viene oscurato):

| `UPSTREAM_PROVIDER` | Chat path | Header client inoltrati |
|---|---|---|
| `openai` (default) | `/v1/chat/completions` | `openai-organization`, `openai-project` |
| `copilot` | `/chat/completions` *(niente `/v1`)* | `editor-version`, `editor-plugin-version`, `copilot-integration-id`, `openai-intent` |
| `anthropic` | `/v1/chat/completions` | `anthropic-version`, `anthropic-beta` |

...o qualsiasi altro endpoint compatibile con OpenAI tramite `UPSTREAM_BASE_URL` (modelli locali
dietro Ollama / vLLM / LM Studio, Groq, Mistral, …).

> **Perché un preset per provider se sono tutti OpenAI-compatibili?** Perché *"compatibile con OpenAI"*
> descrive il **corpo** della richiesta, non l'intera chiamata HTTP: tutti e tre condividono un unico
> schema JSON e differiscono solo nell'**involucro** che mostra la tabella — path + header. Un preset non
> fa che impacchettare questi delta per-provider; non cambia mai lo schema né cosa viene mascherato, ed
> ogni parte è sovrascrivibile (`UPSTREAM_CHAT_PATH`, `UPSTREAM_FORWARD_HEADERS`, `UPSTREAM_EXTRA_HEADERS`).

**Client — cosa può parlargli.** Un client funziona col proxy se puoi puntarlo a un **base URL
compatibile con OpenAI**. La maggior parte degli agent moderni lo permette (via BYOK / provider
custom); l'eccezione è un client legato al protocollo **nativo** di un vendor.

*Compatibilità client verificata il **2026-07-15** — il supporto BYOK / endpoint custom in questi
strumenti cambia in fretta; verifica la doc corrente di ciascuno prima di farci affidamento.*

| Client | Passa dal proxy? | Come |
|---|---|---|
| SDK OpenAI · `curl` (JSON OpenAI) · Cline · Continue | ✅ | imposta il `base_url` del client sul proxy |
| **opencode** | ✅ | un provider `@ai-sdk/openai-compatible` custom, `baseURL: http://127.0.0.1:8080/v1` |
| **pi** (`@earendil-works/pi`) | ✅ | un provider con `"api": "openai-completions"`, `"baseUrl": ".../v1"` |
| **GitHub Copilot CLI** | ✅ | BYOK: `COPILOT_PROVIDER_BASE_URL=http://127.0.0.1:8080/v1` (il modello deve supportare tool + streaming) |
| **GitHub Copilot Chat** (VS Code) | ✅ | BYOK → un provider "OpenAI Compatible" puntato al proxy (solo chat) |
| **Claude Code** · SDK Anthropic | ✅ (M6) | puntalo al proxy con `UPSTREAM_PROVIDER=anthropic`; il body nativo `/v1/messages` è mascherato **in place** (nessuna traduzione OpenAI). Verificato dal vivo end-to-end contro Anthropic reale. |
| **Client nativi di altri vendor** — es. la **Gemini CLI** (API Gemini nativa), gli SDK nativi Bedrock / Vertex | ❌ **non supportati** | parlano il protocollo **nativo** di un vendor che il proxy non serve. Il proxy maschera i client OpenAI-compatibili (sopra) e il `/v1/messages` nativo di Anthropic (M6) — *nient'altro*. Aggiungere un altro schema nativo richiede un adapter per-provider — **[Opzione B, Backlog](docs/ROADMAP.md#backlog)** (Gemini è il prossimo candidato nominato). |

> **Il discrimine è il base URL, non il brand.** GitHub Copilot compare su *entrambi* gli assi — un
> preset *upstream* (`UPSTREAM_PROVIDER=copilot`) **e**, via BYOK, un *client* (Copilot CLI / Chat):
> stesso brand, due impostazioni indipendenti. (Il BYOK a endpoint custom di Copilot Chat è solo per
> la chat ed era in fase di rilascio in VS Code al momento della scrittura; quello della CLI è già
> disponibile.)

Quindi **Anthropic funziona come *upstream*** (via il suo endpoint compatibile con OpenAI) **e ora
anche come *client nativo*** — **M6** serve il `/v1/messages` nativo di Anthropic (blocchi di
contenuto, `tool_use`/`tool_result`, streaming), così un client nativo come Claude Code è mascherato
senza una modalità OpenAI-compat. La rotta è registrata solo quando `UPSTREAM_PROVIDER=anthropic`; su
qualsiasi altro upstream `/v1/messages` restituisce ancora 404.

> **Detto chiaramente: esistono solo due superfici native — OpenAI-compatibile e Anthropic
> `/v1/messages`.** Un coding agent legato al protocollo **nativo di un altro vendor** **non** è
> mascherato. Il caso concreto oggi è la **Gemini CLI** (`generateContent` nativo di Gemini); gli SDK
> nativi Bedrock / Vertex sono la stessa classe. Un client simile richiederebbe un **adapter nativo
> per-provider schema-aware**, e un campo di schema mancato è un leak — quindi è lavoro deliberato e
> **non schedulato**, tracciato come [**Opzione B — adapter provider nativi**](docs/ROADMAP.md#backlog)
> (con Gemini indicato come il più probabile prossimo). Finché non esiste un adapter, usa un agente del
> genere solo tramite una modalità OpenAI-compatibile / BYOK se ne ha una; se parla solo l'API nativa
> del suo vendor, il proxy non può ancora proteggerlo.

---

## Configurazione

Tutto è pilotato da variabili d'ambiente.

| Variabile | Default | Scopo |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Indirizzo di ascolto del proxy |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | URL base del provider a monte |
| `UPSTREAM_API_KEY` | *(non impostata)* | Iniettata come `Authorization: Bearer …` **solo** se il client non ne invia una propria |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — preset di instradamento (`anthropic` abilita anche la rotta nativa `/v1/messages`, M6) |
| `UPSTREAM_CHAT_PATH` | *(preset)* | Sovrascrive il percorso delle chat completions |
| `UPSTREAM_MESSAGES_PATH` | `/v1/messages` | Sovrascrive il percorso Messages nativo di Anthropic (M6; usato solo con `UPSTREAM_PROVIDER=anthropic`) |
| `UPSTREAM_FORWARD_HEADERS` | *(preset)* | Header del client da inoltrare, separati da virgola |
| `UPSTREAM_EXTRA_HEADERS` | *(nessuno)* | `Chiave=Valore;Chiave2=Valore2` header statici per ogni richiesta a monte |
| `MAX_BODY_BYTES` | `16777216` | Limite del corpo della richiesta (16 MiB) |
| `PII_LOCALES` | `it,us` | Governa **solo** il livello di riconoscitori inclini a falsi positivi — attualmente i riconoscitori **telefono nazionale** (`gb`, `de`: numeri domestici senza `+CC`). Aggiungi `gb` / `de` per abilitarli. **I documenti nazionali sono sempre attivi** |
| `PII_CACHE_ENTRIES` | `16` | Cache di rilevamento (S3): il system prompt byte-identico viene scansionato una volta e riusato, risparmiando la passata NER dominante. Con chiave sui byte esatti, un hit non può **mai** mascherare *meno* di una scansione fresca. `0` la disabilita |
| `RUST_LOG` | *(non impostata)* | es. `llm_proxy_pii_rust=debug` |

<details>
<summary><b>NER (entità non strutturate) — <code>--features onnx</code></b></summary>

<br>

| Variabile | Default | Scopo |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(non impostate)* | File di modello locali espliciti — **zero chiamate in uscita**, hanno sempre la precedenza |
| `NER_MODEL_REPO` | *(non impostata)* | Download automatico opzionale (`owner/name`) di un modello con revisione fissata nella cache HuggingFace standard. È l'unica chiamata in uscita dell'intero strumento, fatta una volta all'avvio, e scarica **artefatti del modello, non dati utente** |
| `NER_MODEL_REVISION` | `478a2a3` | Revisione fissata per il download automatico |
| `NER_POOL_SIZE` | `1` | Dimensione del pool di sessioni ONNX concorrenti. **Default `1`** = una sessione — la forma per singolo client (~563 MB, tutto il box per richiesta). Alzalo a **`N`** per un proxy centralizzato: ~30% di throughput in più a ~270 MB di RAM in più per sessione (vedi la nota sulla latenza sopra) |
| `NER_INTRA_THREADS` | *derivato* | Thread **per sessione**. Default `max(1, core / NER_POOL_SIZE)` — le due manopole si **moltiplicano**, e il prodotto deve stare nella macchina. Impostala solo se sai perché |
| `NER_REQUIRED` | disattivato | **Fail closed per i nomi**: se impostato, **almeno un** detector ML (il NER e/o GLiNER sotto) deve caricarsi, e ognuno caricato gira "unwrapped" — un fallimento blocca la richiesta (400) invece di degradare silenziosamente al solo strutturato |

Senza né `NER_MODEL_PATH` né `NER_MODEL_REPO`, la build esegue semplicemente il solo rilevamento
strutturato.

</details>

<details>
<summary><b>GLiNER (entità contestuali / open-label) — <code>--features onnx</code>, opt-in</b></summary>

<br>

Un **secondo** motore ML opzionale (M8). GLiNER è un *estrattore di span zero-shot* — rileva la PII
contestuale e open-label che il layer deterministico non riesce ad ancorare e che il NER XLM-R non
copre: un **telefono nazionale** senza `+CC`, un **indirizzo** libero. È **disattivato di default e non
è un successore** di XLM-R — sul modello int8 la sua recall sui nomi è più bassa — quindi *aggiunge* al
NER invece di sostituirlo (decisione misurata: `docs/DEVLOG.md` 2026-07-19).

| Variabile | Default | Scopo |
|---|---|---|
| `GLINER_MODEL_PATH` + `GLINER_TOKENIZER_PATH` + `GLINER_CONFIG_PATH` | *(non impostate)* | File locali espliciti (`.onnx` + `tokenizer.json` + il `gliner_config.json` del modello) — **zero chiamate in uscita**. Non impostate = GLiNER spento |
| `GLINER_LABELS` | `person,organization,location,phone number,address` | Tipi di entità in linguaggio naturale (separati da virgola); ognuno mappa a un `PiiKind` (un'etichetta non mappabile è rifiutata) |
| `GLINER_THRESHOLD` | `0.15` | Soglia di probabilità per span — bassa perché le confidenze del modello int8 sono basse (misurato); più bassa = più recall, più over-mask |
| `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` | `1` / *derivato* | Pool di sessioni + thread per sessione, stessa forma delle manopole del NER (si moltiplicano) |

Il modello testato è `onnx-community/gliner_multi_pii-v1` (int8 `model_quantized.onnx`). Per **recall più
alta sui nomi** a più RAM, punta invece `GLINER_MODEL_PATH` a `model_fp16.onnx` (~580 MB) — recall Person
misurata **0.67 vs 0.58 di int8** (fp32 non dà guadagni ulteriori su CPU, dove ORT up-casta fp16→fp32).
Nessuna raggiunge lo 0.83 di XLM-R, ed è per questo che GLiNER resta opt-in. Abilitare GLiNER **insieme**
al NER XLM-R carica **due** modelli — considera la RAM di conseguenza.

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
cargo test         # suite solo strutturato  (target/)
cargo test-onnx    # + il percorso NER       (target/onnx/)
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
