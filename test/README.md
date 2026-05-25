# Testes (k6)

Scripts de carga e pontuação no formato da [Rinha 2026](https://github.com/zanfranceschi/rinha-de-backend-2026).

## Pré-requisito

```bash
# na raiz do repo
docker compose up --build -d
```

Aguarde `api1`, `api2` e `lb` **healthy** e ~30 s de warm-up.

Nome do LB:

```bash
docker ps --format "{{.Names}}" | findstr lb
# ex.: rinha2026-lb-1
```

## Onde rodar o k6

O **p99 só é confiável** se o k6 medir pelo mesmo caminho da prova: dentro da rede do container do LB.

```mermaid
flowchart TB
    subgraph valido [Válido — p99 ~0,6–0,9 ms]
        K1[k6 container] -->|127.0.0.1:9999| LB1[LB]
        LB1 --> API[api1 / api2]
    end

    subgraph invalido [Inválido — latência inflada]
        K2[k6 no host Windows] -->|localhost:9999| NAT[port mapping]
        NAT --> LB2[LB]
        K3[k6] -->|host.docker.internal| NAT2[NAT extra]
        NAT2 --> LB3[LB]
    end
```

| Modo | p99 típico | Usar? |
|------|------------|-------|
| `--network container:<lb>` + `127.0.0.1:9999` | ~0,6–0,9 ms | **Sim** |
| Rede compose `http://lb:9999` | ~0,8 ms | Aceitável |
| `host.docker.internal:9999` | ~4 ms+ | Não |
| k6 no host + `localhost:9999` | ~70 ms+ | Só smoke |

### Benchmark completo

PowerShell (`test/`):

```powershell
docker run --rm --user root `
  --network container:rinha2026-lb-1 `
  -e BASE_URL=http://127.0.0.1:9999 `
  -v "${PWD}:/test" -w /test `
  grafana/k6:latest run test.js
```

Linux/macOS:

```bash
docker run --rm --user root \
  --network container:rinha2026-lb-1 \
  -e BASE_URL=http://127.0.0.1:9999 \
  -v "$(pwd)/test:/test" -w /test \
  grafana/k6:latest run test.js
```

No Windows use `--user root` para gravar `results.json`.

## O que o `test.js` faz

```mermaid
flowchart LR
    D[(test-data.json\n54 100 entries)] --> K6[k6 ramp 120s\n~900 req/s]
    K6 --> API[POST /fraud-score]
    API --> CMP{approved ==\nexpected_approved?}
    CMP -->|sim| TP[TN / TP]
    CMP -->|não| ERR[FP / FN]
    K6 --> OUT[results.json\np99 + final_score]
```

- Uma iteração por entrada do dataset.
- Métricas: `tp_count`, `tn_count`, `fp_count`, `fn_count`, `error_count`.
- Saída: `test/results.json` (gitignored).

## Pontuação (`final_score`)

```mermaid
flowchart TB
    P99[p99 HTTP] --> PS[p99_score\nmax 3000 se p99 ≤ 1ms]
    DET[FP FN erros] --> DS[detection_score\nmax 3000 se taxa ok]
    PS --> FS[final_score = p99_score + detection_score\nmáx 6000]
    DS --> FS
```

| Campo | Significado |
|-------|-------------|
| `final_score: 6000` | p99 ≤ 1 ms + zero falhas ponderadas |
| `p99` | Latência end-to-end no caminho medido |
| `scoring.breakdown` | FP, FN, TP, TN, `http_errors` |

Regras completas: [AVALIACAO.md](https://github.com/zanfranceschi/rinha-de-backend-2026/blob/main/docs/br/AVALIACAO.md).

## Outros scripts

```mermaid
flowchart LR
    smoke[smoke.js\n5 requests] --> OK[contrato HTTP]
    acc[accuracy.js\n20 VUs] --> FP[FP/FN no console]
    test[test.js\n900 rps] --> SC[score completo]
```

| Arquivo | Uso |
|---------|-----|
| `test.js` | Benchmark oficial + `results.json` |
| `accuracy.js` | Só acurácia no console |
| `smoke.js` | Sanidade após `docker compose up` |
| `body.json` | Payload para `curl` manual |
| `docker-compose.yml` | k6 `host` network (Linux CI) |

```powershell
# smoke
docker run --rm --network container:rinha2026-lb-1 `
  -e BASE_URL=http://127.0.0.1:9999 -v "${PWD}:/test" -w /test `
  grafana/k6:latest run smoke.js

# acurácia
docker run --rm --network container:rinha2026-lb-1 `
  -e BASE_URL=http://127.0.0.1:9999 -v "${PWD}:/test" -w /test `
  grafana/k6:latest run accuracy.js
```

## Checklist

```mermaid
flowchart TD
    A[Uma stack só] --> B[APIs healthy]
    B --> C[k6 --network container:lb]
    C --> D[BASE_URL 127.0.0.1:9999]
    D --> E[final_score 6000\nhttp_errors 0]
```

## Problemas comuns

| Sintoma | Causa |
|---------|--------|
| p99 ~70 ms | k6 no host + `localhost` |
| p99 ~4 ms | `host.docker.internal` |
| permission denied | Falta `--user root` (Windows) |
| FP/FN > 0 | Imagem antiga — `docker compose build --no-cache` |
| p99 alto | Duas stacks competindo por CPU |

Backend: [README na raiz](../README.md).
