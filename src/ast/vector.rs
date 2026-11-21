use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// Canonical language-agnostic vocabulary for normalized function body tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NormToken {
    Assign,
    Call,
    Field,
    If,
    Branch,
    BranchArm,
    Loop,
    Return,
    Closure,
    Construct,
    Bind,
    Ref,
    RefMut,
    Try,
    Macro,
    Ident,
    Lit,
    Other(String),
}

impl NormToken {
    #[must_use]
    pub fn as_token(&self) -> &str {
        use NormToken::{
            Assign, Bind, Branch, BranchArm, Call, Closure, Construct, Field, Ident, If, Lit, Loop,
            Macro, Other, Ref, RefMut, Return, Try,
        };
        match self {
            Assign => "@assign",
            Call => "@call",
            Field => "@field",
            If => "@if",
            Branch => "@branch",
            BranchArm => "@arm",
            Loop => "@loop",
            Return => "@return",
            Closure => "@closure",
            Construct => "@construct",
            Bind => "@bind",
            Ref => "@ref",
            RefMut => "@ref_mut",
            Try => "@try",
            Macro => "@macro",
            Ident => "IDENT",
            Lit => "LIT",
            Other(s) => s.as_str(),
        }
    }
}

/// Language-agnostic normalized sequence of structural AST tokens for a function body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedAst {
    tokens: Vec<String>,
}

impl NormalizedAst {
    #[must_use]
    pub fn new(tokens: Vec<String>) -> Self {
        Self { tokens }
    }

    #[must_use]
    pub fn from_norm_tokens(tokens: &[NormToken]) -> Self {
        Self::new(tokens.iter().map(|t| t.as_token().to_string()).collect())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    #[must_use]
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    /// Generate sliding n-gram shingles over the normalized token sequence.
    #[must_use]
    pub fn ngrams(&self, n: usize) -> Vec<String> {
        let n = n.max(1);
        if self.tokens.len() < n {
            if self.tokens.is_empty() {
                return Vec::new();
            }
            return vec![self.tokens.join("|")];
        }

        self.tokens
            .windows(n)
            .map(|window| window.join("|"))
            .collect()
    }
}

/// Sorted sparse vector with precomputed Euclidean norm for fast cosine similarity.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseVector {
    entries: Vec<(usize, f64)>,
    norm: f64,
}

impl SparseVector {
    #[must_use]
    pub fn from_entries(mut entries: Vec<(usize, f64)>) -> Self {
        entries.retain(|(_, w)| w.abs() > f64::EPSILON);
        entries.sort_unstable_by_key(|(idx, _)| *idx);
        let norm = entries.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
        Self { entries, norm }
    }

    #[must_use]
    pub fn norm(&self) -> f64 {
        self.norm
    }

    #[must_use]
    pub fn cosine_similarity(&self, other: &Self) -> f64 {
        if self.norm <= f64::EPSILON || other.norm <= f64::EPSILON {
            return 0.0;
        }

        let mut dot = 0.0;
        let mut i = 0;
        let mut j = 0;

        while i < self.entries.len() && j < other.entries.len() {
            let (idx_a, val_a) = self.entries[i];
            let (idx_b, val_b) = other.entries[j];
            match idx_a.cmp(&idx_b) {
                std::cmp::Ordering::Equal => {
                    dot += val_a * val_b;
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }

        (dot / (self.norm * other.norm)).clamp(0.0, 1.0)
    }
}

/// A pair of documents whose cosine similarity met or exceeded a threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimilarityMatch {
    left_id: usize,
    right_id: usize,
    score: f64,
}

impl SimilarityMatch {
    #[must_use]
    pub fn left_id(&self) -> usize {
        self.left_id
    }

    #[must_use]
    pub fn right_id(&self) -> usize {
        self.right_id
    }

