# Rinha 2026 — Detecção de Fraude

API em **Rust** que classifica transações financeiras em tempo real usando um **pipeline híbrido de 3 camadas**: fast path (atalhos determinísticos) → decision tree (21 features) → ratio fallback, com um **índice k-NN** (3 M referências, 14 dimensões, AVX2) carregado em memória como safety net.

![Screenshot do visualizador 3D](visualizador/public/img/print.png)

Visualizador 3D em tempo real: `visualizador/` (veja [`visualizador/README.md`](visualizador/README.md)).

---

## 1. Visão Geral da Arquitetura

O load balancer repassa conexões TCP via `SCM_RIGHTS` para duas instâncias da API. Cada instância classifica a transação em camadas, do mais rápido ao mais preciso.

```mermaid
flowchart TB
    C[Cliente / k6] -->|TCP :9999| LB[lb — accept + round-robin]

    subgraph apis ["APIs — fd_gateway, 1 thread + epoll"]
        API1[server api1]
        API2[server api2]
    end

    LB -->|"8× SCM_RIGHTS"| API1
    LB -->|"8× SCM_RIGHTS"| API2

    API1 --> R1["fast_path → tree → ratio<br/>+ k-NN backup mmap"]
    API2 --> R2["fast_path → tree → ratio<br/>+ k-NN backup mmap"]

    SOCK[("tmpfs /tmp/sockets")]
    LB --- SOCK
    API1 --- SOCK
    API2 --- SOCK

    IDX[("index.bin<br/>91.6 MB mmap")]
    API1 --- IDX
    API2 --- IDX

    style C fill:#3498db,color:#fff,stroke:#2980b9
    style LB fill:#e67e22,color:#fff,stroke:#d35400
    style API1 fill:#2ecc71,color:#fff,stroke:#27ae60
    style API2 fill:#2ecc71,color:#fff,stroke:#27ae60
    style R1 fill:#1abc9c,color:#fff,stroke:#16a085
    style R2 fill:#1abc9c,color:#fff,stroke:#16a085
    style SOCK fill:#95a5a6,color:#fff,stroke:#7f8c8d
    style IDX fill:#9b59b6,color:#fff,stroke:#8e44ad
```

O **LB não parseia HTTP** — é um loop `accept4` → `sendmsg(SCM_RIGHTS)` → `close`. Cada API roda **`fd_gateway`**: uma thread, epoll edge-triggered, spin-read, busy-poll.

---

## 2. Pipeline de Classificação (Híbrido)

Cada request passa por **3 camadas em cascata**. A maioria (~79%) é resolvida na primeira, sem tocar em modelo nenhum.

```mermaid
flowchart TD
    IN["JSON da transação"] --> P["extract — parser customizado"]
    P --> FP{"Fast path<br/>Gasto seguro?"}
    FP -->|"sim — 52.7%"| A0["count 0 → APROVA"]
    FP -->|"não"| FF{"Fast path<br/>Gasto arriscado?"}
    FF -->|"sim — 26.4%"| A5["count 5 → NEGA"]
    FF -->|"não — 20.9%"| T{"Árvore de decisão<br/>21 features, 1039 nós"}
    T -->|fraud| A5
    T -->|legit| A0
    T -->|"parse falhou"| R{"Ratio amount / avg"}
    R -->|"acima do limiar"| A5
    R -->|"abaixo"| A0
    A0 --> HTTP["Resposta HTTP estática"]
    A5 --> HTTP

    KNN["k-NN index (backup)<br/>3M refs, 14 dims, AVX2<br/>192 partições, mmap"]

    style IN fill:#3498db,color:#fff,stroke:#2980b9
    style P fill:#2c3e50,color:#fff,stroke:#1a252f
    style FP fill:#f39c12,color:#fff,stroke:#e67e22
    style FF fill:#e74c3c,color:#fff,stroke:#c0392b
    style T fill:#9b59b6,color:#fff,stroke:#8e44ad
    style R fill:#e67e22,color:#fff,stroke:#d35400
    style A0 fill:#2ecc71,color:#fff,stroke:#27ae60
    style A5 fill:#e74c3c,color:#fff,stroke:#c0392b
    style HTTP fill:#1abc9c,color:#fff,stroke:#16a085
    style KNN fill:#f4ecf7,color:#8e44ad,stroke:#9b59b6,stroke-dasharray: 5 5
```

