#!/usr/bin/env python3
"""
converter.py — WorkBuddy/CodeBuddy 算力转接网关(多账号、配置驱动、可观测)。

原理:
  - 读取本机 WorkBuddy / CodeBuddy 桌面端登录态(*.info, 可能多个账号)；
  - 对齐官方 CLI 头像方案直连腾讯后端(copilot.tencent.com / www.workbuddy.ai)；
  - 在本地暴露 OpenAI Chat / Responses / Anthropic Messages 三协议 API；
  - 按账号头 / 模型能力路由账号,上游鉴权失败自动故障切换；
  - 可选脱敏缓解后端内容审核误伤,轮转日志,积分查询 / 每日签到。

依赖: fastapi + uvicorn + httpx。
用法:
  python3 converter.py
  python3 converter.py --desensitize --log gateway.log
  python3 converter.py --config config.json
"""

from __future__ import annotations

import argparse
import asyncio
import json
import logging
import logging.handlers
import os
import sys
import threading
import time
from pathlib import Path
from typing import Optional

import httpx
from fastapi import FastAPI, Header, HTTPException, Request
from fastapi.responses import JSONResponse, StreamingResponse, FileResponse, HTMLResponse
import uvicorn

from references.codebuddy2api.accounts import (AccountPool, CredentialManager, find_auth_files,
                      default_auth_dirs, BACKEND_CN, BACKEND_GLOBAL)
from references.codebuddy2api.models_catalog import ModelCatalog, CN_FALLBACK_MODELS
from references.codebuddy2api.routing import split_alias, candidate_chain
from references.codebuddy2api.billing import query_credits, daily_checkin
import references.codebuddy2api.config as config_mod
import references.codebuddy2api.ccswitch as ccswitch

try:
    from references.codebuddy2api.desensitize import desensitize_body
except ImportError:  # 模块缺失时降级为不脱敏
    def desensitize_body(body, roles=("system",), desensitize_harness_user=False,
                         desensitize_tools=False, compact_harness=False,
                         strip_tool_metadata=False):
        return body

from references.codebuddy2api.responses_adapter import (
    responses_request_to_chat,
    ResponsesStreamConverter,
)
from references.codebuddy2api.responses_projection import project_responses_chat_body
from references.codebuddy2api.anthropic_adapter import (
    anthropic_request_to_chat,
    AnthropicStreamConverter,
)

# ---------------------------------------------------------------------------
# 全局状态
# ---------------------------------------------------------------------------

app = FastAPI(title="workbuddy2api gateway", version="2.1")

STATE = {
    "cfg": None,          # config.Config
    "api_key": "",
    "pool": None,         # AccountPool
    "catalog": None,      # ModelCatalog
}

# ---------------------------------------------------------------------------
# 日志(轮转)
# ---------------------------------------------------------------------------

_logger = logging.getLogger("wbgateway")
_logger.setLevel(logging.INFO)
_logger.addHandler(logging.NullHandler())

CFG_LOCK = threading.Lock()   # config.json 持久化互斥


def setup_log(path: str, max_bytes: int = 10 * 1024 * 1024, backups: int = 3):
    if not path:
        return
    handler = logging.handlers.RotatingFileHandler(
        path, maxBytes=max_bytes, backupCount=backups, encoding="utf-8")
    handler.setFormatter(logging.Formatter("[%(asctime)s] %(message)s", "%Y-%m-%d %H:%M:%S"))
    _logger.addHandler(handler)
    _logger.removeHandler(logging.NullHandler())


def _log(msg: str):
    _logger.info(msg)


# ---------------------------------------------------------------------------
# 工具函数
# ---------------------------------------------------------------------------

def _truncate(s: str, n: int = 80) -> str:
    s = str(s).replace("\n", " ").strip()
    return s[:n] + ("…" if len(s) > n else "")


def _safe_err_raw(raw: bytes, status: int) -> dict:
    try:
        return json.loads(raw.decode("utf-8", "replace"))
    except Exception:
        return {"error": {"message": raw.decode("utf-8", "replace")[:500],
                          "type": "upstream_error", "code": status}}


def _err_event(msg: bytes, status: int) -> bytes:
    chunk = {
        "error": {"message": msg.decode("utf-8", "replace")[:500],
                  "type": "upstream_error", "code": status},
    }
    return f"data: {json.dumps(chunk, ensure_ascii=False)}\n\n".encode("utf-8")


def _last_user_text(messages: list) -> str:
    for m in reversed(messages):
        if m.get("role") != "user":
            continue
        content = m.get("content", "")
        if isinstance(content, list):
            for blk in content:
                if isinstance(blk, dict) and blk.get("type") == "text":
                    return str(blk.get("text", ""))
            return ""
        return str(content)
    return ""


def _looks_like_content_filter_text(text: str) -> bool:
    text = (text or "").lower()
    return ("content-filter" in text or "content_filter" in text or "敏感内容" in text
            or "内容审核" in text or "无法响应您的请求" in text)


def _is_authish(status: int, raw: bytes) -> bool:
    if status in (401, 403):
        return True
    text = raw[:800].decode("utf-8", "replace").lower()
    return ("clientapiauthenticationexception" in text
            or "authentication failed" in text
            or "not authorized" in text)


