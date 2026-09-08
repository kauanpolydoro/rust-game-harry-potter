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
