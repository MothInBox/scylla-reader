//! Semantic embeddings via bge-small-en-v1.5 (candle, CPU, pure Rust).
//!
//! The model is downloaded from Hugging Face on first use and cached under the
//! data dir. All embedding work is best-effort: callers log failures and skip.
//!
//! bge is an *asymmetric* embedding model: queries get an instruction prefix
//! while passages (chapters, descriptions, genres) get none. See [`EmbedMode`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use candle_core::{DType, Device, Module, Tensor};
use candle_nn::{Linear, VarBuilder, linear};
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::tokenizer::{Tokenizer, TruncationParams, TruncationStrategy};

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

/// Identity of the embedding model. Included in content hashes so a model swap
/// forces a re-embed (vectors from different models live in different spaces).
pub const MODEL_VERSION: &str = "bge-small-en-v1.5";

/// Whether a text is a search query or a passage to be embedded.
///
/// bge uses an asymmetric instruction prefix: queries get
/// `"Represent this sentence for searching relevant passages: "`, passages get
/// nothing. Mixing the two without the prefix measurably degrades retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedMode {
    /// A user search query — gets the bge instruction prefix.
    Query,
    /// A passage (chapter chunk, description, genre) — no prefix.
    Passage,
}

/// bge-small-en-v1.5 sentence embedder.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

/// A callable that embeds a batch of texts into vectors (the real model or a
/// test stub). The mode selects the bge query/passage prefix.
pub type EmbedFn = Arc<dyn Fn(&[&str], EmbedMode) -> anyhow::Result<Vec<Vec<f32>>> + Send + Sync>;

/// A callable that splits a chapter into 512-token chunk windows (the real
/// model's tokenizer or a test stub).
pub type ChunkFn = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// A callable that scores (query, passage) pairs with a cross-encoder (the
/// real model or a test stub). Returns one relevance score per passage.
pub type RerankFn = Arc<dyn Fn(&str, &[&str]) -> anyhow::Result<Vec<f32>> + Send + Sync>;

/// A book aggregate row: (book_url, aggregate_embedding, genres).
pub type BookAggregate = (String, Vec<f32>, Option<Vec<String>>);

/// A book aggregate row with its title: (book_url, title, aggregate_embedding, genres).
pub type BookAggregateWithTitle = (String, String, Vec<f32>, Option<Vec<String>>);

/// A ranked book result: (book_url, score, genres).
pub type RankedBook = (String, f32, Option<Vec<String>>);

/// A chunk embedding row enriched for search.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkHit {
    pub book_url: String,
    pub book_title: String,
    pub chapter_url: String,
    pub chapter_title: String,
    pub chapter_idx: u32,
    pub genres: Option<Vec<String>>,
    pub chunk_idx: u32,
    pub text: String,
    pub embedding: Vec<f32>,
}

/// A ranked chapter hit (best chunk per chapter):
/// (book_url, book_title, chapter_url, chapter_title, chapter_idx, genres,
/// score, best_chunk_text).
pub type RankedChapterHit = (
    String,
    String,
    String,
    String,
    u32,
    Option<Vec<String>>,
    f32,
    String,
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
    chunk: Option<ChunkFn>,
    last_failure: Option<std::time::Instant>,
}