def _first_chunk_authish(chunk: bytes) -> bool:
    text = chunk[:800].decode("utf-8", "replace").lower()
    return ("clientapiauthenticationexception" in text
            or "authorization required" in text)


def _is_model_missing(raw: bytes) -> bool:
    return b"service info not found" in raw[:1200]


def _ensure_first_system(body: dict) -> dict:
    """global 后端要求首条消息 role=system(code 11128)。"""
    msgs = body.get("messages") or []
    if msgs and isinstance(msgs[0], dict) and msgs[0].get("role") == "system":
        return body
    b = dict(body)
    nb = [{"role": "system", "content": ""}]
    if isinstance(msgs, list):
        nb.extend(msgs)
    b["messages"] = nb
    return b


def _normalize_global_body(cred: CredentialManager, body: dict) -> dict:
    """按账号 region 决定是否需要首条 system 补齐。"""
    if cred.region == "global":
        return _ensure_first_system(body)
    return body


def cfg() -> config_mod.Config:
    return STATE["cfg"]


def _redacted_config(c: config_mod.Config) -> dict:
    d = c.to_dict()
    d["api_key"] = "**" if d.get("api_key") else ""
    return d


def _desensitize_body(cfg: config_mod.Config, body: dict) -> dict:
    if not cfg.desensitize:
        return body
    return desensitize_body(body, roles=("system", "developer"),
                            desensitize_harness_user=True,
                            desensitize_tools=True,
                            compact_harness=not cfg.no_compact,
                            strip_tool_metadata=True)


def _check_auth(authorization: Optional[str], x_api_key: Optional[str]):
    key = STATE["api_key"]
    if not key:
        return
    token = authorization[7:].strip() if authorization and authorization.startswith("Bearer ") else ""
    if not token and x_api_key:
        token = x_api_key
    if token != key:
        raise HTTPException(status_code=401, detail={"error": {"message": "invalid api key", "type": "auth_error"}})


def _require_pool() -> AccountPool:
    pool = STATE["pool"]
    if pool is None or not pool.names():
        raise HTTPException(status_code=503, detail={
            "error": {"message": "未找到登录凭据,请先在桌面端登录 CodeBuddy/WorkBuddy", "type": "auth_error"}})
    return pool


def _chain_for(model: str, header: str | None) -> list[CredentialManager]:
    pool = _require_pool()
    names = candidate_chain(pool, STATE["catalog"], model,
                            account_header=header,
                            default_account=(cfg().default_account or ""))
    chain = [pool.get(n) for n in names if pool.get(n)]
    return chain


async def _post_with_failover(chain: list[CredentialManager], path: str, body: dict,
                              *, model: str = "", rid: str = "") -> tuple[CredentialManager | None, int, bytes]:
    """对候选账号逐个尝试,鉴权/模型缺失时切换;返回 (cred, status, raw)。"""
    last_status, last_raw = 502, b""
    for cred in chain:
        url = f"{cred.backend_base}{path}"
        send_body = _normalize_global_body(cred, body)
        try:
            async with httpx.AsyncClient(timeout=300) as c:
                async with c.stream("POST", url, headers=cred.get_headers(), json=send_body) as r:
                    raw = b"".join([chunk async for chunk in r.aiter_bytes()])
                    if _is_authish(r.status_code, raw):
                        if len(chain) > 1:
                            _log(f"[{rid}] ↻ 账号[{cred.name}] 鉴权失败 code={r.status_code},切换账号")
                            continue
                    elif _is_model_missing(raw):
                        if len(chain) > 1:
                            _log(f"[{rid}] ↻ 账号[{cred.name}] 无模型 {model},切换到其他账号")
                            continue
                    return cred, r.status_code, raw
        except httpx.HTTPError as e:
            last_status, last_raw = 502, str(e).encode()
            if len(chain) > 1:
                _log(f"[{rid}] ↻ 账号[{cred.name}] 网络错误,切换账号: {e}")
                continue
            return chain[0], last_status, last_raw
    return (chain[0] if chain else None), last_status, last_raw