| Camada | O que faz | Cobertura | Latência |
|--------|-----------|:---------:|:--------:|
| **Fast path** | Gasto seguro ou arriscado — resposta imediata | ~79% | ~0 μs |
| **Árvore** | `decision_tree` — 21 features, ~1040 nós gerados offline | ~21% | ~0 μs |
| **Ratio** | Fallback só com `amount` e `customer.avg_amount` | raro | ~0 μs |
| **k-NN (backup)** | 5 vizinhos mais próximos em 3 M referências | disponível | ~0.3 ms |

O índice k-NN é treinado a partir de `references.json.gz`, carregado via `mmap` + `mlockall` e compartilhado entre as APIs. No hot path a árvore resolve tudo; o k-NN fica pronto caso a árvore perca acurácia com dados futuros.

---

## 3. Gasto Seguro e Gasto Arriscado

São **checagens rápidas** no início do pipeline. Se a compra parece claramente normal ou claramente perigosa, a API responde na hora — **sem árvore e sem k-NN**. Em cada caso, **todas** as condições precisam ser verdadeiras.

Pense em: mercado perto de casa vs. compra cara, longe, em loja desconhecida e de alto risco.

```mermaid
flowchart LR
    subgraph legit ["✅ Gasto seguro — aprova"]
        direction TB
        L1["Valor ≤ 500"]
        L2["≤ 50% da média do cliente"]
        L3["≤ 3 parcelas, ≤ 5 tx/24h"]
        L4["Loja conhecida do cliente"]
        L5["≤ 50 km de casa"]
        L6["MCC seguro (5411, 5812, 5912, 5311)"]
    end
    subgraph fraud ["❌ Gasto arriscado — nega"]
        direction TB
        F1["Valor ≥ 5000"]
        F2["≥ 5 parcelas, ≥ 6 tx/24h"]
        F3["Loja NÃO conhecida"]
        F4["≥ 150 km de casa"]
        F5["MCC de alto risco (7995, 7801, 7802)"]
    end

    style L1 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style L2 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style L3 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style L4 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style L5 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style L6 fill:#d5f5e3,color:#1e8449,stroke:#27ae60
    style F1 fill:#fadbd8,color:#922b21,stroke:#e74c3c
    style F2 fill:#fadbd8,color:#922b21,stroke:#e74c3c
    style F3 fill:#fadbd8,color:#922b21,stroke:#e74c3c
    style F4 fill:#fadbd8,color:#922b21,stroke:#e74c3c
    style F5 fill:#fadbd8,color:#922b21,stroke:#e74c3c
```

**Exemplo mental — gasto seguro:** R$ 80 no mercado da esquina, 2x, 2 compras no dia, loja já conhecida, 10 km de casa, MCC supermercado.

**Exemplo mental — gasto arriscado:** R$ 8.000 em 10x, 8 compras nas últimas 24 h, loja desconhecida, 200 km de casa, MCC apostas.

**O que fica de fora?** Tudo que não cai nas duas caixas acima segue para a **árvore** (casos "cinza": valor médio, MCC neutro, loja nova mas perto, etc.). Se faltar dado para montar as 21 features (ex.: timestamp inválido), cai no **ratio** `amount / avg_amount`.

Implementação: `src/search/fast_path.rs` (atalhos) + `src/search/tier_score.rs` (árvore + ratio).

---

## 4. Árvore de Decisão

Árvore binária com **21 features**, **1039 nós** e **520 folhas**. Treinada offline com `sklearn.tree.DecisionTreeClassifier(criterion='gini', max_leaf_nodes=520)`.

```mermaid
flowchart TD
    ROOT["Root: feature 3<br/>hour_of_day ≤ 0.283"] -->|"≤ 0.283"| N1["feature 13<br/>merchant_avg ≤ 0.010"]
    ROOT -->|"> 0.283"| N164["feature 11<br/>unknown_merchant ≤ 0.5"]

    N1 -->|"≤ 0.010"| N2["feature 11<br/>unknown_merchant ≤ 0.5"]
    N1 -->|"> 0.010"| N121["...mais ramos..."]

    N2 -->|"known"| N3["feature 0<br/>amount ≤ 0.287"]
    N2 -->|"unknown"| N16["...mais ramos..."]

    N3 -->|"≤ 0.287"| N4["feature 12<br/>mcc_risk ≤ 0.625"]
    N3 -->|"> 0.287"| LEAF_F["FRAUD"]

    N4 --> LEAF_L1["LEGIT"]
    N4 --> LEAF_L2["...520 folhas total..."]

    N164 -->|"known"| LEAF_L3["LEGIT"]
    N164 -->|"unknown"| N_MORE["...mais ramos..."]

    style ROOT fill:#f39c12,color:#fff,stroke:#e67e22
    style N1 fill:#f8c471,color:#fff,stroke:#f39c12
    style N2 fill:#f8c471,color:#fff,stroke:#f39c12
    style N3 fill:#f8c471,color:#fff,stroke:#f39c12
    style N4 fill:#f8c471,color:#fff,stroke:#f39c12
    style N164 fill:#f8c471,color:#fff,stroke:#f39c12
    style N121 fill:#d5d8dc,color:#2c3e50,stroke:#aeb6bf
    style N16 fill:#d5d8dc,color:#2c3e50,stroke:#aeb6bf
    style N_MORE fill:#d5d8dc,color:#2c3e50,stroke:#aeb6bf
    style LEAF_F fill:#e74c3c,color:#fff,stroke:#c0392b
    style LEAF_L1 fill:#2ecc71,color:#fff,stroke:#27ae60
    style LEAF_L2 fill:#95a5a6,color:#fff,stroke:#7f8c8d
    style LEAF_L3 fill:#2ecc71,color:#fff,stroke:#27ae60
```