impl SharedEmbedder {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedEmbedderInner {
                embed: None,
                chunk: None,
                last_failure: None,
            }),
        }
    }

    /// Returns the embed function, loading the model on first use. `None` while
    /// in the post-failure cooldown window.
    pub fn get(&self) -> Option<EmbedFn> {
        self.load_if_needed();
        self.inner.lock().unwrap().embed.clone()
    }

    /// Returns the chunk function (512-token windows from the model's
    /// tokenizer), loading the model on first use. `None` while in the
    /// post-failure cooldown window.
    pub fn get_chunker(&self) -> Option<ChunkFn> {
        self.load_if_needed();
        self.inner.lock().unwrap().chunk.clone()
    }

    /// Loads the model on first use (double-checked, outside the lock). No-op
    /// when already loaded or in the post-failure cooldown window.
    fn load_if_needed(&self) {
        // Fast path: already loaded, or in cooldown.
        {
            let state = self.inner.lock().unwrap();
            if state.embed.is_some() {
                return;
            }
            if let Some(last) = state.last_failure
                && last.elapsed() < std::time::Duration::from_secs(60)
            {
                eprintln!(
                    "EMBED: model load failed recently; skipping retry for 60s \
                     (last failure {}s ago)",
                    last.elapsed().as_secs()
                );
                return;
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
                let e_chunk = Arc::clone(&e);
                let f: EmbedFn =
                    Arc::new(move |texts: &[&str], mode: EmbedMode| e.batch_embed(texts, mode));
                let c: ChunkFn = Arc::new(move |text: &str| e_chunk.chunk(text));
                let mut state = self.inner.lock().unwrap();
                state.embed = Some(f);
                state.chunk = Some(c);
            }
            Err(e) => {
                eprintln!(
                    "EMBED: model load failed in {}ms: {e}",
                    load_start.elapsed().as_millis()
                );
                let mut state = self.inner.lock().unwrap();
                state.last_failure = Some(std::time::Instant::now());
            }
        }
    }

    /// Test-only: an embedder pre-populated with a stub embed function and a
    /// one-chunk-per-chapter chunker.
    #[cfg(test)]
    pub fn with_embed(f: EmbedFn) -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedEmbedderInner {
                embed: Some(f),
                chunk: Some(Arc::new(|text: &str| vec![text.to_string()])),
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
                chunk: None,
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
        let repo = client.model("BAAI", "bge-small-en-v1.5");

        let config_path = repo.download_file().filename("config.json").send()?;
        let tokenizer_path = repo.download_file().filename("tokenizer.json").send()?;
        let weights_path = repo.download_file().filename("model.safetensors").send()?;

        let config: Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;
        // No tokenizer-level truncation: `batch_embed` truncates manually and
        // `chunk` needs the full token sequence to build windows.
        tokenizer
            .with_truncation(None)
            .map_err(|e| anyhow::anyhow!("failed to clear truncation: {e}"))?;

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

    /// Splits `text` into 512-token windows with a 64-token overlap, using the
    /// model's tokenizer so chunk boundaries match the embedder's vocabulary.
    /// Returns the original text as a single chunk when tokenization fails or
    /// the text is empty.
    pub fn chunk(&self, text: &str) -> Vec<String> {
        const CHUNK_TOKENS: usize = 512;
        const CHUNK_OVERLAP: usize = 64;
        let encoding = match self.tokenizer.encode(text, false) {
            Ok(e) => e,
            Err(_) => return vec![text.to_string()],
        };
        let offsets = encoding.get_offsets();
        if offsets.is_empty() {
            return vec![text.to_string()];
        }
        let stride = CHUNK_TOKENS - CHUNK_OVERLAP;
        let mut chunks = Vec::new();
        let mut start = 0usize;
        while start < offsets.len() {
            let end = (start + CHUNK_TOKENS).min(offsets.len());
            let byte_start = offsets[start].0;
            let byte_end = offsets[end - 1].1;
            let chunk = text.get(byte_start..byte_end).map(str::trim).unwrap_or("");
            if !chunk.is_empty() {
                chunks.push(chunk.to_string());
            }
            if end == offsets.len() {
                break;
            }
            start += stride;
        }
        if chunks.is_empty() {
            vec![text.to_string()]
        } else {
            chunks
        }
    }

    /// Embeds a batch of texts in a single forward pass. Each text is truncated
    /// to 512 tokens; shorter sequences are padded to the batch's max length
    /// with the pad token id 0 (masked out of the pooling). Returns one
    /// 384-dim L2-normalized vector per input.
    ///
    /// Queries are prefixed with the bge instruction (see [`EmbedMode`]);
    /// passages are embedded as-is.
    pub fn batch_embed(&self, texts: &[&str], mode: EmbedMode) -> Result<Vec<Vec<f32>>> {
        const MAX_TOKENS: usize = 512;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let tokenize_start = std::time::Instant::now();
        let encodings: Vec<_> = texts
            .iter()
            .map(|t| {
                let t = prefix(t, mode);
                self.tokenizer
                    .encode(&*t, true)
                    .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))
            })
            .collect::<Result<_>>()?;
        let tokenize_time = tokenize_start.elapsed();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len().min(MAX_TOKENS))
            .max()
            .unwrap_or(0);
        let batch = texts.len();
        let mut input_ids = vec![0u32; batch * max_len];
        let mut attention_mask = vec![0u32; batch * max_len];
        let token_type_ids = vec![0u32; batch * max_len];
        for (i, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let len = ids.len().min(MAX_TOKENS);
            for j in 0..len {
                input_ids[i * max_len + j] = ids[j];
                attention_mask[i * max_len + j] = mask[j];
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

/// bge instruction prefix for queries (asymmetric retrieval). Passages are
/// embedded as-is.
fn prefix<'a>(text: &'a str, mode: EmbedMode) -> std::borrow::Cow<'a, str> {
    match mode {
        EmbedMode::Query => {
            const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
            std::borrow::Cow::Owned(format!("{QUERY_PREFIX}{text}"))
        }
        EmbedMode::Passage => std::borrow::Cow::Borrowed(text),
    }
}

