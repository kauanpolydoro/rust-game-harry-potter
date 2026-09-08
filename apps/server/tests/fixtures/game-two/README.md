# Goldens do Jogo 2

Os seis cenários registram comandos enviados pela interface real, com 2, 3 e 4 participantes, até vitória ou derrota.
A captura vem de `apps/web/e2e/adventures.spec.ts`, executado contra o servidor Rust, WebSocket e PostgreSQL.
O harness usa a seed de teste composta por 32 bytes de valor 7, por meio da configuração de construção da aplicação.
A aplicação de produção continua obtendo a seed de um CSPRNG do sistema operacional.

Cada arquivo fixa os comandos, o digest do Snapshot inicial, o digest dos bytes de cada evento persistido e o digest de cada Snapshot subsequente.
Os checkpoints nomeados registram preparação, fases, atordoamento, avanço de Local e encerramento quando esses estados aparecem no cenário.
A preparação inclui a ordem inicial das pilhas e todos os resultados de amostragem com seus limites e proprietários.
As partidas de derrota obrigatoriamente cobrem atordoamento e avanço de Local.

O teste `game_two_browser_transcripts_replay_exact_events_and_snapshot_goldens` usa o manifesto publicado e os codecs de produção.
Ele executa cada comando a partir do estado contínuo e de uma restauração do Snapshot anterior, compara os bytes completos dos eventos e dos estados canônicos e também aplica o evento pelo redutor do domínio.
O agrupamento interno de cartas de participantes diferentes pode variar durante a restauração; a ordem dentro de cada pilha de cada participante e seus bytes canônicos precisam permanecer idênticos.
UUIDs de participantes são fixos somente neste teste, e os IDs idempotentes dos comandos são substituídos por um valor de teste porque não participam das decisões do domínio.

Para conferir os goldens:

```sh
cargo test -p harry-potter-server --lib game_two_browser_transcripts
```

Uma mudança intencional de regras ou de codec exige revisar os comandos e os fatos antes de atualizar os resultados esperados.
A atualização é explícita:

```sh
UPDATE_GAME_TWO_GOLDENS=1 cargo test -p harry-potter-server --lib game_two_browser_transcripts
```

Depois da atualização, revise o diff e execute o teste novamente sem a variável, seguido de `make check`.
Não atualize os goldens automaticamente para contornar uma regressão.
Os testes de navegador exportam `game-two-commands.json` como artefato para comparar ou substituir uma transcrição de entrada quando o cenário mudar intencionalmente.

O arquivo `purchase-priority.json` fixa as prioridades de aquisição dos roteiros de vitória para cada quantidade de participantes.
Os testes HTTP e de navegador leem essa mesma configuração.
Esses roteiros servem como cenários reproduzíveis e não como recomendação de estratégia ótima.
