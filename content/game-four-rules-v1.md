# Jogo 4: regras funcionais v1

Este documento define a adaptação `game-four-v1`, publicada pelo bundle `game-four-en-v1`.
As regras herdadas conservam a proveniência dos [Jogos 1](game-one-rules-v1.md), [2](game-two-rules-v1.md) e [3](game-three-rules-v1.md).
As descrições são regras funcionais da adaptação, sem alegação de transcrição ou localização oficial.

## Inventário e fontes

O [manual inglês do Jogo 4](https://manuals.plus/m/d543cf8ace8b898be6f7f23de27ad445062ba0ac5823cdf3d48ed5a7a3e0cd14.pdf) e o [manual publicado pela Kosmos](https://fragkosmos.zendesk.com/hc/de/article_attachments/8085042338972) confirmam 21 cartas novas de Hogwarts, oito Artes das Trevas, dois Vilões, três Locais substitutos e quatro dados de Casa.
A preparação mantém as versões dos Heróis do Jogo 3 e duas vagas de Vilão ativo.
O inventário cumulativo fecha 98 registros e 169 cartas: 81 de Hogwarts, 40 iniciais, 27 Artes das Trevas, dez Vilões, três Locais, quatro Heróis e quatro referências de Turno.
Catálogo, Ruleset e Aventura têm quantidade zero.
Os dados são definições funcionais e não acrescentam cartas ao inventário.

O [inventário comunitário](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/config/game_four.yaml) sustenta as quantidades por identidade.
A correção de Sirius Black documentada no Jogo 3 permanece cumulativa.
As quantidades maiores de Avada Kedavra, Crucio, Imperio, Morsmordre e Death Eater no catálogo candidato integral pertencem aos Jogos posteriores.
Os módulos comunitários de [Hogwarts](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/hogwarts), [Artes das Trevas](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/dark_arts), [Vilões](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/villains) e [Locais](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/locations.py) fornecem as interpretações candidatas adotadas abaixo.
A confiança funcional pertence a esta adaptação e é concedida externamente pelo servidor após verificar suporte de execução.

## Dados de Casa

As distribuições seguem a [implementação comunitária fixada por commit](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/game.py#L186-L204).
O manual confirma os quatro benefícios coletivos, mas não especifica a numeração de cada face.
Esta adaptação fixa a seguinte ordem, com seis resultados equiprováveis por dado.
Cada símbolo concede um recurso a cada Herói ou permite que cada Herói compre uma carta.

| Identificador | Face 1 | Face 2 | Face 3 | Face 4 | Face 5 | Face 6 |
| --- | --- | --- | --- | --- | --- | --- |
| gryffindor_v1 | Influência | Influência | Influência | Vida | Compra | Ataque |
| hufflepuff_v1 | Influência | Vida | Vida | Vida | Compra | Ataque |
| ravenclaw_v1 | Influência | Vida | Compra | Compra | Compra | Ataque |
| slytherin_v1 | Influência | Vida | Compra | Ataque | Ataque | Ataque |

As faces e os efeitos pertencem ao manifesto imutável.
Um lançamento só ocorre depois da escolha da Casa, quando a carta exige essa escolha.
Não existe relançamento no Jogo 4.
Bloqueios de compra continuam valendo para o símbolo Compra.
Curas respeitam o limite dez e podem ativar Neville, conforme a habilidade herdada.
Atordoamento, ausência de cartas e reembaralhamento preservam as regras anteriores.

Heir of Slytherin usa o mesmo dado `slytherin_v1`, com consequências próprias da carta: Influência acrescenta um Controle, Vida recupera uma Vida de cada Vilão ativo, Compra obriga cada Herói a descartar uma carta e Ataque causa uma perda de Vida a cada Herói.
O código comunitário ordena os resultados desse lançamento de outra forma; esta adaptação conserva a numeração única do dado para permitir auditoria consistente.

## Hogwarts

| ID | Carta | Quantidade | Custo | Categoria | Regra funcional |
| --- | --- | ---: | ---: | --- | --- |
| 032 | Accio | 2 | 4 | Feitiço | Receber duas Influências ou recuperar um Item do próprio descarte para a mão. |
| 033 | Cedric Diggory | 1 | 4 | Aliado | Receber um Ataque e lançar Hufflepuff. |
| 034 | Filius Flitwick | 1 | 6 | Aliado | Receber uma Influência, comprar uma carta e lançar Ravenclaw. |
| 035 | Fleur Delacour | 1 | 4 | Aliado | Receber duas Influências; recuperar duas Vidas uma vez se outro Aliado for jogado no mesmo Turno, antes ou depois de Fleur. |
| 036 | Hogwarts: A History | 6 | 4 | Item | Escolher e lançar um dado de Casa. |
| 037 | Mad-Eye Moody | 1 | 6 | Aliado | Receber duas Influências e remover um Controle. |
| 038 | Minerva McGonagall | 1 | 6 | Aliado | Receber uma Influência, um Ataque e lançar Gryffindor. |
| 039 | Pensieve | 1 | 5 | Item | Escolher dois Heróis distintos; cada um recebe uma Influência e compra uma carta. |
| 040 | Pomona Sprout | 1 | 6 | Aliado | Receber uma Influência, escolher um Herói para recuperar duas Vidas e lançar Hufflepuff. |
| 041 | Protego | 3 | 5 | Feitiço | Receber um Ataque e uma Vida; o dono recebe os mesmos benefícios ao descartar por Artes das Trevas, Vilão ou Atordoamento. |
| 042 | Severus Snape | 1 | 6 | Aliado | Receber um Ataque, recuperar duas Vidas e lançar Slytherin. |
| 043 | Triwizard Cup | 1 | 5 | Item | Receber um Ataque, uma Influência e uma Vida. |
| 044 | Viktor Krum | 1 | 5 | Aliado | Receber dois Ataques; cada Vilão derrotado no restante do Turno concede uma Influência e uma Vida. |

Accio sem Item no descarte concede as duas Influências diretamente.
A recuperação de Item não é compra extra.
Pensieve conserva a Influência quando a compra está bloqueada.
Fleur e sua cópia por Polyjuice têm limites independentes por instância e não contam a cópia de efeito como um novo Aliado jogado.
O bônus é consumido mesmo sem cura efetiva, por exemplo com Vida dez.
Protego segue a regra de descarte nocivo do Jogo 3; descartes voluntários e limpeza de fim de Turno não ativam seu bônus.

## Artes das Trevas

| ID | Carta | Quantidade | Regra funcional |
| --- | --- | ---: | --- |
| 012 | Avada Kedavra | 1 | O Herói ativo perde três Vidas; se essa perda o atordoar, acrescentar um Controle além do Controle estrutural do Atordoamento; revelar outra Arte das Trevas. |
| 013 | Crucio | 1 | O Herói ativo perde uma Vida; revelar outra Arte das Trevas. |
| 014 | Heir of Slytherin | 2 | Lançar Slytherin e resolver a tabela nociva descrita acima. |
| 015 | Imperio | 1 | Escolher outro Herói para perder duas Vidas; revelar outra Arte das Trevas. |
| 016 | Morsmordre | 2 | Cada Herói perde uma Vida; acrescentar um Controle. |
| 017 | Regeneration | 1 | Recuperar duas Vidas de cada Vilão ativo, respeitando sua Vida máxima. |

Revelações adicionais resolvem depois das consequências da carta que as provocou, incluindo Escolhas e reações obrigatórias.
Concluir uma Escolha não repete o efeito já resolvido nem o lançamento que a produziu.
A proteção de Finite aplica-se à perda causada por Avada Kedavra.
Um Herói já Atordoado não gera a penalidade adicional dessa carta.
O número base de revelações do Local continua fixado no início da fase.

## Vilões e Locais

| Vilão | Vida | Habilidade | Recompensa |
| --- | ---: | --- | --- |
| Barty Crouch Jr. | 7 | Impedir remoção de Controle enquanto estiver ativo e sem bloqueio. | Remover dois Controles. |
| Death Eater | 7 | Cada revelação de Morsmordre ou de outro Vilão causa uma perda de Vida a cada Herói. | Cada Herói recupera uma Vida; remover um Controle. |

Derrotar Barty retira seu bloqueio antes de resolver a recompensa.
Petrificus Totalus suspende o bloqueio e as reações enquanto sua duração estiver ativa.
Uma remoção impedida não ativa Harry.
Death Eater não reage à própria revelação; o outro Vilão revelado depois dele ativa a habilidade.
A reação à revelação de Morsmordre resolve antes do efeito da carta, seguindo a fila de consequências obrigatórias.

| ID | Local | Limite de Controle | Artes das Trevas | Revelação |
| --- | --- | ---: | ---: | --- |
| location:009 | Quidditch World Cup | 6 | 1 | Sem efeito. |
| location:010 | Triwizard Tournament | 6 | 2 | Sem efeito. |
| location:011 | Graveyard | 7 | 2 | Cada Herói descarta um Aliado, se possuir algum na mão. |

Os Locais seguem essa ordem e substituem integralmente os Locais do Jogo 3.
O descarte do Cemitério é obrigatório, mas não tem origem em Artes das Trevas ou Vilão.

## Ordem de resolução e persistência

A adaptação preserva a transição estrutural de fim de Turno dos Jogos anteriores: avançar o Local controlado, preencher as vagas de Vilões, limpar a área de jogo e a mão do Herói ativo, zerar seus recursos, recuperar Heróis atordoados e repor sua mão.
No Jogo 4, as consequências das revelações resolvem depois dessa limpeza e antes de passar ao próximo Herói.
O descarte do Cemitério usa as mãos existentes nesse ponto e antecede as reações aos novos Vilões.
Essa ordem faz parte explícita de `game-four-v1`; não altera retroativamente os rulesets anteriores.
Uma Escolha nessa janela conserva o Herói ativo e o número do Turno até sua resolução.

Na preparação, os dois Vilões são considerados na ordem em que entraram na mesa, antes da primeira Arte das Trevas.
Death Eater reage somente às revelações posteriores à sua própria entrada.
O início do próximo Turno renova os limites das habilidades e expira os bloqueios correspondentes antes das Artes das Trevas, como no Jogo 3.

O snapshot 8 mantém o histórico cumulativo `house_die_rolls`.
Cada registro contém a regra de origem como propósito, o contador absoluto do fluxo ChaCha20, o identificador versionado do dado, o limite inclusivo de seis faces e o resultado entre um e seis.
O evento 9 contém os lançamentos produzidos apenas naquela transição.
Os registros são derivados dos resultados consumidos durante a execução; restaurar ou repetir um Comando já aceito não lança novamente os dados.
O banco exige que o novo histórico seja exatamente o histórico anterior seguido pelos registros do evento.
