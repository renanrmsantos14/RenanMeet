# Estado: app-completo

- Nível: 2 (feature grande) · Etapa: 2 de 5 (diarização) pronta para PR.
- Feito: merge do AtaLocal, limpeza, meet-core (opus, resample, captura, gravador, decode, meetcli), protótipos de UI,
  escolha Slint, diarização Sherpa + reagrupamento por trecho + voz conhecida (D4: online 1,0 %, canal único 4,9 % de DER).
- Próximo: PRs de captura e diarização; depois Groq + ata + SQLite/FTS5 + models manager, UI Slint, release.
- Decisões: ver `docs/decisoes.md`. Restrições: sem release/merge; Actions só `workflow_dispatch`; chave Groq só no keyring.
- Pendente de prova: Windows (captura), Android em Rust, DER em áudio real, RAM do app gravando/processando (medir no app).
