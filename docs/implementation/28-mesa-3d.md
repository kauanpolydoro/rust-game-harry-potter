# Mesa 3D do Jogo 1 — issue #28

A entrega acrescenta a mesa horizontal ao fluxo existente de criação e entrada em uma partida. O servidor Rust e os stores de comando/sincronização continuam responsáveis pelo estado oficial. A seleção, a inspeção, as páginas de cartas e os gestos permanecem locais.

## Executar

- Partida integrada: iniciar o ambiente pelo fluxo descrito no README, criar uma sala, reunir de dois a quatro participantes e selecionar o Jogo 1.
- Protótipo: `npm run dev:web`, abrir `/prototype.html`. A página identifica seus dados fictícios e permite ensaiar seleção, jogada, alvo, dano, aquisição/reposição, descarte e passagem de turno. “Ensaio e histórico” também oferece vinte cartas com texto extenso.
- Verificação completa: `make check`. O script usa PostgreSQL em banco de teste isolado, Rust, contratos, lint, tipos, testes, build e E2E.
- Navegadores: `npx playwright install --with-deps chromium firefox webkit`. Em Linux sem display, instalar também `xauth`; `make check` usa `xvfb-run` quando disponível. OpenSSL cria um certificado local efêmero para os E2E. A exceção de confiança existe somente no teste: o cookie da aplicação continua `Secure`, `HttpOnly` e `SameSite=Strict`.

## Implementação

`tablePresentation.ts` oferece o ponto público de decisão local. Ele valida a intenção contra a projeção recebida, limita quantidade, destino e cardinalidade e bloqueia confirmações repetidas enquanto há envio pendente. Alterações de versão invalidam a decisão anterior. `tableProjection.ts` adapta o contrato existente sem reinterpretar regras.

`tableScene.ts` encapsula Babylon.js modular, câmera inclinada, materiais, cartas com frente/verso/espessura, sombras de contato, pilhas, Controle e velas. O módulo possui seus recursos gráficos e os descarta com a cena; Vue não torna objetos da engine reativos. A renderização pausa com a aba oculta ou o canvas sem área. O limite inicial de densidade é 1,5 pixels de renderização por CSS px.

`GameTable3D.vue` posiciona controles semânticos sobre as cartas e mantém texto completo na inspeção. A Mão usa páginas de sete cartas; a área de jogo, páginas de quatro. Ambas preservam o tamanho dos alvos. O modo acessível reutiliza a apresentação semântica existente e é salvo como preferência local. Rotação, perda de foco, cancelamento de ponteiro e versão nova liberam a captura do gesto. A rejeição fecha a decisão; o estado desatualizado usa a recuperação já existente no aplicativo.

O HUD mantém turno, fase, retratos, presença, Vida, Ataque, Influência e Atordoado. Local, Controle, Vilões e custo do mercado têm rótulos. Partida/conexão, histórico e gestão de participantes ficam em menus. Há aviso junto a Encerrar quando ainda existem intenções legais de jogada, ataque ou aquisição.

## Cobertura do aceite

| Requisito | Evidência executável |
| --- | --- |
| Protótipo representativo e consequências do turno | `prototype completes selection, target, damage, acquisition and a turn`; captura, vídeo e arquivo de medições |
| Turno real com 2, 3 e 4 participantes | Três cenários `participants finish a real Game 1 turn through the 3D table`, com servidor Rust, PostgreSQL, HTTP e WebSocket; incluem escolhas, cartas, ataque, aquisição e encerramento |
| Inspeção local e confirmação legal | Testes públicos de `tablePresentation.test.ts` e contagem dos POSTs no E2E de autoridade |
| Arraste, pointercancel e rotação | Testes de decisão pública e cenários de cancelamento/rotação por eventos do navegador |
| Quantidade, destino, alvos e responsável | Testes públicos de Ataque/aquisição/cardinalidade, ensaio com alvo obrigatório e escolhas oficiais nos turnos reais; cobertura de ownership existente preservada |
| Texto, recursos e histórico | Capturas da mesa e da inspeção; contratos existentes fornecem descrições e consequências; menus acessíveis por teclado |
| Teclado e foco | E2E com Enter/Tab/Escape, retorno à carta e confirmação; recuperação do foco após atualização |
| Alvos de 44×44 CSS px | Medição de todos os controles de cartas nos cinco tamanhos, incluindo verificação de alcance pelo centro do alvo |
| Horizontal normal e retrato | Chromium/Pixel 7 e WebKit/iPhone 13 emulados, além de Firefox; rotação conserva a partida e permite o modo acessível, sem fullscreen ou lock |
| Pendência, rejeição, versão antiga e reconexão | E2E contra Rust segura um envio, age em outra aba, recarrega a sessão e provoca rejeições reais; confere sequência e número de POSTs; stores existentes preservam recuperação por recibo/idempotência |
| Mão e área de jogo extensas | Vinte cartas com texto longo; sete cartas jogadas e inspeção nas duas páginas da área de jogo |
| Dependências | Auditoria em `28-evidence/dependencies.json`, lockfile npm sincronizado e resolução Cargo sem alterações |

