# Relatorio de testes de performance

Este documento resume os testes feitos para investigar e reduzir a latencia do projeto `rinha2026`, com foco no caminho de rede, `epoll`, FD passing e envio da resposta HTTP.

## Contexto

A aplicacao usa:

- `lb`: aceita conexoes TCP na porta `9999` e repassa o file descriptor do cliente para as APIs via Unix socket e `SCM_RIGHTS`.
- `api1` e `api2`: recebem o FD do cliente, leem a request, calculam o score e respondem diretamente no socket do cliente.
- `api2` com `network_mode: "none"`.
- `api1` em bridge.
- CPU total: `1.00 CPU`.
- Memoria total observada no resultado final: `346 MB`.

Configuracao final testada:

```yaml
api1/api2:
  - EPOLL_BUSY_POLL=1
  - EPOLL_BUSY_POLL_PROFILE=D

lb:
  - CHANNELS_PER_API=2
```

Telemetria foi desligada no teste final.

## Principais conclusoes

- O gargalo original estava no caminho de escrita da resposta HTTP, principalmente `write()` / `tcp_sendmsg` / TCP stack.
- JSON parsing, decisao de fraude e cache nao eram gargalos relevantes.
- `EPIOCSPARAMS` funciona no ambiente Mac do teste: `enabled=true`, `supported=true`, `errno=0`.
- Busy polling reduziu bastante o custo interno observado no Mac.
- A pinagem de CPU foi decisiva para chegar nas metricas atuais, principalmente por causa do cache e da topologia de cores do Mac Mini 2014.
- `CHANNELS_PER_API=2` manteve a performance e deixou a distribuicao entre APIs mais equilibrada.
- Com telemetria desligada, o resultado final ficou em `p99=0.47ms`, score maximo `6000`, sem erros HTTP.

## Impacto da pinagem de CPU

O ambiente de teste principal foi um Mac Mini 2014 com CPU Intel i5 Haswell, 2 nucleos fisicos e 4 threads logicas:

```text
CPU 0 + CPU 1 = mesmo nucleo fisico
CPU 2 + CPU 3 = outro nucleo fisico
```

A configuracao usada separa as duas APIs nos dois nucleos fisicos:

```text
api1 -> CPU 0
api2 -> CPU 2
lb   -> CPU 1,3
```

Essa pinagem fez diferenca porque cada API mantem seu proprio hot path rodando sempre no mesmo nucleo fisico:

- o codigo quente do parser, cache, arvore e resposta fica mais estavel no L1/L2;
- os dados acessados com frequencia sofrem menos migracao entre cores;
- o scheduler tem menos liberdade para mover a API entre CPUs e invalidar localidade de cache;
- o LB fica nos hyper-threads, fazendo trabalho mais leve de `accept4` e `SCM_RIGHTS`;
- as APIs, que executam a parte critica, ficam nos cores fisicos separados.

Sem pinagem, o kernel pode migrar as threads entre CPUs. Em um workload com latencias na casa de dezenas de microssegundos, pequenas perdas de cache, migracoes e wakeups em outro core aparecem no p99. Com pinagem, o comportamento ficou mais repetivel e as metricas internas estabilizaram perto de:

```text
request_total p99 ~= 60us
write_complete p99 ~= 50us
socket_recv p99 ~= 10us
```

Portanto, os resultados atuais nao vieram apenas de uma otimizacao isolada. Eles dependem da combinacao:

- APIs fixas em cores fisicos diferentes;
- LB em hyper-threads;
- `EPIOCSPARAMS` ativo;
- `CHANNELS_PER_API=2`;
- respostas HTTP estaticas;
- telemetria desligada no teste final.

## Evolucao dos resultados externos

