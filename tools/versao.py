#!/usr/bin/env python3
"""Versão do RenanMeet (não é a do Git): começa em 0.0.0 e cada build de publicação sobe o PATCH.

  versao.py            mostra a versão atual
  versao.py bump       sobe o PATCH (0.0.3 -> 0.0.4)
  versao.py 0.0.9      grava a versão pedida

Atualiza os dois Cargo.toml e as entradas do Cargo.lock. A data/hora (America/Sao_Paulo) vem do build.rs.
"""
import re
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
CRATES = ["meet-core", "meet-app"]
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")


def atual() -> str:
    m = re.search(r'^version = "([^"]+)"', (RAIZ / "crates/meet-app/Cargo.toml").read_text(encoding="utf-8"), re.M)
    return m.group(1)


def gravar(nova: str) -> None:
    for c in CRATES:
        p = RAIZ / f"crates/{c}/Cargo.toml"
        p.write_text(re.sub(r'^version = "[^"]+"', f'version = "{nova}"', p.read_text(encoding="utf-8"), count=1, flags=re.M), encoding="utf-8")
    lock = RAIZ / "Cargo.lock"
    txt = lock.read_text(encoding="utf-8")
    for c in CRATES:
        txt, n = re.subn(rf'(name = "{c}"\nversion = )"[^"]+"', rf'\1"{nova}"', txt)
        if n != 1:
            sys.exit(f"Cargo.lock: não achei {c}")
    lock.write_text(txt, encoding="utf-8")


def main() -> None:
    arg = sys.argv[1] if len(sys.argv) > 1 else ""
    ver = atual()
    if arg == "bump":
        a, b, c = map(int, ver.split("."))
        ver = f"{a}.{b}.{c + 1}"
    elif arg:
        if not SEMVER.match(arg):
            sys.exit(f"versão inválida: {arg}")
        ver = arg
    else:
        print(ver)
        return
    gravar(ver)
    print(ver)


main()
