---
name: verificar-transcricao
description: Use ao mexer em crates/meet-core (groq, transcribe, minutes, pipeline, db, export, models, secrets) — prova com testes unitários e o ponta a ponta com a Groq falsa (sem rede, sem chave) mais o Sherpa real.
---

# Verificar transcrição, ata e pipeline (RenanMeet)

## Passos
1. `cargo test -p meet-core` (Groq com servidor local: User-Agent, Authorization, multipart, 429 com `Retry-After`, 401/404; chunking; ata map-reduce; FTS5; exportação; download com hash).
2. `cargo build --release -p meet-core --bin meetcli` e `tools/testaudio/e2e.sh` (precisa de `.cache/models` e `testdata/out/online_2.wav`, ver `verificar-diarizacao`). Ele prova: importar → processar → cada frase no falante certo → ata → nomear e guardar voz → nota na wiki chegando ao remoto → 2ª reunião reconhecendo Renan e Ana → busca. Referência: ~24 s para 197 s de áudio, pico ~375 MB (o processo sai ao terminar).
3. Chave: `meetcli key check` com a real só quando o Renan tiver cadastrado (`echo "$CHAVE" | meetcli key set`, lida da entrada padrão, nunca em argumento). Sem chave real, diga "Groq real não provada".

## Critérios
- A chave nunca aparece em log, erro ou argumento (`secrets::mask` mostra só o fim).
- Falha na ata mantém a transcrição (`Done` com `error`); falha antes de salvar deixa `Failed` com o áudio preservado.
- A nota da wiki leva só decisões e tarefas, nunca a transcrição.

## Armadilhas
- A Groq responde 403 ao User-Agent de biblioteca (`Python-urllib`); o app usa `renanmeet/<versão>`.
- O modelo de ata é `openai/gpt-oss-120b` (o `llama-3.3-70b-versatile` sai em 16/08/2026).
- `RENANMEET_GROQ_BASE` redireciona a API (e2e); `RENANMEET_DATA_DIR`/`RENANMEET_WIKI_DIR`/`RENANMEET_GROQ_KEY` isolam o teste.