async def _stream_raw_with_failover(chain: list[CredentialManager], path: str, body: dict,
                                    *, model: str = "?", t0: float = 0.0, rid: str = ""):
    """流式透传候选账号的第一个可用流;上游鉴权/首块鉴权错误时切换账号。"""
    prefix = f"[{rid}] " if rid else ""
    usage: dict = {}
    finish_reason = None
    tool_names: list[str] = []
    saw_filter = False
    buf = b""
    raw_parts: list[bytes] = []

    def _feed(chunk: bytes):
        nonlocal finish_reason, saw_filter, buf
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            line = line.strip()
            if not line.startswith(b"data:"):
                continue
            data = line[5:].strip()
            if data == b"[DONE]":
                continue
            try:
                obj = json.loads(data)
            except Exception:
                continue
            if obj.get("usage"):
                usage.update(obj["usage"])
            for ch in obj.get("choices") or []:
                if ch.get("finish_reason"):
                    finish_reason = ch["finish_reason"]
                for tc in (ch.get("delta") or {}).get("tool_calls") or []:
                    nm = (tc.get("function") or {}).get("name")
                    if nm:
                        tool_names.append(nm)
            text = data.decode("utf-8", "replace")
            if "content-filter" in text or "敏感" in text or "审核" in text:
                saw_filter = True

    for idx, cred in enumerate(chain):
        url = f"{cred.backend_base}{path}"
        send_body = _normalize_global_body(cred, body)
        try:
            async with httpx.AsyncClient(timeout=None) as c:
                async with c.stream("POST", url, headers=cred.get_headers(), json=send_body) as r:
                    if r.status_code != 200:
                        err = await r.aread()
                        if (_is_authish(r.status_code, err) or _is_model_missing(err)) and len(chain) > 1:
                            _log(f"{prefix}↻ 账号[{cred.name}] 鉴权/无模型 code={r.status_code},切换账号")
                            continue
                        _log(f"{prefix}✗ HTTP {r.status_code} | {cred.name} | {_truncate(err.decode('utf-8','replace'),200)}")
                        yield _err_event(err, r.status_code)
                        return
                    agen = r.aiter_bytes()
                    try:
                        first = await agen.__anext__()
                    except StopAsyncIteration:
                        yield _err_event(b"upstream returned empty stream", 502)
                        return
                    if _first_chunk_authish(first) and len(chain) > 1:
                        _log(f"{prefix}↻ 账号[{cred.name}] 首块含鉴权错误,切换账号")
                        continue
                    raw_parts.append(first)
                    _feed(first)
                    yield first
                    async for chunk in agen:
                        if not chunk:
                            continue
                        raw_parts.append(chunk)
                        _feed(chunk)
                        yield chunk
                    break
        except httpx.HTTPError as e:
            _log(f"{prefix}✗ 网络错误 | {cred.name} | {e}")
            if len(chain) > 1:
                continue
            yield _err_event(str(e).encode(), 502)
            return

    elapsed = time.time() - t0 if t0 else 0
    tag = " ⚠️内容审核拦截" if (saw_filter or finish_reason == "content-filter") else ""
    _log(f"{prefix}◀ RESPONSE {model} | {elapsed:.1f}s | finish={finish_reason}{tag}"
         + (f" | tool_calls={tool_names[:6]}" if tool_names else "")
         + f" | tokens={usage.get('total_tokens', '?')}")
    _log(f"{prefix}── RESPONSE RAW SSE (前200KB) ──\n{b''.join(raw_parts)[:200000].decode('utf-8','replace')}")


async def _collect_from_raw(raw: bytes) -> dict:
    """消费后端 OpenAI SSE(字节)聚合为单个非流式 chat.completion。"""
    content_parts: list[str] = []
    tool_calls: dict[int, dict] = {}
    model: str | None = None
    finish_reason: str | None = None
    usage: dict | None = None

    for line in raw.decode("utf-8", "replace").splitlines():
        line = line.strip()
        if not line or not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if data == "[DONE]":
            break
        try:
            chunk = json.loads(data)
        except json.JSONDecodeError:
            continue
        model = chunk.get("model") or model
        if chunk.get("usage"):
            usage = chunk["usage"]
        for choice in chunk.get("choices") or []:
            if choice.get("finish_reason"):
                finish_reason = choice["finish_reason"]
            delta = choice.get("delta") or {}
            if delta.get("content"):
                content_parts.append(delta["content"])
            for tc in delta.get("tool_calls") or []:
                idx = tc.get("index", 0)
                slot = tool_calls.setdefault(idx, {"id": None, "name": None, "arguments": ""})
                if tc.get("id"):
                    slot["id"] = tc["id"]
                fn = tc.get("function") or {}
                if fn.get("name"):
                    slot["name"] = fn["name"]
                if fn.get("arguments"):
                    slot["arguments"] += fn["arguments"]

    tcs = None
    if tool_calls:
        tcs = [{"id": v["id"], "type": "function",
                "function": {"name": v["name"], "arguments": v["arguments"]}}
               for _, v in sorted(tool_calls.items())]
        finish_reason = finish_reason or "tool_calls"

    message = {"role": "assistant", "content": "".join(content_parts) or None}
    if tcs:
        message["tool_calls"] = tcs
    return {
        "id": "chatcmpl-" + os.urandom(12).hex(),
        "object": "chat.completion",
        "created": int(time.time()),
        "model": model or "unknown",
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason or "stop"}],
        "usage": usage or {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
    }


# ---------------------------------------------------------------------------
# 模型目录同步
# ---------------------------------------------------------------------------

def _sync_catalog_now():
    if STATE["catalog"] is None or STATE["pool"] is None:
        return
    try:
        STATE["catalog"].sync(STATE["pool"])
        # 应用配置里的模型覆盖表(动态失败时兜底/补充)
        for acc, models in (cfg().model_overrides or {}).items():
            STATE["catalog"].apply_overrides(acc, models)
        # 应用配置里的免费模型标记
        for acc, ids in (cfg().free_models or {}).items():
            STATE["catalog"].mark_free(acc, ids)
        _log(f"模型目录同步完成: {STATE['catalog'].account_names()}")
    except Exception as e:
        _log(f"模型目录同步失败: {e}")


