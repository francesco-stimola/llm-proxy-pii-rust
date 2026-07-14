# llm-proxy-pii-rust

Un reverse proxy veloce e attento alla privacy per API LLM compatibili con
OpenAI, scritto in Rust.

`llm-proxy-pii-rust` si posiziona tra la tua applicazione e qualsiasi provider
compatibile con OpenAI (OpenAI, GitHub Copilot, l'endpoint compatibile di
Anthropic). Rileva le informazioni personali (PII) **in locale, prima che una
richiesta lasci la tua rete**, le sostituisce con segnaposto tipizzati, inoltra
la richiesta anonimizzata al provider e ripristina i valori originali nella
risposta — così la tua applicazione vede dati reali e coerenti, mentre il
provider non li vede mai.

Fai puntare il tuo client compatibile con OpenAI esistente all'URL del proxy;
nient'altro nel tuo stack deve cambiare.

## Perché

Inviare i prompt a un LLM ospitato significa affidare a terzi tutto ciò che gli
utenti scrivono — nomi, email, numeri di telefono, documenti d'identità, numeri
di conto, chiavi API. Questo proxy mantiene sotto il tuo controllo il
rilevamento e il mascheramento, sulla tua infrastruttura, invece di affidarli al
provider. Fallisce **in modo chiuso**: davanti a un input inatteso, blocca o
oscura invece di rischiare di inoltrare qualcosa in chiaro.

## Cosa fa

- **PII strutturate (deterministiche, sempre attive)** — email, telefono (US +
  `+CC`), carta di credito (Luhn), IBAN (mod-97 + lunghezza per paese), chiavi
  API/segreti (`sk-…`, `sk-ant-…`, `AKIA…`) e documenti d'identità nazionali per
  dieci paesi (SSN US, Codice Fiscale IT, NINO GB, DNI/NIE ES, NIR FR, Steuer-ID
  DE, BSN NL, NIF PT, codice personale LV, documento di residenza zh) — ognuno
  validato con checksum o regole specifiche per un tasso di falsi positivi quasi
  nullo, e mascherato indipendentemente dalla configurazione dei locale
  (privacy-first: un documento nazionale che raggiunge il proxy viene mascherato
  anche se il suo paese non è configurato).
- **Entità non strutturate (ML opzionale, feature `onnx`)** — persone,
  organizzazioni e luoghi tramite un modello NER ONNX locale (XLM-R int8), con
  copertura per arabo, tedesco, inglese, spagnolo, francese, italiano, lettone,
  olandese, portoghese e cinese. CPU-first; disattivato di default così il
  binario distribuito resta privo di dipendenze native.
- **Anonimizzazione reversibile e deterministica** — i valori rilevati diventano
  segnaposto tipizzati (`[EMAIL_1]`, `[PERSON_2]`, …); un vault per richiesta
  ripristina gli originali esatti nella risposta. Lo stesso valore ottiene
  sempre lo stesso token all'interno di una richiesta, così una conversazione
  multi-turno stateless (la cronologia viene reinviata e rimascherata a ogni
  turno, come fanno i client stile OpenAI) resta coerente turno dopo turno.
- **Consapevole delle tool call** — i segnaposto vengono ripristinati in
  `tool_calls[].function.arguments` prima che il tuo client esegua uno
  strumento, e rimascherati nei messaggi di risultato `tool` prima che tornino
  al provider. Un'iniezione trasparente nel system prompt dice al modello che i
  segnaposto sono sostituti di dati reali da usare alla lettera, mai da alterare
  o indovinare.
- **Streaming (SSE)** — le richieste con `stream:true` vengono mascherate
  esattamente come quelle bufferizzate e de-anonimizzate in modo incrementale
  man mano che arrivano i token, con un buffer di trattenimento così che un
  segnaposto diviso tra due chunk (`[EMA` + `IL_1]`) si risolva comunque
  correttamente.
- **Multi-provider** — un'unica impostazione `UPSTREAM_PROVIDER` instrada verso
  OpenAI, GitHub Copilot o l'endpoint compatibile con OpenAI di Anthropic. Il
  mascheramento è identico indipendentemente dal provider: i preset cambiano
  solo l'instradamento (percorso, header), mai cosa viene oscurato.
- **Fail-closed per progetto** — una forma di richiesta illeggibile, un
  rilevatore obbligatorio che fallisce, o un mascheramento che non raggiunge un
  punto fisso stabile **bloccano la richiesta (400)** invece di inoltrare
  qualcosa dallo stato PII sconosciuto. Solo `POST /v1/chat/completions` viene
  proxato; tutto il resto è `404`, mai inoltrato.

## Avvio rapido

Richiede Rust (stable, MSRV 1.82). Compila ed esegui il binario di default
(solo strutturato, privo di dipendenze native):

```sh
cargo build --release
UPSTREAM_API_KEY=sk-... ./target/release/llm-proxy-pii-rust
```

