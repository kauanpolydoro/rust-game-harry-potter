# SLOs e evidência operacional

A issue #35 mede o serviço por observações sanitizadas e resultados reais das operações.
Os runbooks ficam em [runbooks.md](runbooks.md).
A autoridade sobre expiração, revogação e persistência continua sendo o PostgreSQL.
Observações do navegador são não confiáveis e não podem acionar alertas de integridade P0.

## Contrato de medição

| SLI | População e resultado | Meta |
| --- | --- | --- |
| Comando | Todas as tentativas HTTP, separando aceitação, rejeição, erro técnico e cancelamento. | p95 300 ms; p99 1 s, sem Argon2id. |
| Disponibilidade | Resultados técnicos por API, recuperação e handshake, com denominadores próprios. | 99,5% mensal. |
| Evento remoto | Confirmação durável até recepção remota, separada do tempo de escrita no socket. | p95 500 ms; p99 1,5 s. |
| Reconexão | Início da tentativa até sincronização, por replay ou Snapshot; falhas e abandonos permanecem no denominador. | p95 3 s por replay e 5 s por Snapshot. |
| Recuperação técnica | Tentativa enviada até resposta, incluindo pool, fila Argon2id e commit. | p95 2 s. |
| Recuperação humana | Entrada no fluxo até acesso recuperado; contar abandono e assistência separadamente. | p75 90 s; pelo menos 95% sem suporte. |
| Revogação | Aceitação de novo Comando após revogação e tempo até fechamento remoto de sockets. | Zero aceitações; fechamento p95 2 s e p99 5 s. |
| Expiração | Aceitação de Comando/recuperação no limite ou depois, detecção e purge, incluindo pendências. | Zero aceitações; detecção p95 300 s; purge p95 3.600 s e máximo 86.400 s. |
| Lifecycle | Etapas, tentativas falhas, órfãos, ledger e idade das pendências, com heartbeat independente. | Não concluir com órfãos ou sem prova externa. |
| Recuperação de desastre | Último ponto recuperável, duração do restore e janela de backups. | RPO 300 s; RTO 3.600 s; janela 604.800 s. |
| Runtime | Atraso do executor, capacidade do pool, memória do processo e disponibilidade da coleta. | Investigar saturação junto com latência e erros. |
| Shell | LCP, INP e CLS observados pelo navegador, sem elementos ou URLs. | p75 2,5 s, 200 ms e 0,1. |

Percentis devem ser calculados sobre as observações da população correspondente.
Não calcule a média dos percentis de instâncias ou de janelas.
Ausência de amostras significa falta de evidência, não disponibilidade de 100% ou latência zero.
Separe medições locais, ensaios de carga e tráfego de produção.

## Fases de latência

Fila é a espera de admissão e execução de Argon2id.
Pool é a aquisição de uma conexão, medida antes de iniciar a transação.
Lock é o trecho que adquire a raiz e revalida acesso, incluindo o round trip da consulta de bloqueio.
Regra é a decisão síncrona do motor e sua preparação em memória.
SQL é persistência ou leitura fora do trecho de lock, com início de transação separado da aquisição do pool.
Commit é a espera pela confirmação da transação; falha durante essa fase é resultado incerto.
`response_prepare_seconds` mede a preparação da resposta depois da persistência.
`socket_write_seconds` mede serialização e escrita do frame, incluindo erro e timeout.
Nenhuma dessas duas fases prova recepção remota.
O tempo total HTTP também inclui validação, autenticação e outras etapas e não deve ser reconstruído somando percentis de fases.

## Privacidade e retenção

Use allowlist de campos e valores antes da escrita.
Targets de dependências não podem habilitar logs SQL, AWS, headers ou protocolos por meio de `RUST_LOG`.
Métricas nunca usam identificadores de Partida, Participante, Sessão ou Comando como dimensões.
Uma correlação aleatória de requisição pode ajudar a investigar uma operação, mas permanece dado identificável de curta duração e não é dimensão de métrica.
Não habilite um segundo destino de traces identificáveis com retenção diferente.
Exportações, dumps e logs de proxies também fazem parte do inventário de retenção.

