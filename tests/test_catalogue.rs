//! **CAT-01 (M10-R55 / M10-R59) — every test that claims a guard id must appear in the catalogue.**
//!
//! `docs/TESTING.md` is this project's test catalogue, and it is hand-maintained. Nine times in M10 a
//! closure fixed a defect and skipped a site its own finding had named, and three of those sites were
//! catalogue entries; M10-R55 prescribed this guard, M10-R59 pointed out it had not been built, and
//! that omission was itself the tenth instance of the class.
//!
//! So this is the mechanical half. It is deliberately the same shape as **CLI-05**, which pins that
//! `--help` names every environment variable the code reads: extract the claims from the **source**,
//! assert each appears in the **document**. A hand-maintained list drifts; a list checked against the
//! thing it describes cannot drift silently.
//!
//! **The guard over the guards had the defect it exists to catch, and M11-R4 is where it was found.**
//! Until then a declaration was recognised only if its family appeared in a hand-kept `ID_PREFIXES`
//! array — so a *new family* was invisible, and the check passed while cataloguing nothing about it.
//! Removing `"VAT"` and `"AUG"` from that array and scrubbing all 24 of their ids from `TESTING.md`
//! left the guard **green**. The non-vacuity floor that was supposed to keep it honest was `20`
//! against 54 declared ids, so 63% could vanish unnoticed.
//!
//! **Two things changed, and the first is the chokepoint.** There is no prefix list any more: an id is
//! recognised by its *shape*, so a family is in scope the moment somebody writes one. That closed the
//! class — and, measured on the way in, it also brought **nine real families under the check that
//! never were**: `CC`, `DBG`, `NER-EP`, `PERF`, `PHONE-COV`, `REG`, `THREAD` and two more, all already
//! catalogued and none of them ever verified. Second, the walk now reads **`//!` module docs** as well
//! as `///` blocks above a `#[test]`, which is how `VAT-OM` came to be declared and uncatalogued.
//!
//! **Scope limits, stated so the guard does not read stronger than it is.**
//! - It proves each declared id is *named* in `TESTING.md` — not that its description is accurate, and
//!   not that every test has an id. Accuracy is review's job.
//! - "Named" means named **anywhere**, so a cross-reference from a sibling entry satisfies it even if
//!   the id's own entry is gone (M11 round 0, mutation M7). Tightening that would mean pinning the
//!   catalogue's bullet format, which is a formatting rule pretending to be a correctness one.
//! - Nothing here can catch an id deleted from *both* source and document in one change. That is a
//!   deliberate act, not drift, and drift is what this guard is for.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Does `token` have the shape of a **catalogued guard id** — `VAT-15`, `PHONE-NAT-01`,
/// `PERF-M7-03`, `DOS-BUD`, `VAT-OM`?
///
/// This replaces the hand-kept prefix list (M11-R4), so the rule has to earn its precision from
/// its shape rather than from an enumeration. Every segment is uppercase-alphanumeric, and:
///
/// - **The first segment is at least two characters.** This is what excludes the NER's BIO tag
///   names — `B-PER`, `I-ORG`, `B-DATE` and five siblings — which are quoted in the eval tests'
///   module docs and are not guard ids. One rule, eight false positives gone, no list.
/// - **The last segment is all digits (1–3) or all letters (2–6).** This is what excludes review
///   references: `M4-R13` and `M10-R55` end in a mixed `R13`/`R55` segment, so a finding cited in
///   a doc comment — which happens constantly here — is never mistaken for a guard.
/// - **A milestone-shaped first segment is rejected outright**, which catches the remaining case
///   (`M2-NER`, a corpus name in two eval tests).
///
/// Measured against the whole tree when it was written: 73 ids accepted, and every one of them is
/// a real guard id. The residue is acronym-plus-number tokens like `UTF-8` or `AGPL-3` — they
/// would be accepted, and none occurs in any scanned doc comment today. If one ever does, the
/// symptom is a clear failure naming it, not a silent pass.
fn looks_like_guard_id(token: &str) -> bool {
    let segments: Vec<&str> = token.split('-').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        return false;
    }
    let uppercase_alnum = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };

    let first = segments[0];
    if first.len() < 2 || !first.starts_with(|c: char| c.is_ascii_uppercase()) {
        return false;
    }
    // `M4-…`, `M2.5-…`: a milestone reference, never a guard id.
    if first[1..].chars().all(|c| c.is_ascii_digit()) && first.starts_with('M') {
        return false;
    }
    if !segments.iter().all(|s| uppercase_alnum(s)) {
        return false;
    }
    if !segments[1..segments.len() - 1]
        .iter()
        .all(|s| s.starts_with(|c: char| c.is_ascii_uppercase()))
    {
        return false;
    }

    let last = segments[segments.len() - 1];
    let all_digits = last.chars().all(|c| c.is_ascii_digit());
    let all_letters = last.chars().all(|c| c.is_ascii_uppercase());
    (all_digits && (1..=3).contains(&last.len())) || (all_letters && (2..=6).contains(&last.len()))
}

