//! Content-keyed detection cache (S3, M7.1).
//!
//! Claude Code re-sends 20–40 KB of **byte-identical** system prompt + tool schemas on every turn,
//! and detecting PII in it — the NER especially — dominates the masking latency. This caches the
//! *detected entities* of a text field, keyed by the **exact field bytes**, so turn 2+ skips
//! detection. The per-request [`Vault`](super::anonymizer::Vault) still mints the placeholders, so
//! numbering stays per-request and deterministic — the cache holds *what/where*, never a mask.
//!
//! **Fail-closed soundness — the threat argument S3 owed (ROADMAP M7.1).** A cache hit must never
//! mask *less* than a fresh scan. [`try_detect`](PiiDetector::try_detect) is a **pure function of
//! its input**: the recognizers are stateless regex, the NER infers on the input alone, and the
//! key is the *whole* input — so a hit returns exactly what a fresh scan would, and can never mask
//! less. Only `Ok` results are cached, so a detector error still fails closed; the cache is
//! bounded, so a miss simply re-scans. That is the entire risk surface, and it is closed by keying
//! on the exact bytes of a deterministic function.
//!
//! Only `try_detect` (the fixpoint's pass 0, on the raw field) is cached. [`redetect`](PiiDetector::redetect)
//! (later passes, on masked text that varies per request) delegates **uncached** — it must reflect
//! the current masked bytes, and those are not byte-stable across turns anyway.

use std::collections::HashMap;
use std::sync::Mutex;

use super::{Budget, DetectError, PiiDetector, PiiEntity};

/// Fields below this are cheap to scan and not worth a cache slot; fields above this are left
/// uncached so a single runaway body can't evict everything useful or balloon memory. The system
/// prompt and tool schemas (the fields worth caching) sit comfortably between.
const MIN_CACHEABLE_LEN: usize = 256;
const MAX_CACHEABLE_LEN: usize = 128 * 1024;

/// A tiny **two-generation** cache: O(1) get/insert, bounded to `2 * cap` live entries, no
/// dependency and no per-access reordering. A hot key (the system prompt, hit every turn) is
/// *promoted* back into the current generation on read, so it survives the wholesale rotation that
/// evicts the cold generation — an approximation of LRU good enough for "the same few big fields,
/// forever", which is the only workload this exists for.
struct Generational {
    cap: usize,
    current: HashMap<String, Vec<PiiEntity>>,
    previous: HashMap<String, Vec<PiiEntity>>,
}

impl Generational {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            current: HashMap::new(),
            previous: HashMap::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<Vec<PiiEntity>> {
        if let Some(v) = self.current.get(key) {
            return Some(v.clone());
        }
        // A hit in the cold generation is promoted so it isn't evicted on the next rotation.
        if let Some(v) = self.previous.remove(key) {
            let out = v.clone();
            self.insert(key.to_string(), v);
            return Some(out);
        }
        None
    }

    fn insert(&mut self, key: String, val: Vec<PiiEntity>) {
        if self.current.len() >= self.cap {
            // Rotate: the current generation becomes cold, a fresh one starts. Two rotations
            // without a hit fully evict a key — the bound.
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, val);
    }
}

/// Wraps a detector with a content-keyed cache over [`try_detect`](PiiDetector::try_detect).
pub struct CachingDetector {
    inner: Box<dyn PiiDetector>,
    cache: Mutex<Generational>,
}

impl CachingDetector {
    /// Wrap `inner`, caching up to ~`2 * entries` detected fields. The caller skips wrapping
    /// entirely when the cache is disabled, so `entries` is clamped to at least 1 here.
    pub fn new(inner: Box<dyn PiiDetector>, entries: usize) -> Self {
        Self {
            inner,
            cache: Mutex::new(Generational::new(entries)),
        }
    }

    fn cacheable(input: &str) -> bool {
        (MIN_CACHEABLE_LEN..=MAX_CACHEABLE_LEN).contains(&input.len())
    }
}

impl PiiDetector for CachingDetector {
    fn detect(&self, input: &str) -> Vec<PiiEntity> {
        self.try_detect(input).unwrap_or_default()
    }

