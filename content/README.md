# Bundles de conteúdo

O arquivo `bundles/game-one-en-v1.json` contém o Jogo 1 jogável, com 45 registros e 93 cartas físicas no catálogo.
As [regras funcionais v1](game-one-rules-v1.md) documentam quantidades, efeitos, cartas exclusivas de cada Herói, fontes e decisões da adaptação.
Cartas de Heróis ausentes permanecem fora da Partida.

Esse bundle usa schema 3 e produz manifesto 4.
O servidor inspeciona a AST validada, compila cada regra, valida o conjunto no motor e fornece separadamente a confiança na fonte versionada da adaptação.
Somente então importa e publica o manifesto jogável.
Regras sem proveniência própria ou sem suporte de execução impedem sua publicação como jogável.
A preparação deve conservar o inventário exato e incluir uma única regra automática de revelação de Artes das Trevas.

O arquivo `bundles/game-two-en-v1.json` contém o Jogo 2 cumulativo, com 63 registros e 116 cartas físicas.
Ele usa schema 4 e manifesto 5, com 44 cartas de Hogwarts, 15 Artes das Trevas, seis Vilões e três Locais que substituem os do Jogo 1.
As [regras do Jogo 2 v1](game-two-rules-v1.md) registram fontes, decisões da adaptação e efeitos completos.
A preparação valida a sequência dos Locais e mantém uma única vaga de Vilão ativo.
A confiança nas fontes é concedida externamente pelo servidor; uma lacuna funcional continua bloqueando a entrada e a publicação da Aventura.

O arquivo `bundles/game-three-en-v1.json` contém o Jogo 3 cumulativo, com 77 registros e 138 cartas físicas.
Ele usa schema 5 e manifesto 6, com 60 cartas de Hogwarts, 19 Artes das Trevas, oito Vilões e três Locais substitutos.
Cada identidade de Herói aparece somente na versão do Jogo 3, com sua habilidade implementada por uma Strategy Rust versionada.
A preparação mantém duas vagas de Vilão ativo e valida o vínculo entre identidade, versão e habilidade de cada participante.
As [regras do Jogo 3 v1](game-three-rules-v1.md) documentam as fontes, incluindo a correção do inventário comunitário que omitia Sirius Black, e as decisões funcionais.

O arquivo `bundles/base-en-candidate-2026-09-02.json` é o catálogo candidato em inglês para o jogo-base.
Ele fecha o inventário declarado em 171 registros e 252 cartas físicas.
Seu schema 2 e manifesto 3 permanecem preservados; construções introduzidas pelo Jogo 1 não são aceitas nesse schema anterior.
Promoções e expansões ficam fora deste escopo.

Cada registro usa um ID de catálogo opaco e independente do idioma.
IDs de instâncias em uma partida são representados por um tipo separado e não substituem esses IDs de catálogo.

A localização em português brasileiro segue o [glossário editorial](localization-pt-BR.md), com os termos revisados e suas referências.
Os campos revisados de `names.pt-BR` apontam para essa decisão editorial em sua proveniência.

O bundle registra proveniência por campo com links para a especificação do projeto, a página oficial do produto e implementações comunitárias fixadas por commit.
As fontes comunitárias sustentam apenas dados candidatos, como nomes e quantidades, e não promovem automaticamente regras funcionais a fatos validados.

O importador de `game-content` rejeita schemas desconhecidos, inventário fora do escopo, proveniência ausente, IDs duplicados, referências quebradas, ciclos de regras, escolhas abertas, cardinalidades inválidas e operações incompatíveis com suas zonas.
O tipo de cada registro determina os campos funcionais obrigatórios, portanto o produtor não pode omitir a lista para promover conteúdo incompleto.
Uma definição funcional só é comprovada quando sua confiança corresponde ao tipo fechado da fonte e referencia uma regra declarativa existente.
Antes de calcular o digest BLAKE3, ele ordena as coleções sem ordem semântica para produzir uma representação canônica.

O catálogo candidato permanece intencionalmente não jogável.
Custos, efeitos, recompensas, habilidades, setup e precedência ainda aparecem como lacunas quando não possuem fonte validada ou regra explícita de adaptação.
Essas lacunas são publicadas no manifesto sem inferir regras ausentes.

