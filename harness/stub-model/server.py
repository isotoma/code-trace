#!/usr/bin/env python3
"""Stub Anthropic Messages API for the Track 1 harness.

Speaks just enough of the API for `claude -p` to complete a scripted text
turn: POST /v1/messages (query string ignored), SSE when the request has
stream=true, plain JSON otherwise. Everything else is a 404. No auth is
validated — the harness passes a dummy ANTHROPIC_API_KEY.

See harness/NOTES.md for the spike that derived this surface.
"""
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("STUB_MODEL_PORT", "3081"))
CANNED_TEXT = os.environ.get("STUB_MODEL_TEXT", "Hello from the stub model.")


def sse(event, data):
    return f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        body = self.rfile.read(n).decode("utf-8", "replace")
        if self.path.split("?")[0] != "/v1/messages":
            self.send_response(404)
            self.end_headers()
            return
        try:
            req = json.loads(body)
        except ValueError:
            req = {}
        model = req.get("model", "claude-stub")
        if req.get("stream"):
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            msg = {
                "id": "msg_stub_1", "type": "message", "role": "assistant",
                "model": model, "content": [], "stop_reason": None,
                "stop_sequence": None,
                "usage": {"input_tokens": 10, "output_tokens": 1},
            }
            self.wfile.write(sse("message_start", {"type": "message_start", "message": msg}))
            self.wfile.write(sse("content_block_start", {
                "type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}))
            self.wfile.write(sse("content_block_delta", {
                "type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": CANNED_TEXT}}))
            self.wfile.write(sse("content_block_stop", {"type": "content_block_stop", "index": 0}))
            self.wfile.write(sse("message_delta", {
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                "usage": {"output_tokens": 6}}))
            self.wfile.write(sse("message_stop", {"type": "message_stop"}))
        else:
            resp = {
                "id": "msg_stub_1", "type": "message", "role": "assistant",
                "model": model,
                "content": [{"type": "text", "text": CANNED_TEXT}],
                "stop_reason": "end_turn", "stop_sequence": None,
                "usage": {"input_tokens": 10, "output_tokens": 6},
            }
            data = json.dumps(resp).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)


if __name__ == "__main__":
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    print(f"stub model listening on {PORT}", flush=True)
    server.serve_forever()
