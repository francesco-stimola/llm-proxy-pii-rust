//! HuggingFace Hub model resolution (milestone M2.5). Behind the `onnx` feature.
//!
//! Fetches a **revision-pinned** NER model + tokenizer (+ `config.json` for the
//! labels) into the *standard* HF cache (`~/.cache/huggingface`) via the official
//! [`hf-hub`](https://crates.io/crates/hf-hub) crate, which owns that
//! content-addressed tree — we never hand-populate it.
//!
//! **Opt-in and one-time.** This runs only when the operator sets
//! `NER_MODEL_REPO` (see [`crate::server`]); an explicit `NER_MODEL_PATH` always
//! wins and makes zero outbound calls. The fetch is **model artifacts, not user
//! data**, and is logged — never silent. A file already in the cache at the
//! pinned revision resolves offline.
//!
//! Pure parsing ([`parse_id2label`]) is unit-tested without network; the actual
//! download is exercised only by the `#[ignore]`d eval harness (`tests/ner_eval.rs`).

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use hf_hub::{HFClient, HFClientBuilder};

/// What to fetch: a repo pinned to a revision, plus the file names within it.
pub struct HfModelSpec {
    /// `owner/name`, e.g. `jiting/xlm-roberta-base-ner-hrl_onnx`.
    pub repo: String,
    /// Pinned git revision (commit hash or branch) — reproducibility.
    pub revision: String,
    /// Model file within the repo, e.g. `onnx/model_quantized.onnx`.
    pub model_file: String,
    /// Fast tokenizer JSON within the repo, e.g. `tokenizer.json`.
    pub tokenizer_file: String,
    /// Config within the repo carrying `id2label`, e.g. `config.json`.
    pub config_file: String,
}

/// Local paths in the HF cache + labels derived from `config.json`.
pub struct ResolvedModel {
    pub model_path: PathBuf,
    pub tokenizer_path: PathBuf,
    /// id → label in class-id order, parsed from `config.json`'s `id2label`.
    pub id2label: Vec<String>,
}

impl HfModelSpec {
    /// Download (or hit the cache for) the three artifacts and parse the labels.
    ///
    /// Async because the fetch is network I/O on the app's `tokio` runtime; a
    /// cached revision returns without a network round-trip.
    pub async fn resolve(&self) -> Result<ResolvedModel> {
        let (owner, name) = self
            .repo
            .split_once('/')
            .ok_or_else(|| anyhow!("NER_MODEL_REPO must be `owner/name`, got `{}`", self.repo))?;

        let client = build_client()?;

        // One-time, logged, never silent. Only model artifacts leave the box —
        // no user data — and only because the operator opted in via NER_MODEL_REPO.
        tracing::info!(
            repo = %self.repo,
            revision = %self.revision,
            "resolving NER model via hf-hub (one-time fetch unless already cached)"
        );

        let model_path = self.fetch(&client, owner, name, &self.model_file).await?;
        let tokenizer_path = self.fetch(&client, owner, name, &self.tokenizer_file).await?;
        let config_path = self.fetch(&client, owner, name, &self.config_file).await?;

        let config = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read cached config {}", config_path.display()))?;
        let id2label = parse_id2label(&config)?;

        Ok(ResolvedModel { model_path, tokenizer_path, id2label })
    }

    /// Resolve one file to a local cache path at the pinned revision.
    async fn fetch(&self, client: &HFClient, owner: &str, name: &str, filename: &str) -> Result<PathBuf> {
        client
            .model(owner, name)
            .download_file()
            .revision(self.revision.clone())
            .filename(filename.to_string())
            .send()
            .await
            // The error carries repo/revision/filename only — never input text.
            .map_err(|e| anyhow!("hf-hub fetch {owner}/{name}@{} :: {filename}: {e}", self.revision))
    }
}

/// Parse a HF `config.json`'s `id2label` map into labels in **class-id order**.
///
/// `{ "id2label": { "0": "O", "1": "B-PER", … } }` → `["O", "B-PER", …]`. The ids
/// must be contiguous from 0 (a gap would misalign class ids to labels, silently
/// mislabelling every token — this is a leak surface, so it fails closed).
pub fn parse_id2label(config_json: &str) -> Result<Vec<String>> {
    let value: serde_json::Value =
        serde_json::from_str(config_json).context("parse config.json")?;
    let map = value
        .get("id2label")
        .and_then(|m| m.as_object())
        .ok_or_else(|| anyhow!("config.json has no object `id2label`"))?;

    let mut pairs: Vec<(usize, String)> = Vec::with_capacity(map.len());
    for (key, label) in map {
        let id: usize = key
            .parse()
            .map_err(|_| anyhow!("id2label key `{key}` is not an integer"))?;
        let label = label
            .as_str()
            .ok_or_else(|| anyhow!("id2label[{key}] is not a string"))?;
        pairs.push((id, label.to_string()));
    }
    pairs.sort_by_key(|(id, _)| *id);

    for (expected, (id, _)) in pairs.iter().enumerate() {
        if expected != *id {
            return Err(anyhow!(
                "id2label ids are not contiguous from 0 (expected {expected}, found {id})"
            ));
        }
    }
    Ok(pairs.into_iter().map(|(_, label)| label).collect())
}

