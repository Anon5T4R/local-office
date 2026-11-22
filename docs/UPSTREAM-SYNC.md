# Sincronização com o upstream (storytold/wordcraft)

## Base

- Upstream: `storytold/wordcraft`, commit **6a2b520f** (`Merge pull request #4401 from
  benandez/fix/main-tests-linear-imports`, 2026-10-26). A árvore partiu desse commit no primeiro
  rebrand (`d2018885`, 27/out), sem histórico.
- Prova (conferida em 2026-11-21 no clone raso de `C:\Dev\storytold\wordcraft`):
  `crates/layout/src/tests_linear.rs` daqui traz o import de `doc::props::Rgb` que só existe a partir
  de `6a2b520f` (`git diff 2c99a427 6a2b520f -- crates/layout/src/tests_linear.rs`), e nenhum dos 14
  arquivos testáveis de `d859c673` (limpeza Sonar, o commit seguinte no `main`) entrou — ex.:
  `crates/ui-egui/src/dialogs_layout.rs` ainda tem `!(f.cells > 1 && !f.merge_first)`, a forma de
  antes. Logo a base é `6a2b520f`, não `d859c673`.
- `c8631193` (27/out, merge da branch `fix/builtin-styles-bcs-ics-4198`) **não** é a base: falta aqui
  o que ele agrega sobre a `main` de então (`2cf1d6c7`) — dos 37 arquivos testáveis do PR de layout,
  nenhum está presente.
- O clone raso começa em `2c99a427` (pai de `6a2b520f`): commits anteriores a ele não são
  conferíveis ali, e nada entre os dois existe no `main`.
- Fork: `Anon5T4R/local-office`, versão própria na linha `0.11.x`.
- Nada é lido de `C:\Dev\storytold` em build, teste ou release; a pasta é só referência local para a
  skill `sync-upstream`.

## Registro de sync

A base continua `6a2b520f`: só pontos escolhidos entram (skill `sync-upstream`), não um bump.