Poi fai puntare un qualsiasi client compatibile con OpenAI a
`http://127.0.0.1:8080` invece che al provider reale:

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"scrivimi a jane@example.com"}]}'
```

Il provider a monte vede solo `[EMAIL_1]`; il tuo client riceve indietro
`jane@example.com`.

Per aggiungere il rilevamento di entità non strutturate (persone,
organizzazioni, luoghi), compila con la feature `onnx` e punta a un modello —
vedi [Entità non strutturate (NER)](#entità-non-strutturate-ner-feature-onnx-opzionale)
più sotto.

## Configurazione

Tutto è pilotato da variabili d'ambiente. Principali:

| Variabile | Default | Scopo |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:8080` | Indirizzo su cui il proxy resta in ascolto |
| `UPSTREAM_BASE_URL` | `https://api.openai.com` | URL base del provider a monte |
| `UPSTREAM_API_KEY` | *(non impostata)* | Iniettata come `Authorization: Bearer …` quando il client non ne invia una propria |
| `UPSTREAM_PROVIDER` | `openai` | `openai` / `copilot` / `anthropic` — sceglie i default di instradamento (percorso chat, header inoltrati) |
| `UPSTREAM_CHAT_PATH` | *(default del preset)* | Sovrascrive il percorso delle chat completions |
| `UPSTREAM_FORWARD_HEADERS` | *(default del preset)* | Header della richiesta client, separati da virgola, da inoltrare (oltre ad `Authorization`) |
| `UPSTREAM_EXTRA_HEADERS` | *(nessuno)* | `Chiave=Valore;Chiave2=Valore2` header statici aggiunti a ogni richiesta a monte |
| `MAX_BODY_BYTES` | `16777216` (16 MiB) | Limite di dimensione del corpo della richiesta |
| `PII_LOCALES` | `it,us` | Locale per il livello opzionale di riconoscitori inclini a falsi positivi (i pacchetti di documenti nazionali sono sempre attivi indipendentemente — vedi `docs/ARCHITECTURE.md`) |
| `PII_DEBUG_SKIP_DEMASK` | disattivato | **Solo debug.** Salta il de-mascheramento della risposta così il client vede i segnaposto che il provider ha visto — prova che il round-trip è collegato. Mai attivarlo in produzione |
| `RUST_LOG` | *(non impostata)* | Filtro ambiente standard di `tracing`, es. `llm_proxy_pii_rust=debug` |

### Entità non strutturate (NER, feature `onnx` opzionale)

```sh
cargo build --release --features onnx
```

| Variabile | Default | Scopo |
|---|---|---|
| `NER_MODEL_PATH` + `NER_TOKENIZER_PATH` + `NER_LABELS` | *(non impostate)* | File di modello locali espliciti — zero chiamate in uscita, ha sempre priorità se impostato |
| `NER_MODEL_REPO` | *(non impostata)* | Download automatico opzionale (`owner/name`) di un modello con revisione fissata nella cache HuggingFace standard; unica chiamata in uscita di tutto lo strumento, fatta una volta all'avvio |
| `NER_MODEL_REVISION` | `478a2a3` | Revisione fissata per il download automatico (l'XLM-R int8 valutato) |
| `NER_POOL_SIZE` | `2` | Dimensione del pool di sessioni ONNX concorrenti |
| `NER_REQUIRED` | disattivato | Fail **closed** per i nomi: un NER mancante/fallito blocca la richiesta (400) invece di ripiegare silenziosamente sul solo strutturato |

Senza né `NER_MODEL_PATH` né `NER_MODEL_REPO` impostate, la build `onnx` esegue
semplicemente solo il rilevamento strutturato, come la build di default. Vedi
`docs/ARCHITECTURE.md` e `docs/SETUP.md` per il contratto completo di gestione
del modello.

## Stato

**M0–M4 completi**, M5 (test di integrazione e prestazioni) in corso. Dieci
pacchetti di documenti nazionali, copertura dei locale a tre livelli, streaming,
instradamento multi-provider e un percorso di mascheramento algoritmicamente
lineare (misurato — vedi `docs/TESTING.md`) sono tutti distribuiti e testati.
`v0.4.0`, pre-1.0: la prima release taggata (`1.0.0`) seguirà una volta
completato il passaggio README/CI di M5.

## Sviluppo

Documenti vivi che tracciano tutto il lavoro, così nulla si perde tra una
sessione e l'altra (tutti in inglese):

- [Development setup (Windows, no admin)](docs/SETUP.md)
- [Architecture & design decisions](docs/ARCHITECTURE.md)
- [Roadmap & milestones](docs/ROADMAP.md)
- [Testing strategy](docs/TESTING.md)
- [Manual verification runbook](docs/MANUAL_VERIFICATION.md)
- [Development log](docs/DEVLOG.md)

```sh
cargo test                    # suite di test di default (solo strutturato)
cargo test --features onnx    # + test del percorso NER (quelli che dipendono dal modello sono #[ignore]d)
```

## Licenza

Copyright (C) 2026 Francesco Stimola.

Distribuito sotto **GNU Affero General Public License v3.0 o successiva**
(`AGPL-3.0-or-later`) — vedi [LICENSE](LICENSE). Trattandosi di un proxy privacy
servito in rete, l'AGPL garantisce che chi esegue una versione **modificata** come
servizio debba condividerne le modifiche; eseguirlo non modificato non comporta
alcun obbligo.

---

🇬🇧 English version: [README.md](README.md).
