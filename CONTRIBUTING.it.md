# Contribuire

Grazie per l'interesse verso `llm-proxy-pii-rust`.

## Perché questo file esiste

Questo progetto ha una **doppia licenza**: open source sotto [AGPL-3.0-or-later](LICENSE), e
disponibile con una licenza commerciale separata per le organizzazioni che hanno bisogno di
termini che l'AGPL non offre (vedi [README.it.md → Licenza](README.it.md#licenza)). Offrire
quella licenza commerciale dipende da un fatto che deve restare vero: il maintainer detiene
tutti i diritti su *ogni* riga del repository, incluse le tue. È a questo che servono i termini
sotto.

## Cessione del copyright

Inviando un contributo a questo repository — una pull request, una patch, o qualunque altra
forma — accetti che:

1. Hai il diritto di inviarlo: è opera tua originale, oppure detieni già i diritti necessari per
   inviarlo secondo questi termini.
2. Cedi a Francesco Stimola tutti i diritti, il titolo e l'interesse, incluso l'intero copyright,
   sul tuo contributo, con effetto dal momento dell'invio.
3. Francesco Stimola può usare, modificare, rilicenziare e sublicenziare il tuo contributo —
   anche con termini proprietari o commerciali — senza bisogno del tuo ulteriore consenso.
4. In cambio, ti vengono garantiti sul tuo stesso contributo gli stessi diritti che ha qualunque
   altro utente del progetto secondo la sua licenza open source vigente (attualmente
   `AGPL-3.0-or-later`).

Se contribuisci per conto di un datore di lavoro, assicurati prima di avere il suo permesso —
questa cessione può trasferire solo diritti che possiedi davvero.

*(È il meccanismo standard per i progetti a doppia licenza con un unico titolare del copyright —
è ciò che permette di vendere licenze commerciali sull'intero codice, contributi della community
inclusi, senza dover ri-negoziare i diritti con ogni contributor passato. Non sostituisce una
tua consulenza legale se stai contribuendo qualcosa di non banale.)*

## Come indicare l'accettazione

- Firma i tuoi commit: `git commit -s` (aggiunge un trailer `Signed-off-by` — la stessa
  convenzione leggera del DCO del kernel Linux).
- Spunta la casella di conferma nel template della pull request.

Una PR priva di entrambi si considera **senza** questo accordo, e non verrà unita.

## Flusso di sviluppo

Toolchain e comandi di build/test: [docs/SETUP.md](docs/SETUP.md) *(in inglese)*. Prima di
aprire una PR:

- `cargo test` (e `cargo test-onnx` se hai toccato la feature `onnx`) verdi, senza warning.
- Nuovi test per ogni cambio di comportamento; casi **avversariali** per i cambi di rilevamento —
  un miss è un leak (vedi [docs/TESTING.md](docs/TESTING.md)).
- Le checkbox di `docs/ROADMAP.md` e `docs/DEVLOG.md` aggiornate se il cambio chiude un elemento
  di milestone.

Vedi [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) per gli invarianti su cui si basa il livello di
rilevamento — leggilo prima di modificare il rilevamento.
