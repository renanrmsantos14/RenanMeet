# Estado: app-completo

- Nível: 2 (feature grande) · Etapa: 3 de 5 (transcrição + ata + banco + wiki) pronta para PR; 1 e 2 em PR (#1, #2).
- Feito: merge do AtaLocal, limpeza, meet-core (opus, resample, captura, gravador, decode, meetcli), protótipos de UI,
  escolha Slint, diarização Sherpa + reagrupamento por trecho + voz conhecida (D4: online 1,0 %, canal único 4,9 % de DER).
- Feito (3): Groq, chunking, ata map-reduce, SQLite/FTS5, models manager, importação m4a/mp3/flac, pipeline, wiki, `meetcli` completo, e2e com Groq falsa (`tools/testaudio/e2e.sh`).
- Próximo: PR do bloco 3; depois UI Slint (4), release (5).
- Decisões: ver `docs/decisoes.md`. Restrições: sem release/merge; Actions só `workflow_dispatch`; chave Groq só no keyring.
- Pendente de prova: Groq real (sem chave), Windows (captura), Android em Rust, DER em áudio real, RAM do app gravando/processando (medir no app).