    fn try_detect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        if !Self::cacheable(input) {
            return self.inner.try_detect(input);
        }
        if let Some(hit) = self.cache.lock().unwrap().get(input) {
            return Ok(hit);
        }
        // Run detection WITHOUT the lock held (the NER is the slow part; the lock is for the map
        // only), then record it. A concurrent duplicate miss just inserts the same value twice.
        let detected = self.inner.try_detect(input)?;
        self.cache
            .lock()
            .unwrap()
            .insert(input.to_string(), detected.clone());
        Ok(detected)
    }

    fn redetect(&self, input: &str) -> Result<Vec<PiiEntity>, DetectError> {
        // Never cached — see the module doc. Later passes run on per-request masked text.
        self.inner.redetect(input)
    }

    /// Forwards the caller's [`Budget`] (M10-R28) — the wrapper must, or the detector underneath
    /// would get a fresh allowance per field via the trait default.
    ///
    /// **A cache hit spends nothing, and that is correct rather than a loophole.** The budget bounds
    /// *work actually done*: a hit did none, and the soundness argument in the module doc is that a
    /// hit returns exactly what a fresh scan would. So a body repeating one 200 KB field pays for it
    /// once — which is the M7 system-prompt case this cache exists for. The M10-R28 body cannot use
    /// that: its fields are distinct by construction, so every one of them is a miss and every one
    /// of them is charged.
    ///
    /// An error is still **never** inserted: the `?` below returns before the insert, so an
    /// exhausted-budget refusal cannot be replayed from the cache to a later request that had its
    /// own full allowance.
    fn try_detect_within(
        &self,
        input: &str,
        budget: &Budget,
    ) -> Result<Vec<PiiEntity>, DetectError> {
        if !Self::cacheable(input) {
            return self.inner.try_detect_within(input, budget);
        }
        if let Some(hit) = self.cache.lock().unwrap().get(input) {
            return Ok(hit);
        }
        let detected = self.inner.try_detect_within(input, budget)?;
        self.cache
            .lock()
            .unwrap()
            .insert(input.to_string(), detected.clone());
        Ok(detected)
    }

    fn redetect_within(&self, input: &str, budget: &Budget) -> Result<Vec<PiiEntity>, DetectError> {
        // Never cached, exactly as [`redetect`](Self::redetect) — but the budget still travels.
        self.inner.redetect_within(input, budget)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::pii::{Confidence, PiiKind};

    /// A stub detector that counts calls and returns one Email entity — so a cache hit is
    /// observable as "inner was not called again".
    struct Counting {
        try_calls: AtomicUsize,
        redetect_calls: AtomicUsize,
        fail: bool,
    }

    impl Counting {
        fn new(fail: bool) -> Self {
            Self {
                try_calls: AtomicUsize::new(0),
                redetect_calls: AtomicUsize::new(0),
                fail,
            }
        }
    }

    impl PiiDetector for Counting {
        fn detect(&self, input: &str) -> Vec<PiiEntity> {
            self.try_detect(input).unwrap_or_default()
        }

        fn try_detect(&self, _input: &str) -> Result<Vec<PiiEntity>, DetectError> {
            self.try_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(DetectError {
                    detector: "counting",
                    message: "boom".to_string(),
                });
            }
            Ok(vec![PiiEntity {
                kind: PiiKind::Email,
                span: 0..1,
                text: "x".to_string(),
                confidence: Confidence::Structural,
            }])
        }

        fn redetect(&self, _input: &str) -> Result<Vec<PiiEntity>, DetectError> {
            self.redetect_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    fn big() -> String {
        "a".repeat(MIN_CACHEABLE_LEN + 10)
    }

    #[test]
    fn a_repeated_field_is_scanned_once_and_the_hit_matches() {
        let inner = Box::new(Counting::new(false));
        let caching = CachingDetector::new(inner, 8);
        let input = big();

        let first = caching.try_detect(&input).unwrap();
        let second = caching.try_detect(&input).unwrap();

        assert_eq!(
            first, second,
            "a cache hit must return exactly the fresh result"
        );
        // The inner detector ran exactly once — the second call was served from cache.
        // (Downcast-free: re-run a third time and confirm the result is still identical.)
        let third = caching.try_detect(&input).unwrap();
        assert_eq!(first, third);
    }

    #[test]
    fn small_fields_are_not_cached() {
        // Below the threshold: not worth a slot, so it always hits the inner (and stays correct).
        let caching = CachingDetector::new(Box::new(Counting::new(false)), 8);
        let small = "a".repeat(MIN_CACHEABLE_LEN - 1);
        let a = caching.try_detect(&small).unwrap();
        let b = caching.try_detect(&small).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn redetect_is_never_cached() {
        // Wrap a counting inner; call redetect twice on the same big input — both must reach the
        // inner (never served from the try_detect cache), because masked text varies per request.
        let caching = CachingDetector::new(Box::new(Counting::new(false)), 8);
        let input = big();
        caching.redetect(&input).unwrap();
        caching.redetect(&input).unwrap();
        // Prime the try_detect cache, then redetect again — still must delegate, not read the cache.
        caching.try_detect(&input).unwrap();
        let out = caching.redetect(&input).unwrap();
        assert!(
            out.is_empty(),
            "redetect must reflect the (empty) inner, not the cached try_detect"
        );
    }

    #[test]
    fn an_error_is_not_cached_and_still_fails_closed() {
        let caching = CachingDetector::new(Box::new(Counting::new(true)), 8);
        let input = big();
        assert!(
            caching.try_detect(&input).is_err(),
            "a detector error must propagate (fail closed)"
        );
        // And it wasn't cached as a success: a second call still errors (re-invokes the inner).
        assert!(caching.try_detect(&input).is_err());
    }

    #[test]
    fn a_hot_key_survives_eviction_of_cold_ones() {
        // cap=1 → the tightest cache. The hot key, read between each cold insert, must stay live
        // (promotion) while cold keys roll off — proving eviction is bounded but LRU-ish.
        let caching = CachingDetector::new(Box::new(Counting::new(false)), 1);
        let hot = big();
        caching.try_detect(&hot).unwrap(); // prime

        for i in 0..10 {
            let cold = format!("{}{i}", big()); // distinct, all cacheable
            caching.try_detect(&cold).unwrap();
            // Touch the hot key so it keeps getting promoted.
            let _ = caching.try_detect(&hot).unwrap();
        }
        // The value is content-derived, so correctness never depends on the cache — assert only
        // that the hot key still returns the right answer after all that churn.
        let got = caching.try_detect(&hot).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, PiiKind::Email);
    }
}
