# Review findings — the record

One file per milestone. Each holds the **full** finding, its fix, its tests, and what it taught us.

**This is the record, not the ledger.** The ledger — *which* findings exist and *whether they are open* —
lives in [`../ROADMAP.md`](../ROADMAP.md), as a table per milestone. A finding has **one home for its whole
life**: closing it flips a box in the ROADMAP and appends a closure note here. Nothing is ever copied or
moved between the two.

**And this is not where the design lives.** Every conclusion that still governs the code was **promoted**
into [`../ARCHITECTURE.md`](../ARCHITECTURE.md) (the invariants) and [`../TESTING.md`](../TESTING.md) (the
guards). Read those to understand the system; read these to answer *"what was M4-R9, and why did we do it
that way?"*

| File | Milestone | Findings |
|---|---|---|
| [M1.md](M1.md) | M1 / M1.5 — structured PII pipeline · fail-closed | 8, all closed *(pre-date the `M<n>-R<n>` ids)* |
| [M2.md](M2.md) | M2 — unstructured entities (ONNX NER) | 10, all closed |
| [M2.5.md](M2.5.md) | M2.5 — HuggingFace model management | 2, all closed |
| [M2.6.md](M2.6.md) | M2.6 — debug & observability modes | 2 nits, all closed |
| [M3.md](M3.md) | M3 — streaming & multi-provider routing | 2, all closed |
| [M4.md](M4.md) | M4 — broad locale & language coverage | **23 — 18 closed, 5 open (one BLOCKER)** |

**M5 does not start until M4's ledger is clean.** All five open findings in the project belong to M4.

---

**If you read one page in this folder, read [the M4 retrospective](M4.md#retrospective).** M4 took six
review rounds and produced more findings than every other milestone combined — because M4-R7 → R9 → R10/R11
→ R13 → R17 is **one bug, rediscovered five times**. Every "fix" that only re-ranked the overlap priorities
*relocated* the leak instead of closing it. The four lessons that came out of it are why this codebase now
reasons in **invariants** rather than rankings.
