use crate::ast::codebase::{Codebase, FuncId, IndexedFunc, IndexedType};
use crate::ast::item::TypeRef;
use std::collections::{HashMap, HashSet};

pub use model::{LexicalParams, SparseTfIdf};

mod model {
    use std::cmp::Ordering;
    use std::collections::HashMap;

    /// Injectable parameters controlling identifier tokenization and stopword filtering.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LexicalParams {
        pub min_token_len: usize,
        pub stopwords: Vec<String>,
    }

    impl Default for LexicalParams {
        fn default() -> Self {
            Self {
                min_token_len: 3,
                stopwords: [
                    "self",
                    "this",
                    "get",
                    "set",
                    "new",
                    "from",
                    "into",
                    "with",
                    "val",
                    "value",
                    "item",
                    "data",
                    "list",
                    "vec",
                    "map",
                    "str",
                    "string",
                    "int",
                    "bool",
                    "void",
                    "async",
                    "await",
                    "return",
                    "result",
                    "option",
                    "error",
                    "err",
                    "true",
                    "false",
                    "null",
                    "undefined",
                    "len",
                    "push",
                    "pop",
                    "iter",
                    "clone",
                    "default",
                    "index",
                    "count",
                    "size",
                    "key",
                    "name",
                    "type",
                    "kind",
                    "span",
                    "node",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            }
        }
    }

    impl LexicalParams {
        /// Split an identifier into lowercase domain tokens using these parameters.
        #[must_use]
        pub fn tokenize(&self, ident: &str) -> Vec<String> {
            let mut out = Vec::new();
            super::tokenize_into(
                ident,
                self.min_token_len,
                |tok| {
                    for sw in &self.stopwords {
                        if sw == tok {
                            return true;
                        }
                    }
                    false
                },
                &mut out,
            );
            out
        }
    }

    /// Sparse L2-indexed TF-IDF vector sorted deterministically by token.
    #[derive(Debug, Clone, Default)]
    pub struct SparseTfIdf {
        weights: Vec<(String, f64)>,
        norm: f64,
    }

    impl SparseTfIdf {
        pub(super) fn from_counts(
            counts: HashMap<String, usize>,
            doc_count: usize,
            doc_freq: &HashMap<String, usize>,
        ) -> Self {
            let n_plus_one = super::usize_to_f64(doc_count + 1);
            let mut weights: Vec<(String, f64)> = Vec::with_capacity(counts.len());
            let mut norm_sq = 0.0;
            for (tok, tf) in counts {
                let df = doc_freq.get(&tok).copied().unwrap_or(0);
                let idf = (n_plus_one / super::usize_to_f64(df + 1)).ln() + 1.0;
                let weight = (1.0 + super::usize_to_f64(tf).ln()) * idf;
                norm_sq += weight * weight;
                weights.push((tok, weight));
            }
            weights.sort_by(|a, b| a.0.cmp(&b.0));
            Self {
                weights,
                norm: norm_sq.sqrt(),
            }
        }

        /// Cosine similarity between two sparse TF-IDF vectors (`0.0` if either vector is empty).
        #[must_use]
        pub fn cosine(&self, rhs: &Self) -> f64 {
            if self.weights.is_empty() || rhs.weights.is_empty() {
                return 0.0;
            }
            let mut dot = 0.0;
            let (mut i, mut j) = (0, 0);
            while i < self.weights.len() && j < rhs.weights.len() {
                match self.weights[i].0.cmp(&rhs.weights[j].0) {
                    Ordering::Equal => {
                        dot += self.weights[i].1 * rhs.weights[j].1;
                        i += 1;
                        j += 1;
                    }
                    Ordering::Less => i += 1,
                    Ordering::Greater => j += 1,
                }
            }
            (dot / (self.norm * rhs.norm)).clamp(0.0, 1.0)
        }
    }
}

/// Split an identifier into lowercase domain tokens filtered by length and stopwords.
#[must_use]
pub fn tokenize_identifier(ident: &str, params: &LexicalParams) -> Vec<String> {
    params.tokenize(ident)
}

/// Cosine similarity between two sparse TF-IDF vectors (`0.0` if either vector is empty).
#[must_use]
pub fn cosine_similarity(lhs: &SparseTfIdf, rhs: &SparseTfIdf) -> f64 {
    lhs.cosine(rhs)
}

fn tokenize_into(
    ident: &str,
    min_token_len: usize,
    is_stopword: impl Fn(&str) -> bool,
    out: &mut Vec<String>,
) {
    let chars: Vec<char> = ident.chars().collect();
    let mut start: Option<usize> = None;
    let mut lower_buf = String::new();

    for (i, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            if let Some(s) = start.take()
                && normalize_token(&chars[s..i], min_token_len, &is_stopword, &mut lower_buf)
            {
                out.push(lower_buf.clone());
            }
            continue;
        }
        if let Some(s) = start {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            let starts_new_word = ch.is_uppercase()
                && ((prev.is_lowercase() || prev.is_ascii_digit())
                    || (prev.is_uppercase() && next_lower));
            if starts_new_word {
                if normalize_token(&chars[s..i], min_token_len, &is_stopword, &mut lower_buf) {
                    out.push(lower_buf.clone());
                }
                start = Some(i);
            }
        } else {
            start = Some(i);
        }
    }
    if let Some(s) = start
        && normalize_token(&chars[s..], min_token_len, &is_stopword, &mut lower_buf)
    {
        out.push(lower_buf);
    }
}

