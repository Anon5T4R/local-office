# Contribuições de volta para o upstream (storytold/wordcraft)

Quando resolvemos um bug no `local-office` que também existe no `wordcraft`,
abrimos Issue/PR no upstream (em inglês, seguindo o estilo deles) e registramos aqui.

| Data | Nosso commit | PR upstream | Bug | Status |
|------|--------------|-------------|-----|--------|
| 2026-10-28 | 3f4586a | #4616 (open) | IME epoch test depende de IDs de documento globais (teste racy) | aberto |
| 2026-10-28 | 6d884792 | #4670 (open) | Janela restaurada maximizada no Windows + DX12 + AMD perde o device antes do primeiro frame (processo vivo, janela nunca aparece) | aberto |
| 2026-10-28 | 6d884792 | #4671 (open) | Um `.md` que contém um ZIP (um `.docx` renomeado `.md`) mostra os bytes binários crus em vez de abrir como o documento Word que é | aberto |
| 2026-10-28 | 6d884792 | #4672 (open) | Cursor "+" do outline expand/collapse é branco puro, invisível na página branca | aberto |
| 2026-10-29 | 90d9f5a8 | #4784 (open) | Janela restaurada em monitor que não existe mais (spacedesk/TV desplugada) abre fora da tela: processo vivo, janela em lugar nenhum; o resgate por tamanho de monitor não percebe quando os dois monitores têm o mesmo tamanho | aberto |
| 2026-10-29 | 715a06d5 | craft-libs#416 (open) | Janela nova aberta com a janela de origem maximizada cai num "monitor fantasma" (tile por canto superior-esquerdo pega o tile anterior quando a borda passa do canto): abre em (-1920,-1080) num único monitor | aberto |
