# Implementação da issue #24

Base inspecionada: `18273f3`.
Issue: <https://github.com/kauanpolydoro/rust-game-harry-potter/issues/24>.
A dependência #23 está fechada e integrada.

## Critérios e evidência necessária

- Conteúdo funcional cumulativo: bundle próprio, manifesto jogável, confiança externa e inventário de 98 registros e 169 cartas.
- Dados versionados: quatro identificadores v1, seis faces ordenadas por dado e efeitos vinculados ao ruleset imutável.
- Auditoria: propósito, intervalo, contador e resultado por lançamento, inclusive no snapshot.
- Continuidade: idempotência HTTP, restauração com nova instância do servidor e replay conservam resultados e não consomem novamente a entropia.
- Exemplos dourados: todas as faces, efeitos favoráveis e Heir of Slytherin, bloqueios, habilidades de Herói e escolhas interrompidas.
- Integração: Jogo 4 selecionável, preparação cumulativa, vitória e derrota com 2/3/4 participantes e compatibilidade dos Jogos 1/2/3.
- Entrega: revisão de padrões e especificação, validação proporcional conforme `AGENTS.md` e commit com identidade de Kauan.

## Fontes e decisões

O manual do Jogo 4 confirma 21 novas cartas de Hogwarts, oito Artes das Trevas, dois Vilões, três Locais substitutos e quatro dados.
Mantém as habilidades de Herói do Jogo 3.
Manual inglês: <https://manuals.plus/m/d543cf8ace8b898be6f7f23de27ad445062ba0ac5823cdf3d48ed5a7a3e0cd14.pdf>.
Manual da editora alemã: <https://fragkosmos.zendesk.com/hc/de/article_attachments/8085042338972>.

O código comunitário fixado em `4cf4e2547e4ba399c8f24e021317e78dd047e5e5` fornece interpretações candidatas de efeitos e distribuições dos dados.
Fonte: <https://github.com/xanderman/hogwarts-battle/tree/4cf4e2547e4ba399c8f24e021317e78dd047e5e5>.
As interpretações são publicadas como adaptação explícita `game-four-v1`, com proveniência própria e sem alterar bundles anteriores.
O inventário preserva Sirius Black, corrigido no Jogo 3.
O catálogo integral possui quantidades dos Jogos posteriores para algumas Artes das Trevas e Death Eater; essas quantidades não pertencem ao Jogo 4.

Os pontos de integração definidos pela especificação principal são importação completa de bundle, GameState + Comando + contexto, contratos e navegador/HTTP/WebSocket com PostgreSQL real.
A apresentação adicional dos dados em 3D pertence à issue #38 conforme a especificação principal.

## Implementação e validação

O bundle funcional, os quatro dados versionados e todos os novos efeitos estão implementados.
A preparação conserva o inventário com 2, 3 e 4 participantes.
O snapshot 8 e o evento 9 mantêm o histórico dos lançamentos, incluindo propósito, contador, dado, intervalo e resultado.
A migração 0026 preserva os validadores anteriores e exige que cada transição acrescente exatamente a auditoria do evento.

Os testes cobrem as 24 faces favoráveis, as seis faces de Heir of Slytherin, escolhas de descarte após restauração, Cemitério, Death Eater, bloqueio de Controle, Pensieve, Fleur e cópia de Fleur.
As 36 partidas com seeds variadas verificaram inventário, recursos, decisões, restauração e replay.
Essa suíte reproduziu e protege a aceitação do resultado de remoção de Controle bloqueada pelo codec.
O fluxo HTTP conservou um lançamento após resposta perdida, nova instância de servidor, repetição idempotente e reconexão por WebSocket.
A validação SQL também rejeitou propósito, contador, intervalo e resultado adulterados.

Os cenários de derrota usam seed 7 com todos os tamanhos de equipe.
Os cenários de vitória usam seeds 7, 23 e 204 para dois, três e quatro Heróis, respectivamente.
Esses cenários são exemplos determinísticos de regressão, sem pretensão de medir a dificuldade ou a taxa de vitória.
As prioridades de compra e seeds ficam versionadas em `apps/server/tests/fixtures/game-four/`.
O harness de navegador permite escolher a seed por requisição, isolada com `tokio::task_local!`; o servidor de produção continua usando a entropia do sistema operacional.
O padrão 7 preserva os cenários e goldens anteriores.

A revisão paralela encontrou duas falhas de especificação: bônus de Fleur em fontes sem estado persistente e eventos públicos de escolha com fase `end_turn`.
Ambas receberam regressões e foram corrigidas.
A revisão das correções não deixou achados pendentes.
A oportunidade de manutenção da revisão de padrões foi atendida centralizando a derivação da auditoria em `HouseDieRoll::from_effects`.

O navegador validou vitória e derrota com 2/3/4 Heróis, inclusive a compatibilidade com o contrato do cliente anterior.
As seis transcrições reais de comandos alimentam os goldens de eventos e snapshots.
As capturas finais em desktop e celular foram inspecionadas, incluindo a descrição compacta dos dados de Casa.

O `make check` confirmou formatação, Clippy, toda a suíte Rust, integrações HTTP com banco real, contratos gerados, lint, tipagem, 119 testes web e build.
Na rodada conjunta do navegador, 40 testes passaram, incluindo todas as partidas dos Jogos 1/2/3, as três derrotas do Jogo 4 e sua vitória com dois Heróis.
As vitórias do Jogo 4 com três e quatro Heróis e um teste de navegação de recuperação falharam com `net::ERR_NETWORK_CHANGED`, confirmado nos traces.
As seis partidas do Jogo 4 haviam passado individualmente antes dessa rodada, e suas transcrições coincidem com as fixtures versionadas.
A suíte ampla foi interrompida após a adoção da política de validação proporcional: dois testes ficaram interrompidos e 18 não foram executados.
Portanto, o `make check` completo não está aprovado; seus resultados parciais e os testes focados anteriores compõem a evidência disponível.
Uma nova validação ampla deve ocorrer somente quando solicitada, em ambiente com rede estável.
O CI remoto permanece desativado conforme `AGENTS.md`.
