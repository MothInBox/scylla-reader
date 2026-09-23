//! Semantic embeddings via all-MiniLM-L6-v2 (candle, CPU, pure Rust).
//!
//! The model is downloaded from Hugging Face on first use and cached under the
//! data dir. All embedding work is best-effort: callers log failures and skip.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::tokenizer::{Tokenizer, TruncationParams};

/// Fixed genre taxonomy for book classification.
pub const GENRES: &[(&str, &str)] = &[
    ("Fantasy", "magic, mythical creatures, medieval worlds"),
    ("LitRPG", "game mechanics, levels, stats, system"),
    ("Romance", "relationships, love, emotional bonds"),
    ("Sci-Fi", "space, technology, future"),
    ("Horror", "fear, dread, supernatural threats"),
    ("Mystery", "investigation, clues, suspense"),
    ("Adventure", "journey, exploration, quest"),
    ("Action", "fights, conflict, fast-paced"),
    ("Comedy", "humor, jokes, lighthearted"),
    ("Drama", "emotional conflict, character growth"),
    ("Slice of Life", "everyday life, mundane, relaxing"),
    ("Isekai", "transported to another world"),
    ("Cultivation", "xianxia, martial arts, qi, immortal"),
    ("System", "system notifications, quests, skills"),
    ("Apocalypse", "end of the world, survival, monsters"),
    ("Urban Fantasy", "magic in modern city settings"),
    ("Historical", "past eras, period settings"),
    ("Military", "armies, war, tactics"),
    ("Gaming", "virtual worlds, video games"),
    ("Thriller", "tension, danger, high stakes"),
];

/// all-MiniLM-L6-v2 sentence embedder.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

/// A callable that embeds a batch of texts into vectors (the real model or a
/// test stub).
pub type EmbedFn = Arc<dyn Fn(&[&str]) -> anyhow::Result<Vec<Vec<f32>>> + Send + Sync>;

/// A book aggregate row: (book_url, aggregate_embedding, genres).
pub type BookAggregate = (String, Vec<f32>, Option<Vec<String>>);

/// A ranked book result: (book_url, score, genres).
pub type RankedBook = (String, f32, Option<Vec<String>>);

/// A chapter embedding row enriched for search:
/// (book_url, book_title, chapter_url, chapter_title, chapter_idx, genres, embedding).
pub type ChapterHit = (
    String,
    String,
    String,
    String,
    u32,
    Option<Vec<String>>,
    Vec<f32>,
);

/// A ranked chapter hit:
/// (book_url, book_title, chapter_url, chapter_title, chapter_idx, genres, score).
pub type RankedChapterHit = (
    String,
    String,
    String,
    String,
    u32,
    Option<Vec<String>>,
    f32,
);

/// Shared, lazily-loaded embedder with a failure cooldown.
///
/// The ~91MB model download happens outside the lock (double-checked loading);
/// after a failed load, retries are skipped for 60s so an offline first run
/// doesn't retry the download on every request.
pub struct SharedEmbedder {
    inner: std::sync::Mutex<SharedEmbedderInner>,
}

struct SharedEmbedderInner {
    embed: Option<EmbedFn>,
    last_failure: Option<std::time::Instant>,
}

