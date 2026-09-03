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
- Continue executando `make check` localmente antes de commits, pull requests e merges.
- Não interprete a ausência de checks remotos como permissão para ignorar os gates locais.