| Configuracao | Telemetria | p99 externo | Score | Observacao |
| --- | --- | ---: | ---: | --- |
| Baseline local anterior | ligada | ~0.60ms a 0.75ms | 6000 | Ambiente local/WSL, sem suporte real a `EPIOCSPARAMS` |
| Perfil A no Mac | ligada | nao informado no JSON final, interno bom | 6000 | `EPIOCSPARAMS` aplicado: `10us`, budget `4` |
| Perfil C no Mac | ligada | `0.48ms` | 6000 | `EPIOCSPARAMS` aplicado: `50us`, budget `16` |
| Perfil D no Mac | ligada | `0.460750ms` | 6000 | `EPIOCSPARAMS` aplicado: `100us`, budget `32` |
| Perfil D + `CHANNELS_PER_API=2` | ligada | ~`0.460ms` | 6000 | Interno igual, distribuicao melhor |
| Perfil D + `CHANNELS_PER_API=2` | desligada | `0.465545ms` | 6000 | Resultado final limpo, sem telemetria |

O resultado final reportado foi:

```json
{
  "p99": "0.47ms",
  "raw": {
    "p99_ms": 0.46554535999999985
  },
  "failure_rate": "0%",
  "final_score": 6000,
  "http_errors": 0
}
```

## Metricas internas do perfil C

Snapshot analisado: request `#33` no webhook.

Configuracao:

```text
EPOLL_BUSY_POLL=1
EPOLL_BUSY_POLL_PROFILE=C
busy_poll_usecs=50
busy_poll_budget=16
```

Status:

```text
api1 enabled=true supported=true errno=0
api2 enabled=true supported=true errno=0
```

Metricas principais:

| Etapa | API1 avg | API1 p99 | API2 avg | API2 p99 |
| --- | ---: | ---: | ---: | ---: |
| request_total | 29us | 60us | 29us | 60us |
| write_complete | 23us | 50us | 23us | 50us |
| send_syscall | 23us | 50us | 23us | 50us |
| socket_recv | 4us | 10us | 4us | 10us |
| api_recv_fd | 10us | 20us | 11us | 20us |
| api_setsockopt | 2us | 7us | 2us | 5us |
| spin_read | 3us | 7us | 3us | 7us |
| epoll_dispatch | ~0us | 1us | ~0us | 1us |

Observacoes:

- `write_eagain=0`
- `partial_writes=0`
- `write_complete` ainda era a maior parte do tempo interno, mas caiu para aproximadamente `23us avg`.
- `request_total p99=60us`, enquanto o k6 media `p99=0.48ms` ponta a ponta. Isso indica que boa parte do p99 externo esta fora da logica da API.

## Metricas internas do perfil D

Snapshot analisado: request `#45`.

Configuracao:

```text
EPOLL_BUSY_POLL=1
EPOLL_BUSY_POLL_PROFILE=D
busy_poll_usecs=100
busy_poll_budget=32
CHANNELS_PER_API=4
```

Status:

```text
api1 enabled=true supported=true errno=0
api2 enabled=true supported=true errno=0
```

Metricas principais:

| Etapa | API1 avg | API1 p99 | API2 avg | API2 p99 |
| --- | ---: | ---: | ---: | ---: |
| request_total | 29us | 60us | 29us | 60us |
| write_complete | 23us | 50us | 23us | 50us |
| send_syscall | 23us | 50us | 23us | 50us |
| socket_recv | 4us | 10us | 4us | 10us |
| api_recv_fd | 10us | 25us | 10us | 20us |
| api_setsockopt | 2us | 5us | 2us | 10us |
| spin_read | 2us | 10us | 3us | 7us |

Conclusao do perfil D:

- Nao piorou CPU de forma visivel.
- Nao melhorou claramente as metricas internas em relacao ao perfil C.
- Foi seguro o suficiente para testar com `CHANNELS_PER_API=2`.

## Teste com CHANNELS_PER_API=2

Snapshot analisado: request `#57`.

Configuracao:

```text
EPOLL_BUSY_POLL_PROFILE=D
CHANNELS_PER_API=2
```

Metricas principais:

| Etapa | API1 avg | API1 p99 | API2 avg | API2 p99 |
| --- | ---: | ---: | ---: | ---: |
| request_total | 29us | 60us | 29us | 60us |
| write_complete | 23us | 50us | 23us | 50us |
| send_syscall | 23us | 50us | 23us | 50us |
| socket_recv | 4us | 10us | 4us | 10us |
| api_recv_fd | 10us | 20us | 10us | 20us |
| api_setsockopt | 2us | 7us | 2us | 3us |
| spin_read | 2us | 4us | 2us | 4us |

