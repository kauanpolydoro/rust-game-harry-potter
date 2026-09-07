# Fronteiras de segurança

Esta política implementa a issue #34 e as decisões de segurança da especificação #1.
O navegador, mensagens, URLs, headers e dados enviados pelo cliente são entradas não confiáveis.
As decisões de jogo, identidade e dispositivo continuam no servidor e nas transações PostgreSQL.

## HTTP e identidade

Toda mutação exige exatamente uma `Origin` igual a `APPLICATION_ORIGIN`, exatamente um `x-csrf-protection: 1` e JSON estrito.
O header personalizado é a proteção CSRF para esta API, em conjunto com CORS fechado e a checagem de Origin, conforme a [orientação da OWASP](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html#employing-custom-request-headers-for-ajaxapi).
Formulários simples e requisições sem o header são recusados; um site externo não consegue adicioná-lo sem preflight, que não recebe autorização CORS.
Esse header não é uma credencial e não substitui a autenticação.

Criar sala e entrar numa sala aberta estabelecem uma identidade nova, usando a chave imprevisível de concessão já prevista no protocolo.
Recuperar uma identidade exige credencial individual e senha da sala.
As demais mutações derivam o ator da sessão server-side e revalidam autorização e estado sob o lock pertinente.
`participant_id`, papel, recursos, estado ou dispositivos enviados fora do schema não concedem autoridade.
Replays continuam sujeitos à autorização, à expiração e às regras de idempotência existentes.

Cookies de sessão são opacos, `__Host-`, `Secure`, `HttpOnly`, `SameSite=Strict` e `Path=/`, sem `Domain`.
Cookies de sessão duplicados são rejeitados.
Quem possui um cookie válido ainda possui uma credencial bearer até sua revogação, substituição ou expiração.
A proteção não depende de fingerprinting nem de vincular identidade ao IP.

O JSON de entrada tem limite de 16 KiB, incluindo corpos sem `Content-Length`.
Caminho mais query têm limite de 2 KiB e os headers recebidos têm limite agregado de 16 KiB na aplicação.
Tipos, campos desconhecidos e estruturas de comandos permanecem estritos.
Rejeições de JSON e query usam um envelope estável sem reproduzir nomes de campos, valores ou detalhes do parser.

## WebSocket

Os dois canais exigem sessão válida, Origin exata e um subprotocolo suportado.
Queries são estritas, frames e mensagens são limitados a 4 KiB no jogo e 1 KiB no canal de segurança.
Texto e dados binários enviados pelo cliente encerram o canal com código 1003 e nunca viram comandos.
Mensagens de controle revalidam a sessão e compartilham um limite de 60 mensagens por minuto por sessão entre canais e abas.
Excesso fecha o canal com código 1008 antes de novo trabalho de presença.
Revogação, substituição e expiração preservam as cercas transacionais e o fechamento de canais existentes.

## Recuperação e abuso

Os orçamentos abaixo usam janelas de 60 segundos por processo, compartilhadas entre clones de `AppState`.
Chaves em memória são HMACs separados por finalidade; IP, cookie, token e senha não são guardados nos buckets nem registrados em logs.
Há no máximo 8.192 buckets e a admissão de uma chave nova falha quando a capacidade está cheia.
Entradas vencidas são removidas durante a admissão de novas requisições.

| Fronteira | Orçamento |
| --- | --- |
| Criação e operações de senha por peer TCP, em conjunto | 60/min |
| Descoberta e entrada em salas por peer TCP, em conjunto | 60/min |
| Demais rotas da API por peer TCP | 600/min |
| Mesma chave de criação de sala | 10/min |
| Mesma credencial de recuperação, incluindo tentativas com IDs novos | 10/min |
| Verificações de senha de recuperação da mesma sala | 20/min |
| Mensagens WebSocket da mesma sessão | 60/min |

O peer vem de `ConnectInfo<SocketAddr>` da conexão aceita pelo servidor.
`Forwarded` e `X-Forwarded-For` enviados pelo cliente não mudam os limites.
Sem informação de conexão, a admissão usa um único bucket conservador.
Atrás de um proxy, o peer observado é o próprio proxy e o orçamento por peer é agregado.
Os limites não são distribuídos: com N processos o teto agregado pode chegar a N vezes o orçamento, e reinícios descartam as janelas em memória.
A configuração de ingresso e capacidade de uma implantação deve considerar esses valores; este trabalho não configura infraestrutura nem um serviço de rate limit externo.

Todo hash e verificação Argon2id, incluindo criação, replay, rotação, proteção e equalização de recuperação inválida, passa pela mesma fila.
Há quatro trabalhos admitidos no executor de bloqueio e até oito aguardando vaga por no máximo um segundo.
Os permits acompanham o trabalho no executor até sua conclusão, mesmo que o cliente cancele a requisição.
Erros por credenciais inválidas mantêm o mesmo envelope e executam Argon2 também quando a credencial não existe.
Excesso retorna 429 com `Retry-After: 60`, sem conceder sessão nem revelar qual limite foi atingido na recuperação.
A tela de recuperação orienta aguardar um minuto e preserva a tentativa para reenvio com o mesmo link.
Não há promessa de tempo constante de ponta a ponta; banco, escalonamento e transporte têm variação.

## Headers, navegador e implantação

Todas as respostas da API, incluindo erros e fallbacks, recebem `no-store`, `no-referrer`, `nosniff`, `X-Frame-Options: DENY` e CSP sem carregamento de recursos.
O documento web, incluindo a recuperação por fragmento, recebe `no-store`, `no-referrer` e uma CSP própria que permite apenas scripts, estilos, fontes, imagens e manifest do mesmo origin e conexão com o próprio serviço.
Ambas as políticas proíbem frames ancestrais, objetos e base URL; formulários só podem usar o mesmo origin.
O bootstrap externo remove o fragmento antes do módulo da aplicação.
A política de produção não autoriza inline nem eval.
Somente o servidor de desenvolvimento usa um nonce de estilo para o HMR do Vite, ausente do build e do preview.
CORS está desativado explicitamente tanto no desenvolvimento quanto no preview.

Configure o mesmo `APPLICATION_ORIGIN` no servidor e no serviço que entrega o documento web.
Clientes carregados antes da exigência do header CSRF precisam recarregar o shell atualizado para voltar a enviar mutações.
O preview aplica os headers sobre os artefatos compilados; outro servidor de assets deve preservar a mesma política.
Produção exige HTTPS/WSS no ingresso e acesso direto ao backend restrito à rede de serviço.
HSTS deve ser habilitado no ingresso quando o domínio estiver estabilizado, conforme a especificação.
O ambiente local usa loopback HTTP para desenvolvimento e testes.

## Logs e evidências

O subscriber de produção aceita somente targets da aplicação, mesmo com `RUST_LOG=trace`.
Os testes usam o mesmo construtor de subscriber, com saída capturada e verbosidade `trace`.
Isso impede que logs internos de bibliotecas exponham frames, queries ou detalhes de rejeições.
O trace HTTP usa correlation ID gerado pelo servidor, método de uma lista fechada, template da rota e status.
Rotas desconhecidas recebem o identificador fixo `unmatched`.
Erros de banco, parser e transporte não são formatados nos logs da aplicação.
Mensagens de fechamento do cliente não são registradas.
Identificadores de recursos emitidos pelo servidor e nomes estáticos de operações continuam disponíveis para diagnóstico.

| Critério da issue | Evidência automatizada |
| --- | --- |
| Origin, CSRF, schema, tamanho e headers | `security_boundaries_http.rs` e `security-boundaries.spec.ts` |
| Limites e fila Argon2id sob saturação/cancelamento | `recover_participation_http.rs` |
| Cliente adulterado, replay e autoridade do ator | `start_game_http.rs`, `create_room_http.rs` e Playwright |
| Recuperação isolada, terceiro dispositivo e revogação | `recover_participation_http.rs`, `manage_recovery_http.rs` e `access_revocation_http.rs` |
| Expiração e limpeza do cliente | Testes existentes de expiração em `start_game_http.rs` e `game-expiration.spec.ts` |
| Ausência de dados privados em logs | Canários em path, query, método, headers, JSON, exceção PostgreSQL e frames/close WebSocket |

Os testes de banco usam PostgreSQL real.
O cenário de logging PostgreSQL instala uma falha controlada em schema isolado e confirma que o erro não copia a linha privada para resposta ou log.
O cenário de saturação ocupa o executor real e cancela as requisições para verificar que Argon2 continua contabilizado.
`make check` permanece o gate local obrigatório; o CI remoto continua desativado.

### Pendência de desempenho observada

Na validação de 2026-09-07, `make check` passou, incluindo os 21 cenários de navegador.
O perfil separado de reconexão preparou 100 jogos com clientes distintos e limites reais, mas excedeu os SLOs de replay p95 de 3 segundos e Snapshot p95 de 5 segundos.
No PostgreSQL isolado, o commit de segurança `7a72977` mediu 7.137 ms e 6.531 ms, respectivamente.
O commit-base `26bf6cabec8e1fff0b2f31e75d9f2bc65d06258c`, sem alterações e no mesmo ambiente, também falhou, com 8.130 ms e 7.613 ms.
Essas execuções confirmam uma violação preexistente dos SLOs; não estabelecem uma comparação estatística de desempenho.
O impacto é a recuperação mais lenta durante a reconexão simultânea de 400 canais.
O próximo passo é reproduzir o perfil com CPU e memória controladas e medir handshake, autorização, presença e consultas PostgreSQL para localizar o gargalo, preservando os SLOs atuais.