def _catalog_worker():
    interval = max((cfg().model_sync_interval_hours or 24) * 3600, 300)
    while True:
        time.sleep(interval)
        _sync_catalog_now()


# ---------------------------------------------------------------------------
# 管理端点
# ---------------------------------------------------------------------------

@app.get("/")
def index():
    web = Path(__file__).parent / "web" / "index.html"
    if web.is_file():
        return FileResponse(web, media_type="text/html")
    return HTMLResponse(
        "<h2>前端未安装</h2><p>缺少 <code>web/index.html</code>。API 端点 <code>/v1/*</code> 仍可用。</p>",
        status_code=200)


@app.get("/health")
def health():
    pool = STATE["pool"]
    info: dict = {
        "status": "ok" if (pool and pool.names()) else "degraded",
        "platform": sys.platform,
        "python": sys.version.split()[0],
        "mode": "direct-proxy (native function calling)",
        "config": _redacted_config(cfg()) if cfg() else None,
    }
    if pool:
        info["accounts"] = pool.summary()
    if STATE["catalog"] is not None:
        info["model_sync"] = {
            "at": STATE["catalog"].last_sync_at(),
            "errors": STATE["catalog"].errors(),
        }
    return info


@app.get("/v1/models")
def list_models(authorization: Optional[str] = Header(default=None),
                x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    if STATE["catalog"] is not None and STATE["catalog"].account_names():
        data = STATE["catalog"].merged_models()
    else:  # 兜底
        data = [{"id": m, "object": "model", "created": 1700000000, "owned_by": "workbuddy"}
                for m in {m["id"] for m in CN_FALLBACK_MODELS}]
    return {"object": "list", "data": data}


@app.get("/credits")
async def credits(authorization: Optional[str] = Header(default=None),
                  x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    pool = _require_pool()
    results = [await query_credits(cred) for cred in pool.all()]
    return {"accounts": results}


@app.post("/credits/checkin")
async def checkin(authorization: Optional[str] = Header(default=None),
                  x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    pool = _require_pool()
    results = [await daily_checkin(cred) for cred in pool.all()]
    return {"accounts": results}


@app.post("/models/reload")
def reload_models(authorization: Optional[str] = Header(default=None),
                  x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    thread = threading.Thread(target=_sync_catalog_now, daemon=True)
    thread.start()
    return {"status": "reloading"}


# ---------------------------------------------------------------------------
# OpenAI Chat Completions
# ---------------------------------------------------------------------------

PASSTHROUGH_BODY_KEYS = {
    "model", "messages", "tools", "tool_choice", "temperature",
    "max_tokens", "max_completion_tokens", "top_p", "stream",
    "stream_options", "stop", "presence_penalty", "frequency_penalty",
    "n", "response_format", "seed", "user", "reasoning_effort",
    "verbosity", "reasoning_summary",
}


def _account_header(request: Request) -> str | None:
    return (request.headers.get("X-WB-Account")
            or request.headers.get("X-Workbuddy-Account"))


@app.post("/v1/chat/completions")
async def chat_completions(request: Request,
                           authorization: Optional[str] = Header(default=None),
                           x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    try:
        payload = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})

    messages = payload.get("messages") or []
    if not messages:
        raise HTTPException(status_code=400, detail={"error": {"message": "messages is required", "type": "invalid_request_error"}})

    raw_model = payload.get("model", "auto") or "auto"
    _, plain_model = split_alias(raw_model)
    chain = _chain_for(raw_model, _account_header(request))
    if not chain:
        raise HTTPException(status_code=503, detail={"error": {"message": "无可用的已登录账号", "type": "auth_error"}})

    client_wants_stream = bool(payload.get("stream"))
    body = {k: payload[k] for k in PASSTHROUGH_BODY_KEYS if k in payload}
    body["model"] = plain_model or "auto"
    body["stream"] = True
    if "stream_options" not in body:
        body["stream_options"] = {"include_usage": True}
    body = _desensitize_body(cfg(), body)

    rid = os.urandom(4).hex()
    tool_names = [t.get("function", {}).get("name") for t in (payload.get("tools") or []) if isinstance(t, dict)]
    last_user = _last_user_text(messages)
    _log(f"[{rid}] ▶ CHAT {raw_model} → 账号[{','.join(c.name for c in chain)}] | stream={client_wants_stream} | msgs={len(messages)}"
         + (f" | tools={tool_names}" if tool_names else "")
         + (f" | last_user={_truncate(last_user, 60)!r}" if last_user else ""))
    _log(f"[{rid}] ── REQUEST BODY ──\n{json.dumps(body, ensure_ascii=False, indent=2)[:60000]}")

    if client_wants_stream:
        return StreamingResponse(
            _stream_raw_with_failover(chain, "/v2/chat/completions", body,
                                      model=raw_model, t0=time.time(), rid=rid),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
        )

    # 非流式:后端只支持流式,这里聚合
    cred, status, raw = await _post_with_failover(chain, "/v2/chat/completions", body,
                                                  model=plain_model, rid=rid)
    if status != 200:
        _log(f"[{rid}] ✗ HTTP {status} | {raw.decode('utf-8','replace')[:300]}")
        raise HTTPException(status_code=status, detail=_safe_err_raw(raw, status))
    result = await _collect_from_raw(raw)
    _log(f"[{rid}] ◀ CHAT 完成 tokens={result.get('usage', {}).get('total_tokens', '?')}")
    return JSONResponse(content=result)


# ---------------------------------------------------------------------------
# OpenAI Responses API(Codex CLI)
# ---------------------------------------------------------------------------

@app.post("/v1/responses")
async def create_response(request: Request,
                          authorization: Optional[str] = Header(default=None),
                          x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    try:
        payload = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})

    try:
        chat_body = responses_request_to_chat(payload)
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"request conversion error: {e}", "type": "invalid_request_error"}})

    chat_body, projection_stats = project_responses_chat_body(chat_body)
    raw_model = payload.get("model", "auto") or "auto"
    _, plain_model = split_alias(raw_model)
    chat_body["model"] = plain_model or "auto"
    chat_body["stream"] = True
    if "stream_options" not in chat_body:
        chat_body["stream_options"] = {"include_usage": True}
    chat_body = _desensitize_body(cfg(), chat_body)

    chain = _chain_for(raw_model, _account_header(request))
    if not chain:
        raise HTTPException(status_code=503, detail={"error": {"message": "无可用的已登录账号", "type": "auth_error"}})

    client_wants_stream = payload.get("stream", True)
    rid = os.urandom(4).hex()
    _log(f"[{rid}] ▶ RESPONSES {raw_model} → 账号[{','.join(c.name for c in chain)}] | input_items={len(payload.get('input', []))}")
    _log(f"[{rid}] ── PROJECTION ── mode={projection_stats.get('mode')} | msgs {projection_stats.get('original_messages')}→{projection_stats.get('projected_messages')} | chars {projection_stats.get('original_message_chars')}→{projection_stats.get('projected_message_chars')} | tools {projection_stats.get('original_tools')}→{projection_stats.get('projected_tools')}")

    converter = ResponsesStreamConverter(model=raw_model)

    if client_wants_stream:
        return StreamingResponse(
            _stream_responses_from_raw(chain, chat_body, converter, raw_model,
                                       time.time(), rid),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
        )

    cred, status, raw = await _post_with_failover(chain, "/v2/chat/completions", chat_body,
                                                  model=plain_model, rid=rid)
    if status != 200:
        _log(f"[{rid}] ✗ HTTP {status} | {raw.decode('utf-8','replace')[:300]}")
        raise HTTPException(status_code=status, detail=_safe_err_raw(raw, status))
    for line in raw.decode("utf-8", "replace").splitlines():
        converter.feed_line(line)
    return JSONResponse(content=converter.get_nonstream_response())


