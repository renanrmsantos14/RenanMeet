---
name: verificar-app
description: "Verifica o app desktop (Slint): lista, ata, player, falantes, vozes, ajustes, gravação, bandeja e atalho. Use após mudar crates/meet-app/** ou quando pedirem para testar a interface."
---
# Verificar app (RenanMeet)

Nunca grave reunião real: dados de demonstração com Groq falsa e áudio sintético.

1. `cargo build --release -p meet-app` (sem avisos) e `tools/testaudio/demo.sh /tmp/rm-demo` (precisa de `testdata/out/online_2.wav`, gerado por `tools/testaudio/gen.py`).
2. Xvfb: `Xvfb :97 -screen 0 1180x760x24 &`; abrir com
   `DISPLAY=:97 RENANMEET_DATA_DIR=/tmp/rm-demo RENANMEET_MODELS_DIR=$PWD/.cache/models RENANMEET_GROQ_KEY=chave-falsa target/release/renanmeet &`
3. Prints com `import -window root x.png`; cliques com `xdotool mousemove X Y click 1` (1180×760):
   reunião (150,200), abas Ata/Transcrição/Falantes (365/446/543, 97), chips Vozes/Ajustes/Reuniões (140/216/60, 669), Gravar (84,80).
   Conferir: lista com busca, ata (resumo, decisões, tarefas clicáveis), transcrição com a frase ativa realçada ao tocar,
   falantes (nomear → "Salvar e lembrar"/"Só aqui"), vozes salvas (renomear, juntar, apagar), ajustes (chave, tema, autostart, limiar, LGPD),
   versão `vX.Y.Z · dd/mm/aaaa HH:mm` no canto em todas as telas, tema claro e escuro (campos legíveis nos dois).
4. Gravar: botão Gravar → "Começar a gravar" → indicador "Gravando · mm:ss" com níveis → Parar. A reunião aparece, vai para "processando"
   e termina em "pronta" (com Groq de verdade) ou "falhou" com a mensagem clara (chave falsa → 401). Nunca fica presa.
5. Overlays: clicar fora do diálogo fecha; apagar pede confirmação.
6. Atalho: `xdotool key ctrl+alt+r` abre o diálogo de gravação (ou para a gravação em curso).
7. Bandeja (Linux): `busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems` lista `renanmeet`.
8. RAM: `ps -o rss= -C renanmeet` parado (medido 26,5 MB) e gravando (27,8 MB); processamento: `/usr/bin/time -f "%M KB" target/release/renanmeet process <id>` (medido 380 MB para 3 min, com Groq falsa).
9. Windows: sem toolchain MSVC aqui, o app não é compilado nem testado — diga isso no PR.

## Armadilhas
- `pkill renanmeet` antes de reabrir (o app fica na bandeja ao fechar a janela).
- Sem `zenity` o botão Importar avisa em vez de abrir o seletor.
- `RENANMEET_GROQ_KEY` tem precedência sobre o cofre: nunca grave a chave falsa em Ajustes.
