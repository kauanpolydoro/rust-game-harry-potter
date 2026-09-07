# Purge operacional e Tombstones

A issue #32 acrescenta `lifecycle` ao monólito e o executável `lifecycle-worker`.
`make dev` inicia o servidor, o cliente e o worker.
O CI remoto continua desativado.

## Execução e armazenamento externo

Em produção, configure `DATABASE_URL`, `TOMBSTONE_HMAC_KEY` com 32 bytes estáveis e `TOMBSTONE_BUCKET`.
O SDK usa a cadeia padrão de credenciais AWS, incluindo a identidade da tarefa ECS.
A chave HMAC deve ser um segredo independente das chaves de Sessão e recuperação, preservado durante toda a janela de restauração e de Tombstones.
Não troque essa chave enquanto houver Partidas ou comprovantes recuperáveis sem um procedimento de reconciliação.

Execute:

```bash
cargo run -p harry-potter-server --bin lifecycle-worker
```

O bucket deve ser exclusivo para os comprovantes, separado dos dados operacionais e do procedimento de restore do PostgreSQL.
Ele não pode ter versionamento e deve ter a regra de expiração de 14 dias de [tombstone-lifecycle.json](tombstone-lifecycle.json).
O worker verifica essa configuração antes de processar a fila e falha se não puder confirmá-la.
A role precisa de `s3:PutObject`, `s3:GetLifecycleConfiguration` e `s3:GetBucketVersioning` nesse bucket.
A configuração do bucket e o deploy não são feitos por este executável.
O TTL do S3 torna os objetos elegíveis para remoção após 14 dias; a remoção física do serviço é assíncrona.

Em desenvolvimento, `TOMBSTONE_LOCAL_DIRECTORY` substitui `TOMBSTONE_BUCKET`.
Configure exatamente uma dessas duas opções.
`make dev` usa por padrão `~/.local/share/batalha-de-hogwarts/tombstones`, separado do volume PostgreSQL.
O adapter local usa publicação atômica, `fsync` do arquivo e do diretório e valida o conteúdo em retries.
Ele mantém comprovantes incompletos e remove os pares concluídos após a janela de 14 dias durante a manutenção do worker.
Esse adapter é destinado a desenvolvimento e testes, não ao deploy AWS.

## Etapas e recuperação de falhas

O relógio de autoridade é sempre o PostgreSQL.
A seleção de até 100 vencidas usa `FOR UPDATE SKIP LOCKED` e chama o mesmo gate de expiração da API.
Uma renovação que tenha vencido a disputa pelo lock é reavaliada antes de criar o trabalho.
A fila registra a expiração original e o instante durável de detecção, inclusive quando a API detectou primeiro.

Cada lote executa até 100 etapas e deixa de iniciar novas etapas quando atinge cinco segundos.
O lock da linha de trabalho permanece durante toda a etapa, inclusive durante a gravação externa.
Não existe lease que possa vencer e permitir um worker antigo concluir trabalho de um sucessor.
Falha ou cancelamento reverte a etapa; etapas anteriores permanecem confirmadas.
Falhas observadas recebem retry em 30 segundos, permitindo que a fila continue com outras Partidas.
Cada etapa tem orçamento de aplicação de cinco segundos; o PostgreSQL limita cada instrução a quatro segundos, inclusive durante cancelamento e rollback.
O SDK S3 tem timeout de três segundos no executável.
Migrations usam um pool separado, sem esses limites de consulta.

| Etapa | Efeito durável |
| --- | --- |
| `tombstone` | Grava a prova externa antes de permitir qualquer remoção da raiz. |
| `detach` | Captura todos os IDs operacionais necessários à exclusão e verificação, incluindo gerações antigas de Sessão. |
| `purge` | Reconfirma o Tombstone e remove histórico, recibos, âncoras, Snapshot, acessos, Participantes e Sala numa transação. |
| `verify` | Confere o inventário e procura os IDs em todas as colunas UUID das tabelas operacionais, inclusive fora das relações esperadas. |
| `complete` | Repete a verificação, grava a prova de verificação externa, registra durações anônimas e remove a própria fila com seus IDs. |

As proteções contra mutação do histórico e contra alteração de Participantes de Sala selada continuam ativas durante o jogo.
A migration 0022 permite exclusivamente a remoção de dados de uma Partida vencida no estágio de purge.
Não há desativação global de triggers, alteração de `session_replication_role` nem `ON DELETE CASCADE` genérico novo.
Locks de raiz seguem a ordem Sala, Partida e Participantes das operações de identidade.

Um Tombstone usa HMAC-SHA-256 com separação de domínio sobre o UUID da Partida.
O UUID, o código da Sala, os nomes, os payloads e as credenciais nunca vão para o ledger.
O corpo guarda somente versão e instantes de expiração, detecção e verificação.
O campo `completed_at_ms` é o instante da verificação durável; a confirmação externa e a remoção final da fila podem ocorrer depois em caso de retry.
A gravação S3 usa criptografia SSE-S3 e renova a retenção antes da remoção da raiz.
Perda de resposta após uma gravação durável é segura: o próximo worker publica a mesma prova.
O ledger fornece a informação necessária à futura reconciliação de restore, que está fora desta issue.

