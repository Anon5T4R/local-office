# AGENTS.md — Local Writer

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
- **Disco:** no workspace do João vale a política de `target/` do dev-notes (`MEMORY.md` §4 — limpa o que
  está frio ou feito, mantém durante iteração ativa); fora dele, `cargo clean` ao terminar. Builds pesados são do CI.

## Estrutura
- `crates/` — engine e módulos (`localwriter-*`)
- `apps/localwriter` — app desktop; `apps/localwriter-cli` — CLI/MCP
- `apps/localwriter/src/legacy.rs` — o que o renome Local Word → Local Writer deixou (o app se chamou Local Word,
  id `localoffice`): na partida move as pastas de nome antigo (preferências, dados e cache do atualizador, conexões de
  IA, portátil), uma vez e sem sobrescrever, copia as chaves do chaveiro e lê `LOCALOFFICE_*` onde `LOCALWRITER_*`
  não está. Nada disso se apaga: o antigo fica para trás. O MSI mantém UpgradeCode e GUIDs; os ProgIds antigos saem
  com o produto antigo (MajorUpgrade) e os `LocalWriter.*` entram
- `libs/` — forks vendored: `craft-libs` (b926b5c), `egui` (6d518fc), `winit` (a517b38),
  `winprint.rs` (f084d35) e `winprint-gdi` (417ac3c - o craft-libs pina dois revs do
  winprint.rs; o gdi só existe no rev antigo), `local-ai` (BYOK — chave de API/config;
  fonte em `C:\Dev\underworld\local-ai`, NÃO é o app LocalAI da suíte antiga)
- `i18n/locales/` — catálogos de tradução (common.tsv + localwriter.tsv por locale,
  do craft-i18n @ 9205108), embutidos pelo `crates/ui-egui/build.rs`
- `assets/craft-fonts/` — fontes CJK/árabe/etc. (craft-fonts @ b076417, build input
  `CRAFT_FONTS_DIR`)
- `packaging/` — Windows (WiX/MSI), Linux (AppImage/deb/rpm/tar FHS com `install.sh`), Arch (PKGBUILD), macOS
- `xtask/` — tarefas de build/CI (`cargo xtask ci`, `fmt`)
- `scripts/check-mirror.sh` — confere as crates e os assets (`craft-fonts`) espelhados do `local-libs` contra a fonte (CI e `xtask ci`)

## Agent-native (MCP)
O app já é agent-native de origem (herdado do wordcraft): `crates/mcp` (servidor stdio + backends
headless/remote), `crates/control-key` (chave do control channel, em arquivo ou
`LOCALWRITER_CONTROL_KEY`), `localwriter-cli mcp` e `mcp --connect ADDR|PORT`. Com `--connect` o
bridge fala com o app aberto (porta loopback; a chave só vai para 127.0.0.1) e re-tenta a cada
chamada até ele abrir. O molde da suíte vive em **`craft-mcp`** (fonte `local-libs`, espelho em
`libs/craft-libs/crates/mcp`) e é usado aqui para o `--connect` (porta nua/loopback). `run --script`
aceita o job (`{file, template, commands, save}`), uma lista de passos ou JSON Lines;
`commands --search TEXT` filtra o catálogo. Docs: `docs/mcp.md` e `docs/control-protocol.md`.

## Build
```
cargo build --locked          # debug
cargo test --locked           # as fontes vendored entram sempre (.cargo/config.toml), igual ao CI
cargo xtask ci                # o gate local: fmt, clippy, testes, i18n, assets, camadas,
                              # cargo-deny e espelhos do local-libs
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
- Ícones: `packaging/icons.sh` regenera tudo a partir de `assets/app-icon/localwriter.svg`
  (viewBox 0 0 1024 1024; resvg + xtask ico + iconutil). Os outputs são commitados.
- Ícones por tipo de arquivo: `packaging/file-icons.py` gera `assets/file-icons/` (um SVG por formato, variantes de
  32 e 16 px e a folha de prova); a regra da família está no README de lá. Ainda não estão no instalador.


## Upstream e a pasta `C:\Dev\storytold`

Menções a `storytold/*` em código, comentários, docs e atribuições (NOTICE/ATTRIBUTION) são referência histórica e obrigação de licença. **Nada** é lido de `C:\Dev\storytold` em build, teste ou release: tudo vem do próprio repo (crates.io pinado pelo lock + `libs/`/`vendor/` vendored + CI que busca craft-fonts/craft-i18n do GitHub). Essa pasta existe somente como referência local para a skill `sync-upstream`.
