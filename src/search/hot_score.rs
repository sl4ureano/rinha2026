//! Hot-path scoring: fast path → k-NN (residual) → tier fallback se índice vazio.

use crate::ingest::{extract, fill_datetime, vectorize_payload};
use crate::index::Index;

#[cfg(feature = "knn-index")]
use super::{complete_cache, fraud_count, tier_gray_count};
#[cfg(feature = "knn-index")]
use super::fast_path::{try_fast_fraud_count, try_obvious_count};

/// Contagem 0–5 para resposta HTTP, ou `None` se o JSON for inválido.
#[inline]
pub fn score_http_count(index: &Index, body: &[u8]) -> Option<u8> {
    #[cfg(feature = "knn-index")]
    {
        let mut p = extract(body)?;

        if let Some(c) = try_obvious_count(&p) {
            return Some(c);
        }

        let mut cache = p.cache;
        fill_datetime(&p, &mut cache);
        p.cache = cache;

        if let Some(c) = try_fast_fraud_count(&p) {
            return Some(c);
        }

        // Residual: busca vetorial oficial (5-NN → count 0..5).
        if index.block_count() > 0 {
            let v = vectorize_payload(index, &p)?;
            return Some(fraud_count(index, &v));
        }

        // Sem índice mmap (`TIER_ONLY` / `Index::empty`): árvore + ratio.
        complete_cache(&mut p);
        return Some(tier_gray_count(&p));
    }

    #[cfg(not(feature = "knn-index"))]
    {
        let _ = (index, body);
        None
    }
}

/// Treino PGO: mesmo caminho que produção (índice vazio no stage instrumented).
#[inline]
pub fn score_for_profile(index: &Index, body: &[u8]) -> u64 {
    score_http_count(index, body).unwrap_or(0) as u64
}