impl SharedEmbedder {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedEmbedderInner {
                embed: None,
                last_failure: None,
            }),
        }
    }

    /// Returns the embed function, loading the model on first use. `None` while
    /// in the post-failure cooldown window.
    pub fn get(&self) -> Option<EmbedFn> {
        // Fast path: already loaded, or in cooldown.
        {
            let state = self.inner.lock().unwrap();
            if let Some(e) = &state.embed {
                return Some(e.clone());
            }
            if let Some(last) = state.last_failure
                && last.elapsed() < std::time::Duration::from_secs(60)
            {
                eprintln!(
                    "EMBED: model load failed recently; skipping retry for 60s \
                     (last failure {}s ago)",
                    last.elapsed().as_secs()
                );
                return None;
            }
        }
        // Slow path: load outside the lock.
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("scylla-reader")
            .join("models");
        eprintln!("Loading embedding model (first run downloads ~91MB)...");
        let load_start = std::time::Instant::now();
        match Embedder::load(cache_dir) {
            Ok(e) => {
                eprintln!(
                    "EMBED: model loaded in {}ms",
                    load_start.elapsed().as_millis()
                );
                let e = Arc::new(e);
                let f: EmbedFn = Arc::new(move |texts: &[&str]| e.batch_embed(texts));
                let mut state = self.inner.lock().unwrap();
                state.embed = Some(f.clone());
                Some(f)
            }
            Err(e) => {
                eprintln!(
                    "EMBED: model load failed in {}ms: {e}",
                    load_start.elapsed().as_millis()
                );
                let mut state = self.inner.lock().unwrap();
                state.last_failure = Some(std::time::Instant::now());
                None
            }
        }
    }

    /// Test-only: an embedder pre-populated with a stub embed function.
    #[cfg(test)]
    pub fn with_embed(f: EmbedFn) -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedEmbedderInner {
                embed: Some(f),
                last_failure: None,
            }),
        }
    }

    /// Test-only: an embedder in the failure cooldown (`get()` returns `None`).
    #[cfg(test)]
    pub fn in_cooldown() -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedEmbedderInner {
                embed: None,
                last_failure: Some(std::time::Instant::now()),
            }),
        }
    }
}

impl Embedder {
    /// Downloads (on first run) and loads the model from the given cache dir.
    pub fn load(cache_dir: PathBuf) -> Result<Self> {
        let client = hf_hub::HFClient::builder().cache_dir(cache_dir).build()?;
        let client = hf_hub::HFClientSync::from_inner(client)?;
        let repo = client.model("sentence-transformers", "all-MiniLM-L6-v2");

        let config_path = repo.download_file().filename("config.json").send()?;
        let tokenizer_path = repo.download_file().filename("tokenizer.json").send()?;
        let weights_path = repo.download_file().filename("model.safetensors").send()?;

        let config: Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(|e| anyhow::anyhow!("failed to set truncation: {e}"))?;

        let device = Device::Cpu;
        // SAFETY: mmap of a local safetensors file; the file outlives the builder.
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)? };
        let model = BertModel::load(vb, &config)?;

        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Embeds `text` into a 384-dim L2-normalized vector.
    ///
    /// Single-text convenience wrapper around [`Self::batch_embed`]; the
    /// embedding pipeline uses the batched path.
    #[allow(dead_code)]
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;
        let token_ids = Tensor::new(encoding.get_ids(), &self.device)?.unsqueeze(0)?;
        let token_type_ids = Tensor::new(encoding.get_type_ids(), &self.device)?.unsqueeze(0)?;
        let attention_mask =
            Tensor::new(encoding.get_attention_mask(), &self.device)?.unsqueeze(0)?;

        let out = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))?;
        let pooled = self.pool(&out, &attention_mask)?;

        Ok(pooled.get(0)?.to_vec1()?)
    }

    /// Embeds a batch of texts in a single forward pass. Each text is truncated
    /// to 512 tokens; shorter sequences are padded to the batch's max length
    /// with the pad token id 0 (masked out of the pooling). Returns one
    /// 384-dim L2-normalized vector per input.
    pub fn batch_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let tokenize_start = std::time::Instant::now();
        let encodings: Vec<_> = texts
            .iter()
            .map(|t| {
                self.tokenizer
                    .encode(*t, true)
                    .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))
            })
            .collect::<Result<_>>()?;
        let tokenize_time = tokenize_start.elapsed();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);
        let batch = texts.len();
        let mut input_ids = vec![0u32; batch * max_len];
        let mut attention_mask = vec![0u32; batch * max_len];
        let token_type_ids = vec![0u32; batch * max_len];
        for (i, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            for (j, (&id, &m)) in ids.iter().zip(mask.iter()).enumerate() {
                input_ids[i * max_len + j] = id;
                attention_mask[i * max_len + j] = m;
            }
        }
        let input_ids =
            Tensor::new(input_ids.as_slice(), &self.device)?.reshape((batch, max_len))?;
        let attention_mask =
            Tensor::new(attention_mask.as_slice(), &self.device)?.reshape((batch, max_len))?;
        let token_type_ids =
            Tensor::new(token_type_ids.as_slice(), &self.device)?.reshape((batch, max_len))?;

        let forward_start = std::time::Instant::now();
        let out = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;
        let pooled = self.pool(&out, &attention_mask)?;
        let forward_time = forward_start.elapsed();

        eprintln!(
            "EMBED: batch_embed {} texts max_len {}: tokenize {}ms, forward {}ms",
            batch,
            max_len,
            tokenize_time.as_millis(),
            forward_time.as_millis()
        );

        let mut result = Vec::with_capacity(batch);
        for i in 0..batch {
            result.push(pooled.get(i)?.to_vec1()?);
        }
        Ok(result)
    }

    /// Masked mean pooling + L2 normalization over the last hidden state.
    fn pool(&self, out: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let mask_f = attention_mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = out
            .broadcast_mul(&mask_f)?
            .sum(1)?
            .broadcast_div(&mask_f.sum(1)?)?;
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(pooled.broadcast_div(&norm)?)
    }
}