async def _stream_responses_from_raw(chain, chat_body, converter: ResponsesStreamConverter,
                                     raw_model: str, t0: float, rid: str):
    raw_iter = _stream_raw_with_failover(chain, "/v2/chat/completions", chat_body,
                                         model=raw_model, t0=t0, rid=rid)
    try:
        async for chunk in raw_iter:
            text = chunk.decode("utf-8", "replace")
            stripped = text.strip()
            if stripped.startswith("data:") and '{"error"' in stripped:
                try:
                    ev_dict = json.loads(stripped[5:].strip())
                    ev = ev_dict.get("error", {})
                    out = {"type": "error", "error": {"message": str(ev.get("message", ""))[:500], "code": ev.get("code", 502)}}
                    yield f"data: {json.dumps(out, ensure_ascii=False)}\n\n".encode("utf-8")
                    return
                except Exception:
                    pass
            for line in text.splitlines():
                events = converter.feed_line(line)
                if events:
                    yield events.encode("utf-8")
        fin = converter.finish()
        if fin:
            yield fin.encode("utf-8")
    except Exception as e:
        err = {"type": "error", "error": {"message": str(e)[:500], "code": 502}}
        yield f"data: {json.dumps(err, ensure_ascii=False)}\n\n".encode("utf-8")


# ---------------------------------------------------------------------------
# Anthropic Messages API(Claude Code / CC Switch)
# ---------------------------------------------------------------------------

@app.post("/v1/messages")
async def create_message(request: Request,
                         authorization: Optional[str] = Header(default=None),
                         x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    try:
        payload = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})

    messages = payload.get("messages") or []
    if not messages:
        raise HTTPException(status_code=400, detail={"error": {"message": "messages is required", "type": "invalid_request_error"}})

    try:
        chat_body = anthropic_request_to_chat(payload)
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"request conversion error: {e}", "type": "invalid_request_error"}})

    raw_model = payload.get("model", "auto") or "auto"
    _, plain_model = split_alias(raw_model)
    chat_body["model"] = plain_model or "auto"
    chat_body["stream"] = True
    if "stream_options" not in chat_body:
        chat_body["stream_options"] = {"include_usage": True}
    chat_body = _desensitize_body(cfg(), chat_body)

    chain = _chain_for(raw_model, _account_header(request))
    if not chain:
        raise HTTPException(status_code=503, detail={"error": {"message": "无可用的已登录账号", "type": "auth_error"}})

    rid = os.urandom(4).hex()
    _log(f"[{rid}] ▶ ANTHROPIC {raw_model} → 账号[{','.join(c.name for c in chain)}] | msgs={len(chat_body.get('messages', []))}")

    return StreamingResponse(
        _stream_anthropic(chain, chat_body, raw_model, time.time(), rid),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
    )


