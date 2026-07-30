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

### Esegui un binario rilasciato

Scarica l'asset della tua piattaforma dall'[ultima
release](https://github.com/francesco-stimola/llm-proxy-pii-rust/releases/latest): un eseguibile,
nessun installer, niente da scompattare. Prendi il nome senza suffisso, a meno che tu non voglia
una variante `-cuda` / `-webgpu` (vedi *Accelerazione GPU*, più sotto).

```powershell
# Windows
Move-Item .\llm-proxy-pii-rust-x86_64-pc-windows-msvc.exe .\llm-proxy-pii-rust.exe

$env:NER_MODEL_REPO   = "jiting/xlm-roberta-base-ner-hrl_onnx"
$env:NER_REQUIRED     = "1"
$env:UPSTREAM_API_KEY = "sk-..."    # opzionale — l'header del client ha la precedenza
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

A quel punto due righe all'avvio ti dicono quale proxy hai davvero ottenuto:

```text
2026-07-29T12:37:04.844780+02:00  INFO … ONNX NER detector loaded model="…model_quantized.onnx" pool_size=1 …
2026-07-29T12:37:04.885031+02:00  INFO … listening on http://127.0.0.1:8080
```

> I timestamp sono in **ora locale con l'offset esplicito**, e il livello di log di default è
> `info`: un binario appena scaricato dice cosa sta facendo senza alcuna configurazione.
> `RUST_LOG` continua a prevalere (`RUST_LOG=warn`, `RUST_LOG=llm_proxy_pii_rust=debug`, …).
> `llm-proxy-pii-rust --version` dice quale build hai, e `--help` elenca tutte le variabili
> d'ambiente.

| | |
|---|---|
| `NER_MODEL_REPO` | **Quella che non puoi saltare.** Il modello non è incluso, e senza il proxy parte lo stesso — mascherando solo le PII strutturate e mandando nomi, organizzazioni e luoghi in chiaro. Fetch una tantum, fissato a una revisione, nella cache HuggingFace |
| `NER_REQUIRED=1` | Trasforma quel declassamento silenzioso in un errore d'avvio. Se manca la prima riga qui sopra, stai girando in solo-strutturato |
| `PII_LOCALES` | Non serve impostarla — tutte e nove le regioni per il telefono domestico sono attive di default. Usala solo per *restringere* l'insieme (lo sostituisce). I documenti nazionali sono sempre attivi comunque |

Non esiste un file di configurazione — la configurazione è solo ambiente, e la tabella completa è
in [Configurazione](#configurazione).

### Compila da sorgente

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

### In entrambi i casi — parlaci come faresti col provider reale

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
| **Telefono domestico** *(9 regioni, tutte attive di default)* | 🇩🇪 🇪🇸 🇫🇷 🇬🇧 🇮🇹 🇱🇻 🇳🇱 🇵🇹 🇨🇳 numeri scritti **senza `+CC`** — `020 7946 0958`, `06 69821234`, `347 1234567`, `91 123 45 67`, `01 23 45 67 89` — ognuno verificato contro il **piano di numerazione reale** di quel paese, così numeri d'ordine e documenti non vengono sovra-mascherati |

I documenti nazionali vengono mascherati **indipendentemente dalla configurazione dei locale** —
privacy-first: un documento che raggiunge il proxy viene mascherato anche se il suo paese non è
quello configurato.

Il livello **telefono domestico** è l'unico che `PII_LOCALES` governa, ed è ora attivo **da
subito** (impostala per restringere l'insieme; impostala *vuota* per spegnere il livello). Un
numero domestico nudo si confonde con normali sequenze di cifre: ciò che lo rende sicuro non è il
pattern ma il controllo — la libreria Rust puro
[`phonenumber`](https://crates.io/crates/phonenumber) conferma che il candidato è un numero
realmente **assegnato** per quella regione. Misurato su 35 rese reali e 453 non-telefoni a forma di
cifre: **recall 1.000**, zero falsi positivi su porte, importi e numeri di riferimento, e **zero
span `Phone`** su un vero turno Claude Code da 22 KiB.

Non è gratis, e i costi sono pubblicati invece che arrotondati via: date separate da spazio o
trattino (`28 01 2026` è un numero lettone valido) e tabelle numeriche separate da spazi
(`512 105 205` è la forma di un vero numero fisso di Suzhou). La matrice completa per categoria, e
perché quel compromesso è stato accettato, è in
[ARCHITECTURE → *Domestic phone coverage*](docs/ARCHITECTURE.md).

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
  sconosciuto, un rilevatore obbligatorio che fallisce, un mascheramento che non raggiunge un
  punto fisso stabile, o una **richiesta** così densa di cifre da esaurire l'ammontare di
  validazioni telefoniche **bloccano la richiesta (400)** invece di inoltrare qualcosa dallo stato
  PII sconosciuto. L'ultimo è l'unico 400 che un corpo per il resto perfettamente legale può
  provocare, ed è utile saperlo prima di incontrarlo: l'ammontare si spende sull'*intera richiesta*,
  quindi distribuire lo stesso contenuto su più campi non aiuta, e rimandarla identica fallisce allo
  stesso modo. Va mandato meno testo denso di cifre — un `LIMIT` sulla query dietro un risultato di
  tool troppo grande, meno righe per chiamata. Solo `POST /v1/chat/completions` (e `POST /v1/messages` quando
  `UPSTREAM_PROVIDER=anthropic`) è proxato — tutto il resto è `404`, mai inoltrato.
- **Mai loggare PII in chiaro.** I log riportano categorie, conteggi e segnaposto — mai i valori.
  Garantito da un test, non da una convenzione.
- **Lineare sotto carico, e limitato per richiesta.** Il percorso di mascheramento è
  dimostrabilmente lineare sia nella *dimensione* del campo che nel *numero* di entità, e il lavoro
  CPU-bound gira fuori dall'executor asincrono. Ma *lineare è una forma, non un tetto*: un corpo
  legale da 15,6 MiB di testo denso di cifre teneva occupato un worker per **57 s**, perché
  l'ammontare di validazioni telefoniche era riferito a un *campo* e quanti campi ha un corpo lo
  sceglie il client. Riferito alla **richiesta**, lo stesso corpo viene rifiutato in **1,6 s**, la
  validazione telefonica è limitata a ~1,5 s di CPU, e su ogni forma misurata una richiesta costa al
  massimo circa **3 s** — il resto è lineare nei byte e limitato da `MAX_BODY_BYTES`. Un corpo grande
  non può bloccare il proxy per tutti. *Un proxy giù non protegge nulla.*
- **L'ammontare è raggiungibile da un corpo grande e legale, ed è bene saperlo.** Un numero di
  telefono costa 1–29 unità di validazione a seconda di come è scritto, quindi un export contatti
  denso comincia a essere rifiutato intorno ai **2,6 MB** (raggruppamento `320 123 4567`) o ai
  **6 MB** (`347 1234567`). Un tool result ordinario — poniamo 367 KB — spende circa l'**1%**
  dell'ammontare, e un turno di chat tipico non spende nulla.
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

### Accelerazione GPU — misura, non indovinare (`--bench-providers`)

Il NER può girare su GPU invece che su CPU (`NER_EXECUTION_PROVIDER`, M9). **Se sia più veloce è
una domanda sul tuo hardware, quindi il tool la misura invece di risponderti a priori.**

**`--bench-providers` funziona in ogni build** — non serve una build speciale per porre la domanda:

```powershell
llm-proxy-pii-rust.exe --bench-providers
```

**Un binario di release per backend — scarichi quello che la tua macchina può eseguire.** Una
distribuzione ONNX Runtime porta un solo set di execution provider, quindi nessuna build può
contenerli tutti: la scelta si sposta su *quale artefatto prendi*. Ogni combinazione qui sotto è una
per cui ONNX Runtime pubblica davvero un prebuilt:

| la tua macchina | scarica | acceleratore che ottieni |
|---|---|---|
| **Windows x64** | `…-x86_64-pc-windows-msvc` | **DirectML** — qualsiasi GPU DX12 (AMD/NVIDIA/Intel, anche integrata) |
| Windows x64, NVIDIA | `…-x86_64-pc-windows-msvc-cuda` | **CUDA** |
| Windows x64, cross-vendor | `…-x86_64-pc-windows-msvc-webgpu` | **WebGPU** (D3D12/Vulkan via Dawn) |
| **Windows arm64** | `…-aarch64-pc-windows-msvc` | **DirectML** |
| **macOS (Apple Silicon)** | `…-aarch64-apple-darwin` | **CoreML** — Neural Engine / GPU |
| macOS, cross-vendor | `…-aarch64-apple-darwin-webgpu` | **WebGPU** (Metal via Dawn) |
| **Linux x64** | `…-x86_64-unknown-linux-gnu` | solo CPU |
| Linux x64, NVIDIA | `…-x86_64-unknown-linux-gnu-cuda` | **CUDA** |
| Linux x64, cross-vendor | `…-x86_64-unknown-linux-gnu-webgpu` | **WebGPU** (Vulkan via Dawn) |
| **Linux arm64** | `…-aarch64-unknown-linux-gnu` | solo CPU |

La riga in **grassetto** è la scelta di default per ogni piattaforma. Le righe mancanti sono quelle
per cui ONNX Runtime non ha un prebuilt: non esiste build CUDA né WebGPU per nessuna delle due
piattaforme arm64, né build ROCm o OpenVINO per **alcun** target — servirebbe un ONNX Runtime
compilato da sorgente, che questo progetto non distribuisce.

Nota che DirectML e CoreML sono gratis: stanno già dentro la distribuzione base della loro
piattaforma, quindi il binario standard li ha. CUDA e WebGPU no — cambiano quale runtime viene
scaricato, ed è esattamente per questo che sono artefatti separati invece che peso nella build di
tutti.

Qualunque tu esegua, il backend resta comunque **spento finché non lo chiedi**
(`NER_EXECUTION_PROVIDER`), ricade su CPU se il device non c'è, e `--bench-providers` ti dice se
conviene davvero usarlo. Senza la feature `onnx` il flag gira lo stesso e spiega che non c'è alcun
layer ML da accelerare.

Esegue la matrice **modello × provider** sulla tua macchina e nomina il vincitore:

Output reale dalla macchina di questo progetto (iGPU AMD DX12), abbreviato solo dove indicato:

```text
Execution-provider benchmark (M9)
  threads: 12 intra-op per session, pool 1 (of 12 cores)
  models: model_quantized

…/onnx/model_quantized.onnx
  provider     |    seq 128 |    seq 256 |    seq 512 | status
  ---------------------------------------------------------------------------------
  cpu          |      26.4ms |      45.9ms |     109.7ms | ok
  directml     |     121.4ms |     380.2ms |     691.7ms | ok

Decision is made at seq 512 — fields are chunked to 480 tokens, so a full window
runs near there, and long fields are what make a slow turn.

=> FASTEST: cpu on model_quantized (109.7ms).
   Keep the default — leave NER_EXECUTION_PROVIDER unset, and point
   NER_MODEL_PATH at that model.

[… blocchi MEASUREMENT e NOTE ON QUANTIZATION …]

  !! An int8/quantized model is in this run and NO fp16 model is, so every GPU
     row here is very likely an UNDERESTIMATE — this comparison cannot answer
     'is the GPU worth it?'. Add an fp16 export via NER_BENCH_MODELS to get the
     honest matrix (CPU-int8 vs GPU-fp16).
```

Nota cosa quella run **non** fa: non conclude che la GPU sia scarsa. Ha misurato il modello int8 di
serie, quindi lo dice e ti avvisa che il confronto è incompleto — che è tutto il punto.

Due errori che non ti lascia fare — entrambi commessi prima da questo progetto:

- **Backend e quantizzazione sono accoppiati.** int8 è un formato *CPU*, i cui op si partizionano
  male sulle GPU: misurare il modello int8 di serie su GPU fa sembrare lenta 2–5× una GPU ottima.
  fp16 è il formato GPU (su CPU viene up-castato a fp32, quindi rende solo su GPU). Passa un export
  fp16 via `NER_BENCH_MODELS` per ottenere il confronto che decide davvero — il report si rifiuta di
  presentare una run solo-int8 come risposta.
- **La decisione si prende a seq 512**, non sulla media. I campi sono spezzati in finestre da 480
  token, quindi le inferenze che dominano la latenza girano lì attorno. Un backend che vince a seq
  128 e perde a 512 non ha vinto: a seq 128 si è già abbastanza veloci che la differenza è invisibile.

Eseguilo a **macchina scarica e collegata alla rete elettrica** — una macchina occupata o in
throttling gonfia tutte le righe (misurato ~3× subito dopo una build pesante; il *ranking* ha retto,
i valori assoluti no).

**Sulla nostra macchina di riferimento (iGPU AMD DX12) vince la CPU**, che resta il default:
DirectML-fp16 ha dato 1.45× a seq 128 ma **0.38× a seq 512**. Una iGPU a memoria condivisa è
limitata dalla banda. È un fatto su *quella* iGPU — una GPU dedicata ha 10–20× la banda e molto
probabilmente vincerebbe, ed è per questo che il selettore esiste. La tua può essere diversa: serve
esattamente a questo il benchmark.

Un provider che non si inizializza non fa mai fallire l'avvio — ricade su CPU (loggato), e il
benchmark segna quella riga come `unavailable — fell back to cpu` invece di spacciare tempi CPU per
tempi GPU. Attenzione al limite di quel controllo: intercetta un provider che fallisce
l'**inizializzazione**, non il partizionamento per-nodo di ONNX Runtime, che può comunque eseguire
singoli nodi su CPU dentro un provider che si è registrato. Leggi `ok` come "il backend è stato
ingaggiato", non "ogni nodo ci ha girato sopra".

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
| `PII_LOCALES` | `de,es,fr,gb,it,lv,nl,pt,cn` | Le regioni di cui vengono rilevati i **numeri di telefono domestici** (senza `+CC`). Tutte e nove sono attive di default; impostarla **sostituisce** l'insieme invece di aggiungersi, e un codice fuori dalla lista non contribuisce nulla. Impostala **vuota** (`PII_LOCALES=`) per spegnere il livello — è diverso dal non impostarla affatto. **I documenti nazionali sono sempre attivi comunque** — vedi la matrice di copertura in [ARCHITECTURE](docs/ARCHITECTURE.md) |
| `PII_CACHE_ENTRIES` | `16` | Cache di rilevamento (S3): il system prompt byte-identico viene scansionato una volta e riusato, risparmiando la passata NER dominante. Con chiave sui byte esatti, un hit non può **mai** mascherare *meno* di una scansione fresca. `0` la disabilita |
| `RUST_LOG` | `info` | Filtro dei log. Non impostata (o vuota) significa `info`, così le righe d'avvio si vedono senza configurazione; imposta es. `warn` o `llm_proxy_pii_rust=debug` per sovrascriverlo. I timestamp sono in ora locale con offset esplicito |

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
| `NER_EXECUTION_PROVIDER` | `cpu` | Backend hardware per le sessioni NER + GLiNER (M9). `cpu` è il default ed è l'unico che ha superato il guard di determinismo. L'acceleratore della tua piattaforma è **già nella build `onnx`** (DirectML su Windows, CoreML su macOS, CUDA su Linux x86_64) — imposta qui il suo nome per usarlo. Un provider assente dalla build, o il cui device manca, **ricade su CPU** (loggato, con `requested=` nella riga di avvio), mai un fallimento all'avvio; un refuso (es. `vulkan`, che non è un backend ORT) invece fa fallire l'avvio. **Solo `directml` è stato misurato** — e qui ha perso contro la CPU. **Non tirare a indovinare — usa `--bench-providers`** (sotto) |
| `NER_BENCH_MODELS` | *(non impostato)* | Modelli aggiuntivi (separati da virgola) che `--bench-providers` confronta con quello configurato — es. un export fp16 accanto all'int8 di serie. Backend e quantizzazione sono **accoppiati**, quindi è così che si ottiene il confronto che decide davvero |
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

Doppia licenza:

- **Open source** — [GNU Affero General Public License v3.0 o successiva](LICENSE)
  (`AGPL-3.0-or-later`). Trattandosi di un proxy privacy servito in rete, l'AGPL garantisce che
  chi esegue una versione **modificata** come servizio debba condividerne le modifiche. Eseguirlo
  non modificato non comporta alcun obbligo — per nessuno, aziende incluse; l'AGPL in sé resta
  sempre gratuita.
- **Commerciale** — per i casi che l'AGPL non copre per te (integrare una versione modificata, o
  linkare il crate dentro un prodotto closed-source, senza l'obbligo di copyleft di rete), è
  disponibile una licenza commerciale separata. [Apri una issue per licenza
  commerciale](https://github.com/francesco-stimola/llm-proxy-pii-rust/issues/new?template=commercial-license.md)
  per discuterne.

I contributi sono benvenuti secondo i termini in [CONTRIBUTING.it.md](CONTRIBUTING.it.md), che è
ciò che rende possibile la doppia licenza.

---

<div align="center">

🇬🇧 [Read in English](README.md)

</div>
