# Localização editorial do Jogo 1

`game-one.pt-BR.v1.json` associa cada identidade do bundle jogável a seu nome original e ao nome editorial em português brasileiro.
As 45 entradas preservam a identidade funcional e o texto original do bundle.
O servidor usa a localização somente quando ID e nome original coincidem, evitando aplicar traduções do jogo a fixtures com identidades reutilizadas.
Nomes localizados presentes no próprio manifesto mantêm precedência.

As traduções foram extraídas do inventário nominal revisado `base-en-candidate-2026-09-02.json` e de [localization-pt-BR.md](../localization-pt-BR.md).
Isso reutiliza decisões editoriais e não atribui autoridade funcional ao inventário candidato.
Encantamentos, nomes próprios e modelos de vassoura sem tradução registrada conservam o nome original.
“Catálogo do Jogo 1” e “Regras do Jogo 1” são rótulos editoriais da adaptação.
Descrições de efeitos e opções continuam derivadas pelo servidor do AST validado.

O arquivo é independente dos bundles imutáveis e de seus digests.
Uma revisão editorial pode criar uma nova versão deste arquivo sem alterar custos, efeitos, recompensas, identidades ou regras de partidas em andamento.
