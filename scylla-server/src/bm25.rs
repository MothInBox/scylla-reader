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

use crate::embeddings::ChunkHit;

/// Lowercases `text` and splits it into `\w+` tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\w+").expect("static regex is valid");
    re.find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect()
}

/// One indexed document: a chapter's concatenated chunk text.
struct Bm25Doc {
    chapter_url: String,
    tokens: Vec<String>,
}

/// In-memory BM25 index over chapters.
pub struct Bm25Index {
    docs: Vec<Bm25Doc>,
    /// term → (doc_idx, term_frequency) postings.
    postings: HashMap<String, Vec<(usize, usize)>>,
    doc_lens: Vec<usize>,
    avg_dl: f64,
    n_docs: usize,
}

impl Bm25Index {
    /// Builds the index from chunk hits, grouping chunks per chapter (a
    /// chapter's document is its chunks' text joined with spaces).
    pub fn build(chunks: &[ChunkHit]) -> Self {
        let mut by_chapter: HashMap<String, Vec<&ChunkHit>> = HashMap::new();
        for c in chunks {
            by_chapter.entry(c.chapter_url.clone()).or_default().push(c);
        }
        let mut docs: Vec<Bm25Doc> = Vec::with_capacity(by_chapter.len());
        for (url, hits) in by_chapter {
            let text = hits
                .iter()
                .map(|h| h.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            docs.push(Bm25Doc {
                chapter_url: url,
                tokens: tokenize(&text),
            });
        }
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

    /// Scores every chapter against `query` and returns the top `top_k` as
    /// `(score, chapter_url)` in descending score order. Chapters with no
    /// matching terms are omitted.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(f64, String)> {
        const K1: f64 = 1.5;
        const B: f64 = 0.75;
        let q_terms = tokenize(query);
        if q_terms.is_empty() || self.n_docs == 0 {
            return Vec::new();
        }
        let mut scores = vec![0.0f64; self.n_docs];
        for term in q_terms {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len();
            let idf = ((self.n_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln();
            for &(doc_idx, tf) in postings {
                let dl = self.doc_lens[doc_idx] as f64;
                let denom = tf as f64 + K1 * (1.0 - B + B * dl / self.avg_dl);
                scores[doc_idx] += idf * (tf as f64 * (K1 + 1.0)) / denom;
            }
        }
        let mut ranked: Vec<(f64, String)> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, s)| *s > 0.0)
            .map(|(i, s)| (s, self.docs[i].chapter_url.clone()))
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
