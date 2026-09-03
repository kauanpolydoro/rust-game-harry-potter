---
name: Batalha de Hogwarts
description: Mesa cooperativa mobile-first conduzida como uma sequência de deixas oficiais.
colors:
  ink: "#08131d"
  ink-raised: "#0d1b27"
  chalk: "#f4f0e7"
  chalk-muted: "#b7c0c5"
  brass: "#c7a35b"
  brass-quiet: "#6f633f"
  ready: "#70c18c"
  warning: "#e2ab55"
  focus: "#9cd7ff"
typography:
  display:
    fontFamily: "Archivo Narrow Variable, ui-sans-serif, sans-serif"
    fontSize: "2.6rem"
    fontWeight: 700
    lineHeight: 0.98
    letterSpacing: "-0.035em"
  title:
    fontFamily: "Archivo Narrow Variable, ui-sans-serif, sans-serif"
    fontSize: "1.5rem"
    fontWeight: 700
    lineHeight: 1.08
    letterSpacing: "-0.025em"
  body:
    fontFamily: "Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "1.05rem"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: "Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "0.78rem"
    fontWeight: 650
    lineHeight: 1.4
    letterSpacing: "0.06em"
  control:
    fontFamily: "Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "1rem"
    fontWeight: 760
rounded:
  control: "0.75rem"
  signal: "50%"
spacing:
  compact: "0.75rem"
  base: "1rem"
  content: "1.25rem"
  section: "1.5rem"
  large: "2rem"
  desktop-gutter: "3rem"
components:
  retry-action:
    backgroundColor: "{colors.brass}"
    textColor: "{colors.ink}"
    typography: "{typography.control}"
    rounded: "{rounded.control}"
    width: "100%"
  retry-action-hover:
    backgroundColor: "{colors.chalk}"
    textColor: "{colors.ink}"
  retry-action-active:
    backgroundColor: "{colors.warning}"
    textColor: "{colors.ink}"
  retry-action-checking:
    backgroundColor: "transparent"
    textColor: "{colors.chalk-muted}"
    typography: "{typography.control}"
    rounded: "{rounded.control}"
    width: "100%"
  cue-number:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.brass}"
    typography: "{typography.control}"
    size: "2.75rem"
  status-signal-ready:
    backgroundColor: "{colors.ready}"
    textColor: "{colors.ready}"
    rounded: "{rounded.signal}"
    size: "0.85rem"
  status-signal-unavailable:
    backgroundColor: "{colors.warning}"
    textColor: "{colors.warning}"
    rounded: "{rounded.signal}"
    size: "0.85rem"
  status-signal-checking:
    backgroundColor: "transparent"
    textColor: "{colors.chalk-muted}"
    rounded: "{rounded.signal}"
    size: "0.85rem"
  continuity-note:
    textColor: "{colors.chalk-muted}"
    typography: "{typography.label}"
  form-field:
    backgroundColor: "{colors.ink-raised}"
    textColor: "{colors.chalk}"
    rounded: "{rounded.control}"
  password-toggle:
    backgroundColor: "transparent"
    textColor: "{colors.chalk}"
    rounded: "{rounded.control}"
---

# Design System: Batalha de Hogwarts

## Overview

**Creative North Star: "A Mesa de Contrarregra"**

A interface trata a mesa autoritativa como uma sequência legível de deixas, inspirada pela precisão de uma mesa de direção de cena.
O sistema evita a coleção de cartões mágicos flutuantes e organiza cada estado como uma marca oficial em um painel contínuo.

Azul-noite fosco, textura material discreta, texto de giz e regras de latão criam uma atmosfera sóbria sem competir com a decisão do grupo.
Hierarquia forte, poucos elementos e estados explicitamente nomeados mantêm a autoridade do servidor e o próximo passo visíveis em celulares e computadores.

**Key Characteristics:**

- Painel único, escuro e texturizado.
- Eixo vertical de deixas com numeração e regras de latão.
- Tipografia condensada para comandos e sans-serif neutra para explicações.
- Verde e âmbar reservados para estados oficiais, sempre acompanhados por texto.
- Ação de recuperação isolada no alcance do polegar.

## Colors

A paleta combina uma base azul-noite quase preta com giz quente e latão envelhecido, reservando cores mais luminosas para estado e acessibilidade.

### Primary

