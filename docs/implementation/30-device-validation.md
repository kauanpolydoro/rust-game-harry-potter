# Protocolo físico da mesa 3D

Este roteiro implementa os critérios da [issue #30](https://github.com/kauanpolydoro/rust-game-harry-potter/issues/30).
Não há resultados físicos preenchidos neste documento.

## Preparação

Use o build de produção, servido em HTTPS com compressão dos arquivos JS/CSS/HTML/JSON.
A configuração de proxy/headers do deploy deve ser registrada porque influencia a transferência e a entrada aquecida.
A mesa de teste pode ser aberta em `/prototype.html?table-diagnostics=1` para reproduzir interações visuais; a validação de comandos e turnos deve usar a partida real em `/?table-diagnostics=1`.
Nunca apresente o protótipo como evidência de regras ou sincronização real.

Registre commit, aparelho e modelo exato, sistema operacional, navegador e versão, viewport, densidade de pixels, perfil selecionado e qualidade efetivamente aplicada.
Registre temperatura ambiente e do aparelho quando disponível, nível de bateria, carregador conectado ou não, economia de energia e condição de rede.
Use apenas a aba da partida durante a medição de memória do processo e descreva exceções.
Mantenha os outros jogadores em clientes separados para permitir decisões reais sem automação do DOM da mesa medida.

O menu Imagem e som exibe Iniciar medição e Exportar diagnóstico somente quando o parâmetro `table-diagnostics` está presente.
A exportação contém distribuições limitadas de duração de frames, espera até o callback de animação após cliques, chamada do handler de comandos e tarefas longas quando o navegador oferece a API.
Há contadores de meshes, texturas e áudio, estimativa conservadora de bytes de texturas e heap JS quando disponível.
Os histogramas têm resolução de 0,25 ms e uma faixa de overflow; percentis são limites superiores do intervalo.
As condições físicas precisam ser preenchidas pelo operador no campo `deviceConditions` e acompanhadas dos registros do sistema.
Métrica indisponível aparece como `null` e permanece pendente, sem ser interpretada como zero.

## Três execuções por aparelho

Execute três rodadas independentes em Galaxy A54 e iPhone 13 no perfil de 60 FPS e em iPhone SE 2020 no perfil Básico de 30 FPS.
Não aprove a média de rodadas quando uma rodada individual falhar.

1. Aqueça a mesa por dois minutos com interações representativas.
2. Acione Iniciar medição e mantenha pelo menos 15 minutos de jogo ativo.
3. Execute pelo menos 200 gestos de seleção/confirmação, distribuídos durante a rodada.
4. Inclua jogar cartas, escolha de alvos, recursos, dano, cura, compra, descarte, atordoamento, Vilão derrotado, perda de Local e conclusão de partida.
5. Grave ao menos um turno completo com os efeitos e os recursos oficiais visíveis.
6. Exporte o diagnóstico antes de sair da mesa e anexe gravação e condições físicas.

Para A54 e iPhone 13, exija mediana de pelo menos 58 FPS e p95 de duração de frame de até 22 ms.
No Básico do SE 2020, exija alvo de 30 FPS e p95 de até 35 ms.
Calcule FPS mediano como `1000 / frameDistribution.medianMs` e use `frameDistribution` para o período inteiro; os campos resumidos `medianFrameMs` e `p95FrameMs` mostram apenas a janela recente.
Exija p95 de resposta local e envio de até 100 ms, usando trace com timestamps de input, apresentação do frame e início da requisição.
`inputToAnimationFrameCallback` e `inputToCommandHandler` são aproximações úteis para triagem; não incluem necessariamente composição/pintura ou o início real da requisição HTTP e não aprovam sozinhas esse critério.
A distribuição de frames acumula fora da cena e preserva os segmentos anteriores quando há recuperação gráfica; `sceneSegments` registra quantas cenas participaram da rodada.
Registre toda tarefa da thread principal acima de 50 ms e investigue qualquer uma acima de 200 ms.
Use o profiler do navegador quando Long Tasks não estiver disponível, especialmente Safari.

## Entradas e condições adversas

Meça três entradas frias, limpando somente o cache do cliente de teste entre tentativas, e três aquecidas com o cache preservado.
Use 10 Mbit/s e RTT de 150 ms, registrando o mecanismo de limitação e seus valores efetivos.
Conte da navegação até cartas visíveis e selecionáveis, com estado sincronizado quando houver partida real.
Exija até dez segundos para cada entrada fria e até três segundos para cada entrada aquecida.
Anexe HAR ou relatório de rede com bytes transferidos; não derive a duração apenas do tamanho do bundle.

Verifique seleção e confirmação durante efeitos, movimento reduzido, perfis automático/manual, rotação durante arraste, troca de aba e retomada.
Provoque perda/restauração de contexto pelo inspetor e confirme continuidade da sessão e ausência de comandos duplicados no registro de rede.
Bloqueie uma arte, o GLB e o worker KTX2 em execuções separadas e confirme identidade, texto e seleção disponíveis.
Ative o áudio por gesto, altere os dois volumes, oculte e retome a página e confirme ausência de reprodução do acúmulo de acontecimentos.

## Memória

Execute pelo menos 60 minutos e 100 turnos, mantendo o perfil registrado e comparando estados equivalentes de cartas visíveis, página da Mão, modo e cena.
Se uma partida terminar antes, registre explicitamente a troca e repita uma sequência equivalente de estados; não compare uma cena desmontada com uma cena ativa.
Colete exportações e medições do processo nos minutos 5, 10, 55 e 60.
Capture memória do processo pelas ferramentas do sistema e memória gráfica pelas ferramentas disponíveis no aparelho.

Exija heap JS até 120 MiB, processo até 250 MiB e texturas até 64 MiB no perfil Equilibrado.
Compare os intervalos 5-10 e 55-60 minutos e rejeite crescimento persistente superior a 10% em estados equivalentes.
Anexe números brutos, ferramenta, método de coleta e observações sobre GC.
A estimativa de texturas do diagnóstico usa dimensões RGBA e mipmaps, sendo um limite conservador; ela não mede a alocação real do driver.
Memória sem API ou ferramenta disponível continua pendente.

## Registro por execução

| Campo | Valor a preencher |
| --- | --- |
| Commit, data e operador | Pendente |
| Aparelho, SO e navegador | Pendente |
| Perfil selecionado e aplicado | Pendente |
| Energia, bateria e temperatura | Pendente |
| Rede e headers de compressão/cache | Pendente |
| Aquecimento, duração ativa e gestos | Pendente |
| FPS mediano, frame p95 e resposta local p95 | Pendente |
| Tarefas acima de 50 ms e 200 ms | Pendente |
| Heap, processo e texturas | Pendente |
| Entradas frias/aquecidas individuais | Pendente |
| Crescimento em 60 minutos/100 turnos | Pendente |
| Diagnósticos, traces e gravação | Pendente |
| Aprovação individual e desvios | Pendente |
