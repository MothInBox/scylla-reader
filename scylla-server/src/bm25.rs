//! BM25 keyword retrieval over chapter chunk text.
//!
//! The index is built from the same `ChunkHit` rows the bi-encoder lane uses
//! (per-request, so it is always fresh — no startup staleness). Tokenization is
//! a pure regex (`\w+`, lowercased) with no stemmer: web-novel terminology
//! (names, system terms, item names) is mostly exact-match, and a stemmer
//! would add a dependency for little gain.
//!
//! BM25 is the keyword lane of the hybrid search: it surfaces exact-name and
//! rare-term queries that cosine similarity misses. The two lanes are fused
//! with Reciprocal Rank Fusion (see [`rrf_fuse`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::embeddings::ChunkHit;

/// The `\w+` tokenizer regex, compiled once (compiling per call at 24K chunks
/// would be 24K compilations per search).
fn token_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\w+").expect("static regex is valid"))
}

/// Lowercases `text` and splits it into `\w+` tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    token_regex()
        .find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect()
}

/// One indexed document: a single chunk (not a whole chapter), so the
/// best-matching chunk's text is available as the reranker passage for
/// BM25-only chapters.
struct Bm25Doc {
    chapter_url: String,
    text: String,
    tokens: Vec<String>,
}

/// In-memory BM25 index over chunks.
pub struct Bm25Index {
    docs: Vec<Bm25Doc>,
    /// term → (doc_idx, term_frequency) postings.
    postings: HashMap<String, Vec<(usize, usize)>>,
    doc_lens: Vec<usize>,
    avg_dl: f64,
    n_docs: usize,
}

impl Bm25Index {
    /// Builds the index from chunk hits, one document per chunk.
    pub fn build(chunks: &[ChunkHit]) -> Self {
        let docs: Vec<Bm25Doc> = chunks
            .iter()
            .map(|c| Bm25Doc {
                chapter_url: c.chapter_url.clone(),
                text: c.text.clone(),
                tokens: tokenize(&c.text),
            })
            .collect();
        let n_docs = docs.len();
        let mut postings: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        let mut doc_lens = Vec::with_capacity(n_docs);
        for (i, doc) in docs.iter().enumerate() {
            let mut tf: HashMap<String, usize> = HashMap::new();
            for t in &doc.tokens {
                *tf.entry(t.clone()).or_insert(0) += 1;
            }
            doc_lens.push(doc.tokens.len());
            for (term, count) in tf {
                postings.entry(term).or_default().push((i, count));
            }
        }
        let avg_dl = if n_docs == 0 {
            0.0
        } else {
            doc_lens.iter().sum::<usize>() as f64 / n_docs as f64
        };
        Self {
            docs,
            postings,
            doc_lens,
            avg_dl,
            n_docs,
        }
    }

    /// Scores every chunk against `query` and returns the top `top_k` chapters
    /// as `(score, chapter_url, best_chunk_text)` in descending score order. A
    /// chapter's score is its best-matching chunk's BM25 score, and
    /// `best_chunk_text` is that chunk's text — the passage the cross-encoder
    /// should score for a BM25-only chapter (the highest-cosine chunk may be
    /// unrelated to the matched keyword). Chapters with no matching terms are
    /// omitted.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(f64, String, String)> {
        const K1: f64 = 1.5;
        const B: f64 = 0.75;
        let q_terms = tokenize(query);
        if q_terms.is_empty() || self.n_docs == 0 {
            return Vec::new();
        }
        let mut chunk_scores = vec![0.0f64; self.n_docs];
        for term in q_terms {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len();
            let idf = ((self.n_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln();
            for &(doc_idx, tf) in postings {
                let dl = self.doc_lens[doc_idx] as f64;
                let denom = tf as f64 + K1 * (1.0 - B + B * dl / self.avg_dl);
                chunk_scores[doc_idx] += idf * (tf as f64 * (K1 + 1.0)) / denom;
            }
        }
        // Aggregate per chapter: max over its chunks, tracking the best chunk's
        // text for the reranker passage.
        let mut by_chapter: HashMap<String, (f64, String)> = HashMap::new();
        for (i, score) in chunk_scores.into_iter().enumerate() {
            if score <= 0.0 {
                continue;
            }
            let doc = &self.docs[i];
            let entry = by_chapter
                .entry(doc.chapter_url.clone())
                .or_insert((0.0, String::new()));
            if score > entry.0 {
                *entry = (score, doc.text.clone());
            }
        }
        let mut ranked: Vec<(f64, String, String)> = by_chapter
            .into_iter()
            .map(|(url, (score, text))| (score, url, text))
            .collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_k);
        ranked
    }
}