CloudWatch EMF extrai observações numéricas do JSON de logs e permite calcular percentis sobre as amostras, conforme a [especificação oficial](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html).
A retenção do grupo deve ser sete dias, usando `RetentionInDays: 7` no [recurso oficial de LogGroup](https://docs.aws.amazon.com/AWSCloudFormation/latest/TemplateReference/aws-resource-logs-loggroup.html).
O TTL torna dados expirados; a remoção física gerenciada pelo serviço pode ocorrer depois.
Métricas agregadas sem identificadores podem ter retenção maior.

## Incidentes P0

| Condição | Evidência exigida | Resposta |
| --- | --- | --- |
| Divergência | Hash, versão, Snapshot, recibo ou sequência incompatível com estado persistido. | Conter mutações e seguir o runbook de integridade. |
| ACK antes do commit | Confirmação de aceitação sem prova de commit durável. | Suspender a versão e resolver o recibo pelo mesmo identificador. |
| Autorização após revogação/expiração | Decisão de aceitação depois do limite no relógio e transação de autoridade. | Conter acesso e reproduzir o corte concorrente. |
| Segredo detectado | Emissão de campo privado ou conteúdo fora da allowlist. | Bloquear a escrita e seguir o runbook de chave. |
| Ressurreição após restore | Raiz restaurada em conflito com Tombstone externo. | Manter isolamento, reconciliar e verificar todas as cópias. |

Uma falha de coleta deve alertar por ausência de evidência.
Não fabrique amostras de sucesso para manter alarmes verdes.
Alertas só são operacionais depois de conectar e testar o destino de notificação no ambiente de implantação.

## Métricas disponíveis e cálculo

Todas as séries usam namespace `Hogwarts` e dimensões fechadas `environment`, `operation` e, opcionalmente, `outcome`.
Contadores emitem incrementos e usam `Sum`; durações emitem amostras em segundos.

| População | Séries e interpretação |
| --- | --- |
| HTTP | `http_requests` e `http_duration_seconds`, por `api`, `command`, `recovery` e `handshake`; saúde e ingestão têm populações separadas. |
| Etapas | `queue_wait_seconds`, `password_seconds`, `pool_wait_seconds`, `lock_seconds`, `rule_seconds`, `sql_seconds`, `commit_seconds` e `response_prepare_seconds`. |
| Realtime | `realtime_syncs`, `realtime_sync_seconds`, `realtime_backlog`, `realtime_snapshot_fallbacks`, `socket_writes`, `socket_write_seconds` e `socket_closes`. |
| Entrega ao cliente | `delivery_events` conta eventos por destinatário; `commit_time_observations` informa cobertura; `delivery_round_trip_seconds` e `commit_to_receipt_upper_bound_seconds` separam confirmação de transporte e tempo desde commit. |
| Reconexão no navegador | `reconnect_attempts`, `replay_seconds` e `snapshot_seconds`, desde abertura ou perda da conexão até projeção validada, incluindo backoff e retries. |
| Recuperação humana | `recovery_journeys` e `recovery_human_seconds` incluem início, sucesso e abandono; `recovery_completions` separa credenciais `self_service`, `host_assisted` e origem `unavailable`. |
| Fim de acesso | `access_end_socket_closures`, `access_end_time_observations` e `access_end_to_close_upper_bound_seconds`, por `revocation` ou `expiration`. |
| Lifecycle | `lifecycle_pending`, `lifecycle_completed`, `lifecycle_failed_attempts`, `lifecycle_undetected`, `lifecycle_overdue`, `lifecycle_oldest_pending_seconds`, `lifecycle_orphan_jobs` e `lifecycle_stage_pending`. |
| Saúde do worker | `worker_heartbeat`, `lifecycle_batches`, `lifecycle_batch_seconds` e `lifecycle_measurements`; falhas de banco, ledger, timeout e órfãos permanecem visíveis. |
| Runtime | `server_heartbeat`, `executor_lag_seconds`, `pool_connections`, `pool_idle_connections`, `process_resident_bytes` e `runtime_measurements`. |
| Navegador | `web_lcp_seconds`, `web_inp_seconds`, `web_cls_ratio`, `long_task_seconds` e `touch_feedback_seconds`. |
| Backup e restore | `backup_rpo_seconds`, `backup_window_seconds`, `backup_retention_days`, `backup_manual_snapshots`, `backup_measurements`, `restore_audits` e `restore_audit_seconds`. |

Disponibilidade técnica mensal é `1 - (error + cancelled + rate_limited) / total` sobre as somas de `http_requests` do mês.
API combina `api` e `command`; recuperação e handshake usam denominadores próprios.
Rejeições esperadas do contrato contam como respostas técnicas, mas não como operações de negócio aceitas.
Os alarmes de disponibilidade usam a mesma fórmula em cinco minutos como proteção rápida, não como substituto da avaliação mensal.

Para recuperação sem assistência, use `self_service / (self_service + host_assisted)` sobre `recovery_completions`.
O contador registra somente consumo novo confirmado; retries idempotentes não acrescentam outro sucesso.
Credenciais antigas ou emitidas pela reconciliação de desastre têm origem desconhecida e impedem afirmar cobertura integral dessa proporção.
A credencial sucessora de uma recuperação assistida volta a ser própria.
Esse SLI mede assistência pelo fluxo do produto; suporte externo deve complementar a avaliação do beta.

Nos contadores com resultado `started`, compare sucessos, falhas, timeouts e abandonos contra os inícios, nunca contra a soma que inclui os próprios inícios.
Não declare sucesso integral quando restarem jornadas sem término observado.
A coleta anônima do navegador é de melhor esforço, limitada a oito envios simultâneos, sem retry nem persistência local.
Fechamento abrupto, bloqueio de rede e limitação podem perder amostras.
O transporte omite credenciais e referrer; somente nomes fixos, números e resultados atravessam `/api/telemetry`.
`touch_feedback_seconds` mede a oportunidade de pintura após toque por dois frames, e não comprova que uma mensagem específica foi pintada.

## Commit, correlação e limites da medição

O Compose habilita `track_commit_timestamp` antes da criação das transações.
Em RDS, habilite o parâmetro no procedimento aprovado de configuração e reinício antes de avaliar a entrega.
O PostgreSQL documenta as condições de disponibilidade de [timestamps de commit](https://www.postgresql.org/docs/18/functions-info.html).
Transações anteriores à configuração, timestamps já removidos e consultas acima do orçamento de 50 ms produzem evidência indisponível.

Depois de um lote de eventos ao vivo, o servidor envia um Ping com nonce aleatório e aguarda o Pong correspondente, conforme o [protocolo WebSocket](https://www.rfc-editor.org/rfc/rfc6455#section-5.5.2).
A idade do commit medida pelo banco, somada ao intervalo local até o Pong, é um limite superior conservador da recepção de transporte.
Ela inclui o caminho de retorno e parte da consulta; não mede renderização JavaScript.
Replay histórico tem população própria e não entra na latência de eventos ao vivo.
Há no máximo uma prova pendente por socket; perda e encerramento ficam no denominador.
Após cinco segundos sem resposta, a prova termina com `timeout`, mas seu nonce permanece reconhecível até o Pong ou o fechamento.
Lotes enviados enquanto outra prova aguarda resposta contam como `unavailable`, sem substituir o nonce anterior ou consumir a cota de mensagens não solicitadas do jogador.
O fechamento após fim de acesso aguarda o Close remoto por até cinco segundos e segue a mesma interpretação de limite superior.

`correlation_id` é gerado pelo servidor por requisição.
`command_trace` deriva do identificador de Comando por HMAC com domínio próprio e permite ligar fases, commit, resposta preparada e entrega entre instâncias que compartilham a chave.
O identificador bruto não é exportado e a correlação não acrescenta uma tabela ou dimensão de métrica.
Ambos os campos seguem a retenção de sete dias dos logs.

As proteções P0 de ACK e acesso verificam a evidência observada pelo fluxo HTTP: commit confirmado ou recibo durável, e recusa de acesso conhecida.
Elas contêm uma resposta de sucesso inconsistente com essa evidência.
Não constituem uma auditoria independente de qualquer corrupção arbitrária nas consultas de autorização.
Os testes concorrentes de revogação e expiração continuam sendo proteção complementar.

## Implantação e verificação operacional

`observability.cloudformation.json` define os grupos de logs e alarmes, com `Environment` e o ARN de um tópico SNS existente.
Configure `TELEMETRY_ENVIRONMENT` com o mesmo valor nas tarefas e encaminhe stdout/stderr sanitizados aos grupos correspondentes pelo coletor ECS.
O template não altera serviços, banco, permissões nem o destino de notificação.
Mantenha também os alarmes nativos de targets saudáveis, quantidade desejada de tarefas, CPU, memória, capacidade RDS e disponibilidade externa.
Um heartbeat agregado não detecta sozinho a perda de uma entre duas instâncias.

Execute `node ops/observe-backups.mjs` a cada minuto no ambiente operacional com Node e AWS CLI, usando a identidade da tarefa e `RDS_DB_INSTANCE_ID` configurado.
O probe só chama `DescribeDBInstanceAutomatedBackups` e `DescribeDBSnapshots` e encaminha JSON EMF ao grupo do worker.
As permissões necessárias são somente leitura dessas duas APIs.
A janela vem de [RestoreWindow](https://docs.aws.amazon.com/AmazonRDS/latest/APIReference/API_RestoreWindow.html); snapshots manuais geram alerta para inventário e verificação de TTL.
Esse probe não inventaria outras contas, regiões, AWS Backup ou exportações; inclua esses destinos no inventário de implantação quando existirem.
RTO exige um exercício real completo: registre o intervalo do início do incidente até reconciliação, teste autorizado e prontidão, conforme o runbook.
`restore_audit_seconds` mede somente a auditoria de Tombstones e não deve ser apresentada como RTO.

Antes de declarar os alarmes ativos, valide o template no CloudFormation, confirme a assinatura do tópico, injete cada cenário P0 em staging e confira entrega e recuperação da notificação.
Confirme retenção de sete dias em todos os destinos identificáveis, incluindo proxies, exportações e logs de banco.
O repositório prepara esses artefatos; uma execução local não atesta configuração ou SLO de produção.

## Validação local

`make check` cobre HTTP e WebSocket com PostgreSQL real, canários privados, corrupção persistida, Pongs atrasados, recuperação assistida, lifecycle e os fluxos de navegador.
Os contratos operacionais verificam os cinco alarmes P0, retenção, falta de heartbeat, purge pendente e o adapter do probe RDS.
O smoke dos binários verifica readiness, métricas de runtime, auditoria de um banco isolado e falhas terminais sanitizadas.
O perfil separado de reconexão de 400 canais tem uma pendência de desempenho preexistente documentada em [SECURITY.md](../SECURITY.md#pendência-de-desempenho-observada).
Essa pendência exige uma nova medição com recursos controlados e não foi reavaliada nesta implementação.
