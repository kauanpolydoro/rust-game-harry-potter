# Recuperação de desastre sem ressuscitar Partidas

A issue [#33](https://github.com/kauanpolydoro/rust-game-harry-potter/issues/33) acrescenta a reconciliação de restore ao módulo `lifecycle`.
O destino permanece isolado até consultar o ledger externo atual, expirar o acesso vencido e substituir as credenciais recuperadas do backup.
O relógio de autoridade continua sendo `clock_timestamp()` do PostgreSQL.

## Preparar a origem

Servidor e worker exigem o mesmo `DEPLOYMENT_EPOCH` UUID e a mesma `SESSION_TOKEN_KEY` de exatamente 32 bytes.
Mantenha esses valores estáveis entre reinícios normais e fora do backup do banco.
O worker também exige a `TOMBSTONE_HMAC_KEY` estável de 32 bytes e exatamente uma opção de armazenamento: `TOMBSTONE_BUCKET` em produção ou `TOMBSTONE_LOCAL_DIRECTORY` em desenvolvimento.
A identidade AWS precisa ler e gravar os comprovantes e conferir a retenção descrita em [lifecycle.md](lifecycle.md).

A migration 0024 registra o deployment, seu login PostgreSQL e fingerprints das chaves, sem guardar os segredos.
Um banco novo e vazio pode ser vinculado no primeiro startup.
Um banco existente anterior à versão 24 precisa ser adotado explicitamente na origem, antes de produzir backups utilizáveis pelo novo procedimento.
Com os processos antigos parados e as variáveis da origem disponíveis, execute uma vez:

```bash
cargo run --locked -p harry-potter-server --bin source-bootstrap
```

Esse comando preserva as Partidas, as Sessões, as senhas e as gerações de recuperação existentes.
Ele também publica uma prova opaca do deployment no ledger externo.
Nunca use `source-bootstrap` para aprovar uma cópia restaurada: um backup anterior à adoção não tem a evidência necessária e é recusado pelo reconciliador.

Para atualizar um volume de desenvolvimento existente, com `./scripts/dev` já encerrado:

```bash
docker compose up -d --wait postgres
docker compose --profile dev run --rm --no-deps lifecycle-worker cargo run --locked -p harry-potter-server --bin source-bootstrap
./scripts/dev
```

Depois da adoção, mantenha o worker ativo.
Cada lote atualiza um checkpoint do relógio do banco e conserva a prova externa do deployment.
Monitore a ausência do heartbeat: o ponto solicitado no restore deve ficar entre o checkpoint recuperado e cinco minutos depois dele.
Um timestamp recente fornecido pelo operador não torna recuperável um backup cujo checkpoint já saiu da janela de sete dias.

## Política de backups e inventário

Os limites estão em [backup-policy.json](backup-policy.json): janela restaurável de no máximo sete dias corridos, RPO de até 300 segundos e RTO de até 3.600 segundos.
Configure PITR do RDS PostgreSQL com `BackupRetentionPeriod=7`, remoção dos backups automatizados ao excluir a instância e sem snapshot final.
Snapshots manuais ficam proibidos enquanto não houver um controlador de exclusão verificado; uma tag com data de expiração não apaga um snapshot.
Não mantenha cópias em outra conta ou região, exports, vaults ou repositórios fora do inventário.
Pontos do AWS Backup precisam de lifecycle de exclusão de até sete dias e data calculada de exclusão dentro desse limite.

Não pare uma instância com backups preservados para prolongar sua vida: o RDS desconsidera o tempo parado no cálculo da retenção automatizada.
Verifique também os backups retidos de instâncias excluídas e o instante original dos dados em snapshots copiados.
Essas particularidades estão documentadas em [retenção RDS](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/USER_WorkingWithAutomatedBackups.BackupRetention.html), [backups retidos](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/USER_WorkingWithAutomatedBackups.Retaining.html) e [metadados de snapshots](https://docs.aws.amazon.com/cli/latest/reference/rds/describe-db-snapshots.html).
Snapshots automatizados também permitem restauração direta e precisam respeitar sete dias de idade, independentemente da janela anunciada de PITR, conforme a [documentação de backups automatizados](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/USER_WorkingWithAutomatedBackups.Enabling.html).

Com AWS CLI e uma identidade de leitura configuradas, audite a origem:

```bash
node scripts/audit-backup-policy.mjs --rds-instance "$RDS_INSTANCE_IDENTIFIER"
```

A coleta consulta a instância, seus snapshots, seus backups automatizados e seus pontos em todos os vaults da conta e região configuradas.
Falhas de coleta, inventário incompleto, origem parada, janela longa ou atraso de PITR superior a cinco minutos produzem falha.
O modo `--inventory CAMINHO` aceita o mesmo formato JSON das respostas AWS, com `observed_at` de no máximo 60 segundos atrás e os arrays obrigatórios `DBInstances`, `DBSnapshots`, `DBInstanceAutomatedBackups` e `RecoveryPoints`.
Trate esse inventário bruto como informação restrita e temporária; o relatório de saída contém somente os achados gerais.

O resultado `source_inventory_compliant` cobre exclusivamente essa origem na conta e região selecionadas.
`external_copies_verified` permanece falso: o comando não prova a ausência de exports nem de cópias em outros ambientes.
Antes de produção, o inventário de infraestrutura e as permissões de cópia precisam demonstrar essa ausência em todo o escopo autorizado.
O repositório ainda não contém provisionamento AWS; o arquivo de política e o auditor não alteram retenção nem implantam controles de infraestrutura.

## Restaurar e reconciliar

1. Registre o início da indisponibilidade e selecione um ponto PITR dentro dos últimos sete dias.
   Preserve a confirmação do provedor sobre o ponto realmente recuperado e a última Ação oficial incluída.
   Pare a origem e seus workers, encerre operações em voo e retire o tráfego antes de promover um destino.
   Mantenha o destino em rede isolada, sem rota de clientes ou workers de origem, durante todo o procedimento.

2. Restaure o backup e o WAL até o ponto escolhido.
   Preserve o bucket externo de Tombstones em seu estado atual; ele não participa do restore do PostgreSQL.
   Sincronize os relógios do host e do banco.
   O reconciliador recusa desvio superior a cinco segundos e timestamps de Ações ou Eventos no futuro.

3. Provisione um novo login PostgreSQL para o destino, desabilite o login antigo nesse destino e encerre suas conexões.
   Gere um novo `DEPLOYMENT_EPOCH` e uma nova `SESSION_TOKEN_KEY` fora do backup.
   Preserve a `TOMBSTONE_HMAC_KEY` e o ledger da origem.
   A troca somente da senha do mesmo login PostgreSQL é insuficiente: o reconciliador compara `session_user` e exige outro login.
   A configuração de permissões e credenciais de infraestrutura é responsabilidade do operador do ambiente.

4. Configure as variáveis abaixo no ambiente privado do operador e execute o reconciliador com o servidor e o worker do destino parados.

| Variável | Conteúdo |
| --- | --- |
| `DATABASE_URL` | URL do destino com o novo login. |
| `DEPLOYMENT_EPOCH` | Novo UUID, depois compartilhado pelo servidor e worker do destino. |
| `SESSION_TOKEN_KEY` | Nova chave de 32 bytes, depois compartilhada pelo servidor e worker. |
| `TOMBSTONE_HMAC_KEY` | Mesma chave estável da origem. |
| `TOMBSTONE_BUCKET` | Bucket externo original com leitura, gravação e retenção válidas. |
| `RESTORE_POINT_MS` | Instante efetivamente recuperado em milissegundos Unix. |
| `RESTORE_ACCESS_FILE` | Caminho novo em diretório privado fora do repositório, dos backups e dos logs. |

```bash
cargo run --locked -p harry-potter-server --bin restore-reconcile
```

O comando tem timeout de uma hora e nunca imprime credenciais ou identificadores de Partidas.
Ele exige um único coordenador, verifica o inventário operacional e fecha o gate persistente antes da consulta ao ledger.
Ausência da prova de deployment, chave incorreta, erro de leitura ou comprovante inválido mantém o destino indisponível.
Um Tombstone externo prevalece mesmo que o backup contenha uma Partida com prazo futuro.
O procedimento invalida todas as Sessões e credenciais anteriores, inclusive recibos de recuperação consumidos, incrementa as gerações e emite material novo somente para Participações ainda válidas.
Expirações são reavaliadas depois das operações externas, antes de liberar readiness.
Snapshots, regras, seed, histórico, recibos de Comando e prazo de retenção das Partidas válidas são preservados.

O arquivo privado usa criação exclusiva, permissão `0600` e `fsync` do arquivo e do diretório.
Cada entrada contém `room_code`, `participant_id`, `position`, `recovery_password` e `recovery_token`.
Se o processo perder a confirmação ou falhar ao persistir o arquivo depois do commit, repita com o mesmo ponto, epoch e chave, mas outro caminho novo.
O retry devolve somente credenciais ainda ativas e não incrementa novamente as gerações.
Não crie outro epoch para esse retry, pois isso representaria outro restore.

5. Inicie servidor e worker com a configuração nova ainda dentro do isolamento.
   Exija `200` de `/health/ready`; `/api/*` também consulta o gate persistente e retorna `503` enquanto o deployment não estiver aprovado.
   Comprove que cookies, tokens e senhas antigos falham, que Partidas excluídas ou expiradas não projetam dados e que o purge converge sem pendências ou órfãos.
   Entregue individualmente o material novo após verificar o destinatário por um canal independente dos acessos restaurados.
   A verificação humana é necessária porque o produto não tem identidade de conta que permita reconhecer automaticamente o titular depois da perda dos acessos.
   Não exponha a lista completa de Participantes a um destinatário nem envie credenciais por logs ou canais públicos.
   Confirme que a recuperação abre a Participação original e aceita uma Ação oficial na Partida válida.

6. Só então encaminhe o tráfego ao destino, registre o RPO/RTO observado e remova os artefatos privados temporários após a entrega.
   Inclua no RTO a provisão, a restauração, a reconciliação, a disponibilização do acesso e a retomada da Ação oficial.
   Se uma etapa falhar, mantenha o isolamento e corrija a causa; não edite o gate, os checkpoints ou os Tombstones para forçar readiness.

## Exercício reproduzível

```bash
make check-restore-profile
```

O perfil cria três Partidas por HTTP, faz `pg_basebackup`, arquiva WAL e grava um ponto de recuperação nomeado.
Ações oficiais posteriores ao backup base precisam vir do WAL; uma Ação posterior ao ponto escolhido precisa ficar de fora.
Uma Partida é excluída na origem depois desse ponto e outra vence durante o desastre.
O PostgreSQL é restaurado em novo volume e rede Docker interna, com sockets Unix privados para o operador e sem portas PostgreSQL publicadas.
O ensaio verifica `503` antes da reconciliação, troca de login e chaves, rejeição dos acessos antigos, purge das duas Partidas inválidas e retomada da terceira com credenciais novas.
Dados, volumes, rede e credenciais temporários são removidos antes da publicação do relatório sanitizado em `.scratch/restore-profile.json`.

A evidência versionada em [restore-exercise.json](restore-exercise.json) registra uma execução local com RPO e RTO abaixo de cinco e 60 minutos, respectivamente.
RPO mede a diferença entre o início do desastre e a última Ação oficial recuperada; RTO usa relógio monotônico até uma nova Ação oficial aceita na Partida sobrevivente.
O mecanismo de backup físico e replay segue a [documentação PostgreSQL de recuperação contínua](https://www.postgresql.org/docs/current/continuous-archiving.html).
Os testes PostgreSQL adicionais cobrem cancelamento do coordenador, retry, expiração durante a reconciliação, falha de ledger, reuso de credenciais, upgrade da origem e backup antigo com timestamp declarado recente.

Esse ensaio mede o ambiente Docker local e não comprova a latência do RDS, a entrega humana ou o inventário de uma conta AWS.
Antes do deploy, repita o exercício no ambiente AWS escolhido com seu volume real de dados, o auditor de backups, o bucket real e o procedimento de entrega de acessos.
Registre novos exercícios após mudanças de infraestrutura e periodicamente na operação.
