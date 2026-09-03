# Bundles de conteúdo

O arquivo `bundles/base-en-candidate-2026-09-02.json` é o catálogo candidato em inglês para o jogo-base.
Ele fecha o inventário declarado em 171 registros e 252 cartas físicas.
Promoções e expansões ficam fora deste escopo.

Cada registro usa um ID de catálogo opaco e independente do idioma.
IDs de instâncias em uma partida são representados por um tipo separado e não substituem esses IDs de catálogo.

O bundle registra proveniência por campo com links para a especificação do projeto, a página oficial do produto e implementações comunitárias fixadas por commit.
As fontes comunitárias sustentam apenas dados candidatos, como nomes e quantidades, e não promovem automaticamente regras funcionais a fatos validados.

O importador de `game-content` rejeita schemas desconhecidos, inventário fora do escopo, proveniência ausente, IDs duplicados, referências quebradas, ciclos de regras, escolhas abertas, cardinalidades inválidas e operações incompatíveis com suas zonas.
O tipo de cada registro determina os campos funcionais obrigatórios, portanto o produtor não pode omitir a lista para promover conteúdo incompleto.
Uma definição funcional só é comprovada quando sua confiança corresponde ao tipo fechado da fonte e referencia uma regra declarativa existente.
Antes de calcular o digest BLAKE3, ele ordena as coleções sem ordem semântica para produzir uma representação canônica.

O catálogo candidato permanece intencionalmente não jogável.
Custos, efeitos, recompensas, habilidades, setup e precedência ainda aparecem como lacunas quando não possuem fonte validada ou regra explícita de adaptação.
Essas lacunas são publicadas no manifesto sem inferir regras ausentes.
