# Estado: app-completo

- Nível: 2 (feature grande) · Etapa: 5 de 5 (release) pronta para PR (#5, base `feature/app-ui`); 1 a 4 em PR (#1 a #4).
- Feito: merge do AtaLocal, limpeza, meet-core (opus, resample, captura, gravador, decode, meetcli), protótipos de UI,
  escolha Slint, diarização Sherpa + reagrupamento por trecho + voz conhecida (D4: online 1,0 %, canal único 4,9 % de DER).
- Feito (3): Groq, chunking, ata map-reduce, SQLite/FTS5, models manager, importação m4a/mp3/flac, pipeline, wiki, `meetcli` completo, e2e com Groq falsa (`tools/testaudio/e2e.sh`).
- Feito (4): app Slint (lista/busca, ata, transcrição com player, falantes, vozes, ajustes, gravar, bandeja, atalho, instância única, Esc nos diálogos, tema claro/escuro); RAM parado 26,5 MB, gravando 27,8 MB, binário 21,6 MB.
- Feito (5): Tauri/Android removidos, updater (minisign + sha256 + versão assinada, provado no app real com release local), `tools/versao.py`, `release.yml` só manual (nunca executado), README. 1 h de áudio: 444,6 s, pico 598 MB.
- Próximo: Renan cria o secret `MINISIGN_SECRET_KEY` (+ backup de `~/.config/RenanMeet-release/release.key`), revisa os PRs na ordem e dispara o Release; Android em Rust é PR próprio.
- Decisões: ver `docs/decisoes.md`. Restrições: sem release/merge; Actions só `workflow_dispatch`; chave Groq só no keyring.
- Pendente de prova: Groq real (sem chave), Windows (captura), Android em Rust, DER em áudio real, RAM do app gravando/processando (medir no app).
