# Runbook: solicitar sincronização com o upstream (app X)

Este documento é para quando você pedir: *"sinca o app X com as mudanças do storytold/<appcraft>"*.  
Ele define o que eu vou perguntar a você, os comandos que rodei, os pontos de decisão e o que **não** entra automaticamente.

---

## 1. Antes de começar — o que vou perguntar (responda só o que for diferente do default)

| Pergunta | Default | Quando mudar |
|----------|---------|--------------|
| Qual app? (ex: local-sheets, local-slides) | — | sempre |
| Qual repo origem? | `github.com/storytold/<appcraft>` | se for outro dono/branch |
| Versão do upstream que é a baseline do rebrand? | o commit que usamos como cópia inicial | se quiser sincronizar a partir de um ponto mais novo/antigo |
| Tem alguma mudança sua não-commitada no app X? | não | se tiver, precisamos commitar ou stashar primeiro |
| O que trazer do upstream? | tudo que casar com o `UPSTREAM-MAP.toml` | se quiser apenas correções específicas (ex: só segurança) |
| Abrir Issue/PR de volta no upstream se encontrarmos bug compartilhado? | sim | se quiser apenas consumir sem contribuir |

---

## 2. Fluxo de sincronização (o que faço, passo a passo)

Tudo ocorre num branch dedicado `sync/upstream-<app>-<YYYYMMDD>`, **nunca direto no `main`**.

### Fase 0 — Preparação
1. Criar remote `upstream` (se ainda não existir):
   ```
   git remote add upstream https://github.com/storytold/<appcraft>.git
   git fetch upstream
   ```
2. Ver commits novos desde a baseline:
   ```
   git log --oneline <baseline>..upstream/main
   ```
3. Listar arquivos tocados:
   ```
   git diff --name-only <baseline> upstream/main
   ```

### Fase 1 — Classificação (usando docs/UPSTREAM-MAP.toml do app)
Cada arquivo tocado vira **aplicar** / **revisar** / **pular**:

- **aplicar**: script tenta `git diff ... -- <arquivo> \| git apply` direto.
- **revisar**: eu baixo o diff, leio, testo e aplico só se fizer sentido para o Local.
- **pular**: ignoro (arquivo no mapa com `strategy = "skip"`).

### Fase 2 — Pontos de bloqueio (nunca passo sozinho)

Se qualquer um desses ocorrer, **paro e pergunto a você**:

- 🛑 **Nova dependência no `Cargo.toml`/`Cargo.lock`** (ex: o wordcraft adicionou `url` no #4652). Não entra por sync automático. Decidimos juntos: adicionar com `cfg` apropriado, registrar no `local-deps` (DEPENDENCIES.md + CHANGELOG-deps.md), ou pular/reimplementar sem a dep.
- 🛑 **Conflito em `packaging/`, `.github/workflows/`, `Cargo.toml` do workspace**. Nosso CI e empacotamento são próprios.
- 🛑 **Mudança de nomes de crate, bundle id, ou marca** (ex: `ai.storyteller.*` → `com.local.*`). É nosso rebrand; não puxamos.
- 🛑 **Teste falha após aplicar**. Não forço. Vou te mostrar o erro e decidir se ajustamos ou descartamos.

### Fase 3 — Verificação
Rodo e te mostro o resultado:
```
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked          (suítes afetadas)
cargo run -q -p xtask -- assets   (gate do ATTRIBUTION.md)
cargo clean                  (nunca deixo target/ para trás)
```

### Fase 4 — Merge e registro
Se tudo verde e você aprovar:
1. Merge no `main`.
2. Registro em `docs/UPSTREAM-SYNC.md` (Decision Ledger): data, PR upstream, o que entrou, o que ficou de fora, por quê.
3. Se encontrarmos bug compartilhado: abro Issue/PR no `storytold/<appcraft>` e registro em `docs/UPSTREAM-CONTRIBUTIONS.md`.

---

## 3. O que NÃO entra no sync automático (hard rules)

- Qualquer nova dependência de crate (mesmo que seja só "uma linha"). É decisão de negócio de independência (WORKFLOW.md).
- Workflows da org `storytold` (`.github/workflows/*`).
- Marca/trademark (`docs/brand/*`, `getartcraft.com`, Discord).
- `ROADMAP.md`, `docs/gaps.md` — são do WordCraft vs Word, não do Local.
- `craft-i18n.rev` — nossos catálogos já são in-repo (`i18n/locales/`).

---

## 4. Exemplo de registro (Decision Ledger)

```markdown
| Data | App | PR upstream | O que entrou | O que ficou de fora | Por quê |
|------|-----|-------------|--------------|---------------------|---------|
| 2026-10-28 | local-office | #4652 | followable_url + dep url 2.5 (desktop) | ROADMAP.md, Cargo.toml workspace | segurança de link; dep nova registrada no local-deps |
| 2026-10-28 | local-office | #4632 | cap de 4096 REP entries | docs/gaps.md | desempenho do corretor, sem risco |
```

---

## 5. Template da sua solicitação

Quando quiser pedir, pode usar este modelo (copiar e preencher):

```
App: local-sheets (ou local-slides, etc.)
Repo upstream: github.com/storytold/<appcraft> (ou outro)
Baseline do rebrand: <commit SHA ou "o que já usamos">
O que trazer: tudo que casar com o UPSTREAM-MAP / apenas correções específicas
Contribuir de volta se achar bug compartilhado: sim/não
```

Com isso, eu rodei o sync, te mostro o relatório (aplicou X / revisar Y / pular Z) e espero sua confirmação antes de tocar no `main`.

---

## 6. Arquivos envolvidos (referência)

- `docs/UPSTREAM-SYNC.md` — plano e princípios (já existe em cada app rebrandado).
- `docs/UPSTREAM-MAP.toml` — contrato de quais caminhos podem ser aplicados automaticamente.
- `docs/UPSTREAM-CONTRIBUTIONS.md` — registro de Issues/PRs abertos no upstream.
- `local-deps/WORKFLOW.md` — regra inegociável de dependências (novo crate = inventário + changelog).
