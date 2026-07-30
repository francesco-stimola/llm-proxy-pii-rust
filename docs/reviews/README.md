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
| [M4.md](M4.md) | M4 — broad locale & language coverage | 24, all closed |
| [M5.md](M5.md) | M5 — integration & performance testing | 12, all closed |
| [M6.md](M6.md) | M6 — native Anthropic `/v1/messages` | 6, all closed |
| [M7.md](M7.md) | M7 / M7.1 — NER latency · system-prompt cache | 23, all closed |
| [M8.md](M8.md) | M8 / M8.1 — GLiNER · national phone recognizer | 8, all closed |
| [M9.md](M9.md) | M9 / M9.1 — GPU optimization · per-backend binaries | 29, all closed |
| [M10.md](M10.md) | M10 — national phone coverage + release hygiene | 34, all closed (round 5 pending) |

**The page to read here is [M10-R28](M10.md#m10-r28), the fourth turn of one wheel.** M10-R2 was a DoS
on the domestic-phone validator. Its fix was undone by the input **repeating**
([M10-R20](M10.md#m10-r20)). *That* fix bounded a **field** while the body chooses its field count, so
the same legal 15.6 MiB payload still cost **57 s** — and a pre-fix build answered it in the same time,
which is how three consecutive fixes were shown to have closed nothing. What finally closed it was
changing the *unit*: one allowance per **request**. The rule it leaves is in ARCHITECTURE — *a budget
scoped to a unit the client can multiply is a rate, not a bound* — with its testing companion in
TESTING: every complexity guard this project had written measured **one string**, because
`try_detect(&str)` is the shape each was handed, while the masking path takes a **body**.

Two more from round 4 are worth the detour. [M10-R29](M10.md#m10-r29): the budget named after the phone
tier was charged by nine always-on checksums, so it refused legal requests **with the phone tier not
loaded at all** — and fixing the unit shrank the effective allowance enough that an ordinary 367 KB
database result started getting a `400`, which is what moved the threshold. And
[M10-R31](M10.md#m10-r31): a closure that corrected one of the two lines its finding named, the second
time in two rounds — *a closure is checked against the finding's own locations, or against nothing.*

Round 4 also went hunting for a path where an exhausted budget fails **open** and found none. The
reason it found none — nothing wraps the structured recognizers in `FailOpen` *today* — turned out to
be the defect underneath.

---

**If you read one page in this folder, read [the M4 retrospective](M4.md#retrospective).** M4 took six
review rounds and produced more findings than every other milestone combined — because M4-R7 → R9 → R10/R11
→ R13 → R17 is **one bug, rediscovered five times**. Every "fix" that only re-ranked the overlap priorities
*relocated* the leak instead of closing it. The four lessons that came out of it are why this codebase now
reasons in **invariants** rather than rankings.
