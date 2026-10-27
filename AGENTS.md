# AGENTS.md — LocalOffice

Fork independente do wordcraft (ArtCraft), rebrandado para a suíte Local do João.
Rust puro + egui. Sem conta, sem telemetria, sem dependência de servidores de terceiros.

## Regras
- **Dependências:** toda dep que entra/sai/atualiza exige `Cargo.lock` commitado +
  linha no inventário do repo `Anon5T4R/local-deps` (regra completa em
  `local-deps/WORKFLOW.md`). PR sem isso não mergea.
- **Sem fontes git:** nenhuma dependência pode apontar para `git+` (o CI valida).
  Tudo vem do crates.io (pinado pelo lock) ou de `libs/` (forks vendored nos revs
  exatos que o upstream pina).
- **Commits:** português, sem Co-Authored-By.
- **Release:** tag `vX.Y.Z` → o Actions builda Windows + Linux e publica. Mínimo local.
- **Disco local:** nunca deixe `target/` para trás — `cargo clean` após uso local;
  builds pesados são do CI.

## Estrutura
- `crates/` — engine e módulos (`localoffice-*`)
- `apps/localoffice` — app desktop; `apps/localoffice-cli` — CLI/MCP; `apps/localoffice-web` — wasm
- `libs/` — forks vendored: `craft-libs` (b926b5c), `egui` (6d518fc), `winit` (a517b38),
  `winprint.rs` (5f80b9b) e `winprint-gdi` (417ac3c — o craft-libs pina dois revs do
  winprint.rs; o gdi só existe no rev antigo)
- `i18n/locales/` — catálogos de tradução (common.tsv + localoffice.tsv por locale,
  do craft-i18n @ 9205108), embutidos pelo `crates/ui-egui/build.rs`
- `assets/craft-fonts/` — fontes CJK/árabe/etc. (craft-fonts @ b076417, build input
  `CRAFT_FONTS_DIR`)
- `packaging/` — Windows (WiX/MSI), Linux (AppImage/deb/rpm/tar), Arch (PKGBUILD), macOS, web
- `xtask/` — tarefas de build/CI (`cargo xtask ci`)

## Build
```
cargo build --locked          # debug
cargo test --locked           # testes
packaging/windows/package.ps1 # MSI + portable (Windows)
packaging/linux/package.sh    # AppImage/deb/rpm/tar (Linux)
```

## Vendoring (como foi feito; reusar nos próximos apps)
- `libs/<repo>` = clone do repo upstream **no rev exato** que o Cargo.toml original
  pina, sem `.git`, com os git deps internos reescritos para path (ex.:
  `libs/craft-libs/crates/print/Cargo.toml` → `../../../winprint.rs/`).
- O `[patch.crates-io]` da raiz aponta egui/winit para `libs/`, krilla para
  `libs/craft-libs/vendor/krilla/` (o patch do craft-libs não se aplica quando ele
  é path dep de outro workspace — só o patch da raiz vale).
- `libs/` mantém o estilo de origem: fmt do CI exclui `libs/`; o gate de assets
  (`xtask assets`) também.
- Ícones: `packaging/icons.sh` regenera tudo a partir de `assets/app-icon/localoffice.svg`
  (viewBox 0 0 1024 1024; resvg + xtask ico + iconutil). Os outputs são commitados.