fn normalize_token(
    slice: &[char],
    min_token_len: usize,
    is_stopword: &impl Fn(&str) -> bool,
    lower_buf: &mut String,
) -> bool {
    if slice.len() < min_token_len {
        return false;
    }
    lower_buf.clear();
    for &ch in slice {
        for lc in ch.to_lowercase() {
            lower_buf.push(lc);
        }
    }
    lower_buf.len() >= min_token_len && !is_stopword(lower_buf)
}

fn usize_to_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

/// Codebase-wide TF-IDF lexical cohesion index over non-test production functions.
pub struct LexicalCorpus {
    min_token_len: usize,
    stopwords: HashSet<String>,
    doc_count: usize,
    doc_freq: HashMap<String, usize>,
}

impl LexicalCorpus {
    /// Build a codebase-wide smoothed IDF corpus across all non-test production functions.
    #[must_use]
    pub fn build(codebase: &Codebase, params: LexicalParams) -> Self {
        let mut corpus = Self {
            min_token_len: params.min_token_len,
            stopwords: params.stopwords.into_iter().collect(),
            doc_count: 0,
            doc_freq: HashMap::new(),
        };
        let mut token_buf: Vec<String> = Vec::new();
        let mut unique_buf: HashSet<String> = HashSet::new();

        for func in codebase.functions() {
            if func.decl.role.is_test_code() || codebase.file(func.file_id).is_test_file() {
                continue;
            }
            corpus.doc_count += 1;
            token_buf.clear();
            unique_buf.clear();
            corpus.push_method_tokens(func, &mut token_buf);
            for tok in token_buf.drain(..) {
                unique_buf.insert(tok);
            }
            for tok in unique_buf.drain() {
                *corpus.doc_freq.entry(tok).or_insert(0) += 1;
            }
        }
        corpus
    }

    /// Build a sublinear TF-IDF vector from an explicit sequence of pre-tokenized terms.
    #[must_use]
    pub fn vector_from_tokens(&self, tokens: &[&str]) -> SparseTfIdf {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for &tok in tokens {
            if !tok.is_empty() {
                *counts.entry(tok.to_string()).or_insert(0) += 1;
            }
        }
        SparseTfIdf::from_counts(counts, self.doc_count, &self.doc_freq)
    }

    /// Build the sublinear TF-IDF vector for a single method document `T(m)`.
    #[must_use]
    pub fn method_vector(&self, func: &IndexedFunc) -> SparseTfIdf {
        let mut tokens = Vec::new();
        self.push_method_tokens(func, &mut tokens);
        self.vector_from_owned(tokens)
    }

    /// Build the sublinear TF-IDF vector for a cluster document `T(C_a)` pooling method documents
    /// and declared field names in the cluster's transitive field union.
    #[must_use]
    pub(crate) fn cluster_vector(
        &self,
        methods: &[&IndexedFunc],
        field_names: &[&str],
    ) -> SparseTfIdf {
        let mut tokens = Vec::new();
        for &func in methods {
            self.push_method_tokens(func, &mut tokens);
        }
        for &field in field_names {
            self.tokenize_text(field, &mut tokens);
        }
        self.vector_from_owned(tokens)
    }

    /// Build the sublinear TF-IDF vector for a type document `T(tau)` pooling its type name,
    /// declared field names, and non-test inherent methods (optionally excluding `exclude_func`).
    #[must_use]
    pub fn type_vector_excluding(
        &self,
        codebase: &Codebase,
        ty: &IndexedType,
        exclude_func: Option<FuncId>,
    ) -> SparseTfIdf {
        let mut tokens = Vec::new();
        self.tokenize_text(&ty.decl.name, &mut tokens);
        for field in &ty.decl.fields {
            self.tokenize_text(&field.name, &mut tokens);
        }
        for method in codebase.inherent_methods_for(ty.id) {
            if Some(method.id) == exclude_func
                || method.decl.role.is_test_code()
                || codebase.file(method.file_id).is_test_file()
            {
                continue;
            }
            self.push_method_tokens(method, &mut tokens);
        }
        self.vector_from_owned(tokens)
    }

    fn tokenize_text(&self, text: &str, out: &mut Vec<String>) {
        tokenize_into(
            text,
            self.min_token_len,
            |tok| self.stopwords.contains(tok),
            out,
        );
    }

    fn push_method_tokens(&self, func: &IndexedFunc, out: &mut Vec<String>) {
        self.tokenize_text(&func.decl.name, out);
        for param in &func.decl.params {
            self.tokenize_text(&param.name, out);
            if let Some(TypeRef::Named(ty_name)) = &param.type_name {
                self.tokenize_text(ty_name, out);
            }
        }
        for (access, resolved) in func
            .decl
            .field_accesses
            .iter()
            .zip(&func.resolved_field_accesses)
        {
            if resolved.is_some() {
                self.tokenize_text(&access.field_name, out);
            }
        }
        for call in &func.decl.calls {
            self.tokenize_text(&call.callee, out);
        }
        for ident in &func.decl.referenced_idents {
            self.tokenize_text(ident, out);
        }
    }

    fn vector_from_owned(&self, tokens: Vec<String>) -> SparseTfIdf {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for tok in tokens {
            *counts.entry(tok).or_insert(0) += 1;
        }
        SparseTfIdf::from_counts(counts, self.doc_count, &self.doc_freq)
    }
}