    #[must_use]
    pub fn score(&self) -> f64 {
        self.score
    }
}

struct CorpusDocument {
    id: usize,
    term_counts: HashMap<usize, usize>,
}

/// TF-IDF corpus over normalized AST n-grams supporting exact pairwise cosine similarity.
pub struct TfIdfCorpus {
    ngram_size: usize,
    vocab: HashMap<String, usize>,
    doc_freq: Vec<usize>,
    docs: Vec<CorpusDocument>,
}

impl TfIdfCorpus {
    #[must_use]
    pub fn new(ngram_size: usize) -> Self {
        Self {
            ngram_size: ngram_size.max(1),
            vocab: HashMap::new(),
            doc_freq: Vec::new(),
            docs: Vec::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn add_document(&mut self, id: usize, ast: &NormalizedAst) {
        let grams = ast.ngrams(self.ngram_size);
        let mut term_counts: HashMap<usize, usize> = HashMap::new();
        let mut seen_terms: HashSet<usize> = HashSet::new();

        for gram in grams {
            let term_id = if let Some(&existing) = self.vocab.get(&gram) {
                existing
            } else {
                let next_id = self.vocab.len();
                self.vocab.insert(gram, next_id);
                self.doc_freq.push(0);
                next_id
            };

            *term_counts.entry(term_id).or_insert(0) += 1;
            if seen_terms.insert(term_id) {
                self.doc_freq[term_id] += 1;
            }
        }

        self.docs.push(CorpusDocument { id, term_counts });
    }

    /// Compute TF-IDF vectors and return all pairs with `cosine_similarity >= threshold`,
    /// sorted deterministically by descending score, then `(left_id, right_id)`.
    #[must_use]
    pub fn similar_pairs(&self, threshold: f64) -> Vec<SimilarityMatch> {
        let doc_count = self.docs.len();
        if doc_count < 2 {
            return Vec::new();
        }

        let n_docs = count_to_f64(doc_count);
        let idf: Vec<f64> = self
            .doc_freq
            .iter()
            .map(|&df| ((n_docs + 1.0) / (count_to_f64(df) + 1.0)).ln() + 1.0)
            .collect();

        let vectors: Vec<(usize, SparseVector)> = self
            .docs
            .iter()
            .map(|doc| {
                let entries: Vec<(usize, f64)> = doc
                    .term_counts
                    .iter()
                    .map(|(&term_id, &tf)| {
                        let weight = (1.0 + count_to_f64(tf).ln()) * idf[term_id];
                        (term_id, weight)
                    })
                    .collect();
                (doc.id, SparseVector::from_entries(entries))
            })
            .collect();

        let mut matches: Vec<SimilarityMatch> = (0..vectors.len())
            .into_par_iter()
            .flat_map_iter(|i| {
                let (left_id, ref vec_a) = vectors[i];
                let vectors_ref = &vectors;
                ((i + 1)..vectors_ref.len()).filter_map(move |j| {
                    let (right_id, ref vec_b) = vectors_ref[j];
                    let score = vec_a.cosine_similarity(vec_b);
                    if score >= threshold {
                        Some(SimilarityMatch {
                            left_id,
                            right_id,
                            score,
                        })
                    } else {
                        None
                    }
                })
            })
            .collect();

        matches.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.left_id.cmp(&b.left_id))
                .then_with(|| a.right_id.cmp(&b.right_id))
        });

        matches
    }
}

fn count_to_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(tokens: &[&str]) -> NormalizedAst {
        NormalizedAst::new(tokens.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn sparse_vector_cosine_handles_identical_orthogonal_and_zero_vectors() {
        let a = SparseVector::from_entries(vec![(0, 1.0), (2, 2.0)]);
        let b = SparseVector::from_entries(vec![(2, 4.0), (0, 2.0)]);
        let ortho = SparseVector::from_entries(vec![(1, 3.0), (3, 1.0)]);
        let zero = SparseVector::from_entries(vec![]);

        assert!((a.cosine_similarity(&b) - 1.0).abs() < 1e-9);
        assert!(a.cosine_similarity(&ortho).abs() < 1e-9);
        assert!(a.cosine_similarity(&zero).abs() < 1e-9);
    }

    #[test]
    fn corpus_finds_similar_documents_and_excludes_distinct_ones() {
        let mut corpus = TfIdfCorpus::new(2);
        corpus.add_document(
            10,
            &ast(&["let", "IDENT", "call", "if", "IDENT", "return", "LIT"]),
        );
        corpus.add_document(
            20,
            &ast(&["let", "IDENT", "call", "if", "IDENT", "return", "LIT"]),
        );
        corpus.add_document(
            30,
            &ast(&["loop", "match", "arm", "break", "struct", "field"]),
        );

        let pairs = corpus.similar_pairs(0.8);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].left_id(), 10);
        assert_eq!(pairs[0].right_id(), 20);
        assert!((pairs[0].score() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ngrams_handle_short_token_sequences_gracefully() {
        let short = ast(&["return", "LIT"]);
        assert_eq!(short.ngrams(3), vec!["return|LIT"]);

        let empty = NormalizedAst::default();
        assert_eq!(empty.ngrams(3), Vec::<String>::new());
    }

    #[test]
    fn norm_tokens_convert_to_canonical_strings() {
        let tokens = [
            NormToken::Bind,
            NormToken::Ident,
            NormToken::Assign,
            NormToken::Construct,
            NormToken::Call,
            NormToken::Field,
            NormToken::If,
            NormToken::Branch,
            NormToken::BranchArm,
            NormToken::Loop,
            NormToken::Return,
            NormToken::Closure,
            NormToken::Ref,
            NormToken::RefMut,
            NormToken::Try,
            NormToken::Macro,
            NormToken::Lit,
            NormToken::Other("+".to_string()),
        ];
        let normalized = NormalizedAst::from_norm_tokens(&tokens);
        assert_eq!(
            normalized.tokens(),
            &[
                "@bind",
                "IDENT",
                "@assign",
                "@construct",
                "@call",
                "@field",
                "@if",
                "@branch",
                "@arm",
                "@loop",
                "@return",
                "@closure",
                "@ref",
                "@ref_mut",
                "@try",
                "@macro",
                "LIT",
                "+",
            ]
        );
    }
}