O script `scripts/gen_decision_tree.py` converte `scripts/decision_tree.nodes` (formato Zig-like) para:
- **Rust**: `src/search/decision_tree.rs` — array estático `NODES` + `fn predict()`
- **C**: `c-tree/{include,src}/decision_tree.{h,c}`

### Pipeline offline: gerar dados → treinar → compilar

A árvore em produção usa **21 features**, das quais 16–17 são valores brutos (`customer_avg_amount`, `amount_ratio`) que **não** estão em `references.json.gz` (só 14 dims normalizadas). Por isso o treino precisa partir de **payloads brutos** ou do **gerador sintético** — não basta expandir o `.json.gz`.

```mermaid
flowchart LR
    GEN["Gerador de dados<br/>C oficial ou synthetic_data.py"]
    RAW["Payloads brutos<br/>+ labels fraud/legit"]
    TRAIN["train_decision_tree.py<br/>sklearn, 520 folhas"]
    NODES["decision_tree.nodes"]
    CODE["gen_decision_tree.py<br/>.rs + .c"]
    REFS["references.json.gz<br/>14 dims → k-NN"]

    GEN --> RAW
    GEN --> REFS
    RAW --> TRAIN
    TRAIN --> NODES
    NODES --> CODE

    style GEN fill:#3498db,color:#fff,stroke:#2980b9
    style RAW fill:#3498db,color:#fff,stroke:#2980b9
    style TRAIN fill:#f39c12,color:#fff,stroke:#e67e22
    style NODES fill:#9b59b6,color:#fff,stroke:#8e44ad
    style CODE fill:#2ecc71,color:#fff,stroke:#27ae60
    style REFS fill:#95a5a6,color:#fff,stroke:#7f8c8d
```

**Pré-requisitos:** Python 3.10+, `pip install scikit-learn numpy`. Configs em `resources/normalization.json` e `resources/mcc_risk.json`.

#### 1. Gerar dados sintéticos