O snapshot 5 preserva a ordem de todas as pilhas, os sorteios da preparação e os bloqueios de compra ainda ativos.
Seu histórico admite o encerramento anterior, Artes das Trevas, Vilões e ações do Herói.
O evento 6 registra os novos efeitos e os sorteios de cada reembaralhamento de fim de Turno.
A migração `0021_game_one.sql` valida essas formas e suas transições no PostgreSQL, sem reescrever snapshots, eventos ou manifestos anteriores.
Os codecs anteriores continuam disponíveis para leitura e rejeitam campos com semântica exclusiva do Jogo 1.

O Jogo 2 usa snapshot 6 e evento 7, com a cópia de Aliados persistida até o fim do Turno.
O transporte mantém o contrato público de evento 6 para os clientes atual e anterior.
O encerramento é transmitido por Snapshot, pois pode interromper o Turno antes do mínimo de etapas aceito pelo contrato de evento anterior.
O vínculo interno de cópia não integra os resumos HTTP e WebSocket; alvos, efeitos copiados, Escolhas e estado resultante continuam disponíveis nos campos existentes.
A migração `0023_game_two.sql` valida a preparação cumulativa, as cópias e as transições correspondentes.
O Jogo 1 conserva seu bundle, manifesto, digest, snapshot 5 e evento 6.
Os codecs anteriores rejeitam campos exclusivos do Jogo 2.
O Jogo 3 usa snapshot 7 e evento 8, com limites das habilidades, Ataque atribuído e bloqueios de Vilões persistidos explicitamente.
A migração `0025_game_three.sql` valida esses estados, as revelações do topo e suas transições, preservando os validadores das versões anteriores.
O transporte conserva o evento público 6 e a forma da projeção aceita pelo cliente anterior.
Descrições dos Vilões apresentam os bloqueios e a cota de Ataque; a descrição da fonte identifica o Herói e a carta revelada.
As Escolhas das habilidades usam os campos existentes de origem e instrução, enquanto o estado consumido permanece no snapshot canônico.
O início do Turno renova os limites e expira os bloqueios correspondentes antes das Artes das Trevas.
Os Jogos 1 e 2 conservam seus bundles, manifestos, digests e codecs.

Os cenários de navegador dos quatro Jogos exercitam vitória e derrota com 2, 3 e 4 participantes, e suas transcrições possuem goldens de eventos e Snapshots.
Os Jogos 3 e 4 também percorrem, cada um, 36 partidas com seeds variadas, verificando conservação do inventário, limites de recursos, unicidade das Escolhas e equivalência de execução, restauração e replay a cada Comando.

Aplicações de teste podem substituir a fonte de seed na construção de `AppState` para reproduzir uma Partida pelo caminho HTTP real.
A aplicação de produção usa a entropia do sistema operacional e não recebe uma seed em requisições.

O Jogo 4 publica `game-four-en-v1` com schema 6, manifesto 7 e ruleset `game-four-v1`.
Seu inventário cumulativo contém 98 registros e 169 cartas, incluindo 21 novas cartas de Hogwarts, oito Artes das Trevas, dois Vilões e três Locais substitutos.
As [regras funcionais do Jogo 4](game-four-rules-v1.md) documentam fontes, adaptações e as seis faces ordenadas de cada dado de Casa.
A preparação conserva as habilidades de Herói do Jogo 3 e revela dois Vilões.

O snapshot 8 e o evento 9 preservam os lançamentos de Casa com propósito, contador, dado versionado, intervalo e resultado.
A migração `0026_game_four.sql` valida os novos estados, as Escolhas no fim de Turno e o acréscimo exato de cada evento ao histórico de lançamentos.
A retomada continua a fila persistida sem consumir novamente resultados anteriores.
O transporte conserva o contrato público de evento 6, representando cada dado como um d6 e mantendo a regra de origem e a face obtida.
Transições com etapas ou fases incompatíveis com o contrato anterior são transmitidas por Snapshot.
A apresentação 3D adicional dos dados está prevista na issue #38.
Os Jogos 1, 2 e 3 conservam seus bundles, digests e codecs.

Os cenários do Jogo 4 fixam as seeds de vitória por tamanho de equipe em `apps/server/tests/fixtures/game-four/scenario-seeds.json`.
O executável `e2e_harness` permite escolher essa seed por requisição, sem compartilhar estado entre cenários concorrentes.
Esse controle existe somente no harness de testes; a aplicação de produção continua sem aceitar seeds do cliente.
