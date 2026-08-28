#!/usr/bin/env python3
"""三种协议的 mock 服务,用于验证 KiteTerm 的 AI 请求分发是否正确。
记录收到的路径、鉴权头与 body,并按各自协议的响应格式返回。"""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

LOG = []


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(n) or "{}")
        entry = {
            "path": self.path,
            "auth": self.headers.get("authorization"),
            "x_api_key": self.headers.get("x-api-key"),
            "anthropic_version": self.headers.get("anthropic-version"),
            "model": body.get("model"),
            "system": body.get("system"),
            "messages": body.get("messages"),
        }
        LOG.append(entry)

        # 模型 id 用作场景开关,便于构造各种"拿不到正文"的真实响应形态
        scenario = body.get("model", "")

        if self.path == "/v1/messages":  # anthropic
            if scenario == "only-thinking":
                # 思考型模型把额度用光,只剩 thinking 块
                resp = {"content": [{"type": "thinking", "thinking": "…"}], "stop_reason": "max_tokens"}
            else:
                # 回一条带代码块的文本,验证前端代码块渲染路径
                resp = {
                    "content": [{"type": "text", "text": "ANTHROPIC_OK\n```\nlsof -i :8080\n```"}],
                    "stop_reason": "end_turn",
                }
        elif self.path == "/v1/chat/completions":  # openai 兼容
            if scenario == "array-content":  # 多模态数组形式
                resp = {"choices": [{"message": {"content": [{"type": "text", "text": "OPENAI_OK"}]}}]}
            elif scenario == "reasoning":  # 推理模型:content 为 null
                resp = {"choices": [{"message": {"content": None, "reasoning_content": "OPENAI_OK"}}]}
            elif scenario == "legacy":  # 旧 completions 风格
                resp = {"choices": [{"text": "OPENAI_OK"}]}
            elif scenario == "truncated":
                resp = {"choices": [{"message": {"content": ""}, "finish_reason": "length"}]}
            else:
                resp = {"choices": [{"message": {"role": "assistant", "content": "OPENAI_OK"}}]}
        elif self.path == "/api/chat":  # ollama
            if scenario == "generate-style":
                resp = {"response": "OLLAMA_OK"}
            else:
                resp = {"message": {"role": "assistant", "content": "OLLAMA_OK"}}
        elif self.path == "/__log":
            resp = LOG
        else:
            self.send_response(404)
            self.end_headers()
            return

        data = json.dumps(resp).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/__log":
            data = json.dumps(LOG).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self.send_response(404)
            self.end_headers()


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", 18900), H).serve_forever()
