# Local Word — estado e fila

Fork independente do wordcraft (ArtCraft), rebrandado para a suíte Local. Versão e release notes:
`cargo xtask version`, tags e GitHub Releases. Onde cada coisa mora: `AGENTS.md`.

## O que está pronto

- Documento de trabalho de ponta a ponta: escrever, formatar, estilos, listas, tabelas, referências,
  revisão e mala direta; abrir e salvar `.docx`; `.odt`, `.rtf`, `.html`, `.md`, `.tex` e `.txt`;
  abrir `.doc` 97-2003; exportar PDF com texto selecionável, links e marcadores.
- Casca no padrão da suíte: barra de cima com tema e Configurações, paleta de comandos, tema
  Automático/Claro/Escuro, ícones do `craft-icons`, cartão de atualização do `craft-update`.
- Agente nativo: cada ação é um comando com id (`localoffice-cli`, canal de controle e MCP —
  `docs/mcp.md`, `docs/control-protocol.md`).
- Impressão direta no desktop (`craft-print`) e atualização assinada (`craft-update`).
- Recuperação de trabalho não salvo (`docs/recovery.md`), Zotero (`docs/zotero.md`), dicionários
  Hunspell (`docs/spelling-dictionaries.md`).

## O que falta

Na ordem em que dói no uso diário (levantado em 2026-10, e desde então corrigido por partes):

1. **`.docx` do mundo real**: corpus de arquivos do Word, Google Docs, LibreOffice e Pages aberto,
   renderizado e comparado página por página com o Word.
2. **Paginação diferente do Word**: não há substituta métrica do Aptos; opções de compatibilidade
   legadas só em parte; falta arnês de comparação página a página.
3. **Gráficos, SmartArt e objetos OLE**: preservados em parte; editar/renderizar por completo falta;
   tinta nativa do Word só aparece pela imagem de fallback.
4. **Profundidade de diálogos**: o app cobre os diálogos que o ribbon alcança; o Word tem mais.
5. **Revisão ortográfica**: inglês, polonês, português, italiano, croata, turco, russo, tcheco,
   indonésio e espanhol; os demais idiomas ficam sem marcação.
6. **Interface espelhada (RTL)** e as traduções que ainda faltam em cada catálogo.

Itens de release, CI e gate local: `AGENTS.md` (§ Regras, § Build) e `docs/git-protocol.md` do
dev-notes. Decisões do sync com o upstream: `docs/UPSTREAM-SYNC.md`.
