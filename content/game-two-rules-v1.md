# Jogo 2: regras funcionais v1

Este documento define a adaptação versionada `game-two-v1`, publicada em `game-two-en-v1`.
As regras herdadas mantêm a identidade e a proveniência de [game-one-v1](game-one-rules-v1.md).
O bundle, o manifesto e as Partidas do Jogo 1 permanecem independentes.
Estas descrições são regras funcionais da adaptação, não transcrições ou localização editorial oficial.

## Inventário e fontes

O [manual do Jogo 2, páginas 1 e 2](https://manuals.plus/m/6eee5435fe2e61773b27760d1a6b38aadfa9b664e36b77b9caeb64e427bb401f.pdf) declara 14 novas cartas de Hogwarts, cinco Artes das Trevas, três Vilões e três Locais.
Hogwarts, Artes das Trevas e Vilões acumulam os dois Jogos.
Os três novos Locais substituem os dois anteriores.
O manifesto fecha 63 registros e 116 cartas: 44 de Hogwarts, 40 iniciais, 15 Artes das Trevas, seis Vilões, três Locais, quatro Heróis e quatro referências de Turno.
Catálogo, Ruleset e Aventura são registros lógicos de quantidade zero.
Os baralhos iniciais de Heróis ausentes ficam fora da Partida.

O [inventário fixado por commit](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/config/game_two.yaml) identifica as cartas e suas quantidades.
As implementações de [Hogwarts](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/hogwarts), [Artes das Trevas](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/dark_arts), [Vilões](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/villains) e [Locais](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/locations.py) sustentam a interpretação adotada explicitamente abaixo.
Essas implementações continuam classificadas como fontes candidatas.
A adaptação é uma decisão de confiança separada, concedida pelo servidor para este documento e sua versão.

`hogwarts-card:012` continua excluída pela divergência de inventário documentada no Jogo 1.
O candidato integral permanece preservado.
Nenhuma variação de impressão não comprovada substitui silenciosamente uma regra.
Ao importar uma impressão cuja definição funcional ou proveniência esteja ausente, desconhecida ou candidata, a entrada afetada permanece bloqueada e a Aventura não pode iniciar com esse manifesto.
Uma correção funcional exige nova versão e novo digest.

## Preparação e estrutura

Cada Herói começa novamente com suas dez cartas iniciais, dez Vidas e nenhum recurso temporário.
Cartas compradas em outra Partida não são carregadas para esta Aventura.
Embaralham-se separadamente as 44 cartas de Hogwarts, 15 Artes das Trevas, seis Vilões e cada baralho inicial.
O mercado recebe seis cartas, existe um Vilão ativo e cada Herói compra cinco cartas iniciais.
Os Locais seguem a ordem abaixo, sem embaralhamento.

| ID | Local | Limite de Controle | Artes das Trevas por Turno |
| --- | --- | ---: | ---: |
| location:003 | Forbidden Forest | 4 | 1 |
| location:004 | Quidditch Pitch | 4 | 1 |
| location:005 | Chamber of Secrets | 5 | 2 |

Os Locais não possuem efeito adicional de revelação.
Na Chamber of Secrets, cada Arte das Trevas resolve integralmente, incluindo Escolhas e Atordoado, antes da próxima revelação.
Fases, reposição, precedência terminal e determinismo seguem a adaptação do Jogo 1.
Derrotar os seis Vilões vence; perder o terceiro Local perde.

## Novas cartas de Hogwarts

| ID | Carta | Quantidade | Custo | Categoria | Regra funcional |
| --- | --- | ---: | ---: | --- | --- |
| 014 | Arthur Weasley | 1 | 6 | Aliado | Cada Herói recebe duas Influências. |
| 015 | Dobby | 1 | 4 | Aliado | Remover um Controle; comprar uma carta. |
| 016 | Expelliarmus | 2 | 6 | Feitiço | Receber dois Ataques e comprar uma carta. |
| 017 | Fawkes | 1 | 5 | Aliado | Escolher entre dois Ataques para si e duas Vidas para cada Herói. |
| 018 | Finite | 2 | 3 | Feitiço | Remover um Controle. |
| 019 | Gilderoy Lockhart | 1 | 2 | Aliado | Comprar uma carta e descartar uma carta; quando descartado por efeito, comprar uma carta. |
| 020 | Ginny Weasley | 1 | 4 | Aliado | Receber um Ataque e uma Influência. |
| 021 | Molly Weasley | 1 | 6 | Aliado | Cada Herói recebe uma Influência e duas Vidas. |
| 022 | Nimbus 2001 | 2 | 5 | Item | Receber dois Ataques; cada Vilão derrotado depois de jogar esta carta concede duas Influências adicionais. |
| 023 | Polyjuice Potion | 2 | 3 | Item | Escolher um Aliado já jogado pelo próprio Herói neste Turno e copiar seus efeitos. |

Gilderoy exige a escolha do descarte depois da compra.
Se compras extras estiverem bloqueadas, seu dono escolhe entre apenas descartar e não aplicar o efeito.
Esse descarte voluntário dispara o efeito da própria carta descartada, mas não a penalidade de Crabbe & Goyle.
O descarte normal de encerramento do Turno não dispara efeitos de descarte.

Polyjuice Potion continua sendo um Item e não dispara um novo evento de Aliado jogado.
A cópia executa as Escolhas do Aliado e conserva suas reações pertinentes enquanto a Poção permanece jogada neste Turno.
Copiar Oliver Wood acrescenta uma segunda oportunidade de cura para cada Vilão derrotado posteriormente.
Não existe cópia de cartas da mão, de outro Herói ou de Itens.
Sem Aliado elegível, a Poção é jogada sem efeito de cópia.

## Artes das Trevas e Vilões

| ID | Carta | Quantidade | Regra funcional |
| --- | --- | ---: | --- |
| dark-arts:005 | Hand of Glory | 2 | O Herói ativo perde uma Vida; acrescentar um Controle. |
| dark-arts:006 | Obliviate | 1 | Cada Herói descarta um Feitiço ou perde duas Vidas. |
| dark-arts:007 | Poison | 1 | Cada Herói descarta um Aliado ou perde duas Vidas. |
| dark-arts:008 | Relashio | 1 | Cada Herói descarta um Item ou perde duas Vidas. |

As decisões individuais seguem a ordem das posições.
Sem carta da categoria exigida, aplica-se a perda de Vida.
Um Herói já Atordoado não precisa escolher uma perda de Vida sem consequência.

| Vilão | Vida | Habilidade | Recompensa |
| --- | ---: | --- | --- |
| Basilisk | 8 | Impedir compras extras enquanto estiver ativo. | Cada Herói compra uma carta; remover um Controle. |
| Lucius Malfoy | 7 | Cada Controle efetivamente acrescentado cura uma Vida de cada Vilão ativo, até seu limite. | Cada Herói recebe uma Influência; remover um Controle. |
| Tom Riddle | 6 | Contar os Aliados na mão do Herói ativo; para cada um, escolher entre perder duas Vidas e descartar uma carta. | Cada Herói escolhe entre duas Vidas e recuperar um Aliado do próprio descarte para a mão. |

O Basilisco deixa de impedir compras antes de resolver sua recompensa.
Petrification continua impedindo compras até o fim do Turno mesmo se o Basilisco for derrotado.
Revelar Petrification novamente no mesmo Turno mantém o bloqueio e aplica novamente a perda de Vida.
A reposição da mão no encerramento permanece permitida.
Tom Riddle fixa a quantidade de repetições antes do primeiro descarte; descartar um Aliado não cancela as repetições já determinadas.
A recompensa de Tom Riddle concede duas Vidas automaticamente quando o Herói não possui Aliado no descarte.
Recuperar uma carta específica do descarte não é comprar uma carta do baralho e continua permitido sob bloqueio de compras.
