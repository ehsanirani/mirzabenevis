#!/usr/bin/env python3
"""Mock LLM servers for mirzabenevis e2e tests.

Usage: mock_server.py PORT LOGFILE
The first path segment picks a scenario, so one server can serve many:
  http://127.0.0.1:PORT/<scenario>/v1/chat/completions   (OpenAI SSE)
  http://127.0.0.1:PORT/<scenario>/v1/messages           (Anthropic SSE)
  http://127.0.0.1:PORT/api/tags, /v1/models             (Ollama / local listing;
      behaviour set by env MOCK_TAGS=ok|embedonly|401|empty|html)
Each request is appended to LOGFILE as one JSON line {path, headers, body}.
"""
import json, os, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]); LOG = sys.argv[2]
TAGS = os.environ.get("MOCK_TAGS", "ok")

def oai(text):
    return "data: " + json.dumps({"choices": [{"delta": {"content": text}}]}) + "\n\n"

def ant(ev, obj):
    return f"event: {ev}\ndata: {json.dumps(obj)}\n\n"

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass

    def log(self, body):
        with open(LOG, "a") as f:
            f.write(json.dumps({"method": self.command, "path": self.path,
                                "headers": {k.lower(): v for k, v in self.headers.items()},
                                "body": body}) + "\n")

    def send_json(self, code, obj):
        raw = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers(); self.wfile.write(raw)

    def start_sse(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()

    def chunk(self, data):
        if isinstance(data, str): data = data.encode()
        self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data)); self.wfile.flush()

    def end(self):
        self.wfile.write(b"0\r\n\r\n"); self.wfile.flush()

    def do_GET(self):
        self.log(None)
        if self.path.endswith("/api/tags") or self.path.endswith("/v1/models"):
            ollama = self.path.endswith("/api/tags")
            if TAGS == "401": return self.send_json(401, {"error": "unauthorized"})
            if TAGS == "html":
                raw = b"<html>hi</html>"; self.send_response(200)
                self.send_header("content-length", str(len(raw))); self.end_headers()
                return self.wfile.write(raw)
            names = {"ok": ["nomic-embed-text:latest", "qwen3:8b"],
                     "embedonly": ["nomic-embed-text:latest", "mxbai-embed-large"],
                     "bge": ["bge-m3:latest", "llama3.2:3b"],
                     "empty": []}[TAGS]
            if ollama: return self.send_json(200, {"models": [{"name": n} for n in names]})
            return self.send_json(200, {"object": "list", "data": [{"id": n} for n in names]})
        self.send_json(404, {"error": "nope"})

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(n)
        try: body = json.loads(raw)
        except Exception: body = raw.decode("utf-8", "replace")
        self.log(body)
        scen = self.path.strip("/").split("/")[0]
        if self.path.endswith("/chat/completions"):
            return self.openai(scen)
        if self.path.endswith("/v1/messages"):
            return self.anthropic(scen)
        self.send_json(404, {"error": {"message": "no such path " + self.path}})

    def openai(self, s):
        if s == "nonstream":
            return self.send_json(200, {"choices": [{"message": {"role": "assistant", "content": "full answer"}, "finish_reason": "stop"}]})
        if s == "http500":
            return self.send_json(500, {"error": {"message": "internal kaboom"}})
        self.start_sse()
        if s == "ok":
            for t in ["Hello", ", ", "world."]: self.chunk(oai(t))
            self.chunk("data: [DONE]\n\n")
        elif s == "comments":
            self.chunk(": OPENROUTER PROCESSING\n\n")
            self.chunk(": keep-alive\n\n")
            self.chunk("data: " + json.dumps({"choices": [{"delta": {"role": "assistant"}}]}) + "\n\n")
            self.chunk("data: " + json.dumps({"choices": [{"delta": {"reasoning_content": "thinking..."}}]}) + "\n\n")
            self.chunk(oai("A")); self.chunk(oai("B"))
            self.chunk("data: " + json.dumps({"choices": [], "usage": {"total_tokens": 3}}) + "\n\n")
            self.chunk("data: [DONE]\n\n")
        elif s == "midstream_error":
            self.chunk(oai("partial "))
            self.chunk("data: " + json.dumps({"error": {"message": "upstream rate limited", "code": 429},
                                               "choices": [{"delta": {"content": ""}, "finish_reason": "error"}]}) + "\n\n")
            self.chunk("data: [DONE]\n\n")
        elif s == "utf8split":
            payload = oai("سلام دنیا ✓ 😀").encode()
            # split inside a multi-byte char: every byte separately
            for i in range(len(payload)): self.chunk(payload[i:i+1])
            self.chunk("data: [DONE]\n\n")
        elif s == "crlf":
            self.chunk(oai("x").replace("\n", "\r\n")); self.chunk("data: [DONE]\r\n\r\n")
        elif s == "nolf":
            # last data line without a trailing newline
            self.chunk(oai("first ")); self.chunk("data: " + json.dumps({"choices": [{"delta": {"content": "last"}}]}))
        elif s == "length":
            self.chunk(oai("cut off mid"))
            self.chunk("data: " + json.dumps({"choices": [{"delta": {}, "finish_reason": "length"}]}) + "\n\n")
            self.chunk("data: [DONE]\n\n")
        elif s == "slow":
            self.chunk(oai("tick")); time.sleep(5); self.chunk(oai("tock")); self.chunk("data: [DONE]\n\n")
        elif s == "trickle":
            # a token every 0.8s for ~4s: total longer than a 2s timeout but never idle
            for i in range(5): self.chunk(oai(f"t{i} ")); time.sleep(0.8)
            self.chunk("data: [DONE]\n\n")
        elif s == "empty":
            self.chunk("data: [DONE]\n\n")
        elif s == "fenced":
            self.chunk(oai("```\nfixed text\n```")); self.chunk("data: [DONE]\n\n")
        elif s == "huge":
            for i in range(20000): self.chunk(oai(f"line {i} " + "x"*20 + "\n"))
            self.chunk("data: [DONE]\n\n")
        elif s == "big":
            for i in range(3000): self.chunk(oai(f"line {i}\n"))
            self.chunk("data: [DONE]\n\n")
        self.end()

    def anthropic(self, s):
        if s == "http401":
            return self.send_json(401, {"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})
        self.start_sse()
        self.chunk(ant("message_start", {"type": "message_start", "message": {"id": "m", "content": []}}))
        if s in ("ok", "thinking"):
            if s == "thinking":
                self.chunk(ant("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}))
                self.chunk(ant("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "SECRET THOUGHTS"}}))
                self.chunk(ant("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig"}}))
                self.chunk(ant("content_block_stop", {"type": "content_block_stop", "index": 0}))
            self.chunk(": ping comment\n\n")
            self.chunk(ant("ping", {"type": "ping"}))
            self.chunk(ant("content_block_start", {"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}))
            for t in ["Fixed ", "text."]:
                self.chunk(ant("content_block_delta", {"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": t}}))
            self.chunk(ant("content_block_stop", {"type": "content_block_stop", "index": 1}))
            self.chunk(ant("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}}))
            self.chunk(ant("message_stop", {"type": "message_stop"}))
        elif s == "error":
            self.chunk(ant("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "half"}}))
            self.chunk(ant("error", {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}))
        elif s == "maxtokens":
            self.chunk(ant("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "truncated"}}))
            self.chunk(ant("message_delta", {"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}))
            self.chunk(ant("message_stop", {"type": "message_stop"}))
        self.end()

ThreadingHTTPServer.allow_reuse_address = True
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
