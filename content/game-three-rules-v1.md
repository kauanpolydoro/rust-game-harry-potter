# Jogo 3: regras funcionais v1

Este documento define a adaptação `game-three-v1`, publicada pelo bundle `game-three-en-v1`.
As regras herdadas preservam a proveniência dos [Jogos 1](game-one-rules-v1.md) e [2](game-two-rules-v1.md).
Os bundles publicados anteriormente permanecem imutáveis.
As descrições são regras funcionais da adaptação, não transcrições nem localização editorial oficial.

## Inventário e evidências

O [manual do Jogo 3, páginas 1 e 2](https://www.scribd.com/document/405719603/HP-DB-Game-3-rules-R1-pdf) identifica 16 novas cartas de Hogwarts, quatro Artes das Trevas, dois Vilões, três Locais e quatro Heróis substitutos.
O manifesto cumulativo fecha 77 registros e 138 cartas: 60 de Hogwarts, 40 iniciais, 19 Artes das Trevas, oito Vilões, três Locais, quatro Heróis e quatro referências de Turno.
Catálogo, Ruleset e Aventura possuem quantidade zero.
Os dois marcadores de escudo representam um efeito persistido e não acrescentam cartas ao inventário.

O [inventário comunitário fixado por commit](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/config/game_three.yaml) omite Sirius Black e soma apenas 15 novas cartas de Hogwarts.
O manual mostra essa carta, seu custo seis e o benefício de dois Ataques e duas Influências.
A [implementação comunitária de Sirius](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/hogwarts/sirius_black.py) concede apenas uma Influência, divergindo da carta ilustrada.
Esta adaptação segue a ilustração e atribui a Sirius o novo ID `hogwarts-card:061`.
Nenhum ID existente é reaproveitado; o candidato integral e a exclusão de `hogwarts-card:012` permanecem preservados.

Os módulos comunitários de [Heróis](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/heroes), [Hogwarts](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/hogwarts), [Artes das Trevas](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/dark_arts), [Vilões](https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/villains) e [Locais](https://github.com/xanderman/hogwarts-battle/blob/4cf4e2547e4ba399c8f24e021317e78dd047e5e5/locations.py) sustentam as demais interpretações abaixo.
Essas fontes continuam candidatas; a confiança funcional pertence à adaptação versionada e é concedida externamente pelo servidor.
Uma impressão sem regra, fonte confiável ou capacidade de execução mantém a entrada e a Aventura bloqueadas.

## Preparação e identidades

Cada Participante conserva sua identidade de Herói e recebe somente a versão do Jogo 3.
Harry usa `hero:002`, Hermione `hero:005`, Neville `hero:008` e Ron `hero:011`.
As versões `hero:001`, `hero:004`, `hero:007` e `hero:010` ficam fora deste manifesto.
Cada Herói começa com dez Vidas, nenhum recurso temporário e suas dez cartas iniciais, das quais compra cinco após embaralhar.
Cartas de Heróis ausentes não entram na Partida.

Hogwarts, Artes das Trevas, Vilões e baralhos iniciais são embaralhados separadamente.
O mercado recebe seis cartas; duas vagas de Vilão ficam ativas e são repostas ao encerrar o Turno.
Derrotar os oito Vilões vence; perder o terceiro Local perde.
Os Locais substituem os anteriores e conservam esta ordem:

| ID | Local | Limite de Controle | Artes das Trevas |
| --- | --- | ---: | ---: |
| location:006 | Hogwarts Express | 5 | 1 |
| location:007 | Hogsmeade Village | 6 | 2 |
| location:008 | Shrieking Shack | 6 | 2 |

## Habilidades dos Heróis

As habilidades usam Strategies Rust fechadas, identificadas e versionadas na AST.
Cada ativação registra o estado consumido antes de produzir uma Escolha, preservando o limite durante reconexões e replay.
Os limites por Turno são renovados antes das Artes das Trevas do próximo Turno.

| Strategy | Condição | Consequência |
| --- | --- | --- |
| harry_game_three_v1 | Primeira remoção efetiva de Controle em qualquer Turno. | Harry escolhe um Herói para receber um Ataque. |
| hermione_game_three_v1 | Hermione joga seu quarto Feitiço no próprio Turno. | Hermione escolhe um Herói para receber uma Influência, uma vez no Turno. |
| neville_game_three_v1 | Primeira recuperação efetiva de Vida de cada Herói no Turno de Neville. | Aquele Herói recupera uma Vida adicional, respeitando o limite dez. |
| ron_game_three_v1 | Ron atribui ao menos três Ataques no próprio Turno, somados entre Vilões. | Ron escolhe um Herói para recuperar duas Vidas, uma vez no Turno. |

Remover Controle de um Local vazio não ativa Harry.
Copiar um Aliado não conta como jogar um Feitiço.
Cura sem aumento efetivo de Vida e a recuperação estrutural de Atordoado não ativam Neville.
O bônus de Neville não ativa a si mesmo.

## Novas cartas de Hogwarts

| ID | Carta | Quantidade | Custo | Categoria | Regra funcional |
| --- | --- | ---: | ---: | --- | --- |
| 024 | Butterbeer | 3 | 3 | Item | Escolher dois Heróis distintos; cada um recebe uma Influência e uma Vida. |
| 025 | Chocolate Frog | 3 | 2 | Item | Escolher um Herói para receber uma Influência e uma Vida; quando descartado por Artes das Trevas ou Vilão, seu dono recebe os mesmos recursos. |
| 026 | Crystal Ball | 2 | 3 | Item | Comprar duas cartas e descartar uma. |
| 027 | Expecto Patronum | 2 | 5 | Feitiço | Receber um Ataque e remover um Controle. |
| 028 | Marauder's Map | 1 | 5 | Item | Comprar duas cartas; quando descartado por Artes das Trevas ou Vilão, cada Herói compra uma carta. |
| 029 | Petrificus Totalus | 2 | 6 | Feitiço | Receber um Ataque e impedir a habilidade de um Vilão ativo até o início do próximo Turno do lançador. |
| 030 | Remus Lupin | 1 | 4 | Aliado | Receber um Ataque e escolher um Herói para recuperar três Vidas. |
| 031 | Sybill Trelawney | 1 | 4 | Aliado | Comprar duas cartas e descartar uma; se a carta descartada for Feitiço, receber duas Influências. |
| 061 | Sirius Black | 1 | 6 | Aliado | Receber dois Ataques e duas Influências. |

Crystal Ball e Sybill permitem optar apenas pelo descarte quando compras extras estiverem bloqueadas, seguindo a decisão registrada para Gilderoy no Jogo 2.
Descartes voluntários não ativam Crabbe & Goyle nem os novos efeitos condicionados a Artes das Trevas ou Vilão.
Descartes obrigatórios por Atordoamento também ativam Chocolate Frog e Marauder's Map, preservando a reação ao dano que causou o Atordoamento.
Descartes do topo causados por essas fontes ativam os efeitos pertinentes, mas revelação sozinha não equivale a descarte.

Petrificus impede efeitos contínuos, reações e a habilidade da fase de Vilões, preservando a recompensa por derrota.
O bloqueio expira antes das Artes das Trevas do próximo Turno do lançador.
Aplicações por lançadores distintos conservam suas durações independentes.
O bloqueio do Basilisk deixa de impedir compras imediatamente; Petrification continua valendo até o encerramento do Turno.

## Artes das Trevas e Vilões

| ID | Carta | Quantidade | Regra funcional |
| --- | --- | ---: | --- |
| dark-arts:009 | Dementor's Kiss | 2 | O Herói ativo perde duas Vidas e cada outro Herói perde uma. |
| dark-arts:010 | Opugno | 1 | Cada Herói revela o topo do próprio baralho; uma carta de custo ao menos um é descartada e seu dono perde duas Vidas. |
| dark-arts:011 | Tarantallegra | 1 | O Herói ativo perde uma Vida; neste Turno, no máximo um Ataque pode ser atribuído a cada Vilão. |

Revelar o topo reembaralha o descarte quando necessário, sem comprar a carta para a mão.
Sem cartas disponíveis, a revelação termina sem efeito.
Cartas iniciais têm custo zero; cartas abaixo do custo mínimo permanecem no topo.
As revelações de Opugno seguem a ordem das posições e suas consequências resolvem antes de passar ao próximo Herói.
Tarantallegra limita o total atribuído no Turno, mesmo após curas de Lucius; uma nova cópia não renova essa cota.

| Vilão | Vida | Habilidade | Recompensa |
| --- | ---: | --- | --- |
| Dementor | 8 | O Herói ativo perde duas Vidas. | Cada Herói recupera duas Vidas; remover um Controle. |
| Peter Pettigrew | 7 | Revelar o topo do baralho ativo; se custar ao menos um, descartar e acrescentar um Controle. | Cada Herói pode recuperar um Feitiço do próprio descarte para a mão; remover um Controle. |

A recuperação de um Feitiço específico não é compra extra e funciona sob bloqueio de compras.
As regras cumulativas de reação, Atordoado, precedência terminal e aleatoriedade permanecem explícitas e determinísticas.

## Persistência e validação

O snapshot 7 pertence ao manifesto 6 e à Aventura `adventure:003`; o evento 8 registra alterações do estado por Turno e revelações do topo.
O PostgreSQL valida o vínculo entre Herói, catálogo e habilidade, a presença do estado dos Vilões, o estado anterior de cada alteração e a carta efetivamente no topo antes da revelação.
Restauração e replay conservam a ordem interna de cada pilha, as Escolhas pendentes e o contador de aleatoriedade.
Os codecs anteriores rejeitam os novos campos.

Os testes de domínio cobrem as quatro habilidades, descarte voluntário e obrigatório, revelação, quota de Ataque e bloqueios com lançadores independentes.
Os cenários HTTP e de navegador usam PostgreSQL real para vitória e derrota com 2, 3 e 4 participantes.
As transcrições do navegador fixam digests de cada evento e snapshot, além dos marcos de preparação, fases, Atordoamento, avanço de Local e estado terminal.
Sequências com 12 seeds por quantidade de participantes verificam inventário, recursos e replay durante partidas completas.

Os guards do cliente do Jogo 2 ficam congelados como fixture e validam as projeções HTTP e mensagens WebSocket reais dos três Jogos.
O formato público permanece estável; bloqueios, cotas e revelações aparecem nas descrições de suas fontes, com Herói e carta identificados.
