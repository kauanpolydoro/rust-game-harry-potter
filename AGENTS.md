# Instruções do repositório

## Política de dependências

- Use a versão estável mais recente de toda dependência direta de produção, desenvolvimento, testes, build e automação.
- Consulte o registry ou repositório oficial imediatamente antes de adicionar, atualizar ou declarar uma dependência como atualizada.
- Considere `latest` como a versão estável corrente.
- Use versões prerelease, release candidate, beta ou nightly somente quando Kauan solicitar explicitamente.
- Fixe versões diretas de forma exata quando o ecossistema permitir e mantenha os lockfiles sincronizados.
- Atualize os lockfiles para a resolução transitiva mais recente compatível oferecida pelo gerenciador de pacotes.
- Não adicione dependências diretas, patches ou overrides apenas para forçar uma dependência transitiva além da faixa aceita pelo projeto upstream.
- Nunca mantenha silenciosamente uma versão anterior por compatibilidade.
- Quando a versão estável mais recente for incompatível, interrompa a atualização e apresente a incompatibilidade com evidências e alternativas.
- Diferencie dependências diretas de transitivas ao relatar o resultado da auditoria.
- Antes de concluir uma tarefa que altera dependências, confirme que nenhum pacote direto possui atualização estável pendente em npm, crates.io, imagens de container e GitHub Actions aplicáveis.

## CI temporariamente desativado

- O CI remoto está propositalmente desativado durante esta fase inicial do projeto.
- A definição preservada fica em `.github/workflows-disabled/ci.yml`, fora do diretório reconhecido pelo GitHub Actions.
- Mantenha `.github/workflows` sem workflows executáveis enquanto esta regra estiver vigente.
- Reative o CI somente após uma instrução explícita de Kauan.
- Antes de commits, pull requests e merges, execute a validação local proporcional descrita abaixo.

## Validação proporcional nesta fase inicial

- Priorize entregar incrementos pequenos e funcionais com testes focados no comportamento alterado.
  Esta política prevalece sobre checklists de skills que exijam suítes completas ou matrizes extensas por tarefa.
- Antes de testar, escolha o menor conjunto capaz de detectar uma regressão relevante e estime seu tempo.
  Execute formatação, lint e checagem de tipos aplicáveis, além dos testes do código afetado.
  Mudanças apenas em documentação dispensam testes de aplicação.
- Cubra regras do jogo, combinações, faces de dados e replay com testes determinísticos no domínio.
  Use testes de integração para persistência e contratos, e navegador para o fluxo de interface afetado.
  Cada camada deve verificar um risco distinto, sem repetir a mesma matriz de partidas completas em todas elas.
- Prefira fixtures pequenas que preparem diretamente o estado necessário.
  Use poucas seeds fixas e justificadas; buscas extensas de seeds, simulações de equilíbrio e matrizes de vitória/derrota por número de jogadores exigem solicitação explícita.
- O orçamento padrão de validação por tarefa é de dez minutos de tempo decorrido, incluindo preparação e reexecuções.
  Se a estimativa exceder esse orçamento, reduza o escopo antes de começar.
  Ao atingir o limite, interrompa a rodada ampla, informe resultados e pendências e proponha o próximo teste focado.
  Rodadas acima do orçamento exigem solicitação explícita de Kauan.
- `make check` permanece disponível como suíte completa, mas sua execução exige solicitação explícita nesta fase.
  Não o execute automaticamente antes de cada commit, PR ou merge.
- Quando os testes selecionados passarem, encerre a validação.
  Após uma falha ou correção, reexecute somente o caso afetado e regressões diretamente relacionadas.
  Amplie a execução apenas quando houver evidência de impacto fora desse escopo.
- Preserve testes existentes e registre o que foi executado, o resultado e o que ficou fora do escopo.
  Diferencie falhas do produto de falhas do ambiente; uma rodada interrompida ou com falhas nunca deve ser descrita como aprovada.

## Uso de disco em builds e validações

- Antes e depois de compilar ou executar `make check`, meça a ocupação do filesystem com `df -h` e os artefatos com `du -sh` nos diretórios efetivos de build.
  Considere todas as cópias do projeto, overrides de `CARGO_TARGET_DIR` e `build-dir`, e volumes de desenvolvimento quando usados.
  Durante comandos longos, confira o espaço livre ao menos a cada minuto.
- Na VPS compartilhada, inicie novas compilações somente com ocupação abaixo de 75% e pelo menos 30 GiB livres no filesystem dos artefatos.
  Se algum limite for atingido durante a execução, interrompa os builds desta sessão, recupere espaço com a limpeza segura abaixo e só então retome a validação autorizada.
- Nas validações automatizadas, use `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only` e `CARGO_PROFILE_TEST_DEBUG=line-tables-only` no ambiente dos comandos, incluindo `make check`.
  Habilite debug completo ou compilação incremental apenas para um diagnóstico que precise desses recursos, registre o motivo e limpe os artefatos adicionais ao terminar.
- Reutilize o diretório de build da mesma cópia e configuração durante a tarefa, sem criar um novo cache por tentativa nem copiar `target` ao criar checkouts ou worktrees.
  Mantenha diretórios de build separados entre cópias usadas simultaneamente por sessões diferentes.
- Aplique a política de validação proporcional acima também aos builds e às reexecuções.
- Mantenha no máximo 10 GiB de artefatos de build por cópia entre rodadas de validação.
  Se ultrapassar esse orçamento, limpe antes da próxima rodada; se uma compilação limpa já o exceder, informe o consumo e proponha um ajuste antes de continuar acumulando artefatos.
- Ao concluir a tarefa, remova os artefatos descartáveis exclusivos da sessão com `cargo clean` no checkout correto, após confirmar os diretórios que serão removidos e a ausência de builds, testes ou servidores usando esses arquivos.
  Retenha artefatos somente para um processo ativo ou próximo passo definido, dentro do orçamento, e registre o motivo na entrega.
- Coordene explicitamente qualquer limpeza ou interrupção que afete outra sessão.
  Restrinja a limpeza a artefatos de build identificados; preserve código, históricos dos agentes, bancos, backups e volumes com dados, e evite comandos globais de remoção ou prune.
- Na entrega, informe a validação executada, o espaço liberado ou retido e a ocupação final do disco.
  A limpeza de artefatos após um gate aprovado não exige recompilá-los apenas para validar a própria limpeza.