/// ms-marco-MiniLM-L-6-v2 cross-encoder: scores (query, passage) pairs.
///
/// Unlike the bi-encoder (which embeds query and passage independently and
/// compares with cosine), a cross-encoder feeds the pair through the model
/// together, so it can attend across the boundary — much more precise, but
/// O(n) forward passes per query. Used to rerank the bi-encoder's top-50.
pub struct CrossEncoder {
    model: BertModel,
    classifier: Linear,
    tokenizer: Tokenizer,
    device: Device,
}

impl CrossEncoder {
    /// Downloads (on first run) and loads the model from the given cache dir.
    pub fn load(cache_dir: PathBuf) -> Result<Self> {
        let client = hf_hub::HFClient::builder().cache_dir(cache_dir).build()?;
        let client = hf_hub::HFClientSync::from_inner(client)?;
        let repo = client.model("cross-encoder", "ms-marco-MiniLM-L-6-v2");

        let config_path = repo.download_file().filename("config.json").send()?;
        let tokenizer_path = repo.download_file().filename("tokenizer.json").send()?;
        let weights_path = repo.download_file().filename("model.safetensors").send()?;

        let config: Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;
        // Pairs are truncated at 256 tokens (the model's training limit).
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 256,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
                direction: tokenizers::tokenizer::TruncationDirection::Right,
            }))
            .map_err(|e| anyhow::anyhow!("failed to set truncation: {e}"))?;

        let device = Device::Cpu;
        // SAFETY: mmap of a local safetensors file; the file outlives the builder.
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)? };
        let model = BertModel::load(vb.pp("bert"), &config)?;
        let classifier = linear(config.hidden_size, 2, vb.pp("classifier"))?;

        Ok(Self {
            model,
            classifier,
            tokenizer,
            device,
        })
    }

    /// Scores (query, passage) pairs in a single forward pass. Returns one
    /// relevance score per passage (the logit of the "relevant" class).
    pub fn score(&self, query: &str, passages: &[&str]) -> Result<Vec<f32>> {
        if passages.is_empty() {
            return Ok(Vec::new());
        }
        let encodings: Vec<_> = passages
            .iter()
            .map(|p| {
                self.tokenizer
                    .encode((query, *p), true)
                    .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))
            })
            .collect::<Result<_>>()?;
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);
        let batch = passages.len();
        let mut input_ids = vec![0u32; batch * max_len];
        let mut attention_mask = vec![0u32; batch * max_len];
        let mut token_type_ids = vec![0u32; batch * max_len];
        for (i, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let types = enc.get_type_ids();
            let len = ids.len();
            for j in 0..len {
                input_ids[i * max_len + j] = ids[j];
                attention_mask[i * max_len + j] = mask[j];
                token_type_ids[i * max_len + j] = types[j];
            }
        }
        let input_ids =
            Tensor::new(input_ids.as_slice(), &self.device)?.reshape((batch, max_len))?;
        let attention_mask =
            Tensor::new(attention_mask.as_slice(), &self.device)?.reshape((batch, max_len))?;
        let token_type_ids =
            Tensor::new(token_type_ids.as_slice(), &self.device)?.reshape((batch, max_len))?;

        let out = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;
        // CLS pooling + classifier head → (batch, 2) logits.
        let cls = out.narrow(1, 0, 1)?.squeeze(1)?;
        let logits = self.classifier.forward(&cls)?;
        // Score = logit of the "relevant" class (index 1).
        let relevant = logits.narrow(1, 1, 1)?.squeeze(1)?;
        Ok(relevant.to_vec1()?)
    }
}