/// Reciprocal Rank Fusion over ranked lists of chapter URLs (each list is
/// ordered best-first). Returns the fused top `k` URLs in descending RRF
/// score order. A document present in both lanes outranks one present in a
/// single lane, which is what makes the hybrid better than either lane alone.
pub fn rrf_fuse(lists: &[Vec<String>], k: usize) -> Vec<String> {
    const RRF_K: f32 = 60.0;
    let mut scores: HashMap<String, f32> = HashMap::new();
    for list in lists {
        for (rank, url) in list.iter().enumerate() {
            *scores.entry(url.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranked: Vec<(f32, String)> = scores.into_iter().map(|(url, s)| (s, url)).collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().take(k).map(|(_, url)| url).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(book_url: &str, chapter_url: &str, text: &str) -> ChunkHit {
        ChunkHit {
            book_url: book_url.to_string(),
            book_title: format!("Book {}", book_url),
            chapter_url: chapter_url.to_string(),
            chapter_title: format!("Chapter {}", chapter_url),
            chapter_idx: 1,
            genres: None,
            chunk_idx: 0,
            text: text.to_string(),
            embedding: vec![1.0, 0.0],
        }
    }

    #[test]
    fn test_tokenize_lowercases_and_splits() {
        assert_eq!(
            tokenize("The Dragon's Lair, 2024!"),
            vec!["the", "dragon", "s", "lair", "2024"]
        );
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn test_bm25_surfaces_exact_term() {
        let chunks = vec![
            chunk("b1", "ch-dragon", "the dragon appears in the cave"),
            chunk("b1", "ch-other", "nothing relevant here at all"),
        ];
        let index = Bm25Index::build(&chunks);
        let hits = index.search("dragon", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "ch-dragon");
        assert!(hits[0].0 > 0.0);
    }

    #[test]
    fn test_bm25_ranks_higher_tf_first() {
        let chunks = vec![
            chunk("b1", "ch-once", "dragon"),
            chunk("b1", "ch-many", "dragon dragon dragon"),
        ];
        let index = Bm25Index::build(&chunks);
        let hits = index.search("dragon", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].1, "ch-many");
        assert!(hits[0].0 > hits[1].0);
    }

    #[test]
    fn test_bm25_best_chunk_text_is_the_matching_chunk() {
        // A chapter with two chunks: the first is unrelated, the second matches
        // the query. The returned passage must be the matching chunk's text.
        let chunks = vec![
            chunk("b1", "ch1", "unrelated filler text"),
            chunk("b1", "ch1", "the dragon sleeps here"),
        ];
        let index = Bm25Index::build(&chunks);
        let hits = index.search("dragon", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "ch1");
        assert_eq!(hits[0].2, "the dragon sleeps here");
    }

    #[test]
    fn test_bm25_empty_corpus_or_query() {
        let index = Bm25Index::build(&[]);
        assert!(index.search("dragon", 10).is_empty());
        let index = Bm25Index::build(&[chunk("b1", "ch1", "dragon")]);
        assert!(index.search("", 10).is_empty());
    }

    #[test]
    fn test_rrf_fuses_both_lanes() {
        // ch1 in both lanes, ch2 only in lane A, ch3 only in lane B.
        let lane_a = vec!["ch1".to_string(), "ch2".to_string()];
        let lane_b = vec!["ch1".to_string(), "ch3".to_string()];
        let fused = rrf_fuse(&[lane_a, lane_b], 10);
        assert_eq!(fused[0], "ch1");
        assert!(fused.contains(&"ch2".to_string()));
        assert!(fused.contains(&"ch3".to_string()));
    }

    #[test]
    fn test_rrf_respects_k() {
        let lane_a = vec!["ch1".to_string(), "ch2".to_string(), "ch3".to_string()];
        let fused = rrf_fuse(&[lane_a], 2);
        assert_eq!(fused.len(), 2);
    }
}
