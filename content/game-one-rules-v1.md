# Jogo 1: regras funcionais v1

Este documento define a interpretação funcional versionada `game-one-v1` da primeira Aventura.
As descrições abaixo são regras da adaptação, não uma localização editorial oficial das cartas.
Os nomes do catálogo permanecem em inglês.
Alterações de comportamento exigem nova versão de conteúdo e ruleset.

## Inventário e proveniência

O manifesto específico tem 45 registros e 93 cartas físicas: 30 de Hogwarts, 40 iniciais, 10 de Artes das Trevas, três Vilões, dois Locais, quatro Heróis e quatro referências de Turno.
Catálogo, Ruleset e Aventura são registros lógicos de quantidade zero.
O [manual do Jogo 1, páginas 4 e 5](https://www.boardgamehelpers.com/HogwartsBattle/Images/Harry-Potter-DB-G1-Rules.pdf) fundamenta as quantidades e a preparação.
O [inventário comunitário fixado por commit](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/config/game_one.yaml) identifica as 30 cartas de Hogwarts.

O candidato integral continua preservado com 171 registros e 252 cartas declaradas.
Ele contém uma divergência: `hogwarts-card:012`, “Sunshine, Daisy, Butter Mellow, Turn This Stupid, Fat Rat Yellow!”, acrescentaria uma 31ª carta ao Jogo 1.
Essa entrada vem da [implementação de eccabay](https://github.com/eccabay/hp-text-game/blob/b32d78bbccf3f204723568a0f7ff08980f4d3bbb/cards/hogwarts/hogwarts_deck.py), está ausente do inventário de 30 cartas e fica excluída do manifesto jogável.
Nenhuma regra funcional dessa entrada é inferida ou publicada.

As fontes comunitárias são evidências candidatas.
Sua semântica é adotada explicitamente pelas regras da adaptação neste documento, sem promovê-las a fontes oficiais.
A confiança é concedida pelo servidor ao identificador, URI e tipo da fonte; a declaração do bundle sozinha não concede confiança.

## Preparação

Cada Herói recebe sete Alohomora e suas três cartas exclusivas.
Cartas exclusivas de Heróis não selecionados permanecem fora da Partida.
Cada baralho inicial é embaralhado e fornece cinco cartas à mão.
Hogwarts, Vilões e Artes das Trevas são embaralhados separadamente.
O mercado recebe seis cartas e existe um Vilão ativo.
Diagon Alley precede Mirror of Erised; os Locais não são embaralhados.
Todos começam com dez de Vida e zero de Ataque e Influência.
O Anfitrião ocupa a primeira posição; os Turnos seguem a ordem das posições.
Os quatro Heróis do Jogo 1 não possuem habilidade especial.
Uma declaração tipada de ausência de habilidade não pode substituir o efeito de uma carta.

## Hogwarts

Cada ID conserva a identidade do catálogo candidato.
As semânticas são adotadas dos [módulos de Hogwarts fixados por commit](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/hogwarts).

| ID | Carta | Quantidade | Custo | Regra funcional |
| --- | --- | ---: | ---: | --- |
| 001 | Albus Dumbledore | 1 | 8 | Cada Herói recebe um Ataque, uma Influência, uma Vida e compra uma carta. |
| 002 | Descendo | 2 | 5 | Receber dois Ataques. |
| 003 | Essence of Dittany | 4 | 2 | Escolher um Herói para receber duas Vidas. |
| 004 | Golden Snitch | 1 | 5 | Receber duas Influências e comprar uma carta. |
| 005 | Incendio | 4 | 4 | Receber um Ataque e comprar uma carta. |
| 006 | Lumos | 2 | 4 | Cada Herói compra uma carta. |
| 007 | Oliver Wood | 1 | 3 | Receber um Ataque; ao derrotar um Vilão após jogar esta carta, escolher um Herói para receber duas Vidas. |
| 008 | Quidditch Gear | 4 | 3 | Receber um Ataque e uma Vida. |
| 009 | Reparo | 6 | 3 | Escolher entre receber duas Influências e comprar uma carta. |
| 010 | Rubeus Hagrid | 1 | 4 | Receber um Ataque; cada Herói recebe uma Vida. |
| 011 | Sorting Hat | 1 | 4 | Receber duas Influências; neste Turno, Aliados adquiridos podem ir ao topo do próprio baralho. |
| 013 | Wingardium Leviosa | 3 | 2 | Receber uma Influência; neste Turno, Itens adquiridos podem ir ao topo do próprio baralho. |

Albus Dumbledore, Oliver Wood e Rubeus Hagrid são Aliados.
Essence of Dittany, Golden Snitch, Quidditch Gear e Sorting Hat são Itens.
As demais cartas de Hogwarts são Feitiços.
Aquisições comuns vão para o descarte do comprador.
A opção de colocar no topo exige uma decisão explícita do comprador.
A reposição do mercado ocorre na mesma ação de aquisição, conforme o fluxo já adotado pelo produto.

## Baralhos iniciais

As semânticas são adotadas dos [módulos dos Heróis fixados por commit](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/heroes).
As quatro cartas de companheiros são Aliados; Alohomora é Feitiço; as demais são Itens.

| ID | Dono | Carta | Regra funcional |
| --- | --- | --- | --- |
| starter:001 | Cada Herói | Alohomora | Receber uma Influência. |
| starter:002 | Harry | Invisibility Cloak | Receber uma Influência ao jogar; enquanto permanece na mão, limitar a uma Vida cada perda causada por Artes das Trevas ou Vilão. |
| starter:003 | Harry | Firebolt | Receber um Ataque; cada Vilão derrotado após jogar esta carta concede uma Influência adicional. |
| starter:004 | Harry | Hedwig | Escolher entre um Ataque e duas Vidas. |
| starter:005 | Ron | Every Flavour Beans | Receber uma Influência e um Ataque por Aliado jogado neste Turno, incluindo os já jogados. |
| starter:006 | Ron | Cleansweep 11 | Receber um Ataque; cada Vilão derrotado após jogar esta carta concede uma Influência adicional. |
| starter:007 | Ron | Pigwidgeon | Escolher entre um Ataque e duas Vidas. |
| starter:008 | Hermione | Time-Turner | Receber uma Influência; neste Turno, Feitiços adquiridos podem ir ao topo do próprio baralho. |
| starter:009 | Hermione | The Tales of Beedle the Bard | Escolher entre receber duas Influências e conceder uma Influência a cada Herói. |
| starter:010 | Hermione | Crookshanks | Escolher entre um Ataque e duas Vidas. |
| starter:011 | Neville | Remembrall | Receber uma Influência ao jogar; receber duas Influências se for descartado por um efeito. |
| starter:012 | Neville | Mandrake | Escolher entre receber um Ataque e conceder duas Vidas a um Herói escolhido. |
| starter:013 | Neville | Trevor | Escolher entre um Ataque e duas Vidas. |

O descarte de encerramento do Turno não dispara efeitos de descarte forçado.
Os bônus e permissões das cartas jogadas terminam com o Turno.

## Artes das Trevas, Vilões e Locais

As semânticas são adotadas dos [módulos de Artes das Trevas](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/dark_arts), [Vilões](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/villains) e [Locais](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/locations.py).

| Carta | Quantidade | Regra funcional |
| --- | ---: | --- |
| Expulso | 3 | O Herói ativo perde duas Vidas. |
| Flipendo | 2 | O Herói ativo perde uma Vida e escolhe uma carta da mão para descartar, se houver. |
| He Who Must Not Be Named | 3 | Acrescentar um Controle ao Local atual. |
| Petrification | 2 | Cada Herói perde uma Vida; compras adicionais ficam impedidas até o encerramento deste Turno. |

Cada fase revela uma carta do baralho de Artes das Trevas e resolve somente seu efeito.
Quando esse baralho acaba, seu descarte é reembaralhado antes da próxima revelação.
A reposição normal da mão no encerramento do Turno continua permitida após Petrification.

| Vilão | Vida | Habilidade | Recompensa |
| --- | ---: | --- | --- |
| Crabbe & Goyle | 5 | O Herói obrigado a descartar uma carta perde uma Vida por descarte. | Cada Herói compra uma carta. |
| Draco Malfoy | 6 | Cada Controle efetivamente acrescentado ao Local causa perda de duas Vidas ao Herói ativo. | Remover um Controle do Local. |
| Quirinus Quirrell | 6 | O Herói ativo perde uma Vida na fase dos Vilões. | Cada Herói recebe uma Vida e uma Influência. |

Somente o Vilão ativo exerce sua habilidade.
A recompensa resolve na mesma Janela da derrota do Vilão, inclusive quando contém Escolhas.
Diagon Alley e Mirror of Erised têm limite de quatro Controles e uma revelação de Artes das Trevas por Turno.
Não possuem efeito adicional de revelação.

## Regras estruturais e determinismo

Vida permanece entre zero e dez; chegar a zero causa Atordoado, sem eliminar o Jogador.
Aplicam-se o descarte de metade da mão e a resolução de Atordoado já definidos pelo motor, com Escolha persistível quando necessária.
Controle pertence ao Local atual e nunca excede seu limite.
O encerramento do Turno verifica o Local, repõe Vilões, descarta mão e área jogada, zera os recursos temporários, recupera Heróis Atordoados e repõe cinco cartas.
Se faltar baralho ao comprar, somente o descarte é reembaralhado; cartas da mão e área jogada não entram nesse embaralhamento.
Efeitos de compra resolvem o máximo possível quando todas as cartas disponíveis acabam.
Derrotar todos os Vilões vence; perder o último Local perde; derrota prevalece se os resultados coincidirem na mesma Janela.
Estado terminal encerra Turno e Escolhas e recusa novos Comandos.

A seed e os algoritmos ChaCha20, Fisher-Yates e rejection sampling são versionados.
Cada amostra consumida avança o contador persistido uma vez, incluindo os embaralhamentos de preparação e durante efeitos.
O replay dos mesmos Comandos com a mesma seed e manifesto deve reproduzir Eventos, Escolhas, ordem dos baralhos e hashes.