/// Shared, lazily-loaded cross-encoder with a failure cooldown (mirrors
/// [`SharedEmbedder`]).
pub struct SharedCrossEncoder {
    inner: std::sync::Mutex<SharedCrossEncoderInner>,
}

struct SharedCrossEncoderInner {
    rerank: Option<RerankFn>,
    last_failure: Option<std::time::Instant>,
}

impl SharedCrossEncoder {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedCrossEncoderInner {
                rerank: None,
                last_failure: None,
            }),
        }
    }

    /// Returns the rerank function, loading the model on first use. `None`
    /// while in the post-failure cooldown window.
    pub fn get(&self) -> Option<RerankFn> {
        self.load_if_needed();
        self.inner.lock().unwrap().rerank.clone()
    }

    /// Loads the model on first use (double-checked, outside the lock). No-op
    /// when already loaded or in the post-failure cooldown window.
    fn load_if_needed(&self) {
        {
            let state = self.inner.lock().unwrap();
            if state.rerank.is_some() {
                return;
            }
            if let Some(last) = state.last_failure
                && last.elapsed() < std::time::Duration::from_secs(60)
            {
                eprintln!(
                    "RERANK: model load failed recently; skipping retry for 60s \
                     (last failure {}s ago)",
                    last.elapsed().as_secs()
                );
                return;
            }
        }
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("scylla-reader")
            .join("models");
        eprintln!("Loading reranker model (first run downloads ~90MB)...");
        let load_start = std::time::Instant::now();
        match CrossEncoder::load(cache_dir) {
            Ok(e) => {
                eprintln!(
                    "RERANK: model loaded in {}ms",
                    load_start.elapsed().as_millis()
                );
                let e = Arc::new(e);
                let f: RerankFn =
                    Arc::new(move |query: &str, passages: &[&str]| e.score(query, passages));
                let mut state = self.inner.lock().unwrap();
                state.rerank = Some(f);
            }
            Err(e) => {
                eprintln!(
                    "RERANK: model load failed in {}ms: {e}",
                    load_start.elapsed().as_millis()
                );
                let mut state = self.inner.lock().unwrap();
                state.last_failure = Some(std::time::Instant::now());
            }
        }
    }

    /// Test-only: a reranker pre-populated with a stub.
    #[cfg(test)]
    pub fn with_rerank(f: RerankFn) -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedCrossEncoderInner {
                rerank: Some(f),
                last_failure: None,
            }),
        }
    }

    /// Test-only: a reranker in the failure cooldown (`get()` returns `None`).
    #[cfg(test)]
    pub fn in_cooldown() -> Self {
        Self {
            inner: std::sync::Mutex::new(SharedCrossEncoderInner {
                rerank: None,
                last_failure: Some(std::time::Instant::now()),
            }),
        }
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

/// Collapses chunk hits to the best-chunk-per-chapter aggregate: each chapter
/// contributes its highest-cosine chunk (chapter-equal weighting). Returns raw
/// cosine scores (index 6) and the best chunk's text (index 7), in first-seen
/// chapter order.
pub fn best_chunks_per_chapter(chunks: &[ChunkHit], query: &[f32]) -> Vec<RankedChapterHit> {
    let mut order: Vec<String> = Vec::new();
    let mut by_chapter: HashMap<String, usize> = HashMap::new();
    let mut best: Vec<RankedChapterHit> = Vec::new();
    for c in chunks {
        let score = cosine_similarity(query, &c.embedding);
        match by_chapter.get(&c.chapter_url) {
            Some(&idx) => {
                if score > best[idx].6 {
                    best[idx].6 = score;
                    best[idx].7 = c.text.clone();
                }
            }
            None => {
                by_chapter.insert(c.chapter_url.clone(), best.len());
                order.push(c.chapter_url.clone());
                best.push((
                    c.book_url.clone(),
                    c.book_title.clone(),
                    c.chapter_url.clone(),
                    c.chapter_title.clone(),
                    c.chapter_idx,
                    c.genres.clone(),
                    score,
                    c.text.clone(),
                ));
            }
        }
    }
    // Reorder to first-seen chapter order (HashMap iteration is unordered).
    let mut out = Vec::with_capacity(best.len());
    for url in order {
        if let Some(idx) = by_chapter.remove(&url) {
            out.push(best[idx].clone());
        }
    }
    out
}

/// Ranks chapters by cosine similarity to the query, descending, truncated to
/// `limit` hits. Returns a flat list (the TUI groups by book). Input is chunk
/// rows; each chapter contributes its best chunk (chapter-equal weighting).
///
/// Diversity: at most [`MAX_PER_BOOK`] chapters per book are taken from the
/// ranked list first; remaining slots (up to `limit`) are backfilled from the
/// rest. The cap is a no-op for single-book inputs (all of that book's hits
/// backfill) — Phase 2's per-book drill-down relies on this. Results below
/// [`SCORE_FLOOR`] are dropped, and the surviving scores are mapped to a 0–100
/// display scale via a fixed mapping (see [`normalize_score`]).
pub fn rank_chapters(chunks: &[ChunkHit], query: &[f32], limit: usize) -> Vec<RankedChapterHit> {
    let mut scored = best_chunks_per_chapter(chunks, query);
    scored.retain(|h| h.6 >= SCORE_FLOOR);
    scored.sort_by(|a, b| b.6.partial_cmp(&a.6).unwrap_or(std::cmp::Ordering::Equal));

    let mut capped = cap_per_book(scored, MAX_PER_BOOK);
    capped.truncate(limit);
    // Backfill can interleave books, so restore descending order before the
    // display mapping.
    capped.sort_by(|a, b| b.6.partial_cmp(&a.6).unwrap_or(std::cmp::Ordering::Equal));
    for hit in &mut capped {
        hit.6 = normalize_score(hit.6);
    }
    capped
}

/// Applies the per-book diversity cap: keeps up to `per_book` hits per book
/// from the (already sorted) list, then backfills the rest. A no-op for
/// single-book inputs (all hits backfill) — the per-book drill-down relies on
/// this. Used both by [`rank_chapters`] (semantic lane) and post-fusion so the
/// hybrid list gets the same diversity guarantee as the semantic lane.
pub fn cap_per_book(hits: Vec<RankedChapterHit>, per_book: usize) -> Vec<RankedChapterHit> {
    let mut per_book_count: HashMap<String, usize> = HashMap::new();
    let mut capped: Vec<RankedChapterHit> = Vec::new();
    let mut overflow: Vec<RankedChapterHit> = Vec::new();
    for hit in hits {
        let count = *per_book_count.get(&hit.0).unwrap_or(&0);
        if count < per_book {
            per_book_count.insert(hit.0.clone(), count + 1);
            capped.push(hit);
        } else {
            overflow.push(hit);
        }
    }
    capped.extend(overflow);
    capped
}

/// Cosine floor: chapters scoring below this are dropped as irrelevant.
pub const SCORE_FLOOR: f32 = 0.25;

/// Per-book cap for library-level chapter ranking (result diversity).
pub const MAX_PER_BOOK: usize = 3;

/// Maps a cosine score to the 0–100 display scale with a fixed mapping from
/// the [`SCORE_FLOOR`]..=1.0 range, clamped to [0, 100]. Unlike min-max, the
/// result is comparable across result sets.
pub(crate) fn normalize_score(cos: f32) -> f32 {
    ((cos - SCORE_FLOOR) / (1.0 - SCORE_FLOOR) * 100.0).clamp(0.0, 100.0)
}

/// Groups ranked chapter hits by book, keeping the top `per_book` per book.
/// The input is sorted by score descending before grouping, so the first
/// `per_book` hits seen per book are its best regardless of input order.
pub fn top_chapters_per_book(
    mut ranked: Vec<RankedChapterHit>,
    per_book: usize,
) -> HashMap<String, Vec<RankedChapterHit>> {
    ranked.sort_by(|a, b| b.6.partial_cmp(&a.6).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_book: HashMap<String, Vec<RankedChapterHit>> = HashMap::new();
    for hit in ranked {
        let group = by_book.entry(hit.0.clone()).or_default();
        if group.len() < per_book {
            group.push(hit);
        }
    }
    by_book
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_of(v: f32, len: usize) -> Vec<f32> {
        vec![v; len]
    }

    fn chunk_hit(book_url: &str, chapter_url: &str, emb: [f32; 2]) -> ChunkHit {
        ChunkHit {
            book_url: book_url.to_string(),
            book_title: format!("Book {}", book_url),
            chapter_url: chapter_url.to_string(),
            chapter_title: format!("Chapter {}", chapter_url),
            chapter_idx: 1,
            genres: None,
            chunk_idx: 0,
            text: format!("text of {}", chapter_url),
            embedding: emb.to_vec(),
        }
    }

    fn ranked_chunk_hit(book_url: &str, chapter_url: &str, score: f32) -> RankedChapterHit {
        (
            book_url.to_string(),
            format!("Book {}", book_url),
            chapter_url.to_string(),
            format!("Chapter {}", chapter_url),
            1u32,
            None,
            score,
            format!("text of {}", chapter_url),
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
    fn test_rank_chapters_flat_sorted_desc() {
        let chapters = vec![
            chunk_hit("book1", "ch1", [1.0, 0.0]),
            // Orthogonal to the query — dropped by the score floor.
            chunk_hit("book2", "ch2", [0.0, 1.0]),
            chunk_hit("book1", "ch3", [0.9, 0.1]),
            chunk_hit("book2", "ch4", [0.5, 0.5]),
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 3);
        // Flat, sorted by score desc.
        assert_eq!(ranked[0].2, "ch1");
        assert_eq!(ranked[0].0, "book1");
        assert_eq!(ranked[0].1, "Book book1");
        assert_eq!(ranked[0].3, "Chapter ch1");
        assert_eq!(ranked[0].4, 1);
        assert_eq!(ranked[0].5, None);
        assert_eq!(ranked[1].2, "ch3");
        assert_eq!(ranked[2].2, "ch4");
        // Fixed mapping: cos 1.0 → 100, cos ~0.994 → ~99.18, cos ~0.707 → ~60.95.
        assert_eq!(ranked[0].6, 100.0);
        assert!((ranked[1].6 - 99.18).abs() < 0.01);
        assert!((ranked[2].6 - 60.95).abs() < 0.01);
        // Descending order holds across the whole result set.
        assert!(ranked.windows(2).all(|w| w[0].6 >= w[1].6));
    }

    #[test]
    fn test_rank_chapters_limit_truncates_hits() {
        let chapters = vec![
            chunk_hit("book1", "ch1", [1.0, 0.0]),
            chunk_hit("book1", "ch2", [0.9, 0.1]),
            chunk_hit("book1", "ch3", [0.8, 0.2]),
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
            chunk_hit("book1", "ch1", [1.0, 0.0]),
            chunk_hit("book2", "ch6", [0.95, 0.05]),
            chunk_hit("book1", "ch2", [0.9, 0.1]),
            chunk_hit("book1", "ch3", [0.8, 0.2]),
            chunk_hit("book1", "ch4", [0.7, 0.3]),
            chunk_hit("book1", "ch5", [0.6, 0.4]),
            chunk_hit("book2", "ch7", [0.5, 0.5]),
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
    fn test_cap_per_book_reserves_top_slots_then_backfills() {
        // The post-fusion cap: a dominant book's hits are capped at `per_book`
        // in the leading positions, then the rest backfill. Single-book inputs
        // are a no-op (everything backfills).
        let hits = vec![
            ranked_chunk_hit("book1", "ch1", 100.0),
            ranked_chunk_hit("book1", "ch2", 90.0),
            ranked_chunk_hit("book1", "ch3", 80.0),
            ranked_chunk_hit("book1", "ch4", 70.0),
            ranked_chunk_hit("book2", "ch6", 95.0),
        ];
        let capped = cap_per_book(hits, 3);
        // book1's first 3 are capped, then book2's hit, then book1's overflow.
        assert_eq!(capped[0].2, "ch1");
        assert_eq!(capped[1].2, "ch2");
        assert_eq!(capped[2].2, "ch3");
        assert_eq!(capped[3].2, "ch6");
        assert_eq!(capped[4].2, "ch4");

        // Single-book input: the cap is a no-op (all hits backfill).
        let hits = vec![
            ranked_chunk_hit("book1", "ch1", 100.0),
            ranked_chunk_hit("book1", "ch2", 90.0),
            ranked_chunk_hit("book1", "ch3", 80.0),
            ranked_chunk_hit("book1", "ch4", 70.0),
        ];
        let capped = cap_per_book(hits, 3);
        assert_eq!(capped.len(), 4);
    }

    #[test]
    fn test_rank_chapters_drops_below_floor() {
        let chapters = vec![
            chunk_hit("book1", "ch1", [1.0, 0.0]), // cos 1.0 — kept
            chunk_hit("book1", "ch2", [0.0, 1.0]), // cos 0.0 — dropped
            chunk_hit("book1", "ch3", [0.3, 0.7]), // cos ~0.39 — kept
            chunk_hit("book1", "ch4", [0.2, 0.8]), // cos ~0.24 — dropped
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
            chunk_hit("book1", "ch1", [1.0, 0.0]), // cos 1.0
            chunk_hit("book1", "ch2", [0.5, 0.5]), // cos ~0.707
        ];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].6, 100.0);
        assert!((ranked[1].6 - 60.95).abs() < 0.01);
    }

    #[test]
    fn test_rank_chapters_single_hit_fixed_mapping() {
        let chapters = vec![chunk_hit("book1", "ch1", [0.5, 0.5])];
        let query = vec![1.0, 0.0];
        let ranked = rank_chapters(&chapters, &query, 10);
        assert_eq!(ranked.len(), 1);
        // Fixed mapping, not min-max: a lone ~0.707 cosine maps to ~61, not 100.
        assert!((ranked[0].6 - 60.95).abs() < 0.01);
    }

    #[test]
    fn test_top_chapters_per_book_keeps_best_per_book() {
        // Input is score-descending (as rank_chapters returns it).
        let ranked = vec![
            ranked_chunk_hit("book1", "ch1", 100.0),
            ranked_chunk_hit("book2", "ch6", 99.5),
            ranked_chunk_hit("book1", "ch2", 99.0),
            ranked_chunk_hit("book1", "ch3", 96.0),
            ranked_chunk_hit("book1", "ch4", 89.0),
            ranked_chunk_hit("book2", "ch7", 61.0),
        ];
        let by_book = top_chapters_per_book(ranked, 3);
        let book1 = by_book.get("book1").unwrap();
        let book2 = by_book.get("book2").unwrap();
        // Top-3 per book, in input (score-descending) order.
        assert_eq!(
            book1.iter().map(|h| h.2.as_str()).collect::<Vec<_>>(),
            vec!["ch1", "ch2", "ch3"]
        );
        assert_eq!(
            book2.iter().map(|h| h.2.as_str()).collect::<Vec<_>>(),
            vec!["ch6", "ch7"]
        );
        // A book with fewer than `per_book` hits keeps all of them.
        assert_eq!(book2.len(), 2);
    }

    #[test]
    fn test_top_chapters_per_book_handles_unsorted_input() {
        // Same data, shuffled — the internal sort must still pick the best 3.
        let ranked = vec![
            ranked_chunk_hit("book1", "ch4", 89.0),
            ranked_chunk_hit("book2", "ch6", 99.5),
            ranked_chunk_hit("book1", "ch1", 100.0),
            ranked_chunk_hit("book1", "ch3", 96.0),
            ranked_chunk_hit("book2", "ch7", 61.0),
            ranked_chunk_hit("book1", "ch2", 99.0),
        ];
        let by_book = top_chapters_per_book(ranked, 3);
        assert_eq!(
            by_book
                .get("book1")
                .unwrap()
                .iter()
                .map(|h| h.2.as_str())
                .collect::<Vec<_>>(),
            vec!["ch1", "ch2", "ch3"]
        );
        assert_eq!(
            by_book
                .get("book2")
                .unwrap()
                .iter()
                .map(|h| h.2.as_str())
                .collect::<Vec<_>>(),
            vec!["ch6", "ch7"]
        );
    }

    #[test]
    fn test_prefix_query_gets_bge_instruction() {
        let q = prefix("dragon", EmbedMode::Query);
        assert_eq!(
            q.as_ref(),
            "Represent this sentence for searching relevant passages: dragon"
        );
        let p = prefix("dragon", EmbedMode::Passage);
        assert_eq!(p.as_ref(), "dragon");
    }
}
