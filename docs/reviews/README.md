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
| [M10.md](M10.md) | M10 — national phone coverage + release hygiene | 55, 52 closed (round 8 open) |

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

**And the wheel turned a fifth time: [M10-R35](M10.md#m10-r35).** Making the unit a request needed a
new `_within` seam on the detector trait, whose default *silently drops the budget*. Round 4 saw that
hazard clearly and closed it for all three **wrappers**. The detector it missed is the **leaf** — the
one whose cost the budget exists to bound, and the only place in the tree that mints an allowance — so
every fixpoint pass after the first started from a full one, and a legal 15.63 MiB body answered
`200` in **17.2 s** against a published ceiling of ~1.4 s. The one-line fix leaves the whole suite
green, which is the part to take away: *an obligation that a trait default can satisfy is not carried
by the type system, and "every test passes" is the signature of that, not evidence against it.*

It was closed by **deleting the seam**, not filling it: `try_detect` and `redetect` now *take* a
`&Budget`, so omitting it does not compile. The generalization is the one to carry out of this folder
— **when forgetting to override is silently valid, the API is the defect.** Its sibling
[M10-R41](M10.md#m10-r41) is the same idea about values rather than methods: `FailOpen` decided what
to swallow by consulting a *global* (`budget.is_exhausted()`) instead of asking the error what kind it
was, and *correlating with a state is not distinguishing a kind.*

**Round 6 verified that fix on the real binary — and found the wheel had turned once more, in the
sentences written *about* it.** The fix holds, and six of round 5's seven closures with it: the
15.63 MiB body is a `400` in 1.0 s. What does not
is *"a seventh detector that drops the allowance fails at DOS-08"* — the guard constructed a bare
`StructuredRecognizers`, so the same one-line defect in a shipped **wrapper** left all 218 tests green
while the request under-charged 21× ([M10-R42](M10.md#m10-r42)) — and *"no method remains that a
default could route to which would mint another"*, because `try_detect`'s own default still routed to
`detect`, which mints *and* swallows the refusal ([M10-R44](M10.md#m10-r44)).

Both are closed, and the two fixes are the pair worth carrying out of this folder. The guard now
quantifies over **every chain the wiring can build** rather than over the type that had the bug —
*a guard aimed at the instance relocates the blind spot*, which is the M4 lesson in its testing form.
And `try_detect` became the **required** trait method with `detect` derived, so a `detect`-only
implementor does not compile: the four test doubles that stopped building are that fix's whole
regression suite, and they are worth more than a test would be. **Prefer the guarantee the compiler
gives over the one a test remembers to ask for.**

**Round 7 verified both by mutating the tree, and the compiler half holds exactly as stated** —
a `detect`-only implementor is `error[E0046]`, and DOS-08/DOS-09 are the *only* two failures in the
suite when the defect is put back. The guard half is one word weaker than the sentence above: the
chains are a **hand-written list of four**, not a projection of the wiring, and the position it omits
is `FailOpen` — where the one line that makes a budget refusal un-swallowable is asserted by nothing
at all ([M10-R47](M10.md#m10-r47), [M10-R48](M10.md#m10-r48)). *A guard widened from a type to a list
of instances has moved the blind spot, not removed it* — which is the same lesson arriving where its
own fix landed. **Round 8 closed that loop**: deleting the guard clause now reds `FAILOPEN-BUD` *and*
DOS-08, because the `FailOpen` positions are in the list.

**Where the wheel stopped being about the code and became about the *instrument* — round 8.** Six
rounds moved the budget's unit; the seventh discovered that the harness measuring it had been fed
eleven-digit non-numbers, and fixed the values. The eighth found the fix had corrected the harness's
*values* and not its *axis*: DOS-BUD builds one rendering of one region, `347 XXXXXXX`, which is the
**cheapest legal phone in the shipped set** — `LongBlock` is the only shape family with a single
applicable region, and the only rendering no other family's regex matches inside. From that single
point four documents concluded that *a legal phone-bearing body cannot reach the allowance at all*;
measured, five of six legal 16 MiB payloads are refused and the real binary refuses a 2.6 MB contact
export ([M10-R53](M10.md#m10-r53), [M10-R54](M10.md#m10-r54)). The generalization worth carrying out
of this folder: **a measurement harness needs a declared scope the way a guard does** — R49 gave it a
non-vacuity assertion, which proves it measures *something*, and that is not the same as proving what
it measures is representative. *An instrument with no declared scope has its narrowest reading quoted
as its widest.*

---

**If you read one page in this folder, read [the M4 retrospective](M4.md#retrospective).** M4 took six
review rounds and produced more findings than every other milestone combined — because M4-R7 → R9 → R10/R11
→ R13 → R17 is **one bug, rediscovered five times**. Every "fix" that only re-ranked the overlap priorities
*relocated* the leak instead of closing it. The four lessons that came out of it are why this codebase now
reasons in **invariants** rather than rankings.
