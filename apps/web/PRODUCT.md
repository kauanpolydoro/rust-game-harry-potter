# Produto

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Vue, Vite, TypeScript e Pinia no frontend.

O frontend integra um monólito modular Rust com Axum, Tokio, Tower, PostgreSQL e SQLx.

Estas escolhas são requisitos explícitos da spec #1.

## Usuários

Grupos privados de dois a quatro amigos jogam cooperativamente em celulares e computadores, sem contas permanentes.

Cada participante precisa compreender o estado compartilhado, agir somente quando autorizado e retomar a mesma posição depois de falhas de conexão ou dispositivo.

## Propósito do produto

Oferecer uma adaptação web mobile-first, síncrona e recuperável do jogo base Harry Potter: Batalha de Hogwarts.

O sucesso exige fidelidade às regras comprovadas, convergência entre clientes e uma experiência guiada que torne o próximo passo inequívoco.

## Posicionamento

O backend é a única autoridade e transforma intenções idempotentes em eventos oficiais pós-commit, permitindo que várias abas, dispositivos e processos recuperem a mesma partida sem inventar estado local.

## Contexto de uso

Amigos usam seus próprios dispositivos ao redor de uma mesa física ou durante uma sessão remota.

A sessão pode durar vários encontros e precisa sobreviver a reload, troca de rede, reinício de processo e retorno em outro dispositivo.

## Capacidades e restrições

A interface segue a hierarquia de Mesa guiada: turno e situação, perigo compartilhado, mão e ação principal.

Toda informação e ação essencial existe em DOM semântico.

Canvas ou WebGL, quando existirem, serão apenas decorativos.

Conteúdo nominal candidato não se torna regra sem semântica funcional e proveniência validadas.

O MVP exclui entrada tardia, espectadores, bots, chat público, matchmaking, contas permanentes e expansões.

## Evidência disponível

A issue #1 contém a especificação do produto, 158 histórias de usuário, decisões arquiteturais, critérios de qualidade e seams de teste.

O repositório começou vazio e não contém assets licenciados, identidade visual anterior nem conteúdo editorial autorizado.

Nenhum claim, tradução, texto de carta ou efeito ausente deve ser fabricado.

## Princípios do produto

- A confirmação pós-commit sempre vence a ilusão de velocidade.

- Uma decisão humana permanece com o participante responsável.

- Falhas de rede mudam a disponibilidade, nunca a regra.

- Conteúdo incompleto falha fechado.

- A interface explica o estado e a recuperação possível.

## Acessibilidade e inclusão

Fluxos essenciais devem atender WCAG 2.2 AA, teclado, VoiceOver e TalkBack.

O produto deve suportar zoom de 200%, safe areas, foco visível, contraste AA e alvos de toque de pelo menos 44 por 44 pixels CSS.

`prefers-reduced-motion` prevalece sobre perfis gráficos, e cor, som, vibração ou movimento nunca são o único canal de informação.
