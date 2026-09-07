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
A confiança nas fontes dos dois Jogos é concedida externamente pelo servidor; uma lacuna funcional continua bloqueando a entrada e a publicação da Aventura.

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
A migração `0022_game_two.sql` valida a preparação cumulativa, as cópias e as transições correspondentes.
O Jogo 1 conserva seu bundle, manifesto, digest, snapshot 5 e evento 6.
Os codecs anteriores rejeitam campos exclusivos do Jogo 2.
Os cenários de navegador dos dois Jogos exercitam vitória e derrota com 2, 3 e 4 participantes, e suas transcrições possuem goldens de eventos e Snapshots.

Aplicações de teste podem substituir a fonte de seed na construção de `AppState` para reproduzir uma Partida pelo caminho HTTP real.
A aplicação de produção usa a entropia do sistema operacional e não recebe uma seed em requisições.
