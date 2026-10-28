# Sincronização com o upstream (storytold/wordcraft)

## Objetivo
Manter o `local-office` próximo do `storytold/wordcraft` (upstream) sem perder a independência do rebranding: nenhum merge cego, nenhuma dependência de servidor da org deles, nenhum `git+` no lock. Tudo que entrar no main deste repo passa por revisão de arquitetura primeiro.

## Princípios
1. **O main é nosso.** O upstream é uma fonte de mudanças candidatas, nunca um destino automático.
2. **Revisão humana obrigatória.** Nada entra no `main` sem passar pela revisão de arquitetura (Phase 3 / Architecture Gate).
3. **Mapeamento de renames é explícito.** Todo nome/caminho do wordcraft que difere do local-office fica registrado em `docs/UPSTREAM-MAP.toml` (ver abaixo) para que um script tente aplicar sozinho e pare onde não conseguir.
4. **Puxamos apenas o que é nosso.** Não puxamos branding, nomes de marca, links `getartcraft.com`, Discord, `docs/brand/`, ou CI que dependa de workflows compartilhados da org `storytold`.
5. **Contribuímos de volta.** Quando resolvemos um bug que também existe upstream, abrimos Issue/PR no `storytold/wordcraft` (em inglês, como eles usam). Registramos em `UPSTREAM-CONTRIBUTIONS.md`.

## O que puxar vs o que não puxar

| Categoria | Puxar? | Motivo / exemplo |
|-----------|--------|------------------|
| Correção de segurança (ex: link followable, parser) | ✅ Sim | `crates/doc/src/link.rs`, `crates/proof/` |
| Correção de corrupção / perda de dados | ✅ Sim | Engine, formatos `.docx`/`.odt` |
| Crash / travamento | ✅ Sim | UI, engine, print |
| Performance crítica (O(n²) → O(n)) | ✅ Sim | Layout, busca, indexação |
| Bug de ortografia/gramática compartilhado | ✅ Sim | `crates/proof/`, dicionários |
| Nova feature de UI / ribbon específico | ❌ Não | É do wordcraft; nosso rebrand tem UI própria |
| Mudança de nomes / bundle id / marca | ❌ Não | É nosso rebrand; conflito garantido |
| Dependências novas / atualizações de crates | ⚠️ Caso a caso | Só via regra `local-deps/WORKFLOW.md` |
| CI / workflows da org storytold | ❌ Não | Nosso CI é próprio (`.github/workflows/ci.yml`) |

## Fluxo de sincronização (por app)

1. **Criar remote upstream** (uma vez):
   ```
   git remote add upstream https://github.com/storytold/wordcraft.git
   git fetch upstream
   ```
2. **Branch de sincronização** (nunca direto no `main`):
   ```
   git checkout -b sync/upstream-<YYYYMMDD>
   ```
3. **Tentar aplicar mudanças** com base em `docs/UPSTREAM-MAP.toml`:
   - Arquivos mapeados 1:1 (caminho igual): `git cherry-pick` ou `git diff upstream/main..<commit> -- <caminho> | git apply`.
   - Arquivos com rename de crate/caminho: aplicar via script que lê o mapa e tenta traduzir.
   - Arquivos sem mapeamento: parar, relatar, decidir manualmente.
4. **Testar**:
   - `cargo check --workspace --locked`
   - `cargo clippy --workspace --all-targets --locked -- -D warnings`
   - `cargo test --locked` (ao menos as suites afetadas)
   - `cargo run -q -p xtask -- assets` (gate de ATTRIBUTION)
5. **Revisão de arquitetura** (Phase 3): trade-off, padrões, release flow.
6. **Merge no `main`** só se tudo verde e aprovado.
7. **Registro** em `docs/UPSTREAM-SYNC.md` (este arquivo): data, PR upstream, o que entrou, o que ficou de fora, por quê.

## Arquivo de mapeamento: `docs/UPSTREAM-MAP.toml`

Formato proposto (criar quando o primeiro sync for feito):

```toml
# Cada entrada traduz um caminho do upstream para o caminho no local-office.
# paths ausentes do mapa = "não tentar aplicar automaticamente".

[[map]]
# ex.: o mesmo caminho nos dois repos
upstream = "crates/doc/src/link.rs"
local = "crates/doc/src/link.rs"
strategy = "apply"   # apply | review | skip

[[map]]
upstream = "crates/ui-egui/src/lib.rs"
local = "crates/ui-egui/src/lib.rs"
strategy = "review"

[[map]]
upstream = "packaging/windows/wordcraft.wxs"
local = "packaging/windows/localoffice.wxs"
strategy = "skip"   # nosso packaging é independente

[[map]]
upstream = "Cargo.toml"
local = "Cargo.toml"
strategy = "skip"   # deps e nomes de crates diferem; nunca aplicar cego
```

