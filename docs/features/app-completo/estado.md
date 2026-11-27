# Estado: app-completo

- Nível: 2 (feature grande) · Etapa: 4 de 5 (app Slint) pronta para PR (#4, base `feature/transcricao`); 1 a 3 em PR (#1 a #3).
- Feito: merge do AtaLocal, limpeza, meet-core (opus, resample, captura, gravador, decode, meetcli), protótipos de UI,
  escolha Slint, diarização Sherpa + reagrupamento por trecho + voz conhecida (D4: online 1,0 %, canal único 4,9 % de DER).
- Feito (3): Groq, chunking, ata map-reduce, SQLite/FTS5, models manager, importação m4a/mp3/flac, pipeline, wiki, `meetcli` completo, e2e com Groq falsa (`tools/testaudio/e2e.sh`).
- Feito (4): app Slint (lista/busca, ata, transcrição com player, falantes, vozes, ajustes, gravar, bandeja, atalho, instância única, Esc nos diálogos, tema claro/escuro); RAM parado 26,5 MB, gravando 27,8 MB, binário 21,6 MB.
- Próximo: PR do bloco 4; depois release (5: versão por build, `release.yml` só `workflow_dispatch`, updater, remover Tauri, destino do `android/`).
- Decisões: ver `docs/decisoes.md`. Restrições: sem release/merge; Actions só `workflow_dispatch`; chave Groq só no keyring.
- Pendente de prova: Groq real (sem chave), Windows (captura), Android em Rust, DER em áudio real, RAM do app gravando/processando (medir no app).
