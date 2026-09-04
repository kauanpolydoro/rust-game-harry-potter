# Batalha de Hogwarts

Fatia executável mobile-first do jogo cooperativo descrito na [spec #1](https://github.com/kauanpolydoro/rust-game-harry-potter/issues/1).

Este primeiro incremento entrega PostgreSQL, backend Rust e shell Vue sob um único fluxo reproduzível.

## Pré-requisitos

- Docker com Compose.

- Node.js 24.18.0 ou mais recente.

- Rustup, que instala automaticamente o Rust 1.98.0 fixado em `rust-toolchain.toml`.

## Executar

Em um checkout limpo, execute:

```bash
make dev
```

O comando instala as dependências fixadas, inicia o PostgreSQL, aplica migrations pelo backend e abre os servidores de desenvolvimento.

A interface fica em `http://127.0.0.1:5173` e apresenta explicitamente os estados pronto e indisponível do serviço autoritativo.

Interrompa com `Ctrl+C`.

O PostgreSQL permanece no volume local do Compose entre execuções.

O WebSocket autenticado aceita somente a origem exata configurada em `APPLICATION_ORIGIN`.

Em desenvolvimento, o valor padrão é `http://127.0.0.1:5173`.

## Validar

Instale o Chromium do Playwright uma vez no ambiente local:

```bash
npx playwright install chromium
```

Depois execute todos os gates:

```bash
make check
```

O gate cria um banco temporário isolado para validar migrations desde zero e o remove ao terminar.

Ele executa formatação, Clippy, testes Rust, limites de módulos, geração de contratos, lint, typecheck, testes Vue, build, Playwright e secret scan.

Valide separadamente os SLOs de reconexão no perfil de referência:

```bash
make check-reconnect-profile
```

Esse perfil usa 100 partidas, 400 WebSockets, 20 Comandos por segundo, RTT simulado de 150 ms e 1% de perda para medir o p95 de replay e Snapshot.

## Endpoints de saúde

- `GET /health/live` confirma que o processo HTTP responde, mesmo antes da inicialização.

- `GET /health/startup` abre somente depois das migrations.

- `GET /health/ready` abre somente depois do startup e de uma consulta bem-sucedida ao PostgreSQL.

Readiness tem timeout de um segundo e não mascara falhas de banco como disponibilidade.

## Estrutura

- `crates/game-content` importa bundles declarativos, valida publicação em modo fechado e produz manifestos canônicos com digest BLAKE3.

- `crates/game-domain` contém somente regras puras e não pode importar infraestrutura.

- `apps/server` contém os adapters HTTP e PostgreSQL.

- `apps/web` contém o cliente Vue e considera oficial apenas o que chega pelos contratos do servidor.

- `contracts` é a fonte canônica para artefatos gerados compartilhados.

- `content/bundles` contém os catálogos candidatos e sua proveniência por campo.

O gate `scripts/check-boundaries.mjs` impede dependências de infraestrutura no módulo de domínio.

## Expiração da Partida

Uma Ação oficial aceita define `last_game_action_at` pelo relógio do PostgreSQL e `expires_at` exatamente 168 horas depois, usando uma única observação.
Leituras, presença, reconexão, retry, erro e operações de recuperação ou proteção não renovam esse prazo.
Comandos e recuperação revalidam a expiração após obter o lock da raiz, com rejeição no instante exato do limite.

A migration `0020_game_expiration.sql` registra a expiração de acesso de forma durável e encerra as Sessões ativas e leases da Partida.
Cada instância verifica até 100 candidatas por segundo com `SKIP LOCKED`, e a decisão final sempre consulta `clock_timestamp()` depois do lock.
A autenticação também aplica esse gate sob demanda.
O processamento é idempotente, notifica as outras instâncias após o commit e não modifica o histórico oficial.
Purge, ledger de Tombstones e política de backup pertencem aos tickets posteriores de ciclo de vida.

HTTP responde `GAME_EXPIRED` sem projeção privada e apaga o cookie da Sessão.
Recuperação preserva o erro genérico `RECOVERY_FAILED`.
Os canais de jogo e segurança fecham com código privado `4001`, levando o cliente à tela de Partida expirada e removendo stores privados, credenciais exibidas, animações e intenções pendentes.
Respostas em voo de uma geração anterior de Sessão são descartadas.
Uma aba suspensa confirma a Sessão ao voltar a ficar visível, e uma falha de reconexão também consulta o endpoint HTTP.

Os testes usam PostgreSQL real para locks, revogação, relógio corrente e concorrência.
O parâmetro opcional de observação de `expire_game_access` permite exercitar o corte em menos um, igual e mais um milissegundo, sem depender de acertar essa janela pela rede.
Chamadas da aplicação omitem esse parâmetro.
Playwright cobre dois navegadores e uma segunda aba, incluindo um Comando pendente, fechamento dos canais, remoção do cookie, recarga e rejeição da recuperação.