- **Latão de marcação** (`brass`): destaca a ação de recuperação, o marcador triangular, a numeração, os traços curtos e a seleção de texto.
- **Latão quieto** (`brass-quiet`): desenha divisores e o eixo estrutural sem competir com a informação principal.

### Neutral

- **Azul-noite fosco** (`ink`): cobre o documento, o painel principal e o fundo dos marcadores de deixa.
- **Azul-noite elevado** (`ink-raised`): diferencia campos editáveis do painel sem criar cartões ou profundidade independente.
- **Giz quente** (`chalk`): carrega títulos e também clareia a ação de recuperação no hover.
- **Giz frio atenuado** (`chalk-muted`): sustenta descrições, metadados, notas e estados ainda não confirmados.

### Functional

- **Verde de confirmação** (`ready`): identifica disponibilidade confirmada e ações concluídas com sucesso.
- **Âmbar de atenção** (`warning`): identifica indisponibilidade, validação, falhas recuperáveis e o estado pressionado de ações.
- **Azul de foco** (`focus`): torna o foco por teclado inequivocamente visível sobre a base escura.

**The Regra do Latão Raro Rule.** O latão estrutura a leitura e marca a ação principal, mas não preenche superfícies inteiras fora do controle de recuperação.

## Typography

**Display Font:** Archivo Narrow Variable com fallback para `ui-sans-serif`.

**Body Font:** Inter com fallbacks nativos de interface.

**Character:** A voz condensada funciona como uma chamada de palco firme, enquanto a sans-serif aberta mantém instruções e estados legíveis.
A diferença de família separa comando de explicação sem recorrer a ornamentação temática.

### Hierarchy

- **Display** (700, `2.6rem`, `0.98`): nomeia o estado oficial dominante e cresce para `4rem` a partir do breakpoint amplo.
- **Title** (700, `1.5rem`, `1.08`): identifica a experiência no masthead e cresce para `1.8rem` em telas amplas.
- **Body** (400, `1.05rem`, `1.55`): explica disponibilidade e recuperação em linhas de até `34rem`, crescendo para `1.2rem` em telas amplas.
- **Label** (650 a 700, `0.7rem` a `0.78rem`, tracking positivo): rotula edição, eixo e continuidade em caixa alta quando atua como metadado.
- **Control** (760, `1rem`): mantém a ação de recuperação direta, estável e fácil de localizar.

**The Regra da Voz de Comando Rule.** Archivo Narrow pertence aos títulos que orientam a mesa; explicações, metadados e controles permanecem em Inter.

## Layout

O layout é mobile-first e ocupa um painel central com largura máxima de `52rem`, altura mínima do viewport e três faixas: masthead, estado flexível e ação.
O conteúdo começa em `320px` e respeita safe areas com um recuo mínimo de `1.25rem` em todos os lados.

O estado usa duas colunas, uma estreita para o trilho de deixa e outra para a mensagem oficial.
O eixo vertical atravessa a composição e alinha o rodapé de ação com a leitura iniciada no masthead.
A faixa principal mantém pelo menos `22rem`, o que protege a separação entre confirmação e recuperação.

A partir de `48rem`, o recuo horizontal cresce para `3rem`, o trilho ganha largura e o intervalo entre trilho e conteúdo aumenta para `2.5rem`.
O masthead passa a acomodar a edição na terceira coluna, sem abandonar o mesmo eixo de leitura.

**The Regra do Eixo de Deixa Rule.** Numeração, linha vertical, estado dominante e ação final devem compartilhar uma progressão visual contínua, nunca uma grade de módulos independentes.

## Elevation & Depth

O sistema combina uma única sombra ambiente no painel principal com textura fosca repetida e divisores tonais.
Não existem cartões elevados ou sombras por componente.
A profundidade pertence ao painel inteiro, enquanto a hierarquia interna depende de contraste, linhas e espaço.

### Shadow Vocabulary

- **Painel ambiente** (`0 1.5rem 4rem rgb(0 0 0 / 28%)`): separa discretamente a mesa central do fundo somente quando há espaço ao redor dela.

**The Regra do Painel Único Rule.** A sombra enquadra a mesa completa; estados, trilhos e ações permanecem planos dentro dela.

## Shapes

