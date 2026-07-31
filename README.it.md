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
[![Categorie PII](https://img.shields.io/badge/categorie%20PII-10-green.svg)](#cosa-maschera)

[Avvio rapido](#avvio-rapido) · [Cosa maschera](#cosa-maschera) · [Come funziona](#come-funziona) · [Configurazione](#configurazione) · [Architettura](docs/ARCHITECTURE.md)

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

## Cosa maschera

**Dieci categorie**, ognuna diventa un segnaposto tipizzato e numerato. Lo stesso valore riceve
sempre lo stesso token all'interno di una richiesta, così una conversazione mascherata resta
comprensibile per il modello.

| categoria | segnaposto | motore | cosa fa scattare il match | copertura |
|---|---|---|---|---|
| **Email** | `[EMAIL_1]` | deterministico · sempre attivo | forma dell'indirizzo, su confini di parola ASCII | universale |
| **Carta di credito** | `[CARD_1]` | deterministico · sempre attivo | checksum di **Luhn** | universale |
| **IBAN** | `[IBAN_1]` | deterministico · sempre attivo | **mod-97** + lunghezza del paese | universale |
| **Secret / chiave API** | `[SECRET_1]` | deterministico · sempre attivo | prefisso dell'emittente — `sk-…`, `sk-ant-…`, `AKIA…` | universale |
| **Telefono** | `[PHONE_1]` | deterministico · sempre attivo | il **piano di numerazione reale** del paese dice che il numero è *assegnato* | `+CC` e USA sempre · **9 piani nazionali** |
| **SSN (USA)** | `[SSN_1]` | deterministico · sempre attivo | regole area / group / serial | 🇺🇸 |
| **Documento nazionale** | `[NATID_1]` | deterministico · sempre attivo | il checksum proprio di quello schema — mod-23, mod-97, mod-11, ISO 7064, regole NINO | **10 paesi** |
| **Persona** | `[PERSON_1]` | **ML — richiede un modello** | il modello legge la frase: un nome non ha forma verificabile | 10 lingue · recall **0,83** |
| **Organizzazione** | `[ORG_1]` | **ML — richiede un modello** | ″ | 10 lingue |
| **Luogo** | `[LOCATION_1]` | **ML — richiede un modello** | ″ | 10 lingue |

**La divisione è la verificabilità, non l'importanza.** Un IBAN è mascherato perché il mod-97
*torna*, non perché somigli a un IBAN — quelle sette sono dimostrabili, indipendenti dalla lingua
e costano ~20 ms. Un nome non lo conferma nessuna regola, quindi serve un modello, ed è lì che sta
tutto il costo (~4,7 s, ~563 MB) — ed è lì che la garanzia si indebolisce: **senza modello quelle
tre viaggiano in chiaro**, ed è ciò che `NER_REQUIRED=1` trasforma in un errore all'avvio.

**I due girano affiancati, non uno dopo l'altro.** Entrambi scandiscono tutto il testo e le loro
annotazioni vengono unite; dove si contendono gli stessi caratteri vince il deterministico.
Nessuna categoria ha due motori — il NER emette solo Persona/Organizzazione/Luogo e non potrebbe
produrre un'email nemmeno volendo. Quale livello faccia di più dipende interamente dal traffico:
un risultato SQL o un export CSV è quasi tutto deterministico, un turno di chat ordinario è in
gran parte NER.

**I 10 schemi di documento nazionale:** 🇺🇸 SSN · 🇮🇹 Codice Fiscale · 🇬🇧 NINO · 🇪🇸 DNI/NIE ·
🇫🇷 NIR · 🇩🇪 Steuer-ID · 🇳🇱 BSN · 🇵🇹 NIF · 🇱🇻 codice personale · 🇨🇳 Resident ID. Mascherati
**indipendentemente dalla configurazione dei locale** — un documento che arriva al proxy viene
mascherato anche se il suo paese non è quello che hai configurato. Il documento di un undicesimo
paese non viene riconosciuto: qui la copertura è un elenco, non una regola, e *uno schema non
misurato non è uno schema che spediamo*.

**Telefoni nazionali — 9 regioni, attive di default:** 🇩🇪 🇪🇸 🇫🇷 🇬🇧 🇮🇹 🇱🇻 🇳🇱 🇵🇹 🇨🇳 numeri
scritti **senza `+CC`** (`020 7946 0958`, `06 69821234`, `347 1234567`, `91 123 45 67`). È l'unico
livello che `PII_LOCALES` restringe. Un numero nazionale nudo collide con qualunque sequenza di
cifre, quindi ciò che lo rende sicuro non è il pattern ma il controllo: la libreria Rust
[`phonenumber`](https://crates.io/crates/phonenumber) conferma che il candidato è un numero
realmente **assegnato** per quella regione. Misurato su 35 rese reali e 453 non-telefoni a forma
di cifre: **recall 1.000**, zero falsi positivi su porte, importi e numeri di riferimento, e
**zero span `Phone`** su un turno reale di Claude Code da 22 KiB.

> I costi sono pubblicati, non arrotondati: una data separata da spazi (`28 01 2026` è un numero
> lettone valido) o la colonna di una tabella numerica (`512 105 205` è la forma di un fisso di
> Suzhou) possono essere mascherate. La matrice completa per categoria, e perché il compromesso è
> stato preso, sono in
> [ARCHITECTURE → *Domestic phone coverage*](docs/ARCHITECTURE.md#domestic-phone-coverage--re-measured-2026-07-29-m10).

---

## Avvio rapido

### Esegui un binario rilasciato

Scarica l'asset della tua piattaforma dall'[ultima
release](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest): un eseguibile,
nessun installer, niente da scompattare. Prendi il nome senza suffisso, a meno che tu non voglia
una variante `-cuda` / `-webgpu` (vedi [Accelerazione GPU](#accelerazione-gpu)).

```powershell
# Windows
Move-Item .\llm-proxy-pii-rust-x86_64-pc-windows-msvc.exe .\llm-proxy-pii-rust.exe

$env:NER_MODEL_REPO   = "jiting/xlm-roberta-base-ner-hrl_onnx"
$env:NER_REQUIRED     = "1"
$env:UPSTREAM_API_KEY = "sk-..."    # opzionale — l'header del client vince
.\llm-proxy-pii-rust.exe
```

```sh
# Linux / macOS  ·  su macOS anche: xattr -d com.apple.quarantine ./llm-proxy-pii-rust
mv llm-proxy-pii-rust-x86_64-unknown-linux-gnu llm-proxy-pii-rust
chmod +x llm-proxy-pii-rust

export NER_MODEL_REPO=jiting/xlm-roberta-base-ner-hrl_onnx
export NER_REQUIRED=1
export UPSTREAM_API_KEY=sk-...
./llm-proxy-pii-rust
```

Due righe all'avvio ti dicono **quale proxy hai davvero ottenuto**:

```text
2026-07-29T12:37:04.844780+02:00  INFO … ONNX NER detector loaded model="…model_quantized.onnx" pool_size=1 …
2026-07-29T12:37:04.885031+02:00  INFO … listening on http://127.0.0.1:8080
```

| | |
|---|---|
| `NER_MODEL_REPO` | **Quella che non puoi saltare.** Il modello non è incluso nel binario, e senza di esso il proxy parte lo stesso — mascherando solo le PII strutturate e mandando nomi, organizzazioni e luoghi in chiaro. Download una tantum, a revisione fissata, nella cache HuggingFace |
| `NER_REQUIRED=1` | Trasforma quel degrado silenzioso in un errore all'avvio. Se la prima riga qui sopra manca, stai girando in solo-strutturata |
| `PII_LOCALES` | Non serve — tutte e nove le regioni dei telefoni nazionali sono attive di default. Impostala solo per *restringere* l'insieme. I documenti nazionali restano attivi comunque |

Non c'è file di configurazione: si configura solo da ambiente, e la tabella completa è in
[Configurazione](#configurazione). `--version` dice quale build hai, `--help` elenca ogni
variabile, e `RUST_LOG` (default `info`) regola il livello di log.

### Compila dai sorgenti

Richiede Rust **1.89+**.

```sh
cargo build-onnx --release
UPSTREAM_API_KEY=sk-... ./target/onnx/release/llm-proxy-pii-rust
```

> `build-onnx` è un alias del repo per `build --features onnx --target-dir target/onnx`. L'ibrido
> compila in una **directory sua**, così un `cargo build`/`cargo test` normale — che scrive lo
> stesso nome file sotto `target/` — non può mai sovrascriverlo con un binario solo-strutturato.
> Anche la build di default funziona, e lascia fuori il NER.

### In ogni caso — parlagli esattamente come al provider vero

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user",
       "content":"email jane@example.com about invoice IT60X0542811101000000123456"}]}'
```

**Quello che il provider ha davvero ricevuto:**

```json
{"role":"user","content":"email [EMAIL_1] about invoice [IBAN_1]"}
```

**Quello che il tuo client si è ripreso:** la risposta, con `jane@example.com` e l'IBAN
ripristinati.

> Il proxy non ha bisogno di custodire la tua chiave: un header `Authorization` del client vince
> sempre su `UPSTREAM_API_KEY`, quindi puoi passare il tuo token per richiesta e lasciare il proxy
> senza credenziali.

---

## Come funziona

1. **Rileva** — i riconoscitori deterministici scandiscono ogni campo testuale dello schema
   (`content`, `name`, argomenti delle tool-call, descrizioni dei tool, descrizioni annidate dei
   parametri), più il NER per nomi / organizzazioni / luoghi.
2. **Maschera** — ogni valore diventa un segnaposto tipizzato, registrato in un vault per
   richiesta. Il mascheramento gira **fino a punto fisso**, perché sostituire un valore può
   scoprirne un altro.
3. **Istruisce il modello** — viene iniettata un'istruzione di sistema che gli dice che i
   segnaposto stanno per dati reali e vanno riusati alla lettera, anche negli argomenti dei tool.
4. **Inoltra** — il provider vede segnaposto e nient'altro.
5. **Ripristina** — il vault rimette i valori reali: nelle risposte bufferizzate, in
   `tool_calls[].function.arguments`, e **incrementalmente negli stream SSE** (un segnaposto
   spezzato tra due chunk, `[EMA` + `IL_1]`, si ricompone lo stesso).

### L'asticella che si è data

- **Fail closed.** Una richiesta di forma illeggibile, un tipo di blocco sconosciuto, un rilevatore
  obbligatorio che fallisce, un mascheramento che non raggiunge un punto fisso stabile, o una
  richiesta così densa di cifre da esaurire il budget di validazione telefonica: tutti **bloccano
  la richiesta (400)** invece di inoltrare qualcosa di cui non si conosce lo stato PII. Solo
  `POST /v1/chat/completions` (e `POST /v1/messages` con `UPSTREAM_PROVIDER=anthropic`) viene
  proxato — tutto il resto è `404`, mai inoltrato.
- **Mai PII in chiaro nei log.** I log portano categorie, conteggi e segnaposto — mai valori.
  Garantito da un test, non da una convenzione.
- **Lineare sotto carico, limitato per richiesta.** Il percorso di mascheramento è dimostrabilmente
  lineare nella *dimensione* del campo e nel *numero* di entità, e il lavoro CPU-bound gira fuori
  dall'executor async. Il budget di validazione telefonica è per **intera richiesta**, quindi
  nessun client può comprarsi più CPU spezzando il corpo su più campi; su ogni forma misurata una
  richiesta costa al massimo circa **3 s**. *Un proxy giù non protegge nulla.*
- **Deterministico.** Lo stesso valore mappa sempre sullo stesso segnaposto dentro una richiesta,
  così le conversazioni multi-turno stateless restano coerenti.

> **Il budget è raggiungibile da un corpo grande e perfettamente legale — meglio saperlo prima di
> incontrarlo.** Conta *numeri di telefono*, non byte: circa **62.500** per richiesta se scritti
> nella resa raggruppata comune (`320 123 4567`), circa 500.000 nella resa più economica. Su quale
> dimensione in byte cada dipende da cosa gli sta intorno — gli stessi 62.500 numeri vengono
> rifiutati a **793 KB** come colonna nuda, **2 MB** come `name,phone`, **4,4 MB** come export a
> sei colonne. Un normale export da 5.000 righe ne spende l'**8%**; un turno di chat tipico, nulla.
> Riprovare senza modifiche fallisce allo stesso modo: manda testo meno denso di cifre (un `LIMIT`
> sulla query dietro un tool result sovradimensionato, meno righe per chiamata).

---

## Impronta e latenza — misurate, non dichiarate

Memoria residente (Windows, build debug, a riposo dopo l'avvio):

| build | private | working set | cosa domina |
|---|---|---|---|
| **solo strutturata** (feature di default) | **~10 MB** | ~36 MB | niente — sono regex |
| **ibrida**, default `NER_POOL_SIZE=1` | **~563 MB** | ~585 MB | il NER: **una** sessione ONNX |
| **ibrida**, `NER_POOL_SIZE=2` | ~834 MB | ~856 MB | due sessioni — ognuna aggiunge **~270 MB** |
| **ibrida + GLiNER int8** soltanto (opt-in) | ~537 MB | ~560 MB | l'unica sessione di GLiNER |
| **ibrida, XLM-R + GLiNER int8** (entrambi, opt-in) | **~1073 MB** | ~1094 MB | **due modelli caricati** |

Il livello deterministico è praticamente gratis; **il modello è tutto il costo**, e scala col pool
(~290 MB di base condivisa + ~270 MB per sessione). Quindi scegli la build in base alla minaccia
che hai davvero: la solo-strutturata copre già sette categorie su dieci ed è indipendente dalla
lingua — il NER ti compra nomi, organizzazioni e luoghi, nient'altro.

Mascheramento di un **turno reale di Claude Code** (22,3 KiB: system prompt + 10 schemi di tool +
un messaggio utente), laptop di riferimento (Ryzen 5 PRO 8540U) sul suo profilo di risparmio
energetico ordinario, build debug:

| rilevamento | per turno | per KiB |
|---|---|---|
| **solo strutturata** | **~20 ms** | ~0,9 ms |
| **ibrida** | **~4,7 s** | ~210 ms |

**Il NER è ~100% del costo** — il livello deterministico è oltre 100× più veloce sugli stessi byte.
Tratta `~4,7 s` come il numero *di questa macchina*: su sette esecuzioni dello stesso codice e
della stessa fixture l'intervallo è stato **2,5–7,1 s**, e 4,7 è la cifra che si è riprodotta due
volte, indipendentemente, in isolamento. Ciò che è stabile è il *miglioramento*, perché è la parte
che riguarda il codice: il default rilasciato è **almeno 1,5× più veloce** della versione
pre-threading — il pavimento che il test asserisce davvero.

> **Misura sulla tua macchina:** `cargo test-onnx --test m7_latency -- --ignored --nocapture
> --test-threads=1`. Il `--test-threads=1` è portante — senza, cargo esegue i benchmark in
> parallelo e si misurano a vicenda (1,5×). L'harness stampa uno sweep sui thread, i numeri di
> concorrenza e un rapporto di speedup contro una gamba di calibrazione misurata nella stessa
> esecuzione. Perché `NER_POOL_SIZE` vale `1` di default, e quando un proxy centralizzante dovrebbe
> alzarlo (~30% di throughput in più a `N×` la RAM del modello):
> [ARCHITECTURE → *NER threading*](docs/ARCHITECTURE.md).

---

## Accelerazione GPU

Il NER può girare su GPU invece che su CPU. **Se sia più veloce è una domanda sul tuo hardware,
quindi lo strumento la misura invece di risponderti a priori** — `--bench-providers` esegue la
matrice modello × provider sulla tua macchina e nomina il vincitore. Funziona in *ogni* build:

```powershell
llm-proxy-pii-rust.exe --bench-providers
```

**Un binario di release per backend — scarica quello che la tua macchina sa eseguire.** Una singola
distribuzione di ONNX Runtime porta un solo insieme di execution provider, quindi nessuna build può
contenerli tutti.

| la tua macchina | scarica | acceleratore che ottieni |
|---|---|---|
| **Windows x64** | `…-x86_64-pc-windows-msvc` | **DirectML** — qualsiasi GPU DX12, anche integrata |
| Windows x64, NVIDIA | `…-x86_64-pc-windows-msvc-cuda` | **CUDA** |
| Windows x64, cross-vendor | `…-x86_64-pc-windows-msvc-webgpu` | **WebGPU** |
| **Windows arm64** | `…-aarch64-pc-windows-msvc` | **DirectML** |
| **macOS (Apple Silicon)** | `…-aarch64-apple-darwin` | **CoreML** — Neural Engine / GPU |
| macOS, cross-vendor | `…-aarch64-apple-darwin-webgpu` | **WebGPU** (Metal) |
| **Linux x64** | `…-x86_64-unknown-linux-gnu` | solo CPU |
| Linux x64, NVIDIA | `…-x86_64-unknown-linux-gnu-cuda` | **CUDA** |
| Linux x64, cross-vendor | `…-x86_64-unknown-linux-gnu-webgpu` | **WebGPU** (Vulkan) |
| **Linux arm64** | `…-aarch64-unknown-linux-gnu` | solo CPU |

La riga in **grassetto** è la scelta di default per ogni piattaforma; DirectML e CoreML sono già
dentro, gratis. Mancano le righe dove ONNX Runtime non pubblica un prebuilt (niente CUDA/WebGPU su
arm64; niente ROCm o OpenVINO da nessuna parte).

Qualunque tu esegua, il backend resta **spento finché non lo chiedi** (`NER_EXECUTION_PROVIDER`) e
ricade su CPU se il dispositivo non c'è. **Sulla macchina di riferimento di questo progetto (iGPU
AMD DX12) vince la CPU** e resta il default — DirectML-fp16 è uscito 1,45× a seq 128 ma **0,38× a
seq 512**, che è la lunghezza che conta perché i campi sono spezzati a 480 token. Una GPU discreta
ha 10–20× la banda e con ogni probabilità vincerebbe: è esattamente per questo che esiste il
benchmark e non una dichiarazione.

> Due trappole in cui il report non ti lascia cadere — **backend e quantizzazione sono accoppiati**
> (int8 è un formato da CPU; misurare il modello int8 rilasciato su GPU fa sembrare lenta 2–5×
> anche una buona GPU, quindi passa un export fp16 con `NER_BENCH_MODELS`) e **la decisione si
> prende a seq 512**, non sulla media. Ragionamento completo, matrice degli artefatti e cosa `ok`
> **non** significa: [ARCHITECTURE → *Execution providers*](docs/ARCHITECTURE.md).

---

## Provider e client

Il proxy parla lo schema OpenAI Chat Completions su *entrambi* i lati, più il `/v1/messages` nativo
di Anthropic. Quindi *"funziona con X?"* sono due domande: dove **inoltra**, e chi può
**parlargli**.

**Upstream — dove inoltra.** Qualsiasi endpoint compatibile OpenAI. Un preset `UPSTREAM_PROVIDER`
sceglie la *busta* giusta per un provider noto; il mascheramento è **identico** in ogni caso — il
preset cambia il routing, mai cosa viene ripulito.

| `UPSTREAM_PROVIDER` | Path chat | Header del client inoltrati |
|---|---|---|
| `openai` (default) | `/v1/chat/completions` | `openai-organization`, `openai-project` |
| `copilot` | `/chat/completions` *(senza `/v1`)* | `editor-version`, `editor-plugin-version`, `copilot-integration-id`, `openai-intent` |
| `anthropic` | `/v1/chat/completions` | `anthropic-version`, `anthropic-beta` |

…oppure qualsiasi altro endpoint compatibile OpenAI via `UPSTREAM_BASE_URL` (modelli locali dietro
Ollama / vLLM / LM Studio, Groq, Mistral, …). Ogni parte è sovrascrivibile (`UPSTREAM_CHAT_PATH`,
`UPSTREAM_FORWARD_HEADERS`, `UPSTREAM_EXTRA_HEADERS`).

**Client — chi può parlargli.** Un client funziona se puoi puntarlo a un base URL compatibile
OpenAI. Quasi tutti gli agent moderni possono, via BYOK o provider custom. *Verificato il
2026-07-15 — il supporto BYOK in questi strumenti si muove in fretta; controlla la loro
documentazione attuale prima di farci affidamento.*

| Client | Passa dal proxy? | Come |
|---|---|---|
| SDK OpenAI · `curl` · Cline · Continue | ✅ | imposta il `base_url` del client sul proxy |
| **opencode** | ✅ | un provider custom `@ai-sdk/openai-compatible`, `baseURL: http://127.0.0.1:8080/v1` |
| **pi** (`@earendil-works/pi`) | ✅ | un provider con `"api": "openai-completions"` |
| **GitHub Copilot CLI** | ✅ | BYOK: `COPILOT_PROVIDER_BASE_URL=http://127.0.0.1:8080/v1` |
| **GitHub Copilot Chat** (VS Code) | ✅ | BYOK → un provider "OpenAI Compatible" (solo chat) |
| **Claude Code** · SDK Anthropic | ✅ | `UPSTREAM_PROVIDER=anthropic`; il corpo nativo `/v1/messages` è mascherato **sul posto**, senza traduzione OpenAI. Verificato dal vivo end-to-end contro Anthropic reale |
| **Client nativi di altri vendor** — Gemini CLI, SDK nativi Bedrock / Vertex | ❌ | parlano un protocollo **nativo** del vendor che il proxy non serve |

> **Esistono solo due superfici native: compatibile OpenAI e `/v1/messages` di Anthropic.** Un agent
> cablato sul protocollo nativo di qualunque altro vendor **non** è mascherato — il caso concreto
> oggi è la Gemini CLI. Servirebbe un adattatore per-provider consapevole dello schema, e un campo
> di schema mancato è una fuga: quindi è lavoro deliberatamente **non pianificato**, tracciato come
> [Option B — native provider adapters](docs/ROADMAP.md#backlog). Fino ad allora, metti davanti a un
> agent del genere una modalità compatibile OpenAI se ce l'ha; se parla solo l'API nativa del suo
> vendor, il proxy non può ancora proteggerlo.

---

## Configurazione

Tutto guidato da ambiente.

| Variabile | Default | Scopo |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Indirizzo su cui il proxy ascolta |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | Base URL del provider upstream |
| `UPSTREAM_API_KEY` | *(non impostata)* | Iniettata come `Authorization: Bearer …` **solo** se il client non ne manda una sua |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — preset di routing (`anthropic` abilita anche la rotta nativa `/v1/messages`) |
| `UPSTREAM_CHAT_PATH` | *(dal preset)* | Sovrascrive il path chat-completions |
| `UPSTREAM_MESSAGES_PATH` | `/v1/messages` | Sovrascrive il path Messages nativo di Anthropic (usato solo con `UPSTREAM_PROVIDER=anthropic`) |
| `UPSTREAM_FORWARD_HEADERS` | *(dal preset)* | Header del client da inoltrare, separati da virgola |
| `UPSTREAM_EXTRA_HEADERS` | *(nessuno)* | `Key=Value;Key2=Value2` statici su ogni richiesta upstream |
| `MAX_BODY_BYTES` | `16777216` | Limite del corpo richiesta (16 MiB) |
| `PII_LOCALES` | `de,es,fr,gb,it,lv,nl,pt,cn` | Le regioni di cui si rilevano i **telefoni nazionali** (senza `+CC`). Impostarla **sostituisce** l'insieme invece di aggiungersi; impostala **vuota** per spegnere il livello — cosa diversa dal lasciarla non impostata. **I documenti nazionali restano sempre attivi** |
| `PII_CACHE_ENTRIES` | `16` | Cache di rilevamento: un system prompt identico byte a byte viene scandito una volta sola e riusato, risparmiando il passaggio NER dominante. Chiave sui byte esatti, così un hit non può mai mascherare *meno*. `0` la disabilita |
| `RUST_LOG` | `info` | Filtro di log. I timestamp sono ora locale con offset esplicito |

<details>
<summary><b>NER (entità non strutturate) — <code>--features onnx</code></b></summary>

<br>

| Variabile | Default | Scopo |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(non impostate)* | File del modello locali ed espliciti — **zero chiamate in uscita**, vincono sempre se impostati |
| `NER_MODEL_REPO` | *(non impostata)* | Download automatico opt-in (`owner/name`) di un modello a revisione fissata nella cache HuggingFace standard. È l'unica chiamata in uscita dell'intero strumento, fatta una volta all'avvio, e scarica **artefatti del modello, non dati utente** |
| `NER_MODEL_REVISION` | `478a2a3` | Revisione fissata per il download automatico |
| `NER_POOL_SIZE` | `1` | Dimensione del pool di sessioni ONNX. **Default `1`** = la forma single-client (~563 MB, tutta la macchina per richiesta). Alzalo a **`N`** per un proxy centralizzante: ~30% di throughput in più a ~270 MB di RAM in più per sessione |
| `NER_INTRA_THREADS` | *derivato* | Thread **per sessione**. Default `max(1, core / NER_POOL_SIZE)` — le due manopole si **moltiplicano**, e il prodotto deve stare nella macchina |
| `NER_EXECUTION_PROVIDER` | `cpu` | Backend hardware per le sessioni NER + GLiNER. `cpu` è il default e l'unico che ha superato la guardia di determinismo. L'acceleratore della tua piattaforma è **già dentro la build `onnx`**; un provider assente dalla build, o il cui dispositivo manca, **ricade su CPU** (loggato), mai un errore d'avvio — ma un errore di battitura sì. **Non tirare a indovinare: esegui `--bench-providers`** |
| `NER_BENCH_MODELS` | *(non impostata)* | File di modello aggiuntivi (separati da virgola) da confrontare con quello configurato in `--bench-providers` — es. un export fp16 accanto all'int8 rilasciato |
| `NER_REQUIRED` | off | **Fail closed sui nomi**: almeno un rilevatore ML deve caricarsi, e ognuno di quelli caricati gira senza rete di protezione — un fallimento blocca la richiesta (400) invece di degradare in silenzio a solo-strutturata |

Senza né `NER_MODEL_PATH` né `NER_MODEL_REPO`, la build gira semplicemente in solo-strutturata.

</details>

<details>
<summary><b>GLiNER (entità contestuali / a etichetta aperta) — <code>--features onnx</code>, opt-in</b></summary>

<br>

Un **secondo** motore ML, opzionale: un estrattore di span zero-shot per le PII contestuali e a
etichetta aperta che il livello deterministico non può ancorare e che il NER XLM-R non copre (un
telefono nazionale nudo, un indirizzo in forma libera). È **spento di default e non è un
successore** di XLM-R — sul modello int8 rilasciato la sua recall sui nomi è più bassa (0,58 contro
0,83 di XLM-R) — quindi si *aggiunge* al NER invece di sostituirlo, e abilitarli entrambi carica
**due** modelli (~1,07 GB).

| Variabile | Default | Scopo |
|---|---|---|
| `GLINER_MODEL_PATH` + `GLINER_TOKENIZER_PATH` + `GLINER_CONFIG_PATH` | *(non impostate)* | File locali espliciti — **zero chiamate in uscita**. Non impostate = GLiNER spento |
| `GLINER_LABELS` | `person,organization,location,phone number,address` | Tipi di entità in linguaggio naturale; ognuno mappa su un `PiiKind` (un'etichetta non mappabile viene rifiutata) |
| `GLINER_THRESHOLD` | `0.15` | Soglia di probabilità per span — bassa perché le confidenze del modello int8 sono basse; più bassa = più recall, più over-mask |
| `GLINER_POOL_SIZE` / `GLINER_INTRA_THREADS` | `1` / *derivato* | Pool di sessioni + thread per sessione, stessa forma delle manopole del NER |

`onnx-community/gliner_multi_pii-v1` (int8) è il modello testato; `model_fp16.onnx` alza la recall
su Persona a 0,67 al costo di più RAM. La decisione misurata è nel
[DEVLOG 2026-07-19](docs/DEVLOG.md).

</details>

<details>
<summary><b>Debug — guarda il mascheramento con i tuoi occhi</b></summary>

<br>

| Variabile | Scopo |
|---|---|
| `PII_DEBUG_SKIP_DEMASK` | Salta il de-mask della risposta, così il tuo client riceve **i segnaposto che ha visto il provider** — prova diretta che il giro è cablato. Stampa un avviso rumoroso all'avvio. Mai in produzione |
| `RUST_LOG=llm_proxy_pii_rust=trace` | Logga i byte esatti (mascherati) mandati upstream. La risposta de-mascherata non è **mai** loggata |

Esegui lo stesso prompt due volte — una con `PII_DEBUG_SKIP_DEMASK=1`, una senza — e confronta.
Procedura completa: [`docs/MANUAL_VERIFICATION.md`](docs/MANUAL_VERIFICATION.md).

</details>

---

## Documentazione

| | |
|---|---|
| [Changelog](CHANGELOG.md) | Cosa è cambiato in ogni release — ed è ciò che mostra la pagina Release |
| [Architettura e invarianti](docs/ARCHITECTURE.md) | Come funziona, e **cosa non deve mai rompersi** |
| [Strategia di test](docs/TESTING.md) | Le guardie, e perché esiste ciascuna |
| [Verifica manuale](docs/MANUAL_VERIFICATION.md) | Dimostra la catena end-to-end contro un provider reale |
| [Roadmap](docs/ROADMAP.md) · [Diario di sviluppo](docs/DEVLOG.md) | Cosa viene dopo; come si è arrivati a ogni decisione |
| [Setup di sviluppo](docs/SETUP.md) | Toolchain (incl. Windows, senza admin) |

```sh
cargo test         # suite solo-strutturata  (target/)
cargo test-onnx    # + il percorso NER       (target/onnx/)
```

> La documentazione sotto `docs/` è **solo in inglese** ed è canonica; questa pagina e
> [CONTRIBUTING.it.md](CONTRIBUTING.it.md) sono le uniche traduzioni.

---

## Licenza

Copyright (C) 2026 Francesco Stimola.

Doppia licenza:

- **Open source** — [GNU Affero General Public License v3.0 o successiva](LICENSE)
  (`AGPL-3.0-or-later`). Trattandosi di un proxy per la privacy servito in rete, l'AGPL garantisce
  che chi esegue come servizio una versione **modificata** condivida le proprie modifiche.
  Eseguirlo non modificato non comporta alcun obbligo del genere — per nessuno, aziende incluse;
  l'AGPL in sé è libera, sempre.
- **Commerciale** — per i casi che l'AGPL non copre per te (incorporare una versione modificata, o
  linkare il crate in un prodotto closed-source, senza l'obbligo di network-copyleft), è
  disponibile una licenza commerciale separata. [Apri una issue di licenza
  commerciale](https://github.com/francesco-stimola/llm-proxy-pii-rust/issues/new?template=commercial-license.md)
  per parlarne.

I contributi sono benvenuti secondo i termini in [CONTRIBUTING.it.md](CONTRIBUTING.it.md), che è
ciò che rende possibile la doppia licenza.

---

<div align="center">

🇬🇧 [Read in English](README.md)

</div>
