# Remote Profiling

Esta aplicação pode operar como uma caixa-preta observável quando o ambiente de benchmark não permite acesso ao host, ao container, a logs locais ou a arquivos gerados. Toda telemetria sai por `PERF_WEBHOOK_URL` e o servidor continua respondendo normalmente se o webhook cair.

## Arquivos alterados

- `src/perf.rs`: contadores globais, histogramas, coleta `/proc`, snapshots, eventos e envio por webhook em fila limitada.
- `src/main.rs`: inicializa a telemetria a partir das variáveis de ambiente.
- `src/lib.rs`: expõe o módulo `perf`.
- `src/platform/fd_gateway.rs`: instrumenta o caminho principal epoll/fd-passing/direct TCP.
- `src/http/handler.rs`: instrumenta o modo TCP `monoio`.
- `src/http/sync_handler.rs`: instrumenta o modo bloqueante por conexão.
- `src/ingest/json.rs`: separa tempo de parse JSON e preenchimento do cache derivado.
- `Cargo.toml` / `Cargo.lock`: adiciona dependências para HTTPS webhook, hostname e stack samples.

## Como ativar

Por padrão não há coleta nem webhook:

```bash
ENABLE_REMOTE_PROFILING=false
```

Para benchmark remoto:

```bash
ENABLE_REMOTE_PROFILING=true
PERF_WEBHOOK_URL=https://webhook.site/xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
```

Variáveis opcionais:

- `PERF_SNAPSHOT_INTERVAL_SECS`: intervalo de snapshot, padrão `10`.
- `PERF_WEBHOOK_TIMEOUT_MS`: timeout do POST, padrão `800`.
- `PERF_WEBHOOK_QUEUE_CAP`: tamanho da fila limitada, padrão `64`.
- `PERF_STACK_SAMPLE_EVERY`: captura uma stack a cada N requests, padrão `10000`; use `0` para desativar.
- `PERF_P99_LIMIT_US`: limite para evento de p99 alto, padrão `50000`.
- `PERF_CPU_LIMIT_PERCENT`: limite para evento de CPU, padrão `90`.
- `PERF_CACHE_HIT_RATE_MIN`: limite mínimo de hit rate, padrão `70`.
- `PERF_MEMORY_GROWTH_LIMIT_KB`: crescimento de RSS por janela para alerta, padrão `65536`.

## Webhook.site

1. Abra `https://webhook.site`.
2. Copie a URL única exibida.
3. Configure `PERF_WEBHOOK_URL` com essa URL.
4. Rode o container com `ENABLE_REMOTE_PROFILING=true`.
5. Observe os POSTs recebidos na página do webhook.

Falhas de DNS, TLS, timeout ou HTTP são ignoradas. A fila é limitada; se o webhook não acompanhar, `webhook_dropped` aumenta e a aplicação não acumula memória.

## Snapshot enviado

Exemplo resumido:

```json
{
  "timestamp": 1780182000000,
  "hostname": "container-id",
  "pid": 1,
  "requests": 120000,
  "requests_success": 119990,
  "requests_error": 10,
  "rps": 11850.4,
  "p50_us": 300,
  "p90_us": 750,
  "p95_us": 1000,
  "p99_us": 3000,
  "p999_us": 10000,
  "cache_hits": 90000,
  "cache_misses": 30000,
  "cache_hit_rate": 75.0,
  "parse_avg_us": 18,
  "validation_avg_us": 2,
  "decision_tree_avg_us": 8,
  "cache_lookup_avg_us": 1,
  "cache_insert_avg_us": 3,
  "serialize_avg_us": 0,
  "write_response_avg_us": 5,
  "rss_mb": 42.5,
  "virtual_mb": 180.0,
  "memory_growth_mb": 0.2,
  "memory_per_request_bytes": 371,
  "user_cpu_ticks": 12345,
  "system_cpu_ticks": 678,
  "cpu_percent": 86.4,
  "active_connections": 128,
  "bytes_received": 85000000,
  "bytes_sent": 5100000,
  "webhook_queue_len": 0,
  "webhook_dropped": 0,
  "hotspots": [
    { "symbol": "parse_json", "samples": 120000, "total_us": 2160000, "percent": 41.2 }
  ]
}
```

## Eventos adicionais

Quando um gargalo é detectado, um POST extra é enviado:

```json
{
  "event": "bottleneck_detected",
  "timestamp": 1780182000000,
  "reason": "p99_above_limit",
  "details": {
    "p99_us": 85000,
    "limit_us": 50000
  }
}
```

Também podem aparecer eventos `stack_sample`:

```json
{
  "event": "stack_sample",
  "timestamp": 1780182000000,
  "request": 10000,
  "symbols": [
    "fraud_detector::platform::fd_gateway::fraud_response",
    "fraud_detector::ingest::json::extract"
  ]
}
```

## Como interpretar

CPU bound:

- `cpu_percent` perto ou acima de `90`.
- `rps` estabiliza enquanto `active_connections` sobe.
- `hotspots` concentrado em `decision_tree`, `parse_json` ou `cache_lookup`.
- `user_cpu_ticks` cresce muito mais que `system_cpu_ticks`.

Memory bound:

- `rss_mb` cresce continuamente.
- `memory_growth_mb` positivo em várias janelas.
- `memory_per_request_bytes` não estabiliza com o volume de requests.
- `webhook_queue_len` ou `webhook_dropped` crescendo indica pressão causada pelo destino remoto, não pelo core da aplicação.

Cache bound:

- `cache_hit_rate` baixo.
- `cache_misses` cresce próximo de `requests`.
- `decision_tree_avg_us` sobe junto com `cache_lookup_avg_us` baixo, indicando que o fast path não está eliminando trabalho suficiente.

Parse JSON bound:

- `parse_avg_us` ou `parse_max_us` domina os hotspots.
- p99 alto acompanha aumento de `parse_avg_us`.
- `bytes_received` alto com payloads maiores tende a pressionar essa métrica.

Decision Tree bound:

- `decision_tree_avg_us` alto e `cache_hit_rate` baixo.
- `hotspots` mostra maior percentual em `decision_tree`.
- `p99_us` sobe quando `cache_misses` sobem.

Serialização bound:

- `serialize_avg_us` alto seria anormal neste projeto, porque as respostas são estáticas.
- Se `serialize_avg_us` subir sem aumento de `write_response_avg_us`, o gargalo está na escolha/formatação da resposta.

I/O HTTP bound:

- `write_response_avg_us` alto.
- `system_cpu_ticks` cresce proporcionalmente ao total.
- `active_connections` alto com `cpu_percent` moderado sugere espera de socket/backpressure.
- `bytes_sent` ou `bytes_received` crescem sem aumento equivalente de `rps`.

Latência p99:

- Compare `p99_us`/`p999_us` com os máximos por etapa.
- Se só `write_response_avg_us` cresce, o problema tende a ser rede/socket.
- Se `parse_max_us` cresce, o problema está em payload ou parsing.
- Se `decision_tree_avg_us` cresce com miss rate, o problema está no caminho de decisão.
- Se `rss_mb` cresce junto com p99, investigue pressão de memória/alocação.
