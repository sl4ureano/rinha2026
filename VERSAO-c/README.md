# VERSAO-c

Implementação **C11** do mesmo desenho da raiz: `lb` (epoll) → FD-pass → 2× `server`, scorer `tier_score` (gasto seguro → gasto arriscado → árvore → ratio).

## Arquitetura

```mermaid
flowchart LR
    C[Cliente] --> LB[lb.c — epoll]
    LB -->|SCM_RIGHTS| G1[fd_gateway]
    LB -->|SCM_RIGHTS| G2[fd_gateway]
    G1 --> S1[http_handler — pthread/conn]
    G2 --> S2[http_handler — pthread/conn]
    S1 --> TS[tier_score.c]
    S2 --> TS
    TS --> DT[decision_tree.c]
```

## Módulos

```mermaid
flowchart TB
    subgraph http [HTTP]
        H[http_handler.c]
        R[http_response.c — bodies estáticos]
    end
    subgraph score [Classificação]
        Z[tier_score.c]
        D[decision_tree.c]
        J[ingest_json.c]
    end
    subgraph platform [Runtime]
        LB[platform_lb.c]
        FD[platform_fd_gateway.c]
        SCM[platform_scm.c]
    end
    H --> J --> Z --> D
    H --> R
    FD --> H
    LB --> FD
```

## Scorer (`tier_score`)

Mesma lógica da versão Rust.

**Atalhos** (antes da árvore):

- **Gasto seguro:** valor ≤ 500, ≤ 50% da média do cliente, ≤ 3 parcelas, ≤ 5 tx/24h, loja em `known_merchants`, ≤ 50 km de casa, MCC 5411/5812/5912/5311 → aprova.
- **Gasto arriscado:** valor ≥ 5000, ≥ 5 parcelas, ≥ 6 tx/24h, loja desconhecida, ≥ 150 km, MCC 7995/7801/7802 → nega.

Detalhes: [README.md — Gasto seguro e arriscado](../README.md#gasto-seguro-e-gasto-arriscado).

```mermaid
stateDiagram-v2
    [*] --> Parse: extract_json
    Parse --> Seguro: gasto seguro
    Parse --> Arriscado: gasto arriscado
    Parse --> Tree: build_tree_features
    Legit --> Approve: count 0
    Fraud --> Deny: count 5
    Tree --> Approve: tree_predict false
    Tree --> Deny: tree_predict true
    Tree --> Ratio: features inválidas
    Ratio --> Approve: norm baixo
    Ratio --> Deny: norm alto
    Approve --> [*]
    Deny --> [*]
```

## Rodar

Na raiz do repositório:

```bash
docker compose -f VERSAO-c/docker-compose.yml -p versao-c up --build -d
```

```mermaid
flowchart LR
    subgraph stack [projeto versao-c]
        L[versao-c-lb-1 :9999]
        A1[versao-c-api1-1]
        A2[versao-c-api2-1]
    end
    K6[k6] -->|network container:versao-c-lb-1| L
    L --> A1
    L --> A2
```

Benchmark:

```bash
docker run --rm --user root --network container:versao-c-lb-1 \
  -e BASE_URL=http://127.0.0.1:9999 \
  -v "$(pwd)/test:/test" -w /test \
  grafana/k6:latest run test.js
```

## Regenerar a árvore

```bash
python scripts/gen_decision_tree.py
```

Gera `VERSAO-c/src/decision_tree.c` e `src/search/decision_tree.rs` a partir de `scripts/decision_tree.nodes`.

## Build local (opcional)

```bash
cd VERSAO-c && make build
```

Binários: `server`, `lb`, `healthcheck`.
