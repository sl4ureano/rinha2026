# Rinha 2026 — detecção de fraude

API em **Rust**: load balancer repassa conexões TCP para duas instâncias que classificam cada transação em **camadas** (`tier_score`: gasto seguro → gasto arriscado → árvore → ratio), sem k-NN no caminho quente.

![Screenshot do visualizador 3D](visualizador/public/img/print.png)

Visualizador 3D em tempo real: `visualizador/` (veja [`visualizador/README.md`](visualizador/README.md)).

## Arquitetura

```mermaid
flowchart TB
    C[Cliente / k6] -->|TCP :9999| LB[lb — accept bloqueante, round-robin]

    subgraph apis [APIs — fd_gateway, 1 thread + epoll]
        API1[server api1]
        API2[server api2]
    end

    LB -->|8× SCM_RIGHTS por API| API1
    LB -->|8× SCM_RIGHTS por API| API2

    API1 --> R1[spin-read / epoll → tier_score → HTTP estático]
    API2 --> R2[spin-read / epoll → tier_score → HTTP estático]

    SOCK[(tmpfs /tmp/sockets)]
    LB --- SOCK
    API1 --- SOCK
    API2 --- SOCK
```

O **LB não parseia HTTP**: loop `accept4` → `sendmsg(SCM_RIGHTS)` → `close` (`src/platform/load_balancer.rs`). Cada API roda **`fd_gateway`** — uma thread, epoll edge-triggered, spin-read antes do epoll, busy-poll (`EPIOCSPARAMS`) — e classifica com `tier_score` (sem k-NN no runtime da submissão).

## Scorer em camadas (`tier_score`)

```mermaid
flowchart TD
    IN[JSON da transação] --> P[extract — parser customizado]
    P --> L{Gasto seguro?}
    L -->|sim| A0[count 0 → aprova]
    L -->|não| F{Gasto arriscado?}
    F -->|sim| A5[count 5 → nega]
    F -->|não| T{Árvore de decisão\n21 features}
    T -->|fraud| A5
    T -->|legit| A0
    T -->|parse falhou| R{Ratio amount / avg}
    R -->|acima do limiar| A5
    R -->|abaixo| A0
    A0 --> HTTP[Resposta HTTP estática]
    A5 --> HTTP
```

| Camada | O que faz |
|--------|-----------|
| **Atalhos** | Gasto seguro ou arriscado — resposta imediata (ver abaixo) |
| **Árvore** | `decision_tree` — 21 features, ~1040 nós gerados offline |
| **Ratio** | Fallback só com `amount` e `customer.avg_amount` |

### Gasto seguro e gasto arriscado

São **checagens rápidas** no início do `tier_score`. Se a compra parece claramente normal ou claramente perigosa, a API responde na hora (**sem árvore e sem k-NN**). Em cada caso, **todas** as condições da lista precisam ser verdadeiras.

Pense em: mercado perto de casa vs. compra cara, longe, em loja desconhecida e de alto risco.

```mermaid
flowchart LR
    subgraph legit [Gasto seguro — aprova]
        direction TB
        L1[Valor ≤ 500]
        L2[≤ 50% da média do cliente]
        L3[≤ 3 parcelas, ≤ 5 tx/24h]
        L4[Loja conhecida do cliente]
        L5[≤ 50 km de casa]
        L6[MCC seguro]
    end
    subgraph fraud [Gasto arriscado — nega]
        direction TB
        F1[Valor ≥ 5000]
        F2[≥ 5 parcelas, ≥ 6 tx/24h]
        F3[Loja NÃO conhecida]
        F4[≥ 150 km de casa]
        F5[MCC de alto risco]
    end
```

#### **Gasto seguro** (aprova, `count = 0`)

Perfil de gasto **baixo, habitual e em contexto seguro**:

| Campo | Condição | Intuição |
|-------|----------|----------|
| `transaction.amount` | ≤ **500** | Compra modesta |
| `amount / customer.avg_amount` | ≤ **0,50001** | Não é um pico em relação ao histórico |
| `installments` | ≤ **3** | Poucas parcelas |
| `customer.tx_count_24h` | ≤ **5** | Pouca atividade no dia |
| `merchant.id` | está em `known_merchants` | Cliente já compra nessa loja |
| `terminal.km_from_home` | ≤ **50** | Perto de “casa” |
| `merchant.mcc` | **5411**, **5812**, **5912** ou **5311** | Supermercado, restaurante, farmácia, varejo “comum” |