async def _stream_anthropic(chain, chat_body, raw_model: str, t0: float, rid: str):
    converter = AnthropicStreamConverter(model=raw_model)
    raw_iter = _stream_raw_with_failover(chain, "/v2/chat/completions", chat_body,
                                         model=raw_model, t0=t0, rid=rid)
    try:
        async for chunk in raw_iter:
            text = chunk.decode("utf-8", "replace")
            stripped = text.strip()
            if stripped.startswith("data:") and '{"error"' in stripped:
                try:
                    ev_dict = json.loads(stripped[5:].strip())
                    ev = ev_dict.get("error", {})
                    err = {"type": "error",
                           "error": {"message": str(ev.get("message", ""))[:500],
                                     "type": "api_error", "code": ev.get("code", 502)}}
                    yield f"event: error\ndata: {json.dumps(err, ensure_ascii=False)}\n\n".encode("utf-8")
                    return
                except Exception:
                    pass
            for line in text.splitlines():
                events = converter.feed_line(line)
                if events:
                    yield events.encode("utf-8")
        fin = converter.finish()
        if fin:
            yield fin.encode("utf-8")
    except Exception as e:
        err = {"type": "error", "error": {"message": str(e)[:500], "type": "api_error", "code": 502}}
        yield f"event: error\ndata: {json.dumps(err, ensure_ascii=False)}\n\n".encode("utf-8")
    _log(f"[{rid}] ◀ ANTHROPIC {raw_model} 完成")


@app.post("/v1/messages/count_tokens")
async def count_tokens(request: Request,
                       authorization: Optional[str] = Header(default=None),
                       x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    """Anthropic token 计数端点(stub)。Claude Code 可能在发送前调用。"""
    _check_auth(authorization, x_api_key)
    return {"input_tokens": 0}


# ---------------------------------------------------------------------------
# Agent 连接管理与算力测试
# ---------------------------------------------------------------------------

def _display_host(c: config_mod.Config) -> str:
    h = c.host
    return "127.0.0.1" if h in ("0.0.0.0", "::", "") else h


def _agent_effective_model(agent: dict, model: str | None = None,
                           account: str | None = None) -> str:
    """模型(明文)+账号 → 实际发给网关的模型 id(global 账号加 global/ 前缀)。"""
    model = (model if model is not None else agent.get("model")) or "auto"
    account = account if account is not None else (agent.get("account") or "")
    region, plain = split_alias(model)
    if account == "global":
        return f"global/{plain or model}"
    return plain or model


def _persist_cfg() -> str:
    """把当前 cfg 的「可管理字段」写回配置文件(不覆盖 host/port/api_key 等)。"""
    with CFG_LOCK:
        path = STATE.get("config_file") or str(Path("config.json"))
        try:
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
        except Exception:
            data = {}
        c = cfg()
        data.update({
            "agents": c.agents,
            "default_account": c.default_account,
            "free_models": c.free_models,
            "model_overrides": c.model_overrides,
            "model_sync_interval_hours": c.model_sync_interval_hours,
        })
        tmp = path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        os.replace(tmp, path)
        _log(f"配置已持久化: {path}")
        return path


@app.get("/agents")
def get_agents(authorization: Optional[str] = Header(default=None),
               x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    c = cfg()
    end = {
        "base": f"http://{_display_host(c)}:{c.port}",
        "chat": f"http://{_display_host(c)}:{c.port}/v1/chat/completions",
        "responses": f"http://{_display_host(c)}:{c.port}/v1/responses",
        "messages": f"http://{_display_host(c)}:{c.port}/v1/messages",
    }
    return {"agents": c.agents, "endpoints": end, "config_file": STATE.get("config_file")}


@app.put("/agents")
async def put_agents(request: Request,
                     authorization: Optional[str] = Header(default=None),
                     x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    try:
        body = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})
    new_agents = body.get("agents")
    if not isinstance(new_agents, dict):
        raise HTTPException(status_code=400, detail={"error": {"message": "agents must be object", "type": "invalid_request_error"}})
    clean = config_mod.merge_agents(new_agents)
    cfg().agents = clean
    path = _persist_cfg()
    return {"ok": True, "config_file": path, "agents": clean}


@app.get("/settings")
def get_settings(authorization: Optional[str] = Header(default=None),
                 x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    c = cfg()
    return {"default_account": c.default_account,
            "free_models": c.free_models,
            "model_overrides": c.model_overrides,
            "model_sync_interval_hours": c.model_sync_interval_hours}


@app.put("/settings")
async def put_settings(request: Request,
                       authorization: Optional[str] = Header(default=None),
                       x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    _check_auth(authorization, x_api_key)
    try:
        body = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})
    c = cfg()
    if "default_account" in body:
        c.default_account = str(body["default_account"] or "")
    if isinstance(body.get("free_models"), dict):
        c.free_models = body["free_models"]
    if isinstance(body.get("model_overrides"), dict):
        c.model_overrides = body["model_overrides"]
    if "model_sync_interval_hours" in body:
        try:
            c.model_sync_interval_hours = int(body["model_sync_interval_hours"] or 24)
        except (TypeError, ValueError):
            pass
    path = _persist_cfg()
    return {"ok": True, "config_file": path}


async def _quick_chat(chain: list[CredentialManager], model_effective: str,
                      prompt: str = "ping", n: int = 120) -> dict:
    """极小请求验证某账号+模型真实可用。返回 ok/内容/preview/积分/账号。"""
    _, plain = split_alias(model_effective)
    body = {
        "model": plain or "auto",
        "messages": [{"role": "system", "content": ""}, {"role": "user", "content": prompt}],
        "max_tokens": n,
        "stream": True,
        "stream_options": {"include_usage": True},
    }
    cred, status, raw = await _post_with_failover(chain, "/v2/chat/completions", body,
                                                  model=plain, rid="test")
    if status != 200:
        return {"ok": False, "http": status, "error": raw[:300].decode("utf-8", "replace")}
    result = await _collect_from_raw(raw)
    content = (result["choices"] or [{}])[0].get("message", {}).get("content") or ""
    return {
        "ok": True,
        "http": 200,
        "account": (chain[0].name if chain else "?"),
        "model": model_effective,
        "content_preview": content[:160],
        "credit": (result.get("usage") or {}).get("credit"),
    }


@app.post("/agents/test")
async def test_agent(request: Request,
                     authorization: Optional[str] = Header(default=None),
                     x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    """一键连通测试: 按 agent 的连接偏好(或显式 model/account)发极小请求验证。"""
    _check_auth(authorization, x_api_key)
    try:
        body = await request.json()
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})

    agent_id = body.get("agent")
    agent = None
    if agent_id and (cfg().agents or {}).get(agent_id):
        agent = cfg().agents[agent_id]
    model_explicit = body.get("model")
    account_explicit = body.get("account")
    prompt = body.get("prompt") or "ping"

    effective = _agent_effective_model(agent or {}, model_explicit, account_explicit)
    account_header = account_explicit if account_explicit is not None else (agent.get("account") if agent else None)
    pool = _require_pool()
    names = candidate_chain(pool, STATE["catalog"], effective,
                            account_header=account_header,
                            default_account=cfg().default_account)
    chain = [pool.get(n) for n in names if pool.get(n)]
    if not chain:
        raise HTTPException(status_code=503, detail={"error": {"message": "无可用的已登录账号", "type": "auth_error"}})
    result = await _quick_chat(chain, effective, prompt=prompt)
    result["account"] = names[0] if names else "?"
    return result


