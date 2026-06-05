# Rinha Flow Visualizador

Visualizador 3D do fluxo atual da API:

1. Cliente envia `POST /fraud-score`.
2. O load balancer repassa o fd para uma API.
3. A API faz parse HTTP/JSON.
4. Gasto claramente seguro aprova no fast path.
5. Qualquer gasto não seguro cai no k-NN exato.
6. A resposta volta ao cliente.

## Rodar

```bash
npm install
npm start
```

Variáveis:

| Variavel | Padrao | Descricao |
| --- | --- | --- |
| `VIZ_PORT` | `3333` | Porta do visualizador |
| `FRAUD_API_URL` | `http://127.0.0.1:9999` | API real usada pelo botão de envio |

Endpoints internos:

| Metodo | Rota | Descricao |
| --- | --- | --- |
| `GET` | `/api/health` | Status do visualizador |
| `GET` | `/api/examples` | Payloads de exemplo |
| `POST` | `/api/simulate` | Simula o fluxo localmente |
| `POST` | `/api/trace` | Chama a API real e anima o fluxo |
| `GET` | `/api/events` | Eventos SSE para animação |