Os testes de navegador verificam comportamento em motores reais com perfis e tamanhos emulados. Eles não representam uma medição em aparelhos físicos. A recuperação por recibo e as regras de sessão também permanecem cobertas pelos testes existentes de stores, aplicativo e servidor.

## Capturas e medições

- Protótipo: [mesa desktop](28-evidence/prototype-desktop.png), [início](28-evidence/prototype-start.png), [turno concluído](28-evidence/prototype-turn-complete.png) e [vídeo do turno](28-evidence/prototype-turn.webm).
- Partida integrada: [dois participantes](28-evidence/real-turn-2-players.png), [três participantes](28-evidence/real-turn-3-players.png) e [quatro participantes](28-evidence/real-turn-4-players.png).
- Tamanhos avaliados: [667×375](28-evidence/table-667x375.png), [844×390](28-evidence/table-844x390.png), [915×412](28-evidence/table-915x412.png), [1024×768](28-evidence/table-1024x768.png) e [1366×768](28-evidence/table-1366x768.png). As dimensões são CSS px; os PNGs preservam a densidade do perfil emulado.
- Leitura: [inspeção da carta 20 com texto extenso](28-evidence/long-hand-inspection.png).
- Medições iniciais: [Chromium](28-evidence/mobile-chromium-measurements.txt), [WebKit](28-evidence/table-webkit-measurements.txt) e [Firefox](28-evidence/table-firefox-measurements.txt).

As medições registram intervalos entre frames durante o ensaio automatizado, com viewport de 844×390 CSS px, Playwright 1.63.0 e renderização no ambiente Linux/WSL. Incluem aquecimento, interação e concorrência dos testes; não são um benchmark controlado nem uma comprovação da meta física de 60 fps. A cena ao final do ensaio contém 74 meshes e 17 texturas. O módulo gráfico é carregado sob demanda ao abrir a mesa 3D.

## Achados incorporados

- A mesa passou a ocupar toda a largura útil; a altura usa flex e `100dvh`, respeitando safe areas e os controles do navegador.
- O WebKit revelou que o ambiente E2E precisava de HTTPS para enviar o cookie de sessão protegido. A validação agora usa TLS local.
- O Firefox em Linux precisou de display virtual para a criação real de WebGL. A matriz conserva esse caminho gráfico e limita a concorrência a dois workers.
- A captura do ponteiro era mantida depois de girar a tela. O cancelamento agora a libera, e a proteção contra clique residual preserva a ativação por teclado.
- As ações das cartas não acionavam a recuperação existente após `STALE_STATE_VERSION`. A integração agora faz essa recuperação e exige uma decisão nova.
- A revisão independente identificou sobreposição após seis cartas jogadas e a falta do aviso ao encerrar. Paginação e aviso foram incorporados, com regressão para sete cartas jogadas.
- O container de testes com rede do host recebia mudanças de interfaces de outros containers; traces registraram `ERR_NETWORK_CHANGED`. A validação final usa um namespace de rede isolado e quatro threads Rust para reduzir a concorrência no PostgreSQL, sem retirar testes ou ampliar timeouts.

## Arte e limites desta entrega

As imagens iniciais têm procedência em `apps/web/public/table-art/sources.json`; a seleção visual por identidade está em `tableArt.ts`. Cartas sem imagem mapeada exibem uma identidade tipográfica, nome e dados oficiais. Texto completo e comandos continuam disponíveis sem depender da imagem.

A issue #30 conclui animações, áudio, assets finais, qualidade adaptável e benchmark físico de 60 fps no Galaxy A54/iPhone 13. A issue #29 conclui a auditoria acessível; a #38 integra visualmente os Jogos 2–7. Esta entrega não declara esses trabalhos concluídos.

## Resultado final da validação

`RUST_TEST_THREADS=4 make check` passou com código de saída 0: 353 testes Rust, nove testes dos scripts Node, 118 testes web e 51 E2E em Chromium, WebKit e Firefox (8,4 minutos de E2E). Os 24 cenários da mesa incluem nove turnos reais, cobrindo dois, três e quatro participantes nos três motores. Formatação, clippy, contratos, limites de módulos, lint, tipos, build e varredura de segredos também passaram. O [registro da validação](28-evidence/validation.txt) preserva os resultados de navegador.

Dois perfis Rust de SLO permanecem ignorados na suíte comum, conforme a configuração anterior do repositório: carga de reconexão e ciclo de vida de cem partidas. A comprovação física de desempenho continua em #30.

A [revisão independente](28-evidence/review.md) terminou com **0 achados de Standards e 0 de Spec**. A [auditoria de dependências](28-evidence/dependencies.json) confirma as versões diretas nas fontes oficiais e distingue as resoluções transitivas. O CI remoto permanece desativado.