@app.post("/ccswitch/register")
async def register_ccswitch(request: Request,
                            authorization: Optional[str] = Header(default=None),
                            x_api_key: Optional[str] = Header(default=None, alias="X-Api-Key")):
    """一键注册: 生成 `ccswitch://v1/import` deeplink 交由 CC Switch 添加 provider。

    body: {model?, account?, name?, endpoint?, api_key?, launch?}
    - 默认取「claude-code」agent 预设的 账号+模型;
    - launch=true(前端默认)时会调用本机 open 唤起 CC Switch 落库;
    - endpoint 默认 = 网关地址(不含 /v1/messages,协议由 CC Switch 拼)。
    """
    _check_auth(authorization, x_api_key)
    try:
        body = await request.json() or {}
    except Exception as e:
        raise HTTPException(status_code=400, detail={"error": {"message": f"bad json: {e}", "type": "invalid_request_error"}})

    agent = (cfg().agents or {}).get("claude-code") or {}
    model_ex = body.get("model")
    account_ex = body.get("account")
    effective = _agent_effective_model(agent, model_ex, account_ex)
    c = cfg()
    endpoint = body.get("endpoint") or f"http://{_display_host(c)}:{c.port}"
    api_key = body.get("api_key") if body.get("api_key") is not None else (c.api_key or "workbuddy")
    name = body.get("name") or "WorkBuddy 算力网关"

    url = ccswitch.build_deeplink(endpoint=endpoint, name=name, api_key=api_key,
                                  model=effective, app="claude")
    opened = False
    if body.get("launch"):
        opened = ccswitch.open_deeplink(url)
    _log(f"CC Switch 注册: model={effective} endpoint={endpoint} url={url[:80]}… opened={opened}")
    return {"ok": True, "url": url, "opened": opened, "model": effective}


# ---------------------------------------------------------------------------
# 启动
# ---------------------------------------------------------------------------

