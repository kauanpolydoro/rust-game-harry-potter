# Cenários do Jogo 4

Os arquivos `{participantes}-{resultado}.json` registram comandos executados pela interface real em `apps/web/e2e/adventures.spec.ts`.
Cada cenário percorre uma Partida completa por HTTP e WebSocket com PostgreSQL real e verifica a convergência dos participantes.
Os campos `event_digest`, `snapshot_digest`, `opening_digest` e `checkpoints` são calculados pelo teste Rust de replay a partir dessas transcrições.
O replay verifica cada comando em memória e após restauração, além da aplicação do evento persistido.

As derrotas usam seed 7.
As vitórias usam as seeds de `scenario-seeds.json`: 7, 23 e 204 para 2, 3 e 4 participantes, respectivamente.
As prioridades de compra em `purchase-priority.json` são compartilhadas entre os cenários Rust, HTTP e navegador.
Esses exemplos determinísticos protegem a regressão e não estimam dificuldade ou taxa de vitória.

Para renovar uma transcrição, execute o cenário correspondente no Playwright, preserve os comandos capturados no artefato `game-four-commands.json` como objetos `{ "request": comando }` e revise a mudança funcional antes de recalcular os digests:

```sh
UPDATE_GAME_FOUR_GOLDENS=1 cargo test -p harry-potter-server --lib game_four_browser_transcripts
cargo test -p harry-potter-server --lib game_four_browser_transcripts
```

Não altere os goldens dos Jogos anteriores ao atualizar o Jogo 4.