## Contribuições de volta para o upstream

Quando resolvemos um bug no `local-office` que também existe no `wordcraft`:

1. Reproduzir no `storytold/wordcraft` (clone separado, sem tocar no nosso main).
2. Abrir Issue no `storytold/wordcraft` com reprodução mínima.
3. Abrir PR com o fix (em inglês, seguindo o estilo deles).
4. Registrar em `docs/UPSTREAM-CONTRIBUTIONS.md`:

   ```
   | Data | Nosso commit | PR upstream | Bug | Status |
   | 2026-10-28 | 3f4586a | #4616 (open) | IME epoch test depends on document ids | aberto |
   ```

   Exemplo já identificado: o fix de `ime_body_epoch_marks_body_replacements_not_edits` (nosso `3f4586a`) espelha o PR aberto #4616 upstream. Podemos subir nosso fix como PR lá.

## Nova dependência no upstream — o ponto de atrito

Se um commit do upstream adiciona, remove ou atualiza uma dependência de crate (incluindo `url` como no #4652), **nunca** entra no `main` pelo sync automático. O tratamento é:

1. **Pare o sync** naquele commit; marque como "revisar".
2. **Avalie a dep** com as mesmas regras do `local-deps/WORKFLOW.md`:
   - É estritamente necessária? Há alternativa sem dep (código próprio) ou com dep já no lock?
   - Qual o custo de manutenção se a dep sumir do crates.io?
   - O que o upstream ganhou (segurança? performance? correção de bug crítico?)
3. **Se aprovada**: adicione ao `Cargo.toml` do local-office (com `cfg` se for desktop-only, como o `url`), atualize o `Cargo.lock`, registre em `local-deps/DEPENDENCIES.md` e `local-deps/CHANGELOG-deps.md` (mesmo PR), e only então rode `cargo check --workspace --locked`.
4. **Se rejeitada**: adapte o código local para não depender dela (ex: reimplementar a função) ou pule o commit daquele arquivo. Registre a recusa em `docs/UPSTREAM-SYNC.md` com o motivo.

Exemplo já ocorrido: o #4652 upstream adiciona `url = "2.5"`. No local-office aplicamos a correção de segurança (links seguros), adicionamos `url` como dep direta só para desktop (`cfg(not(wasm32))`) e registramos no `local-deps` — porque a alternativa era continuar vulnerável à abertura de caminhos locais via links malformados.

## Automação (o que um script pode fazer sozinho)

Pode fazer (sem interação humana):
- `git fetch upstream`
- Listar commits novos no `upstream/main` desde o último sync registrado
- Tentar `git apply` nos arquivos mapeados com `strategy = "apply"`
- Rodar `cargo check --workspace --locked` nos crates afetados
- Gerar relatório: "aplicou X, falhou Y, revisar Z"

Não pode fazer (exige humano):
- Resolver conflitos de merge em `Cargo.toml`, `packaging/`, `.github/`
- **Adicionar, remover ou atualizar qualquer dependência** (nova dep = decisão de negócio de independência)
- Decidir se uma feature nova entra no nosso escopo
- Fazer merge no `main`
- Abrir PR/Issue no upstream (pode sugerir, mas não executar)

## Checklist para cada novo rebrand (Sheets, Slides, etc.)

- [ ] Adicionar remote `upstream` apontando para o repo origem (`storytold/<appcraft>`)
- [ ] Criar `docs/UPSTREAM-MAP.toml` com os caminhos que diferem (crates, .wxs, packaging, CI)
- [ ] Registrar o primeiro sync como "baseline" (nenhuma mudança aplicada, apenas marca o ponto de partida)
- [ ] Definir qual versão do upstream é a baseline do rebrand (ex: `wordcraft` @ `2c99a42`)
- [ ] Validar que o CI próprio (`.github/workflows/ci.yml`) passa antes do primeiro sync

## Decision Ledger

| Data | Decisão | Justificativa |
|------|---------|---------------|
| 2026-10-28 | Sincronização com upstream é manual-revisada, nunca automática no main | Protege o rebrand de regressões silenciosas e de mudanças de marca da org upstream |
| 2026-10-28 | `docs/UPSTREAM-MAP.toml` é o contrato do que pode ser aplicado por script | Evita que nomes de crate/bundle id do upstream entrem no nosso repo |
| 2026-10-28 | Bugs compartilhados viram Issue/PR no upstream, registrados em `UPSTREAM-CONTRIBUTIONS.md` | O rebrand não é um fork abandonado; ajudamos o ecossistema |
| 2026-10-28 | Nova dep do upstream **nunca** entra no sync automático: `url 2.5` (#4652) foi adicionada manualmente com justificativa de segurança e registrada no `local-deps` | Independência de dependências é regra inegociável (WORKFLOW.md); cada dep nova custa manutenção e risco de sumiço do crates.io |
