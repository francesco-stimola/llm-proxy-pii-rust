# llm-proxy-pii-rust

Un proxy LLM veloce e attento alla privacy, scritto in Rust.

`llm-proxy-pii-rust` si posiziona tra la tua applicazione e qualsiasi provider
di LLM. Rileva e anonimizza le informazioni personali (PII) in locale, prima che
le richieste lascino la tua rete, e può ripristinare i valori originali nella
risposta del provider, così che la tua applicazione riceva un output coerente e
non oscurato.

L'obiettivo è essere un layer di privacy "drop-in": basta far puntare il tuo
client compatibile con l'API OpenAI all'URL del proxy e nient'altro nel tuo
stack deve cambiare.

## Perché

Inviare i prompt a un LLM ospitato significa affidare a terzi tutto ciò che gli
utenti scrivono — nomi, email, numeri di telefono, indirizzi, numeri di conto.
Questo proxy mantiene sotto il tuo controllo la fase di rilevamento e
mascheramento, sulla tua infrastruttura, invece di affidarla al provider.

## Obiettivi

- **Rilevamento local-first** — le PII vengono identificate sul tuo hardware;
  nulla viene inviato altrove per la fase di filtraggio.
- **Anonimizzazione reversibile** — dei segnaposto sostituiscono i valori
  sensibili in uscita e vengono ripristinati al ritorno, mantenendo le risposte
  utilizzabili.
- **Indipendente dal provider** — funziona come proxy trasparente davanti ad API
  compatibili con OpenAI.
- **Stabilità e prestazioni** — costruito in Rust per gestire concorrenza e
  streaming in modo affidabile anche sotto carico.

## Stato

Sviluppo iniziale. L'architettura e il motore di rilevamento sono ancora in fase
di progettazione e potrebbero cambiare.

## Sviluppo

Documenti vivi che tracciano tutto il lavoro, così nulla si perde tra una
sessione e l'altra (tutti in inglese):

- [Development setup (Windows, no admin)](docs/SETUP.md)
- [Architecture & design decisions](docs/ARCHITECTURE.md)
- [Roadmap & milestones](docs/ROADMAP.md)
- [Testing strategy](docs/TESTING.md)
- [Development log](docs/DEVLOG.md)

## Licenza

Copyright (C) 2026 Francesco Stimola.

Distribuito sotto **GNU Affero General Public License v3.0 o successiva**
(`AGPL-3.0-or-later`) — vedi [LICENSE](LICENSE). Trattandosi di un proxy privacy
servito in rete, l'AGPL garantisce che chi esegue una versione **modificata** come
servizio debba condividerne le modifiche; eseguirlo non modificato non comporta
alcun obbligo.

---

🇬🇧 English version: [README.md](README.md).
