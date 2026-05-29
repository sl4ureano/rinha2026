# Rinha Flow — visualizador 3D

Apresentação **lúdica e em tempo real** do fluxo `POST /fraud-score`: load balancer, FD-pass, APIs, pipeline híbrido (fast path → árvore → ratio, com k-NN backup) e métricas da última request.

## Screenshot

![Screenshot do visualizador 3D](public/img/print.png)

## Pré-requisitos

- [Node.js](https://nodejs.org/) 18+
- Stack da Rinha rodando (opcional, para bater na API real):

```bash
docker compose up --build -d
```

## Rodar

Na raiz do repositório:

```bash
cd visualizador
npm start
```

Abra **http://localhost:3333**

| Variável | Padrão | Descrição |
|----------|--------|-----------|
| `VIZ_PORT` | `3333` | Porta do visualizador |
| `FRAUD_API_URL` | `http://127.0.0.1:9999` | URL do LB da API |

## Como usar

1. Escolha um exemplo no select ou edite o JSON.
2. **Enviar** — chama a API real em `:9999` e mostra o trace local (mesma lógica do pipeline híbrido: fast path → árvore → ratio).
3. **Só simular** — trace sem rede (útil se o Docker não estiver no ar).
4. A cena 3D anima a partícula pelo caminho; o painel direito lista checks, árvore e latências.
5. Várias abas recebem o mesmo fluxo via **SSE** (`/api/events`).

## Pipeline híbrido (reproduzido em JS)

```
JSON extract → Gasto seguro? ─sim→ aprova
                    └─não→ Gasto arriscado? ─sim→ nega
                              └─não→ Árvore (21 features) ─┬→ aprova/nega
                                                          └─parse falhou→ Ratio → resposta

  [k-NN index: 3M refs, 14 dims — backup em memória, não no hot path]
```

O visualizador replica **exatamente** a mesma lógica do pipeline Rust:
- **Fast path** (atalhos determinísticos) — verde tracejado na cena
- **Decision tree** (21 features, 1039 nós) — roxo na cena
- **Ratio fallback** — laranja na cena

O índice k-NN é um componente server-side (AVX2, mmap) e não é simulado no visualizador. No hot path atual, a árvore resolve todos os casos cinza.

## Integração

- Lê a árvore de decisão direto de `../scripts/decision_tree.nodes` (sem duplicar dados).
- Exemplos de `../resources/example-payloads.json`.
- Não altera o binário Rust nem o caminho quente de produção.

## API interna

| Método | Rota | Descrição |
|--------|------|-----------|
| `GET` | `/` | Interface 3D |
| `GET` | `/api/health` | Status (inclui nº de nós da árvore) |
| `GET` | `/api/examples` | Payloads de exemplo |
| `GET` | `/api/events` | SSE — fluxos em tempo real |
| `POST` | `/api/trace` | Trace + proxy para API real |
| `POST` | `/api/simulate` | Apenas trace local |
