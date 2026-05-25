/* AVX2 distance_block8 — L2 sobre blocos int16 (7 pares x 8 lanes). */
#include <immintrin.h>
#include <stdint.h>

__attribute__((target("avx2"))) void distance_block8_avx2(const int16_t *vectors,
                                                           size_t block_off_i16,
                                                           const void *q_broadcast,
                                                           int64_t out[8])
{
    const __m256i *q = (const __m256i *)q_broadcast;
    __m256i acc_lo = _mm256_setzero_si256();
    __m256i acc_hi = _mm256_setzero_si256();
    const int16_t *base = vectors + block_off_i16;

    for (int d = 0; d < 14; d++) {
        if (d + 1 < 14)
            _mm_prefetch((const char *)(base + (d + 1) * 8), _MM_HINT_T0);
        __m128i packed = _mm_loadu_si128((const __m128i *)(base + d * 8));
        __m256i values = _mm256_cvtepi16_epi32(packed);
        __m256i diff = _mm256_sub_epi32(values, q[d]);
        __m256i sq = _mm256_mullo_epi32(diff, diff);
        __m128i sq_lo = _mm256_castsi256_si128(sq);
        __m128i sq_hi = _mm256_extracti128_si256(sq, 1);
        acc_lo = _mm256_add_epi64(acc_lo, _mm256_cvtepi32_epi64(sq_lo));
        acc_hi = _mm256_add_epi64(acc_hi, _mm256_cvtepi32_epi64(sq_hi));
    }
    _mm256_storeu_si256((__m256i *)out, acc_lo);
    _mm256_storeu_si256((__m256i *)(out + 4), acc_hi);
}
