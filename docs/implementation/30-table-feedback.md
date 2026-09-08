# Issue 30: animações, áudio e desempenho da mesa

Fonte: [issue #30](https://github.com/kauanpolydoro/rust-game-harry-potter/issues/30).
Base revisada: `34a32a1`.

## Estado da entrega

A implementação de feedback, áudio, perfis gráficos, carregamento e instrumentação está disponível para validação.
A issue permanece **incompleta** porque as artes finais e os ensaios nos aparelhos físicos ainda não foram entregues.
Nenhuma medição de navegador emulado neste relatório comprova as metas físicas de FPS, latência ou memória.

## Comportamento implementado

A apresentação consome exclusivamente acontecimentos confirmados, com cursor independente da projeção HTTP.
As chaves usam Partida, sequência e índice canônico, incluindo fases e efeitos de `steps` e `end_turn`.
Um HTTP N+2 seguido por WebSocket N+1 preserva a projeção N+2 e apresenta cada acontecimento novo uma vez.
Duplicatas não repetem animações nem sons.

A fila conserva até 20 grupos e resume acontecimentos com mais de dois segundos.
Dano e cura permanecem separados no resumo.
Lacunas visuais expiradas aposentam o intervalo obsoleto sem solicitar recuperação de uma projeção antiga.
Snapshots, recuperação, ocultação, rotação, troca de modo e perda de contexto descartam apresentação obsoleta.
A recuperação gráfica não emite comandos nem cria sessões.

O número de Vida acompanha imediatamente a projeção oficial.
A trilha e os indicadores de dano/cura mostram perdas intermediárias, inclusive quando a Vida final do lote retorna ao valor inicial.
Movimentos usam representações transitórias independentes das cartas oficiais, com limites de duração e quantidade.
Seleção e envio de comandos não aguardam a fila visual.

O modo Básico mantém seleção e movimentos de cartas, com feedback textual de recursos e sem partículas.
Movimento reduzido remove trajetórias e mantém os indicadores e a Vida.
A qualidade automática usa janelas de cinco segundos, redução após duas janelas ruins e recuperação após 30 segundos estáveis.
A escolha manual permanece persistida.
Os perfis alteram resolução, sombras e limite de partículas.

O áudio só cria o contexto e carrega recursos após o botão Ativar som.
Efeitos e ambiente têm volumes independentes; vibração é opcional.
Sons anteriores ao desbloqueio, recebidos em segundo plano ou resumidos não são reproduzidos posteriormente.
A desmontagem encerra contexto, fontes, requisições e buffers.
O modo acessível também interrompe o loop de feedback; a reprodução anterior ainda agendava 61 callbacks por segundo após descartar a cena.
Atualizações recebidas com a aba oculta preservam a projeção e adiam alterações de meshes e texturas até a retomada.
O teste usa WebGL e AudioContext reais com sinal de visibilidade explícito, porque abas headless não reproduzem automaticamente a ocultação do sistema operacional.

## Recursos e localização

[Manifesto visual v1](../../apps/web/public/table-assets/v1/manifest.json) relaciona as 45 identidades do Jogo 1 a arte, resolução, GLB, materiais e áudio, com tamanho e SHA-256.
As artes existentes estão explicitamente marcadas `provisional`; identidades sem arte usam `null`.
Nomes e texto continuam disponíveis quando uma imagem, modelo ou textura falha.

A madeira e o verso usam KTX2/UASTC com mipmaps e Zstandard.
ASTC e BC7 permanecem comprimidos nas GPUs compatíveis; demais GPUs usam o decodificador RGBA incluído no build.
Workers e módulos WASM são servidos pela própria aplicação.
Falhas explícitas ou silenciosas do worker têm fallback PNG/JPEG limitado a oito segundos.
A cena termina os workers mesmo quando a inicialização falha.
Respostas KTX2 atrasadas após a desmontagem encontram uma entrada encerrada, que rejeita a decodificação sem criar workers.
Essa pequena entrada sem recursos permanece como referência até a montagem seguinte, impedindo que o Babylon tente seu fallback de worker `blob:` fora da CSP.

O estoque de cartas também carrega um GLB 2.0 original, com geometria procedural como fallback.
O loader ignora seus materiais PBR implícitos porque a mesa usa materiais próprios, evitando a tentativa de carregar uma textura BRDF embutida que a CSP bloqueava.
O cache de imagens decodificadas guarda até 16 entradas e carrega apenas cartas visíveis.
Texturas de cartas fora da página são descartadas.
O orçamento conservador de texturas está no manifesto; a estimativa exportada não substitui a medição física de memória gráfica.

`generate-table-model.mjs` e `generate-table-audio.py` reproduzem o modelo e os sete sons originais.
O áudio foi sintetizado localmente, sem amostras externas.
Os KTX2 foram produzidos com KTX-Software v4.4.2, obtido do release oficial e verificado pelo checksum publicado, usando `toktx --t2 --encode uastc --genmipmap --zcmp 18` sobre os originais em `public/table-art`.
`generate-visual-manifest.mjs --check` verifica identidades, hashes, dimensões e referências atuais.

A [localização editorial](../../content/locales/README.md) preserva os bundles e digests imutáveis.
O servidor aplica os nomes por identidade e nome original, mantendo precedência de traduções presentes no bundle.
O nome exibido de Rony foi localizado sem mudar a identidade funcional `ron`.

A tentativa de produzir a arte final pelo gerador disponível retornou `moderation_blocked`, categoria `other`, sem motivo específico.
Nenhuma imagem foi gerada nessa tentativa.
É necessário fornecer os recursos finais ou definir outra fonte permitida para concluir esse requisito.

## Validação e revisão

Testes focados passaram para ordenação, deduplicação, resumo, histerese, sincronização e localização do catálogo real.
No navegador com HTTP/WebSocket e PostgreSQL reais, o caso HTTP N+2 antes de WebSocket N+1 passou.
Também passaram o turno completo do protótipo, dano/cura com movimento reduzido e perfil Básico, desbloqueio de áudio sem reprodução atrasada e falha de worker com fallback e seleção preservada.
O `make check` completo terminou com código zero em 8 de setembro de 2026.
Passaram 368 testes Rust, 16 testes de scripts, 128 testes do frontend e 67 testes de navegador, sem retries.
Os dois perfis de carga Rust permanecem separados nos comandos próprios já definidos pelo projeto.
Formatação, Clippy, contratos, manifesto, lint, tipos, build, orçamentos e varredura de segredos também passaram.
O [registro estruturado](30-evidence/validation.json) e o [resumo do gate](30-evidence/check-summary.txt) preservam os resultados locais.
Os 67 cenários incluem 41 em Chromium, 13 em WebKit e 13 em Firefox, com PostgreSQL, HTTP, WebSocket, WebGL e AudioContext reais.
O uso de renderer por software, viewport emulado e saída virtual de áudio mantém pendentes os critérios físicos.
Uma rodada intermediária passou em sete partidas completas, mas três requisições foram abortadas pelo Chromium com `net::ERR_NETWORK_CHANGED` e uma captura excedeu o prazo.
Uma reprodução curta de 600 requisições no mesmo navegador não reproduziu o erro de rede; a origem da mudança de rede da VPS permanece sem confirmação.
Essa rodada foi interrompida para corrigir o loop residual de animação e a textura implícita bloqueada, e não conta como gate aprovado.
Outra rodada revelou uma suposição instável na preparação de `concurrent_purges_remove_the_last_copy_of_a_shared_identity`: a primeira varredura retornou `Ok(0)` em vez da falha simulada do ledger.
Como o enqueue usa `SKIP LOCKED`, a preparação agora admite até oito varreduras e continua exigindo `Err(Ledger)` antes da concorrência.
O teste isolado e a suíte Rust completa passaram após o ajuste, sem alterar o expurgo em produção ou suas verificações finais.
Uma rodada posterior excedeu dez segundos esperando a pausa do ledger no teste de órfãos; doze repetições isoladas não reproduziram o timeout.
O worker também pode encerrar uma varredura após cinco segundos antes de alcançar essa etapa, por contrato.
Os três testes que aguardam a pausa agora conduzem até oito varreduras, mantendo os timeouts, o ponto de pausa e as verificações de estado e concorrência.
Na etapa de navegador seguinte, a partida com três jogadores terminou, mas revelou dois erros de worker por respostas KTX2 que chegaram após a troca para o modo acessível.
A reprodução mínima reteve duas texturas, descartou a cena e liberou as respostas, reproduzindo um worker `blob:` e um erro não tratado antes da correção.
A regressão cobre essa ordem e a montagem seguinte com os dois materiais comprimidos disponíveis.
A desmontagem pode cancelar uma das requisições; o teste exige sua conclusão ou cancelamento explícito e ao menos uma resposta tardia para exercitar a rejeição do decoder.
O ensaio manual com remontagem anterior à liberação das respostas antigas encerrou as duas requisições e manteve dois materiais comprimidos e 17 texturas, sem crescimento da cena, workers `blob:` ou erros.
As capturas de página inteira no fluxo acessível agora usam escala CSS, reduzindo a exportação ampliada pela densidade de pixels do aparelho emulado.

O runner usa um worker de navegador porque duas cenas simultâneas no renderer por software da VPS excederam o timeout de interação do turno completo.
O mesmo fluxo passou isolado em 22 segundos.
Os fluxos funcionais de mesa recebem até 60 segundos porque a mesma sequência no renderer por software variou de 12 a 28 segundos na VPS compartilhada.
Esse prazo não é usado para aprovar latência física.
Os turnos reais com múltiplos participantes recebem três minutos: a execução com quatro cenas avançou até o encerramento, mas excedeu dois minutos durante a disputa de CPU com outra suíte na VPS.
Uma repetição concluiu o turno e as verificações, mas excedeu o prazo ao fechar convidados sequencialmente; o fechamento agora usa `Promise.all`, conforme o padrão da suíte de aventuras.
O prazo de inicialização do servidor de preview foi ampliado para dois minutos porque inclui checagem de tipos e bundling; as metas de entrada do produto continuam dez e três segundos, medidas separadamente.
Isso evita que disputa entre navegadores de teste seja confundida com latência do produto em um aparelho físico.

Na VPS, WebKit e Firefox usam as bibliotecas Ubuntu extraídas e o launcher local já disponíveis em `/tmp/hp-35-browser-libraries` e `/tmp/hp-35-browser-runtime`, sem instalar pacotes no sistema.
`LD_LIBRARY_PATH` aponta para essas bibliotecas e `__EGL_VENDOR_LIBRARY_DIRS` para seu diretório `egl_vendor.d`.
O preflight confirmou WebGL2 real nos dois navegadores antes da suíte.
A consulta de bibliotecas opcionais do Playwright usa somente o cache global de `ldconfig` e ignora esses caminhos; `PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1` desativa essa consulta, sem omitir testes ou substituir WebGL.
Um ambiente com `npx playwright install --with-deps chromium firefox webkit` dispensa essa adaptação local.
O áudio headless do WebKit também exige uma saída disponível: a sessão usa um servidor PulseAudio temporário com `module-null-sink`, carregado de pacotes Ubuntu extraídos em `/tmp/hogwarts-issue30-audio-runtime`.
`GST_PLUGIN_PATH` aponta para os plugins de áudio GStreamer extraídos junto das bibliotecas do navegador.
O preflight passou de `InvalidStateError: Failed to start the audio device` para contexto `running` e decodificação mono do WAV entregue.
Essa saída virtual valida o uso da API e não mede qualidade sonora ou latência em hardware.
O teste de desbloqueio espera a confirmação visível de som ativado antes de amostrar as métricas; observar as sete requisições não garante que `AudioContext.resume()` tenha concluído.
O monitor de CSP exclui somente a mensagem de stylesheet durante as capturas explícitas do WebKit: o Playwright injeta `body {}` para sincronizar screenshots e a política da aplicação bloqueia esse estilo.
Demais erros de CSP, inclusive imagens, scripts e workers durante capturas, continuam causando falha.
No Firefox, uma reprodução WebSocket isolada confirmou que o observador Juggler expõe frames textuais como strings de bytes, enquanto `MessageEvent.data` da aplicação recebe UTF-8 correto.
O suporte de E2E reconstrói UTF-8 apenas nesses frames do Firefox, evitando procurar `EssÃªncia` quando o botão exibe `Essência`.
A reprodução incluiu acentos, emoji e kanji; o fluxo real de aquisição verifica a correspondência com o texto do navegador.

A revisão de padrões identificou a rejeição não tratada do pool original de workers.
A correção foi revisada novamente e recebeu proteção ponta a ponta.
A revisão de especificação identificou trilha de Vida intermediária, ancoragem de Controle e partículas por perfil; os três pontos foram corrigidos.
Uma revisão adicional confirmou o descarte do loop de animação no modo acessível e a retomada de GPU e áudio após ocultação.
Artes finais e ensaios físicos continuam pendências explícitas.

A inspeção visual móvel identificou colisão entre nome e recursos do Herói; o layout foi ajustado para separar os recursos em telas estreitas.
A captura de confirmação mostra nomes e recursos dos Heróis sem colisão.
A inspeção [desktop após o turno](30-evidence/desktop-turn-complete.png) também confirmou Vida e recursos legíveis, sem sobreposição dos controles.
Os rótulos estreitos das cartas ainda quebram palavras no estilo herdado da issue #28, prejudicando a leitura rápida; a inspeção continua mostrando o nome completo.
A próxima revisão tipográfica deve tratar esses rótulos junto da incorporação das artes finais.
O detector Impeccable encontrou avisos de escala, raio e cor contra o DESIGN.md anterior à mesa 3D.
A implementação preserva o estilo da mesa entregue na issue #28 e usa suas cores de dano e cura; não foi feita uma migração visual geral.

[Auditoria de dependências](30-evidence/dependencies.json) separa dependências diretas de resoluções transitivas.
Todos os pacotes diretos npm/crates.io, imagens e GitHub Actions preservados estavam na versão estável corrente na consulta registrada.
Foram adicionados os loaders glTF e o decoder KTX2 Babylon 9.25.0, com versões exatas.
`on-headers` 1.1.0 e seus tipos 1.0.4 são dependências de desenvolvimento para aplicar a política de cache no envio final dos headers do preview, sem serem incluídas no JavaScript do cliente.
Os lockfiles foram atualizados para a resolução compatível mais recente, sem overrides transitivos.
O Vite ainda avisa sobre um chunk 3D acima de 500 kB sem compressão; o carregamento é dinâmico e o gate verifica os limites gzip exigidos pela issue.
O CI remoto permanece propositalmente desativado.
No preview, assets com hash ou versão recebem cache imutável, imagens sem versão exigem revalidação e documentos de recuperação continuam `no-store`.

[Orçamentos do build](30-evidence/build-budgets.json) são verificados automaticamente em `make check`.
O cálculo de transferência total inclui conservadoramente protótipo, áudio, fallbacks, fontes e chunks opcionais.
Não é uma medição de tempo de entrada ou uma garantia sobre headers de compressão de um deploy futuro.

## Limpeza da sessão

Após o gate aprovado e a confirmação de ausência de builds, testes ou servidores usando os arquivos, `cargo clean` removeu o `target` desta cópia.
Também foram removidos o build web, os resultados brutos de E2E e as ferramentas temporárias KTX e PulseAudio desta sessão.
A limpeza liberou 4,64 GiB de blocos alocados identificados; o filesystem terminou em 38%, com 121 GiB disponíveis.
O [registro de limpeza](30-evidence/cleanup.json) detalha os diretórios e bytes medidos antes da remoção.
Foram preservados as evidências revisáveis, as dependências e ferramentas locais preexistentes e os recursos pertencentes a outras sessões.
O banco exclusivo de validação foi removido após confirmar ausência de conexões; o volume PostgreSQL compartilhado foi preservado.
Nenhum cache de build desta sessão foi retido.

## Pendências de aceitação

- Substituir as artes provisórias e completar todas as identidades sem arte, preservando procedência e identidade.
- Executar o [protocolo físico](30-device-validation.md) no Galaxy A54, iPhone 13 e iPhone SE 2020.
- Entregar três execuções individuais de 15 minutos após aquecimento, entradas frias/aquecidas e sessão de memória de 60 minutos/100 turnos.
- Confirmar FPS, latência, tarefas longas, memória e ausência de crescimento persistente nas condições exigidas.
- Repetir orçamento, inspeção visual e validação de carregamento após incorporar a arte final.
