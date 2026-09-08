# Resposta operacional

Use somente correlações aleatórias, métricas agregadas e nomes de operações nos registros do incidente.
Não copie URLs, cookies, tokens, senhas, seed, mãos, payloads ou resultados SQL para logs, tickets ou chats.
Consulte dados privados somente no ambiente restrito e dentro da retenção operacional.
Alterações destrutivas, mudanças de credenciais ou permissões e ações em produção exigem confirmação explícita do responsável.

## Banco indisponível

1. Confirme `/health/live`, `/health/startup` e `/health/ready` separadamente.
   Falha de readiness com liveness saudável indica retirar tráfego, sem reiniciar repetidamente o processo.
2. Compare falhas de API, aquisição do pool, fase de lock, SQL e commit.
   Verifique conexões RDS, disponibilidade, failover, armazenamento, CPU e bloqueadores antes de aumentar o pool.
3. Se o pool estiver esgotado, identifique transações longas pelo estado do PostgreSQL sem exportar texto de consultas ou parâmetros.
   Não cancele transações nem aumente limites sem avaliar seu proprietário e o efeito sobre commits em andamento.
4. Restabeleça conectividade e capacidade.
   Verifique readiness, execute uma criação e um Comando com retry da mesma chave, e confirme sincronização em outra instância.
5. Encerre após estabilização das taxas de erro, pool e latência e confirmação de ausência de divergência.

## Commit incerto e confirmação antes do commit

1. Diferencie falha do comando, rejeição de regra e resultado incerto do commit.
   Timeout ou desconexão durante commit não provam rollback.
2. Consulte o recibo pelo mesmo `command_id` usando a Sessão autorizada.
   Repita a intenção original com o mesmo identificador e conteúdo se necessário.
   Nunca crie outro identificador para contornar incerteza.
3. Compare recibo durável, sequência do Evento e Snapshot pelo fluxo autorizado.
   Não publique Evento manualmente nem reconstrua estado por heurística.
4. Uma confirmação sem evidência durável é P0.
   Interrompa novas mutações da versão afetada, preserve somente evidência sanitizada e reverta o binário compatível após aprovação.
5. Valide perda de resposta e reconexão em duas instâncias antes de liberar novamente.
   A recuperação deve devolver o recibo original e uma única aplicação da intenção.

## Divergência e autorização após revogação ou expiração

1. Trate divergência de hash/versão e aceitação após o limite de acesso como P0.
   Uma tentativa corretamente rejeitada é comportamento esperado.
2. Contenha as mutações afetadas e confirme a versão do servidor e das migrations.
   Preserve o relógio PostgreSQL como autoridade.
3. Examine a ordem entre lock, decisão de autorização, expiração, persistência e commit.
   Não use o instante de entrega HTTP para concluir que um comando anterior à revogação foi autorizado depois dela.
4. Corrija com uma reprodução concorrente em PostgreSQL real e valide o corte exato de expiração.
   Nunca altere Snapshot, hash, epoch ou status de Sessão para silenciar o alarme.
5. Libere após revisão da correção, gates locais e verificação de recibos e sincronização.

## Worker atrasado, ledger e órfãos

1. Confirme heartbeat do worker e disponibilidade da medição.
   Ausência de observações não comprova fila vazia.
2. Examine pendências, não detectadas, estágio, tentativas falhas, idade da mais antiga e violações de 24 horas.
   Compare detecção p95 com 300 segundos e purge p95 com 3.600 segundos.
3. Em falha de banco, restaure disponibilidade.
   Em falha de ledger, confirme identidade da tarefa, conectividade, bucket exclusivo, ausência de versionamento e TTL de 14 dias.
4. Em órfãos, corrija a cópia ou o inventário antes de retomar.
   Nunca avance estágio manualmente, remova Tombstones, desabilite triggers ou marque conclusão pela simples ausência da raiz.
