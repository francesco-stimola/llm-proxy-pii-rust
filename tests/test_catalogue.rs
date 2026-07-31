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
//! **Scope limit, stated so the guard does not read stronger than it is.** It proves each declared id
//! is *named* in `TESTING.md` — not that its description is accurate, and not that every test has an
//! id. Accuracy is review's job. What this ends is the specific failure that kept recurring: a guard
//! added, catalogued nowhere, and remembered only by the round that asked for it.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Prefixes that mark a **catalogued guard id** in a doc comment, e.g. `**DOS-07 (M10-R28) — …**`.
///
/// Longest-first so `ANT-E2E` is not read as `E2E`. Adding a prefix here is how a new family of ids
/// joins the check; a family that is *not* here is silently unchecked, which is the one way this
/// guard can go quiet — so the non-vacuity floor below is what keeps that honest.
const ID_PREFIXES: &[&str] = &[
    "FAILOPEN",
    "ANT-E2E",
    "PHONE-NAT",
    "PHONE-BUD",
    "PHONE-OM",
    "PERF-M7",
    "SMOKE-GLINER",
    "BENCH",
    "PROP",
    "DOS",
    "CLI",
    "E2E",
    "CFG",
    "DEP",
    "LOG",
    "INT",
    "CAT",
];

/// Every id declared by a `#[test]`'s own doc comment, as `(id, file)`.
///
/// A declaration is an id inside a doc comment (`///`) in the block immediately above a `#[test]`
/// attribute. Ids mentioned in *prose* elsewhere — a finding's narrative, a cross-reference — are not
/// declarations and are not collected, because the claim under test is *"a test that says it is a
/// catalogued guard is catalogued"*, not *"every string that looks like an id exists"*.
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
            if line.trim_start() != "#[test]" {
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
                if !doc.trim_start().starts_with("///") {
                    continue;
                }
                for prefix in ID_PREFIXES {
                    let mut rest = *doc;
                    while let Some(at) = rest.find(prefix) {
                        let tail = &rest[at + prefix.len()..];
                        // `PREFIX-<alnum>` — the suffix must start with `-` and a digit or letter.
                        let id_tail: String = tail
                            .strip_prefix('-')
                            .unwrap_or("")
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric())
                            .collect();
                        if !id_tail.is_empty() {
                            out.insert((format!("{prefix}-{id_tail}"), name.clone()));
                        }
                        rest = &rest[at + prefix.len()..];
                    }
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

    // Non-vacuity: if the extractor stops matching — a doc-comment convention changes, a prefix is
    // renamed — this must fail loudly rather than pass by finding nothing. Same discipline as CLI-05,
    // and the same reason: a guard that can quietly observe zero things is not a guard (M4-R13).
    assert!(
        declared.len() >= 20,
        "the extractor found only {} declared guard ids — it has stopped matching the source's \
         doc-comment convention, and would pass vacuously",
        declared.len()
    );

    let missing: Vec<String> = declared
        .iter()
        .filter(|(id, _)| !catalogue.contains(id.as_str()))
        .map(|(id, file)| format!("{id} (declared in {file})"))
        .collect();

    assert!(
        missing.is_empty(),
        "{} guard id(s) are declared by a #[test] but named nowhere in docs/TESTING.md:\n  {}\n\n\
         The catalogue is what the next person reads to find out what is guarded, and a guard that \
         is not in it is remembered only by the review round that asked for it — which is how this \
         drifted five times in M10 (R51, R55). Add the entry, or drop the id from the doc comment.",
        missing.len(),
        missing.join("\n  ")
    );
}