/// Every guard id declared in `line`, by the shape rule above.
///
/// Splits on anything that cannot appear inside an id, so `**DOS-07 (M10-R28) — …**` yields
/// `DOS-07` and not `M10-R28`.
fn ids_in(line: &str, file: &str, out: &mut BTreeSet<(String, String)>) {
    for token in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        // Trim hyphens the prose put there — an em-dash-free `—` is already a split point, but
        // `DOS-07-` or `-VAT-15` can survive tokenisation.
        let token = token.trim_matches('-');
        if looks_like_guard_id(token) {
            out.insert((token.to_string(), file.to_string()));
        }
    }
}

/// Every guard id **declared** by this tree's source, as `(id, file)`.
///
/// A declaration is an id inside a doc comment that introduces a guard: either a `///` block
/// immediately above a `#[test]` attribute, or a `//!` module-level doc. Ids mentioned in *prose*
/// elsewhere — an ordinary `//` comment, a finding's narrative inside a function body — are not
/// declarations, because the claim under test is *"a test that says it is a catalogued guard is
/// catalogued"*, not *"every string that looks like an id exists"*.
///
/// **`//!` is not a widening for its own sake (M11-R4).** Four ids are declared that way —
/// `CAT-01` (this file), `DOS-01`, `PHONE-OM` and `VAT-OM` — because a file whose *whole subject*
/// is one guard family names it at the top. `VAT-OM` was declared there, catalogued nowhere, and
/// invisible to this check on both counts at once.
fn declared_ids(root: &Path, out: &mut BTreeSet<(String, String)>) {
    for entry in fs::read_dir(root).expect("readable directory") {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            declared_ids(&path, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("readable source");
        let lines: Vec<&str> = text.lines().collect();
        let name = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .display()
            .to_string();

        for (i, line) in lines.iter().enumerate() {
            // Module-level docs: the file's own subject.
            if line.trim_start().starts_with("//!") {
                ids_in(line, &name, out);
            }
            if line.trim() != "#[test]" {
                continue;
            }
            // Walk back over the contiguous doc-comment / attribute block above this `#[test]`.
            let mut j = i;
            while j > 0 {
                let above = lines[j - 1].trim_start();
                if above.starts_with("///") || above.starts_with("#[") || above.is_empty() {
                    j -= 1;
                    // A blank line ends the block unless a doc comment continues above it.
                    if above.is_empty() && j > 0 && !lines[j - 1].trim_start().starts_with("///") {
                        break;
                    }
                } else {
                    break;
                }
            }
            for doc in &lines[j..i] {
                if doc.trim_start().starts_with("///") {
                    ids_in(doc, &name, out);
                }
            }
        }
    }
}

#[test]
fn every_declared_guard_id_appears_in_the_test_catalogue() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalogue =
        fs::read_to_string(manifest.join("docs/TESTING.md")).expect("docs/TESTING.md is readable");

    let mut declared = BTreeSet::new();
    declared_ids(&manifest.join("tests"), &mut declared);
    declared_ids(&manifest.join("src"), &mut declared);

    // Non-vacuity: if the extractor stops matching — a doc-comment convention changes, the shape
    // rule is narrowed — this must fail loudly rather than pass by finding nothing. Same discipline
    // as CLI-05, and the same reason: a guard that can quietly observe zero things is not a guard
    // (M4-R13).
    //
    // **The floor is close to the real count on purpose (M11-R4).** It used to be 20 against 54,
    // which let 63% of the ids disappear unnoticed — a floor that loose is a floor in name only.
    // With the prefix list gone, its job is no longer "notice a missing family" (that cannot happen
    // any more) but "notice the extractor breaking", and for that it should sit just under reality.
    // Measured 73 when written; raise this deliberately when guards are added.
    assert!(
        declared.len() >= 70,
        "the extractor found only {} declared guard ids, and there were 73 when this floor was \
         set — it has stopped matching the source's doc-comment convention, and a check that \
         observes almost nothing passes almost always",
        declared.len()
    );

    let missing: Vec<String> = declared
        .iter()
        .filter(|(id, _)| !catalogue.contains(id.as_str()))
        .map(|(id, file)| format!("{id} (declared in {file})"))
        .collect();

    assert!(
        missing.is_empty(),
        "{} guard id(s) are declared by a #[test] or a module doc but named nowhere in \
         docs/TESTING.md:\n  {}\n\n\
         The catalogue is what the next person reads to find out what is guarded, and a guard that \
         is not in it is remembered only by the review round that asked for it — which is how this \
         drifted five times in M10 (R51, R55) and once more in M11 (R4). Add the entry, or drop the \
         id from the doc comment.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// **CAT-02 (M11-R4) — the shape rule accepts guard ids and rejects the things that look like them.**
///
/// `looks_like_guard_id` replaced a hand-kept prefix list, which means the *rule* is now the thing
/// that can be wrong — and a rule that quietly stopped matching would take CAT-01 down with it while
/// leaving it green (the floor would catch a total collapse, not a narrowing). So the rule is pinned
/// directly, by a matrix of the cases it has to separate, rather than only through its effect.
#[test]
fn the_guard_id_shape_rule_separates_ids_from_prose() {
    for id in [
        "VAT-15",
        "VAT-OM",
        "CAT-01",
        "AUG-02",
        "KIND-01",
        "DOS-BUD",
        "PHONE-OM",
        "PHONE-NAT-01",
        "PHONE-COV",
        "PERF-M7-03",
        "ANT-E2E-04",
        "E2E-INT-02",
        "FAILOPEN-BUD",
        "SMOKE-GLINER",
        "NER-EP-01",
        "THREAD-01",
        "DBG-02",
        "CC-05",
    ] {
        assert!(
            looks_like_guard_id(id),
            "{id} is a real guard id in this repo"
        );
    }

    for not_an_id in [
        // Review references. These appear in nearly every doc comment here, so mistaking one for
        // a guard id would demand that `TESTING.md` name every finding ever filed.
        "M4-R13",
        "M10-R55",
        "M11-R4",
        "M2-NER",
        // The NER's BIO tag names, quoted in the eval tests' module docs.
        "B-PER",
        "I-ORG",
        "B-DATE",
        // Ordinary hyphenated prose and identifiers.
        "ASCII-only",
        "CPU-first",
        "fail-closed",
        "mod-97",
        "NO_COLOR",
        "",
        "-",
        "VAT-",
    ] {
        assert!(
            !looks_like_guard_id(not_an_id),
            "{not_an_id} is not a guard id — accepting it would make CAT-01 demand a catalogue \
             entry for it"
        );
    }
}