5. Execute novamente o worker idempotente e confirme a queda da fila, medição disponível e inventário vazio.
   Registre qualquer extrapolação do limite de 24 horas como violação, mesmo depois da recuperação.

## Segredo detectado ou chave comprometida

1. Trate como P0 uma tentativa de registrar segredo ou conteúdo privado.
   Não reproduza o valor detectado no incidente.
2. Identifique o campo e o caminho de emissão a partir do código e da correlação sanitizada.
   Verifique coletores, proxies, dumps e exportações fora do subscriber.
3. Desabilite o emissor defeituoso e delimite os destinos que receberam o valor.
   Após autorização, elimine as cópias identificáveis e preserve apenas contagens e evidência de saneamento.
4. Rotacione a credencial comprometida pelo procedimento do seu domínio após aprovação.
   A chave de Sessão/recuperação exige avaliação dos acessos e retries pendentes.
   A chave HMAC de Tombstone exige preservar a capacidade de reconciliar toda a janela restaurável e os comprovantes antigos.
5. Valide o bloqueio com canários fictícios, confirme a retenção dos destinos e encerre somente após conter exposição e acessos indevidos.

## Rollback

1. Suspenda o rollout e conserve o digest anterior do binário e do HTML.
   Confirme que esse binário suporta todas as versões recuperáveis de Snapshot, Evento, ruleset, Manifesto e PRNG.
2. Execute os gates de compatibilidade e `make check` no artefato de correção.
   Não faça down migration destrutiva.
3. Após aprovação, reverta o binário compatível, drene conexões e confirme reconexão por replay e Snapshot.
   Reverta o HTML somente na ordem compatível com o backend.
4. Compare erros, latência, confirmações e integridade durante pelo menos 15 minutos de canário.
   Encerre com duas instâncias convergindo e nenhuma condição P0 ativa.

## Restore e ressurreição

1. Registre o início da recuperação e o último instante recuperável confirmado.
   Exija RPO de até 300 segundos, RTO de até 3.600 segundos e janela restaurável máxima de 604.800 segundos.
2. Restaure exclusivamente em ambiente isolado, com credenciais novas e sem readiness ou tráfego de usuários.
   Preserve o ledger externo original e a capacidade de reproduzir seus HMACs.
3. Reconcilie o relógio atual, expirações, revogações, epochs e todos os Tombstones antes de liberar tráfego.
   A presença de uma raiz cujo Tombstone comprova exclusão é P0.
4. Reaplique o ciclo de vida idempotente e verifique banco, Eventos, recibos, Snapshots, Sessões, inventário e qualquer armazenamento privado externo.
   Não apague o Tombstone nem prorrogue expiração para fazer o restore parecer saudável.
5. Verifique versões suportadas, um fluxo HTTP/WebSocket autorizado e a ausência de ressurreição.
   Registre RPO e RTO reais, inclusive quando violados, e obtenha aprovação antes de readiness/tráfego.
6. Backups ou snapshots manuais sem TTL impedem declarar conformidade.
   Falta de evidência sobre ledger ou retenção mantém o ambiente isolado.

Após a reconciliação, execute `lifecycle-worker --audit-restore` com `DATABASE_URL` apontando exclusivamente para o banco isolado e a configuração do ledger original.
Configure também `DEPLOYMENT_EPOCH` e `SESSION_TOKEN_KEY` do destino, conforme o [procedimento de restore](restore.md).
Esse modo não aplica migrations nem inicia o purge: percorre as raízes restauradas, verifica seus HMACs no ledger independente e falha se alguma raiz conflitar com um comprovante.
O adapter S3 precisa de `s3:GetObject` para essa leitura, além das permissões já descritas no runbook do lifecycle.
Saída zero é somente a verificação de Tombstones; não substitui reconciliação de expirações, revogações, versões e cópias externas.
Mantenha o isolamento em erro de leitura, prova inválida ou ausência da chave histórica necessária.
