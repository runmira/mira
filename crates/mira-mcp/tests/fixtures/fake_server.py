#!/usr/bin/env python3
"""A tiny stdio MCP server for mira-mcp's tests.

Tools:
  echo(text)        -> text back
  picture()         -> a text block and a PNG image block
  add_tool()        -> registers `extra` and sends tools/list_changed
  crash()           -> exits the process (tests reconnect)
  roots()           -> asks the client for its roots and returns them
Also one resource and one prompt. Writes a line to stderr on start so
tests can check it lands in the log file, not the terminal.
"""
import json
import sys

sys.stderr.write("fake server starting\n")
sys.stderr.flush()

tools = [
    {"name": "echo", "description": "Echo text.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
     "annotations": {"readOnlyHint": True}},
    {"name": "picture", "description": "Return a picture.", "inputSchema": {"type": "object"}},
    {"name": "add_tool", "description": "Add a tool.", "inputSchema": {"type": "object"}},
    {"name": "crash", "description": "Exit.", "inputSchema": {"type": "object"}},
    {"name": "roots", "description": "Report client roots.", "inputSchema": {"type": "object"}},
]
next_id = 1000
pending_roots = None


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def result(mid, res):
    send({"jsonrpc": "2.0", "id": mid, "result": res})


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    mid = msg.get("id")
    if method is None:
        # A response to our roots/list request.
        if pending_roots is not None and mid == pending_roots[0]:
            roots = msg.get("result", {}).get("roots", [])
            result(pending_roots[1], {"content": [{"type": "text", "text": json.dumps(roots)}]})
            pending_roots = None
        continue
    if method == "initialize":
        result(mid, {
            "protocolVersion": msg["params"].get("protocolVersion", "2025-06-18"),
            "capabilities": {"tools": {"listChanged": True}, "resources": {}, "prompts": {}},
            "serverInfo": {"name": "fake", "version": "1.2.3"},
            "instructions": "Use echo to repeat things.",
        })
    elif method == "tools/list":
        result(mid, {"tools": tools})
    elif method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments") or {}
        if name == "echo":
            result(mid, {"content": [{"type": "text", "text": args.get("text", "")}]})
        elif name == "picture":
            result(mid, {"content": [
                {"type": "text", "text": "a red dot"},
                {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
            ]})
        elif name == "add_tool":
            tools.append({"name": "extra", "description": "Added later.",
                          "inputSchema": {"type": "object"}})
            result(mid, {"content": [{"type": "text", "text": "added"}]})
            send({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
        elif name == "crash":
            sys.exit(3)
        elif name == "roots":
            next_id += 1
            pending_roots = (next_id, mid)
            send({"jsonrpc": "2.0", "id": next_id, "method": "roots/list"})
        else:
            result(mid, {"content": [{"type": "text", "text": "no such tool"}], "isError": True})
    elif method == "resources/list":
        result(mid, {"resources": [{"uri": "mem://notes", "name": "notes",
                                    "description": "Some notes", "mimeType": "text/plain"}]})
    elif method == "resources/read":
        result(mid, {"contents": [{"uri": msg["params"]["uri"], "mimeType": "text/plain",
                                   "text": "remember the milk"}]})
    elif method == "prompts/list":
        result(mid, {"prompts": [{"name": "review", "description": "Review a file",
                                  "arguments": [{"name": "file", "required": True}]}]})
    elif method == "prompts/get":
        f = (msg["params"].get("arguments") or {}).get("file", "?")
        result(mid, {"messages": [{"role": "user",
                                   "content": {"type": "text", "text": f"Please review {f}."}}]})
    elif method == "ping":
        result(mid, {})
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "not found"}})