## Inventário de cópias

O inventário fica no adapter privado `apps/server/src/lifecycle/postgres.rs`.
Ele cobre Sala, Participantes, identidades, todas as Sessões, credenciais consumidas ou substituídas, recibos de criação/entrada/início/Comando/recuperação administrativa/proteção, Eventos de jogo e segurança, destinatários, âncoras e leases de presença.
O Snapshot e a seed residem na raiz `games` e somem com ela.
Identidades ainda usadas em outra Participação são preservadas; referências incompatíveis impedem a exclusão em vez de apagar dados da outra Partida.

O produto atual não grava blobs privados nem possui um repositório externo de telemetria identificável.
Os JSONs de conteúdo e as imagens do shell são públicos e compartilhados; não são cópias de uma Partida.
Novas tabelas, tabelas estrangeiras ou views materializadas não cadastradas bloqueiam o purge e a conclusão.
Ao introduzir outro armazenamento privado, acrescente sua exclusão e sua verificação no ciclo de vida antes de habilitar escritas.
O verificador não considera a ausência da raiz, sozinha, uma prova de exclusão.

As mensagens de aplicação usam operações e rotas registradas, sem IDs de domínio, URLs concretas, payloads, erros SQL completos ou frames de fechamento enviados pelo cliente.
Os executáveis limitam os targets de tracing para impedir logs de protocolo SQL/AWS.
O Compose desliga statements e parâmetros nos logs e usa verbosidade `terse` para não registrar o conteúdo de linhas em erros de constraint.
Em RDS, aplique os mesmos parâmetros no parameter group: `log_error_verbosity=terse`, `log_min_error_statement=panic`, `log_statement=none`, `log_parameter_max_length=0`, `log_parameter_max_length_on_error=0`.
Coletores HTTP, proxies e tracing da infraestrutura também precisam excluir corpos, cookies e caminhos concretos.
Se houver um coletor legado identificável, ele precisa ser inventariado e limpo antes de declarar o ambiente compatível com este ciclo de vida.

## Métricas e resposta operacional

O worker emite JSON `lifecycle metrics` a cada lote, sem labels por Partida.
A janela de observações concluídas é de 14 dias, com limpeza automática.
`pending`, `stages`, `failed_attempts`, `orphan_jobs`, `undetected`, `oldest_pending_seconds` e `overdue` mostram trabalho incompleto.
Partidas expiradas pela API mas ainda sem trabalho na fila continuam visíveis.

`detection_p95_seconds` inclui observações concluídas, fila e limites inferiores das expirações ainda não detectadas.
O alvo é até 300 segundos.
`purge_p95_seconds` inclui limites inferiores das pendências e tem alvo de 3.600 segundos, e `purge_max_seconds` não pode ultrapassar 86.400 segundos desde a expiração original.
`oldest_pending_seconds` e `overdue` impedem que uma fila parada pareça saudável por conter apenas amostras antigas de sucesso.
A violação desses limites gera `lifecycle SLO breached`; falha de medição gera `lifecycle measurement unavailable`.
Configure alertas de infraestrutura também para ausência do heartbeat de métricas, pois um processo desligado não consegue emitir seu próprio alerta.

Em falha de ledger, restaure primeiro o acesso ao bucket e sua retenção; não avance o estágio manualmente.
Em falha de banco ou lock, restaure a disponibilidade e deixe o retry continuar.
Em órfãos, examine o inventário no ambiente restrito, corrija o adapter ou a cópia indevida e só então retome a fila.
Não marque o trabalho como concluído nem remova Tombstones para contornar o verificador.
Nenhum SLO pode garantir remoção durante uma indisponibilidade externa superior ao prazo; o worker mantém a evidência de atraso e alerta em vez de declarar sucesso.

## Validação

`make check` executa os testes PostgreSQL/HTTP/WebSocket, incluindo purge integral, concorrência, perda de ACK, cancelamento de processo, armazenamento esquecido e órfão após remoção da raiz.
Os testes de ledger exercitam persistência local e o adapter S3 contra um endpoint HTTP controlado, com assinatura, criptografia, retenção e falhas de armazenamento.
Esse teste de contrato não substitui um smoke test no bucket AWS antes do deploy.

```bash
make check-lifecycle-profile
```

O perfil cria 100 Partidas por HTTP com eventos oficiais persistidos, expira todas e usa dois workers concorrentes.
Ele publica os percentis medidos e exige detecção p95 de até cinco minutos, purge p95 de até uma hora e máximo de 24 horas.
Na execução local de 07/09/2026 com PostgreSQL 18.6, as 100 Partidas concluíram sem falhas ou órfãos: detecção p95 de 1,332 s, purge p95 de 18,454 s e máximo de 19,264 s.
O perfil mede o ambiente local; não constitui uma medição de produção.
Backups, WAL/MVCC e reconciliação antes de readiness após restore continuam sob a política e os tickets próprios de recuperação de desastre.