Gerador oficial (C, repositório da prova): [zanfranceschi/rinha-de-backend-2026/data-generator](https://github.com/zanfranceschi/rinha-de-backend-2026/tree/main/data-generator)

```bash
cd data-generator && make

# Referências para o índice k-NN (3M × 14 dims)
./data-generator \
  --refs 3000000 \
  --refs-seed 42 \
  --fraud-ratio-refs 0.30 \
  --norm-cfg ../resources/normalization.json \
  --mcc-cfg ../resources/mcc_risk.json \
  --refs-out ../resources/references.json

gzip -k ../resources/references.json   # → references.json.gz

# Payloads de teste (k6 / verify-tier)
./data-generator \
  --payloads 54100 \
  --payloads-seed 4242 \
  --fraud-ratio-payloads 0.30 \
  --randomize-payload-dates \
  --reuse-refs --refs-in ../resources/references.json \
  --payloads-out ../test/test-data.json
```

Port Python equivalente (sem compilar C): `scripts/synthetic_data.py`

```bash
# Verificar amostra contra example-references.json
python scripts/synthetic_data.py --verify resources/example-references.json --n-check 100

# Inspecionar uma transação gerada
python scripts/synthetic_data.py
```

| Parâmetro | Valor padrão | Uso |
|-----------|:------------:|-----|
| `--refs-seed` / `REF_SEED` | **42** | Referências + treino da árvore |
| `--payloads-seed` / `PAY_SEED` | **4242** | Payloads de teste da prova |
| `--fraud-ratio-*` | **0.30** | 70% legit, 27% fraud, 3% borderline |
| `--refs` | 200 (demo) / **3000000** (produção) | Tamanho do índice k-NN |

#### 2. Treinar a árvore

**Recomendado** — gera payloads em memória com features brutas 16/17 corretas:

```bash
python scripts/train_decision_tree.py --from-generator 3000000
```

Opções úteis:

```bash
python scripts/train_decision_tree.py \
  --from-generator 3000000 \
  --seed 42 \
  --fraud-ratio 0.30 \
  --max-leaf-nodes 520 \
  --random-state 42
```

**Alternativa** — a partir de JSON com payloads completos (formato `test/test-data.json`):

```bash
python scripts/train_from_payloads.py test/test-data.json \
  --max-leaf-nodes 520 \
  --random-state 42 \
  --output scripts/decision_tree.nodes
```

**Aproximação (não reproduz a árvore atual)** — só `references.json.gz`, perde ratios > 10:

```bash
python scripts/train_decision_tree.py
```

Hiperparâmetros fixos do sklearn: `criterion='gini'`, `max_leaf_nodes=520`, `random_state=42` → **1039 nós** (520 folhas + 519 internos).

#### 3. Compilar para Rust e C

```bash
python scripts/gen_decision_tree.py          # gera .rs e .c
python scripts/gen_decision_tree.py --rust-only  # só src/search/decision_tree.rs
```

Recompilar a API após alterar a árvore:

```bash
cargo build --release --features submission
```

#### Resumo rápido (do zero)

```bash
pip install scikit-learn numpy

# Treinar + exportar .nodes (usa gerador Python, ~5–15 min com 3M)
python scripts/train_decision_tree.py --from-generator 3000000
python scripts/gen_decision_tree.py
cargo build --release --features submission
```

---

## 5. Índice k-NN

Construído offline a partir de `resources/references.json.gz` (3 M entries × 14 features normalizadas [0,1]):

```bash
cargo run --release --bin build-index -- resources data/index.bin
```

```mermaid
flowchart LR
    REF["references.json.gz<br/>3M entries, 14 features"]
    MCC["mcc_risk.json"]
    BUILD["build-index<br/>quantiza → particiona → KD-tree"]
    IDX["index.bin<br/>91.6 MB"]

    REF --> BUILD
    MCC --> BUILD
    BUILD --> IDX

    style REF fill:#3498db,color:#fff,stroke:#2980b9
    style MCC fill:#3498db,color:#fff,stroke:#2980b9
    style BUILD fill:#f39c12,color:#fff,stroke:#e67e22
    style IDX fill:#9b59b6,color:#fff,stroke:#8e44ad
```

| Propriedade | Valor |
|-------------|-------|
| Tamanho | ~91.6 MB |
| Partições | 192 (KD-tree por bucket) |
| Nós | 69.342 |
| Blocos (SoA, 8 vetores i16) | 389.823 |
| Quantização | float → i16 × 10.000 |
| Busca | AVX2 SIMD, poda por bbox, early termination |
| Decisão | top-5 vizinhos: ≥ 3 fraud → nega |

O `Dockerfile` gera o índice no stage 2 e copia para o runtime. Ambas APIs fazem `mmap` read-only do mesmo arquivo.

---

## 6. Fluxo de um Request

```mermaid
sequenceDiagram
    participant K as k6
    participant LB as lb
    participant API as server
    participant IDX as index.bin

    rect rgb(235, 245, 251)
        K->>LB: POST /fraud-score
        LB->>API: sendmsg SCM_RIGHTS (fd do cliente)
        API->>API: read headers + body
    end

    rect rgb(234, 250, 241)
        API->>API: fast_path (atalhos)
        alt ~79% ObviousLegit / ObviousFraud
            API->>K: 200 JSON approved / fraud_score
        else ~21% gray area
            API->>API: tier_fraud_count (árvore + ratio)
            API->>K: 200 JSON approved / fraud_score
        end
    end

    Note over IDX: mmap em memória<br/>disponível como backup
```

---

## 7. Dockerfile (3 Stages)

```mermaid
flowchart LR
    S1["Stage 1: Build<br/>rust:1.84-bookworm<br/>cargo build --release<br/>--features submission"]
    S2["Stage 2: Index<br/>build-index<br/>references.json.gz → index.bin"]
    S3["Stage 3: Runtime<br/>debian:bookworm-slim<br/>server + lb + index.bin"]
    S1 --> S2 --> S3

    style S1 fill:#f39c12,color:#fff,stroke:#e67e22
    style S2 fill:#9b59b6,color:#fff,stroke:#8e44ad
    style S3 fill:#2ecc71,color:#fff,stroke:#27ae60
```

| Stage | Artefatos | Notas |
|-------|-----------|-------|
| 1 — Build | `server`, `lb`, `healthcheck`, `build-index` | `RUSTFLAGS="-C target-cpu=haswell"` |
| 2 — Index | `data/index.bin` (91.6 MB) | A partir de `references.json.gz` + `mcc_risk.json` |
| 3 — Runtime | Binários + index | `debian:bookworm-slim`, sem ferramentas de build |

---

## 8. Limites Docker (Prova)

Quota total: **1,00 CPU** (`0,10 + 0,45 + 0,45`) e **350 MB** de RAM.

```mermaid
pie title "RAM total 350 MB"
    "api1 (169 MB)" : 169
    "api2 (169 MB)" : 169
    "lb (8 MB)" : 8
    "tmpfs sockets (4 MB)" : 4
```

| Serviço | CPU | RAM | Notas |
|---------|-----|-----|--------|
| lb | 0,10 | 8 MB | `CHANNELS_PER_API=8` → 16 upstreams |
| api1 | 0,45 | 169 MB | rede `rinha`; healthcheck TCP :8080; index.bin mmap |
| api2 | 0,45 | 169 MB | `network_mode: none` (só Unix); index.bin mmap shared |
| volume `sockets` | — | 4 MB tmpfs | `/tmp/sockets` |

Pinagem (`docker-compose-ghcr.yml`): api1 → CPU 0, api2 → CPU 2, lb → CPUs 1 e 3 (HT).

---

## 9. Performance

Resultados k6 (ramping 1 → 900 req/s, 120s, 54.100 entries):

```mermaid
xychart-beta
    title "p99 Latência por Versão (ms)"
    x-axis ["OLD tier-only", "KNN puro", "OTIMIZADA"]
    y-axis "p99 ms" 0 --> 0.6
    bar [0.31, 0.43, 0.31]
```

| Métrica | Tier-only (legado) | k-NN puro | Híbrido (atual) |
|---------|:--:|:--:|:--:|
| **p99** | **0,31 ms** | 0,43 ms | **0,31 ms** |
| FP / FN | 0 / 0 | 0 / 0 | 0 / 0 |
| **Score** | **6.000** | **6.000** | **6.000** |

---

## 10. Rodar

```bash
docker compose up --build -d
```

Imagem publicada + pinagem de CPU (Mac Mini da prova):
```bash
docker compose -f docker-compose-ghcr.yml up -d
```
(`ghcr.io/sl4ureano/rinha2026:megazord`)

Benchmark: [test/README.md](test/README.md) (rede do container do LB).

```bash
docker run --rm --user root --network container:rinha2026-lb-1 \
  -e BASE_URL=http://127.0.0.1:9999 \
  -v "$(pwd)/test:/test" -w /test \
  grafana/k6:latest run test.js
```

Validação offline:

```bash
cargo run --release --bin verify-tier -- test/test-data.json
```

---

## 11. Variáveis

| Variável | Serviço | Descrição |
|----------|---------|-----------|
| `LB_PORT` | lb | Porta pública (9999) |
| `API1_SOCKET` / `API2_SOCKET` | lb | Paths dos sockets Unix das APIs |
| `CHANNELS_PER_API` | lb | Canais duplicados por API (padrão **8**) |
| `CTRL_SOCK` | api | Socket de controle FD-pass |
| `FD_PASS=1` | api | Modo submissão (hybrid: fast_path + tree + k-NN mmap) |
| `PORT` | api | Porta do healthcheck TCP (`/ready`) |
| `INDEX_PATH` | api | Caminho do `index.bin` (padrão `data/index.bin`) |
| `TIER_ONLY=1` | api | Desabilita carregamento do índice k-NN (modo legado) |

---

## 12. Por que essa Arquitetura?

| Problema | Solução |
|----------|---------|
| Árvore treinada de `references.json.gz` perde 15.4% accuracy (features 16/17 clampadas) | k-NN index usa apenas as 14 features disponíveis → 0 FP/FN |
| k-NN puro aumenta p99 de 0.31 ms → 0.43 ms (+39%) | Decision tree existente como classificador primário → p99 = 0.31 ms |
| Test data muda entre ambientes de prova | k-NN index (3M refs) disponível como backup instantâneo |
| Index.bin ocupa 91.6 MB | mmap shared entre api1 e api2, cabe nos 169 MB por instância |

---

## 13. Versão em C

Implementação **C11** do mesmo desenho (lb → FD-pass → scorer `tier_score`): repositório separado em [github.com/adsanla/rinha2026](https://github.com/adsanla/rinha2026).
