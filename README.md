# RenanMeet

App de desktop (Linux e Windows) que grava reuniões, transcreve em português, diz quem falou e gera a ata.
Tudo em Rust; interface em Slint; parado, usa menos de 40 MB de RAM. O trabalho pesado só roda depois da gravação.

## Fluxo

`Gravar (Ctrl+Alt+R) → Parar → processa → ata, transcrição e falantes`

- **Captura**: microfone + áudio do sistema (online) ou só microfone (presencial, um mic para a sala toda). Opus mono.
- **Transcrição**: Groq `whisper-large-v3`, em blocos de até 10 min.
- **Falantes**: diarização local (Sherpa-ONNX) e voz conhecida: nomeie uma vez e o app reconhece nas próximas.
- **Ata**: resumo, decisões, tarefas e pontos em aberto (LLM da Groq). Exporta Markdown e manda as tarefas para a wiki.
- **Lista**: busca FTS5, player sincronizado com a transcrição, tema claro/escuro, bandeja, iniciar com o computador.
- **LGPD**: lembrete de aviso aos participantes e botão para copiar o texto.

A chave da Groq fica no cofre do sistema (Ajustes → Chave da Groq). Nunca vai para o repositório.

## Instalar e atualizar

Baixe o executável da última versão em [Releases](https://github.com/renanrmsantos14/RenanMeet/releases)
(`renanmeet-linux-x86_64` ou `renanmeet-windows-x86_64.exe`). O app consulta o GitHub 60 s depois de abrir e a cada 6 h;
em Ajustes → Atualização há "Verificar agora" e "Atualizar e reiniciar". Só troca o executável se a assinatura minisign
do `SHA256SUMS` bater com a chave embutida e o sha256 do arquivo conferir.

## Desenvolver

```bash
cargo run -p meet-app                 # app
cargo run -p meet-core --bin meetcli  # linha de comando (importar, processar, vozes, chave)
cargo test -p meet-core
cargo build --release -p meet-app     # target/release/renanmeet
```

Linux precisa de `libasound2-dev`, `libdbus-1-dev`, `libxdo-dev`, `libfontconfig1-dev`, `pkg-config`, além de `parec`/`paplay`
(PipeWire ou PulseAudio) e `zenity`. Windows: toolchain MSVC.

## Estrutura

```
crates/meet-core/   captura, voz (Sherpa), Groq, ata, banco SQLite, exportação, atualização
crates/meet-app/    app Slint (janela, bandeja, atalho, player)
tools/              versao.py (versão por build), testaudio/ (áudio sintético e Groq falsa)
docs/               decisões (decisoes.md), estado da feature
.github/workflows/  release.yml (só manual)
```

## Versão e release

A versão é do app, não do Git: começa em `0.0.0` e cada release sobe o PATCH (`tools/versao.py bump`). O app mostra
`v0.0.1 · dd/mm/aaaa HH:mm` no canto (hora de Brasília, gravada no build). O release sai pelo workflow **Release**
(Actions → Run workflow; só dispara à mão) e precisa do secret `MINISIGN_SECRET_KEY`.

## Privacidade

O áudio fica neste computador. Só os blocos de áudio vão à Groq para transcrever, e a transcrição vai para gerar a ata.
Sem telemetria.
