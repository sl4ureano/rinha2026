/* AVX2 distance_block8 — L2² sobre blocos int16 (14 dims, 8 lanes).
 *
 * Layout do índice por bloco (SoA):
 *   para cada dim d: 8 valores i16 (lanes) contíguos.
 *
 * Otimização: processa 2 dims por vez e usa _mm_madd_epi16 em pares
 * (dim d, dim d+1) por lane:
 *   out_lane += (qd - vd)² + (qe - ve)²
 */
#include <immintrin.h>
#include <stdint.h>

#include "index.h"

__attribute__((target("avx2"))) void distance_block8_avx2(const int16_t *vectors,
                                                          size_t block_off_i16,
                                                          const query_vec_t *query,
                                                          int64_t out[8])
{
    __m256i acc32 = _mm256_setzero_si256();
    const int16_t *base = vectors + block_off_i16;

    // 14 dims => 7 pares (d, d+1)
    for (int d = 0; d < IDX_VECTOR_DIM; d += 2) {
        if (d + 2 < IDX_VECTOR_DIM) {
            _mm_prefetch((const char *)(base + (d + 2) * IDX_LANES), _MM_HINT_T0);
        }

        // Carrega 8 lanes para as duas dims (128-bit cada)
        __m128i vd = _mm_loadu_si128((const __m128i *)(base + (d + 0) * IDX_LANES));
        __m128i ve = _mm_loadu_si128((const __m128i *)(base + (d + 1) * IDX_LANES));

        // Intercala para formar pares por lane: [d0,e0,d1,e1,...]
        __m128i v_lo = _mm_unpacklo_epi16(vd, ve); // lanes 0..3
        __m128i v_hi = _mm_unpackhi_epi16(vd, ve); // lanes 4..7

        // Broadcast dos dois componentes do query e intercala no mesmo padrão
        __m128i qd = _mm_set1_epi16((*query)[d + 0]);
        __m128i qe = _mm_set1_epi16((*query)[d + 1]);
        __m128i q_lo = _mm_unpacklo_epi16(qd, qe);
        __m128i q_hi = _mm_unpackhi_epi16(qd, qe);

        __m128i diff_lo = _mm_sub_epi16(v_lo, q_lo);
        __m128i diff_hi = _mm_sub_epi16(v_hi, q_hi);

        // Para cada lane: (diff_d)^2 + (diff_e)^2
        __m128i sumsq_lo = _mm_madd_epi16(diff_lo, diff_lo); // 4x i32
        __m128i sumsq_hi = _mm_madd_epi16(diff_hi, diff_hi); // 4x i32

        __m256i sumsq = _mm256_set_m128i(sumsq_hi, sumsq_lo); // 8x i32
        acc32 = _mm256_add_epi32(acc32, sumsq);
    }

    // Widen i32 -> i64 (mantém API existente)
    int32_t tmp32[8];
    _mm256_storeu_si256((__m256i *)tmp32, acc32);
    for (int i = 0; i < 8; i++) out[i] = (int64_t)tmp32[i];
}