/// Build the client, pinning the **standard** HF hub cache when no explicit HF
/// cache env is set.
///
/// hf-hub honors `HF_HUB_CACHE` / `HF_HOME` itself, so when either is set we defer
/// to it. But with neither set, hf-hub 1.0.0 falls back to `/tmp/.cache/huggingface`
/// on Windows (where `HOME` is unset) — a non-shared, drive-relative location that
/// defeats the whole point of using the library-managed cache. So we pin the
/// conventional `<home>/.cache/huggingface/hub` (matching `huggingface_hub`), where
/// models dedupe with every other tool on the box.
fn build_client() -> Result<HFClient> {
    let mut builder = HFClientBuilder::new();
    if std::env::var_os("HF_HUB_CACHE").is_none() && std::env::var_os("HF_HOME").is_none() {
        if let Some(cache) = standard_hub_cache_dir() {
            builder = builder.cache_dir(cache);
        }
    }
    builder.build().map_err(|e| anyhow!("hf-hub client init: {e}"))
}

/// The conventional HF *hub* cache root, `<home>/.cache/huggingface/hub`, where
/// `<home>` is `USERPROFILE` on Windows or `HOME` elsewhere. `None` if no home
/// directory can be determined (then the caller lets hf-hub apply its own default).
fn standard_hub_cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("hub"),
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_id2label, standard_hub_cache_dir};

    #[test]
    fn standard_cache_is_the_conventional_hf_hub_path() {
        // Whatever the home is on this box, the cache must end in the standard
        // `.cache/huggingface/hub` tail (not hf-hub's `/tmp` fallback).
        let dir = standard_hub_cache_dir().expect("a home dir on the test box");
        assert!(
            dir.ends_with("huggingface/hub") || dir.ends_with(r"huggingface\hub"),
            "unexpected cache dir: {}",
            dir.display()
        );
        let mut it = dir.iter().rev();
        assert_eq!(it.next().unwrap(), "hub");
        assert_eq!(it.next().unwrap(), "huggingface");
        assert_eq!(it.next().unwrap(), ".cache");
    }

    #[test]
    fn id2label_is_ordered_by_class_id_not_json_order() {
        // Keys deliberately out of order (and two-digit) — output must be by id.
        let cfg = r#"{
            "id2label": {
                "2": "I-DATE", "0": "O", "10": "I-LOC", "1": "B-DATE",
                "3": "B-PER", "4": "I-PER", "5": "B-ORG", "6": "I-ORG",
                "7": "B-LOC", "8": "I-LOC", "9": "B-MISC"
            }
        }"#;
        let labels = parse_id2label(cfg).expect("parse");
        assert_eq!(
            labels,
            vec![
                "O", "B-DATE", "I-DATE", "B-PER", "I-PER", "B-ORG", "I-ORG", "B-LOC", "I-LOC",
                "B-MISC", "I-LOC",
            ]
        );
    }

    #[test]
    fn id2label_matches_the_xlmr_config() {
        // The exact XLM-R (Davlan) label set the picked model ships — 9 labels
        // incl. DATE (mapped to None downstream). Deriving this removes the
        // error-prone hand-typed NER_LABELS list.
        let cfg = r#"{"id2label":{"0":"O","1":"B-DATE","2":"I-DATE","3":"B-PER","4":"I-PER","5":"B-ORG","6":"I-ORG","7":"B-LOC","8":"I-LOC"}}"#;
        assert_eq!(
            parse_id2label(cfg).unwrap(),
            vec!["O", "B-DATE", "I-DATE", "B-PER", "I-PER", "B-ORG", "I-ORG", "B-LOC", "I-LOC"]
        );
    }

    #[test]
    fn non_contiguous_ids_fail_closed() {
        // A missing id (2) would misalign labels to logits → reject, don't guess.
        let cfg = r#"{"id2label":{"0":"O","1":"B-PER","3":"B-ORG"}}"#;
        assert!(parse_id2label(cfg).is_err());
    }

    #[test]
    fn missing_id2label_is_an_error() {
        assert!(parse_id2label(r#"{"architectures":["X"]}"#).is_err());
    }

    #[test]
    fn non_integer_key_is_an_error() {
        assert!(parse_id2label(r#"{"id2label":{"O":"O"}}"#).is_err());
    }
}