/// Mean-pools the description (weighted 2x) and chapter embeddings into a
/// single L2-normalized book-level vector. Returns an empty vector when there
/// is nothing to pool.
pub fn aggregate(
    description_embedding: Option<&[f32]>,
    chapter_embeddings: &[Vec<f32>],
) -> Vec<f32> {
    let dim = description_embedding
        .map(|d| d.len())
        .or_else(|| chapter_embeddings.first().map(|c| c.len()));
    let Some(dim) = dim else {
        return Vec::new();
    };
    let mut sum = vec![0.0f32; dim];
    let mut count = 0usize;
    if let Some(desc) = description_embedding {
        for (i, v) in desc.iter().enumerate() {
            sum[i] += v * 2.0;
        }
        count += 2;
    }
    for ch in chapter_embeddings {
        for (i, v) in ch.iter().enumerate() {
            sum[i] += v;
        }
        count += 1;
    }
    if count == 0 {
        return Vec::new();
    }
    let mut agg: Vec<f32> = sum.iter().map(|s| s / count as f32).collect();
    l2_normalize(&mut agg);
    agg
}

/// In-place L2 normalization (no-op for a zero vector).
fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Returns the top-`k` genre names by cosine similarity to `aggregate`.
pub fn classify(
    aggregate: &[f32],
    genre_embeddings: &[(String, Vec<f32>)],
    k: usize,
) -> Vec<String> {
    let mut scored: Vec<(f32, &str)> = genre_embeddings
        .iter()
        .filter_map(|(name, emb)| {
            if emb.len() != aggregate.len() {
                return None;
            }
            Some((cosine_similarity(aggregate, emb), name.as_str()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
        .into_iter()
        .map(|(_, name)| name.to_string())
        .collect()
}

/// Cosine similarity between two vectors. Returns 0.0 on dimension mismatch or
/// when either vector is zero.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Ranks books by cosine similarity to the query, descending, truncated to
/// `limit`. Input is (book_url, aggregate_embedding, genres).
pub fn rank_books(books: &[BookAggregate], query: &[f32], limit: usize) -> Vec<RankedBook> {
    let mut scored: Vec<RankedBook> = books
        .iter()
        .map(|(url, agg, genres)| {
            let score = cosine_similarity(query, agg);
            (url.clone(), score, genres.clone())
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// Ranks chapters by cosine similarity to the query, descending, truncated to
/// `limit` hits. Returns a flat list (the TUI groups by book). Input is
/// enriched `ChapterHit` tuples.
///
/// Diversity: at most [`MAX_PER_BOOK`] chapters per book are taken from the
/// ranked list first; remaining slots (up to `limit`) are backfilled from the
/// rest. Results below [`SCORE_FLOOR`] are dropped, and the surviving scores
/// are normalized to a 0–100 display scale over the final result set.
pub fn rank_chapters(
    chapters: &[ChapterHit],
    query: &[f32],
    limit: usize,
) -> Vec<RankedChapterHit> {
    let mut scored: Vec<RankedChapterHit> = chapters
        .iter()
        .filter_map(
            |(book_url, book_title, chapter_url, chapter_title, chapter_idx, genres, emb)| {
                let score = cosine_similarity(query, emb);
                (score >= SCORE_FLOOR).then(|| {
                    (
                        book_url.clone(),
                        book_title.clone(),
                        chapter_url.clone(),
                        chapter_title.clone(),
                        *chapter_idx,
                        genres.clone(),
                        score,
                    )
                })
            },
        )
        .collect();
    scored.sort_by(|a, b| b.6.partial_cmp(&a.6).unwrap_or(std::cmp::Ordering::Equal));

    // Per-book cap: take up to MAX_PER_BOOK per book, then backfill the rest.
    let mut per_book: HashMap<String, usize> = HashMap::new();
    let mut capped: Vec<RankedChapterHit> = Vec::new();
    let mut overflow: Vec<RankedChapterHit> = Vec::new();
    for hit in scored {
        let count = *per_book.get(&hit.0).unwrap_or(&0);
        if count < MAX_PER_BOOK {
            per_book.insert(hit.0.clone(), count + 1);
            capped.push(hit);
        } else {
            overflow.push(hit);
        }
    }
    capped.extend(overflow);
    capped.truncate(limit);
    normalize_scores(&mut capped);
    capped
}

/// Cosine floor: chapters scoring below this are dropped as irrelevant.
pub const SCORE_FLOOR: f32 = 0.25;

/// Per-book cap for library-level chapter ranking (result diversity).
pub const MAX_PER_BOOK: usize = 3;

/// Normalizes cosine scores to a 0–100 display scale (min-max over the set).
/// A degenerate set (single score or all equal) maps to 100.
fn normalize_scores(hits: &mut [RankedChapterHit]) {
    let min = hits.iter().map(|h| h.6).fold(f32::INFINITY, f32::min);
    let max = hits.iter().map(|h| h.6).fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;
    for hit in hits.iter_mut() {
        hit.6 = if range <= f32::EPSILON {
            100.0
        } else {
            ((hit.6 - min) / range) * 100.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_of(v: f32, len: usize) -> Vec<f32> {
        vec![v; len]
    }

    fn chapter_hit(book_url: &str, chapter_url: &str, emb: [f32; 2]) -> ChapterHit {
        (
            book_url.to_string(),
            format!("Book {}", book_url),
            chapter_url.to_string(),
            format!("Chapter {}", chapter_url),
            1u32,
            None,
            emb.to_vec(),
        )
    }

    #[test]
    fn test_aggregate_description_only() {
        let desc = vec![3.0, 4.0];
        let agg = aggregate(Some(&desc), &[]);
        // Mean of [desc, desc] == desc, then L2-normalized (norm 5).
        assert_eq!(agg, vec![0.6, 0.8]);
    }

    #[test]
    fn test_aggregate_chapters_only() {
        let ch1 = vec![1.0, 0.0];
        let ch2 = vec![3.0, 0.0];
        let agg = aggregate(None, &[ch1, ch2]);
        // Mean [2, 0], normalized -> [1, 0].
        assert_eq!(agg, vec![1.0, 0.0]);
    }

    #[test]
    fn test_aggregate_description_weighted_2x() {
        let desc = vec![10.0, 0.0];
        let ch1 = vec![0.0, 0.0];
        let ch2 = vec![0.0, 0.0];
        // (2*10 + 0 + 0) / 4 = [5, 0], normalized -> [1, 0].
        let agg = aggregate(Some(&desc), &[ch1, ch2]);
        assert_eq!(agg, vec![1.0, 0.0]);
    }

    #[test]
    fn test_aggregate_empty_returns_empty() {
        assert!(aggregate(None, &[]).is_empty());
    }

    #[test]
    fn test_aggregate_mixed() {
        let desc = vec![4.0, 0.0];
        let ch1 = vec![2.0, 0.0];
        let ch2 = vec![6.0, 0.0];
        // (2*4 + 2 + 6) / 4 = [4, 0], normalized -> [1, 0].
        let agg = aggregate(Some(&desc), &[ch1, ch2]);
        assert_eq!(agg, vec![1.0, 0.0]);
    }

    #[test]
    fn test_aggregate_is_l2_normalized() {
        let desc = vec![1.0, 2.0, 3.0];
        let ch = vec![4.0, 5.0, 6.0];
        let agg = aggregate(Some(&desc), &[ch]);
        let norm: f32 = agg.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_classify_top_k_ordering() {
        let agg = vec_of(1.0, 4);
        let genres = vec![
            ("far".to_string(), vec_of(0.0, 4)),
            ("close".to_string(), vec_of(1.0, 4)),
            ("mid".to_string(), vec_of(0.5, 4)),
        ];
        let top = classify(&agg, &genres, 2);
        assert_eq!(top, vec!["close".to_string(), "mid".to_string()]);
    }

    #[test]
    fn test_classify_k_larger_than_genres() {
        let agg = vec_of(1.0, 4);
        let genres = vec![("a".to_string(), vec_of(1.0, 4))];
        let top = classify(&agg, &genres, 5);
        assert_eq!(top, vec!["a".to_string()]);
    }

    #[test]
    fn test_classify_skips_dim_mismatch() {
        let agg = vec_of(1.0, 4);
        let genres = vec![
            ("ok".to_string(), vec_of(1.0, 4)),
            ("bad".to_string(), vec_of(1.0, 8)),
        ];
        let top = classify(&agg, &genres, 2);
        assert_eq!(top, vec!["ok".to_string()]);
    }

    #[test]
    fn test_cosine_similarity_identical_is_one() {
        let v = vec_of(3.0, 4);
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_dim_mismatch_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[]), 0.0);
    }

    #[test]
    fn test_rank_books_orders_by_similarity() {
        let books = vec![
            ("b".to_string(), vec![0.0, 1.0], None),
            (
                "a".to_string(),
                vec![1.0, 0.0],
                Some(vec!["Fantasy".to_string()]),
            ),
            ("c".to_string(), vec![0.5, 0.5], None),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_books(&books, &query, 10);
        assert_eq!(ranked[0].0, "a");
        assert_eq!(ranked[1].0, "c");
        assert_eq!(ranked[2].0, "b");
        assert_eq!(ranked[0].2, Some(vec!["Fantasy".to_string()]));
    }

    #[test]
    fn test_rank_books_limit() {
        let books = vec![
            ("a".to_string(), vec![1.0, 0.0], None),
            ("b".to_string(), vec![0.0, 1.0], None),
            ("c".to_string(), vec![0.5, 0.5], None),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_books(&books, &query, 2);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0, "a");
    }

    #[test]
    fn test_rank_books_ties_keep_input_order() {
        let books = vec![
            ("a".to_string(), vec![1.0, 0.0], None),
            ("b".to_string(), vec![1.0, 0.0], None),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_books(&books, &query, 10);
        assert_eq!(ranked[0].0, "a");
        assert_eq!(ranked[1].0, "b");
    }

    #[test]
    fn test_rank_books_skips_dim_mismatch() {
        let books = vec![
            ("a".to_string(), vec![1.0, 0.0], None),
            ("bad".to_string(), vec![1.0, 0.0, 0.0], None),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_books(&books, &query, 10);
        // The mismatched book scores 0.0 and sorts last.
        assert_eq!(ranked[0].0, "a");
        assert_eq!(ranked[1].0, "bad");
        assert_eq!(ranked[1].1, 0.0);
    }

    #[test]
    fn test_rank_chapters_flat_sorted_by_score() {
        let chapters = vec![
            chapter_hit("book1", "ch1", [1.0, 0.0]),
            // Orthogonal to the query — dropped by the score floor.
            chapter_hit("book2", "ch2", [0.0, 1.0]),
            chapter_hit("book1", "ch3", [0.9, 0.1]),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 2);
        // Flat, sorted by score desc.
        assert_eq!(ranked[0].2, "ch1");
        assert_eq!(ranked[0].0, "book1");
        assert_eq!(ranked[0].1, "Book book1");
        assert_eq!(ranked[0].3, "Chapter ch1");
        assert_eq!(ranked[0].4, 1);
        assert_eq!(ranked[0].5, None);
        assert_eq!(ranked[1].2, "ch3");
        // Normalized: best hit maps to 100, worst surviving hit to 0.
        assert_eq!(ranked[0].6, 100.0);
        assert_eq!(ranked[1].6, 0.0);
    }

    #[test]
    fn test_rank_chapters_limit_truncates_hits() {
        let chapters = vec![
            chapter_hit("book1", "ch1", [1.0, 0.0]),
            chapter_hit("book1", "ch2", [0.9, 0.1]),
            chapter_hit("book1", "ch3", [0.8, 0.2]),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 2);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].2, "ch1");
        assert_eq!(ranked[1].2, "ch2");
    }

    #[test]
    fn test_rank_chapters_caps_per_book_then_backfills() {
        // book1 dominates the ranked list; book2 has one strong hit.
        let chapters = vec![
            chapter_hit("book1", "ch1", [1.0, 0.0]),
            chapter_hit("book2", "ch6", [0.95, 0.05]),
            chapter_hit("book1", "ch2", [0.9, 0.1]),
            chapter_hit("book1", "ch3", [0.8, 0.2]),
            chapter_hit("book1", "ch4", [0.7, 0.3]),
            chapter_hit("book1", "ch5", [0.6, 0.4]),
            chapter_hit("book2", "ch7", [0.5, 0.5]),
        ];
        let query = vec![1.0, 0.0];

        // limit 4: the top-3-per-book pass fills the window — book1 capped at 3.
        let ranked = rank_chapters(&chapters, &query, 4);
        assert_eq!(ranked.len(), 4);
        assert_eq!(ranked.iter().filter(|h| h.0 == "book1").count(), 3);
        assert_eq!(ranked.iter().filter(|h| h.0 == "book2").count(), 1);
        // book2's best chapter made the cut.
        assert!(ranked.iter().any(|h| h.2 == "ch6"));

        // limit 6: remaining slots are backfilled from the overflow. book2's
        // second chapter (ch7) takes a capped slot, so only ch4 backfills.
        let ranked = rank_chapters(&chapters, &query, 6);
        assert_eq!(ranked.len(), 6);
        assert_eq!(ranked.iter().filter(|h| h.0 == "book1").count(), 4);
        assert_eq!(ranked.iter().filter(|h| h.0 == "book2").count(), 2);
        assert!(ranked.iter().any(|h| h.2 == "ch4"));
        assert!(!ranked.iter().any(|h| h.2 == "ch5"));
    }

    #[test]
    fn test_rank_chapters_drops_below_floor() {
        let chapters = vec![
            chapter_hit("book1", "ch1", [1.0, 0.0]), // cos 1.0 — kept
            chapter_hit("book1", "ch2", [0.0, 1.0]), // cos 0.0 — dropped
            chapter_hit("book1", "ch3", [0.3, 0.7]), // cos ~0.39 — kept
            chapter_hit("book1", "ch4", [0.2, 0.8]), // cos ~0.24 — dropped
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].2, "ch1");
        assert_eq!(ranked[1].2, "ch3");
    }

    #[test]
    fn test_rank_chapters_normalizes_scores_to_0_100() {
        let chapters = vec![
            chapter_hit("book1", "ch1", [1.0, 0.0]), // cos 1.0
            chapter_hit("book1", "ch2", [0.5, 0.5]), // cos ~0.707
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].6, 100.0);
        assert_eq!(ranked[1].6, 0.0);
    }

    #[test]
    fn test_rank_chapters_single_hit_normalizes_to_100() {
        let chapters = vec![chapter_hit("book1", "ch1", [0.5, 0.5])];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].6, 100.0);
    }
}
