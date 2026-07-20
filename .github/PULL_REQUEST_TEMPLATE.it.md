<!--
Mirror italiano di PULL_REQUEST_TEMPLATE.md, per parità con la convenzione README/CONTRIBUTING
del repo. GitHub compila automaticamente solo PULL_REQUEST_TEMPLATE.md (l'inglese) per ogni
nuova PR — non esiste un meccanismo di selezione lingua per le PR come per le issue. Questo file
è quindi di sola consultazione: copialo a mano nella descrizione della PR se preferisci compilare
la checklist in italiano.
-->

## Cosa & perché

<!-- Cosa è cambiato, e perché. Collega l'elemento di ROADMAP se questa PR lo chiude. -->

## Checklist

- [ ] Ho letto [CONTRIBUTING.it.md](../CONTRIBUTING.it.md) e accetto i suoi termini, inclusa la
      cessione del copyright per questo contributo.
- [ ] I miei commit sono firmati (`git commit -s`).
- [ ] `cargo test` (e `cargo test-onnx` se applicabile) passa senza warning.
- [ ] Test aggiunti/aggiornati per questo cambio di comportamento (casi avversariali se tocca il
      rilevamento).
- [ ] `docs/ROADMAP.md` / `docs/DEVLOG.md` / `docs/ARCHITECTURE.md` / `docs/TESTING.md`
      aggiornati se pertinente.