Exemplo mental: R$ 80 no mercado da esquina, 2x, 2 compras no dia, loja já conhecida, 10 km de casa, MCC supermercado.

#### **Gasto arriscado** (nega, `count = 5`)

Perfil de gasto **alto, agressivo e em contexto de risco**:

| Campo | Condição | Intuição |
|-------|----------|----------|
| `transaction.amount` | ≥ **5000** | Valor alto |
| `installments` | ≥ **5** | Muitas parcelas |
| `customer.tx_count_24h` | ≥ **6** | Muitas transações no dia |
| `merchant.id` | **não** está em `known_merchants` | Loja nunca vista pelo cliente |
| `terminal.km_from_home` | ≥ **150** | Longe de casa |
| `merchant.mcc` | **7995**, **7801** ou **7802** | Apostas / serviços financeiros de risco |

Exemplo mental: R$ 8.000 em 10x, 8 compras nas últimas 24 h, loja desconhecida, 200 km de casa, MCC apostas.

#### O que fica de fora?

Tudo que **não** cai nas duas caixas acima segue para a **árvore** (casos “cinza”: valor médio, MCC neutro, loja nova mas perto, etc.). Se faltar dado para montar as 21 features (ex.: timestamp inválido), cai no **ratio** `amount / avg_amount`.

Implementação: `src/search/tier_score.rs` (`obvious_legit`, `obvious_fraud`).

Validação offline:

```bash
cargo run --release --bin verify-tier -- test/test-data.json
```

## Fluxo de um request

```mermaid
sequenceDiagram
    participant K as k6
    participant LB as lb
    participant API as server
    K->>LB: POST /fraud-score
    LB->>API: sendmsg SCM_RIGHTS (fd do cliente)
    API->>API: read headers + body
    API->>API: tier_fraud_count
    API->>K: 200 JSON approved / fraud_score
```

## Rodar

```bash
docker compose up --build -d
```

Imagem publicada + pinagem de CPU (Mac Mini da prova): `docker compose -f docker-compose-ghcr.yml up -d` (`ghcr.io/sl4ureano/rinha2026:megazord`).

Benchmark: [test/README.md](test/README.md) (rede do container do LB).

```bash
docker run --rm --user root --network container:rinha2026-lb-1 \
  -e BASE_URL=http://127.0.0.1:9999 \
  -v "$(pwd)/test:/test" -w /test \
  grafana/k6:latest run test.js
```

## Limites Docker (prova)

Quota total: **1,00 CPU** (`0,10 + 0,45 + 0,45`) e **350 MB** de RAM (`169 + 169 + 8 + 4` tmpfs).

```mermaid
pie title RAM total 350 MB
    "api1" : 169
    "api2" : 169
    "lb" : 8
    "tmpfs sockets" : 4
```

| Serviço | CPU | RAM | Notas |
|---------|-----|-----|--------|
| lb | 0,10 | 8 MB | `CHANNELS_PER_API=8` → 16 upstreams |
| api1 | 0,45 | 169 MB | rede `rinha`; healthcheck TCP :8080 |
| api2 | 0,45 | 169 MB | `network_mode: none` (só Unix) |
| volume `sockets` | — | 4 MB tmpfs | `/tmp/sockets` |

Pinagem (`docker-compose-ghcr.yml`): api1 → CPU 0, api2 → CPU 2, lb → CPUs 1 e 3 (HT).

## Variáveis

| Variável | Serviço | Descrição |
|----------|---------|-----------|
| `LB_PORT` | lb | Porta pública (9999) |
| `API1_SOCKET` / `API2_SOCKET` | lb | Paths dos sockets Unix das APIs |
| `CHANNELS_PER_API` | lb | Canais duplicados por API (padrão **8**) |
| `CTRL_SOCK` | api | Socket de controle FD-pass |
| `FD_PASS=1` | api | Modo submissão (tier-only, sem mmap de índice) |
| `PORT` | api | Porta do healthcheck TCP (`/ready`) |
| `INDEX_PATH` | api | Só builds com `knn-index` / tooling offline |

## Build do índice (tooling)

Runtime da submissão usa só `tier_score` (Dockerfile com `--features submission`, sem `index.bin` na imagem). `build-index` gera `data/index.bin` para ferramentas offline e builds legados com k-NN:

```bash
cargo run --release --bin build-index -- resources data/index.bin
```

## Versão em C

Implementação **C11** do mesmo desenho (lb → FD-pass → scorer `tier_score`): repositório separado em [github.com/adsanla/rinha2026](https://github.com/adsanla/rinha2026).