| Data | Alvo (wordcraft) | Pegos | Descartados relevantes |
|---|---|---|---|
| 2026-11-21 | `origin/main` do dia | `cdebeace` → `b082b1bb` (DOCX: run `w:cs` só fica negrito/itálico por `w:bCs`/`w:iCs`, inclusive nos botões, no Merge Formatting e nos style sets); `93bf1488` → `eaa5afaa` (cada processo grava só o que mudou no `ui.json`, com `prefs_merge.rs` e lock) | `0b99c871` (cura o cache de "parágrafo medido sozinho", caminho que não existe no nosso layout), `8d257954` (depende do modelo compartilhado `craft_wordmodel`), `4216089a` (teste do comportamento #6536, que não temos) |

## Objetivo
Manter o `local-office` próximo do `storytold/wordcraft` (upstream) sem perder a independência do rebranding: nenhum merge cego, nenhuma dependência de servidor da org deles, nenhum `git+` no lock. Tudo que entrar no main deste repo passa por revisão de arquitetura primeiro.

## Princípios
1. **O main é nosso.** O upstream é uma fonte de mudanças candidatas, nunca um destino automático.
2. **Revisão humana obrigatória.** Nada entra no `main` sem passar pela revisão de arquitetura (Phase 3 / Architecture Gate).
3. **Mapeamento de renames é explícito.** Todo nome/caminho do wordcraft que difere do local-office fica registrado em `docs/UPSTREAM-MAP.toml` (ver abaixo) para que um script tente aplicar sozinho e pare onde não conseguir.
4. **Puxamos apenas o que é nosso.** Não puxamos branding, nomes de marca, links `getartcraft.com`, Discord, `docs/brand/`, ou CI que dependa de workflows compartilhados da org `storytold`.
5. **Contribuímos de volta.** Quando resolvemos um bug que também existe upstream, abrimos Issue/PR no `storytold/wordcraft` (em inglês, como eles usam). Registramos em `UPSTREAM-CONTRIBUTIONS.md`.
6. **Independência de conteúdo (regra da suíte, PLANO-REBRAND §4.6).** Feature upstream que baixa conteúdo do servidor deles (dicionários, revisores, modelos locais de filtro/OCR/voz, fontes) não entra como está: ou o conteúdo vem embutido no repo (`assets/`), ou com fetch com checksum fixado no build, ou a feature é removida. Download de runtime, se inevitável, só das nossas releases do GitHub com digest validado. Nada aponta pra `storytold`/`getartcraft` em runtime.

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

Exemplo já ocorrido: o #4652 upstream adiciona `url = "2.5"`. No local-office aplicamos a correção de segurança (links seguros), adicionamos `url` como dep direta e registramos no `local-deps` — porque a alternativa era continuar vulnerável à abertura de caminhos locais via links malformados.

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
- [ ] Validar que o CI próprio (`.github/workflows/ci.yml`) passa antes do primeiro sync

## Decision Ledger

| Data | Decisão | Justificativa |
|------|---------|---------------|
| 2026-10-28 | Sincronização com upstream é manual-revisada, nunca automática no main | Protege o rebrand de regressões silenciosas e de mudanças de marca da org upstream |
| 2026-10-28 | `docs/UPSTREAM-MAP.toml` é o contrato do que pode ser aplicado por script | Evita que nomes de crate/bundle id do upstream entrem no nosso repo |
| 2026-10-28 | Bugs compartilhados viram Issue/PR no upstream, registrados em `UPSTREAM-CONTRIBUTIONS.md` | O rebrand não é um fork abandonado; ajudamos o ecossistema |
| 2026-10-28 | Nova dep do upstream **nunca** entra no sync automático: `url 2.5` (#4652) foi adicionada manualmente com justificativa de segurança e registrada no `local-deps` | Independência de dependências é regra inegociável (WORKFLOW.md); cada dep nova custa manutenção e risco de sumiço do crates.io |

## Vendorizado

Forks de upstream que este app vendoriza e cujo sync este documento cobre; os demais forks vendored
(`craft-libs`, `egui`, `winit`, `local-ai`) têm o rev no `AGENTS.md`.

| Crate | Upstream | Rev | O que entrou |
|-------|----------|-----|--------------|
| `libs/winprint.rs` | `storytold/winprint.rs` | `f084d35` (2026-11-21) | job de PDF por stream (`17f5f1e`, `b334ebb`, `03a4fb6`, `800a6f9`), página renderizada na resolução do job (`6f3012d`), fábrica Direct2D multi-thread (`f256775`), ticket XML (`2390a37`, `cbffb07`), `PrintTicket::from_dev_mode` e `PrinterDevice::is_xps_driver` sem uso por ora (`8ae097e`, `bc06a7c`) e o teardown das portas de arquivo dos testes (`f084d35`) |
| `libs/winprint-gdi` | `storytold/winprint.rs` (`crates/gdi`) | `417ac3c` | não mudou neste sync: é o crate que o `craft-libs` pina no rev antigo, onde o gdi existe |

O código do upstream entra com o `rustfmt` da suíte (150 colunas), como o resto de `libs/` aqui.
Ficaram de fora: os commits de CI, StepSecurity e Harden-Runner do upstream, e `6758393` (deixa as
build-dependencies do download do PDFium opcionais atrás da feature `pdfium`), que só mexe em
`Cargo.toml`/`build.rs` de build — o `build.rs` aqui continua o do `5f80b9b`.

## craft-update vendored: fork permanente da suíte (fora do sync)

`libs/craft-libs/crates/update` está **fora do escopo** deste sync com o upstream. É um crate nosso, hospedado no `craft-libs` da suíte Local, e um **fork permanente consciente** do `craft-update` upstream — unificado nos 10 apps em 2026-11-13, com um diff de milhares de linhas contra `storytold/craft-libs/crates/update` (MSI context/migrate, pins do launcher, `offers_update`, `desktop_register`, catálogo único de apps). Nenhum diff de sync encaixa nele.

Bugfix e feature vão **direto na fonte única** — repo `Anon5T4R/local-libs` (crate `crates/update`) — e daí são replicados para os apps pelo script de sincronização (`sync.ps1 -Mode push`, guarda de hash ×10; a cópia vendored aqui é espelho). Nunca edite a cópia local diretamente: o próximo sync sobrescreve. Contribuições ao upstream só via **Issue/PR manual**, em inglês, quando o problema também existir lá - mesma política do `UPSTREAM-CONTRIBUTIONS.md` do local-office. O restante do `craft-libs` segue o fluxo normal deste documento.