def build_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="WorkBuddy -> OpenAI/Anthropic 算力转接网关")
    ap.add_argument("--config", default=None, help="config.json 路径")
    ap.add_argument("--host", default=None)
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--api-key", default=None)
    ap.add_argument("--auth-dir", dest="auth_dir", default=None, help="auth 目录(覆盖自动检测)")
    ap.add_argument("--log", default=None, metavar="PATH", help="轮转日志文件路径")
    ap.add_argument("--desensitize", action="store_true",
                    help="启用脱敏,缓解后端内容审核误伤(合规模板敏感词零宽空格)")
    ap.add_argument("--no-compact", action="store_true",
                    help="配合 --desensitize:仅零宽脱敏,保留完整 system 原文")
    ap.add_argument("--skip-check", action="store_true", help="跳过启动预检")
    return ap.parse_args()


def preflight(pool: AccountPool) -> bool:
    sys.stderr.write("==== 预检 ====\n")
    sys.stderr.write(f"平台      : {sys.platform}\n")
    sys.stderr.write(f"Python    : {sys.version.split()[0]}\n")
    ok = True
    if not pool.names():
        sys.stderr.write("[警告] 未找到登录文件。请在桌面端完成登录(CodeBuddy/WorkBuddy)。\n")
        ok = False
    for name, cred in pool.ordered:
        try:
            s = cred.summary()
            sys.stderr.write(f"账号[{name}] {s['region']} {s['nickname']} | 后端 {s['backend']} | token过期: {'是(将自动刷新)' if s['token_expired'] else '否'}\n")
        except Exception as e:
            sys.stderr.write(f"账号[{name}] 读取凭据失败: {e}\n")
            ok = False
    sys.stderr.write(f"后端 CN  : {BACKEND_CN}\n")
    sys.stderr.write(f"后端 GLOB : {BACKEND_GLOBAL}\n")
    sys.stderr.write("================\n")
    return ok


def _effective_config(args: argparse.Namespace) -> config_mod.Config:
    """合并配置: 显式 --config 文件 / 默认搜索 config.json + CLI 参数。"""
    if args.config:
        path = os.path.abspath(os.path.expanduser(args.config))
        try:
            with open(path, encoding="utf-8") as f:
                file_conf = json.load(f)
        except Exception as e:
            sys.stderr.write(f"[warn] 读取配置 {path} 失败({e}),使用默认\n")
            file_conf = {}
        base = config_mod.Config.from_dict(file_conf)
    else:
        base = config_mod.load_config()
    return config_mod.merge_with_cli(
        base,
        host=args.host, port=args.port, api_key=args.api_key,
        auth_dir=args.auth_dir,
        desensitize=(args.desensitize or None),
        no_compact=(args.no_compact or None),
        log_file=args.log,
    )


def _resolve_config_file(args: argparse.Namespace) -> str | None:
    """返回实际使用的配置文件路径(用于 PUT /agents 持久化)。"""
    if args.config:
        return os.path.abspath(os.path.expanduser(args.config))
    for p in config_mod.config_path_candidates():
        if os.path.isfile(p):
            return os.path.abspath(p)
    return None


def main():
    args = build_args()
    cfg = _effective_config(args)
    cfg.agents = config_mod.merge_agents(cfg.agents)
    STATE["cfg"] = cfg
    STATE["api_key"] = cfg.api_key
    STATE["config_file"] = _resolve_config_file(args)

    # 账号池
    auth_dir = cfg.auth_dir or None
    if auth_dir:
        files = list(Path(auth_dir).glob("*.info")) if Path(auth_dir).is_dir() else []
    else:
        files = find_auth_files()
    pool = AccountPool.from_files(files, accounts_config=cfg.accounts)
    STATE["pool"] = pool

    # 日志
    setup_log(cfg.log_file, cfg.log_max_bytes, cfg.log_backups)

    if not args.skip_check:
        preflight(pool)

    # 模型目录:立即后台同步 + 周期同步
    STATE["catalog"] = ModelCatalog()
    threading.Thread(target=lambda: (_sync_catalog_now()), daemon=True).start()
    threading.Thread(target=_catalog_worker, daemon=True).start()

    sys.stderr.write(f"\n✅ 监听 http://{cfg.host}:{cfg.port}(账号: {', '.join(pool.names()) or '(无)'})\n")
    sys.stderr.write("   GET  /v1/models\n")
    sys.stderr.write("   POST /v1/chat/completions\n")
    sys.stderr.write("   POST /v1/responses           (Codex CLI)\n")
    sys.stderr.write("   POST /v1/messages            (Claude Code / CC Switch)\n")
    sys.stderr.write("   GET  /credits | POST /credits/checkin | GET /health\n")
    if cfg.api_key:
        sys.stderr.write("   鉴权已启用(API key)\n")
    if cfg.log_file:
        sys.stderr.write(f"   日志      : {cfg.log_file} (轮转 {cfg.log_max_bytes}B x {cfg.log_backups})\n")
    if cfg.desensitize:
        sys.stderr.write(f"   脱敏      : 已启用({'零宽脱敏+保留全文' if cfg.no_compact else '零宽脱敏+压缩'})\n")
    sys.stderr.write("按 Ctrl+C 退出。\n\n")

    _log(f"==== gateway 启动 host={cfg.host} port={cfg.port} accounts={pool.names()} ====")
    uvicorn.run(app, host=cfg.host, port=cfg.port, log_level="warning")


if __name__ == "__main__":
    main()