O vocabulário mistura geometria ortogonal de régua com sinais circulares de estado.
Numeração, divisores e marcas de direção mantêm cantos retos, enquanto controles de entrada e ação recebem uma curva de `0.75rem`.
Os sinais usam círculos perfeitos e o marcador do masthead usa um triângulo compacto, criando silhuetas funcionais sem ornamentação.

**The Regra do Contraste de Silhueta Rule.** Cantos arredondados indicam interação ou estado; a estrutura da mesa continua reta e precisa.

## Components

### Masthead

- **Character:** compacto e editorial, com um marcador triangular de latão que antecede identidade e edição.
- **Structure:** duas colunas no celular e três a partir de `48rem`, separadas do conteúdo por uma regra fina de latão quieto.
- **Typography:** título em Archivo Narrow e edição em Inter, caixa alta e tracking aberto.

### Official State Rail

- **Character:** funciona como a contrarregra da mesa, numerando o estado e conduzindo o olhar até a mensagem dominante.
- **Shape:** número quadrado de `2.75rem`, linha vertical de `1px` e rótulo vertical em caixa alta.
- **Color:** fundo azul-noite, contorno e número em latão, com rótulo em giz atenuado.

### Status Signals

- **Shape:** círculo compacto de `0.85rem` com borda de `2px` na mesma cor do estado.
- **Ready:** preenchimento verde acompanha o texto “Servidor pronto”.
- **Unavailable:** preenchimento âmbar acompanha o texto “Servidor indisponível”.
- **Checking:** permanece sem preenchimento e pulsa em `1.4s`; `prefers-reduced-motion` reduz a animação a uma mudança praticamente instantânea.

### Recovery Action

- **Shape:** controle largo com altura mínima de `3.25rem`, borda de latão e curva de `0.75rem`.
- **Default:** latão sobre azul-noite, peso 760 e largura total.
- **Hover / Active:** o hover clareia para giz e o estado pressionado usa âmbar, ambos preservando texto azul-noite.
- **Focus:** contorno azul de `3px` com afastamento de `4px`.
- **Checking:** fundo transparente, texto atenuado, cursor de espera e foco preservado enquanto a confirmação está pendente.

### Form Fields

- **Surface:** azul-noite elevado, contorno de latão quieto e altura mínima de `3.25rem` distinguem edição sem fragmentar o painel.
- **Structure:** rótulos permanecem visíveis, e controles auxiliares ocupam a mesma altura do campo associado.
- **Password:** o toggle textual nomeia a ação “Mostrar senha” ou “Ocultar senha” e permanece ligado semanticamente ao campo.
- **Validation:** erros usam âmbar com texto explícito junto ao campo, associação semântica e foco no primeiro valor inválido.

### Action Feedback

- **Success:** confirma a conclusão em verde com texto anunciado por tecnologia assistiva.
- **Failure:** usa âmbar, nomeia o problema e oferece uma recuperação concreta em uma região de alerta.
- **Clipboard:** a ação permanece repetível depois do sucesso e orienta a cópia manual quando o navegador recusa o acesso.

### Continuity Note

- **Character:** substitui a ação quando o serviço está pronto e comunica espera sem parecer um controle.
- **Structure:** traço curto de latão seguido por texto atenuado em corpo compacto.

**The Regra do Próximo Passo Rule.** A região inferior mostra uma única ação de recuperação ou uma única nota de continuidade, nunca as duas ao mesmo tempo.

## Do's and Don'ts

### Do:

- **Do** preservar o painel contínuo, o eixo de deixas e a hierarquia entre estado oficial e próximo passo.
- **Do** manter estados verde e âmbar acompanhados por títulos e descrições explícitas.
- **Do** respeitar safe areas, zoom, foco visível e alvos interativos com pelo menos `44px` por `44px`.
- **Do** usar textura, latão e tipografia condensada como sinais discretos de direção de cena.
- **Do** associar validação ao campo correspondente e comunicar o resultado de ações assíncronas com texto explícito.

### Don't:

- **Don't** transformar a mesa em um dashboard de cartões mágicos independentes.
- **Don't** usar cor, pulso ou preenchimento como único canal para disponibilidade.
- **Don't** arredondar trilhos, marcadores numéricos ou divisores estruturais.
- **Don't** introduzir componentes, ornamentos ou conteúdo de franquia sem presença comprovada no produto.
- **Don't** deixar uma falha de ação silenciosa ou depender somente de cor para comunicar sucesso e erro.
