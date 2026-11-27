#!/usr/bin/env python3
"""Groq falsa para o teste ponta a ponta, sem rede e sem chave.

Uso: groq_mock.py <verdade.json> [porta]
O STT devolve os trechos da verdade (início/fim reais, texto "[Falante] frase N"); o chat devolve uma
ata fixa. Escreve a porta em stdout e fica no ar até ser morta. Só aceita um áudio de um bloco (< 10 min).
"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

truth = json.load(open(sys.argv[1]))
ATA = {"resumo": "Reunião de teste com duas pessoas.", "decisoes": ["Seguir com o plano A"],
       "pendencias": ["Definir o prazo"], "tarefas": [{"texto": "Enviar a proposta", "responsavel": "Ana", "prazo": None}]}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, obj, code=200):
        b = json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        self._send({"data": []})
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0)); body = self.rfile.read(n)
        if not self.headers.get("Authorization", "").startswith("Bearer "):
            return self._send({"error": "sem chave"}, 401)
        if self.path.endswith("/audio/transcriptions"):
            segs = [{"start": t["start"], "end": t["end"], "text": f" [{t['speaker']}] frase {i}"} for i, t in enumerate(truth)]
            return self._send({"segments": segs})
        if self.path.endswith("/chat/completions"):
            return self._send({"choices": [{"message": {"content": json.dumps(ATA, ensure_ascii=False)}}]})
        self._send({"error": "rota"}, 404)

srv = HTTPServer(("127.0.0.1", int(sys.argv[2]) if len(sys.argv) > 2 else 0), H)
print(srv.server_address[1], flush=True)
srv.serve_forever()