Distribuicao de requests:

| Configuracao | API1 requests | API2 requests |
| --- | ---: | ---: |
| D + `CHANNELS_PER_API=4` | 22440 | 21511 |
| D + `CHANNELS_PER_API=2` | 22065 | 21981 |

Com `CHANNELS_PER_API=2`, a distribuicao ficou mais equilibrada e as metricas internas nao pioraram.

## Load balancer

No teste com `CHANNELS_PER_API=2`, o LB permaneceu saudavel:

```text
lb_accept avg=7us p99=15us
lb_handoff_scm_rights avg=11us p99=20us
handoff_error=0
```

Isso indica que o FD passing nao virou gargalo relevante.

## Busy poll

Perfis usados:

| Perfil | busy_poll_usecs | busy_poll_budget |
| --- | ---: | ---: |
| A | 10 | 4 |
| B | 25 | 8 |
| C | 50 | 16 |
| D | 100 | 32 |

No ambiente local WSL/Docker, `EPIOCSPARAMS` falhou com:

```text
EPIOCSPARAMS unsupported, continuing without epoll busy poll: Inappropriate ioctl for device (os error 25)
```

No Mac do teste, o mesmo codigo aplicou corretamente:

```text
enabled=true
supported=true
errno=0
```

O comportamento final e best-effort:

- Se o kernel suportar `EPIOCSPARAMS`, aplica busy poll.
- Se nao suportar, registra o erro e continua sem quebrar.

## O que cada tempo significa

| Campo | Significado |
| --- | --- |
| `request_total` | tempo interno total medido dentro da API |
| `socket_recv` | tempo para ler bytes do request do socket |
| `parse_json` | tempo de extracao/parsing do JSON |
| `decision_tree` | tempo de decisao de fraude quando nao caiu em fast path |
| `api_recv_fd` | tempo para receber o FD do LB via Unix socket |
| `api_setsockopt` | tempo para aplicar opcoes TCP no FD recebido |
| `spin_read` | tentativa curta de ler logo apos receber o FD |
| `send_syscall` | syscall de escrita no socket |
| `write_complete` | tempo ate completar o envio da resposta |
| `epoll_wait` | tempo esperando eventos; inclui tempo ocioso, nao e custo direto por request |
| `epoll_dispatch` | tempo entre evento de epoll e handler |

Conversao:

```text
1ms = 1000us
60us = 0.060ms
460us = 0.460ms
```

## Configuracao final recomendada

```yaml
api1:
  environment:
    - FD_PASS=1
    - CTRL_SOCK=/tmp/sockets/api1.sock
    - PORT=8080
    - TIER_ONLY=1
    - EPOLL_BUSY_POLL=1
    - EPOLL_BUSY_POLL_PROFILE=D

api2:
  environment:
    - FD_PASS=1
    - CTRL_SOCK=/tmp/sockets/api2.sock
    - PORT=8080
    - TIER_ONLY=1
    - EPOLL_BUSY_POLL=1
    - EPOLL_BUSY_POLL_PROFILE=D

lb:
  environment:
    - LB_PORT=9999
    - API1_SOCKET=/tmp/sockets/api1.sock
    - API2_SOCKET=/tmp/sockets/api2.sock
    - CHANNELS_PER_API=2
```

Para teste final de pontuacao, manter telemetria desligada:

- sem `ENABLE_REMOTE_PROFILING=true`
- sem `PERF_WEBHOOK_URL`
- sem `PERF_RELAY_SOCKET`
- sem `PERF_RELAY_LISTEN`
- sem `PERF_SNAPSHOT_INTERVAL_SECS`

## Conclusao

A aplicacao chegou a um ponto em que o processamento interno esta muito baixo:

```text
request_total p99 ~= 60us
write_complete p99 ~= 50us
socket_recv p99 ~= 10us
```

O p99 externo final ficou:

```text
0.465545ms
```

Com score maximo:

```text
final_score=6000
failure_rate=0%
http_errors=0
```

Nesse ponto, os ganhos restantes provavelmente dependem mais de ruido do ambiente, Docker/network stack, scheduler e configuracao do benchmark do que de logica Rust/JSON/score.
