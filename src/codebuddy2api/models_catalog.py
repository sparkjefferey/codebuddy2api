#!/usr/bin/env python3
"""
models_catalog.py — 按账号收集可用模型目录及权威定价,支持动态拉取与静态回退。

定价/免费判定以**后端元数据 `credits` 字段(积分倍率)为准**:
  - `credits == "x0.00"`(且常带 `badge:限时免费`) → 该模型免费;
  - 其余按倍率计费(auto 为浮动倍率,无固定值)。
CN 动态接口返回该字段;global 接口当前 500,使用静态回退表(已实测校准)。

x_free 判定的优先级(降序):
  1. 后端元数据 credits==0
  2. 配置 free_models(账号级种子/覆盖),用于无元数据账号(如 global)
"""

from __future__ import annotations

import json
import re
import threading
import time
from typing import Any, Optional

import httpx

from codebuddy2api.accounts import CredentialManager

# global 账号真实探测出的可用模型(2026-08-29);credits 置空表示**定价未知**。
# 注意:国际站(workbuddy.ai)的模型目录接口当前 500 取不到权威 credits 倍率,
# 而用量里的 usage.credit 在请求量小时会被舍入成 0(DeepSeek/GLM 均如此),
#   **绝不能**因此把某个 global 模型判成免费。经大产出实测:
#   glm-5.2~x0.06/篇、minimax-m3~x0.04、auto~x0.07、glm-5v-turbo~x0.07、glm-5.3~x0.2,
#   全部计费(hy4-preview 因思考型耗光 max_tokens 无产出而显示 0,同样是假象)。
GLOBAL_FALLBACK_MODELS: list[dict] = [
    {"id": "auto", "name": "auto", "maxInputTokens": None, "maxOutputTokens": None},
    {"id": "hy4-preview", "name": "hy4-preview", "maxInputTokens": 1000000, "maxOutputTokens": 64000},
    {"id": "glm-5.3", "name": "glm-5.3", "maxInputTokens": 1000000, "maxOutputTokens": 48000},
    {"id": "glm-5.2", "name": "glm-5.2", "maxInputTokens": 1000000, "maxOutputTokens": 48000},
    {"id": "glm-5v-turbo", "name": "glm-5v-turbo", "maxInputTokens": 200000, "maxOutputTokens": 64000},
    {"id": "minimax-m3", "name": "minimax-m3", "maxInputTokens": 512000, "maxOutputTokens": 128000},
]

# CN 兜底(动态失败时使用)。credits 取自 CN 后端 2026-08-29 的实际元数据:
# hy3 / hy4-preview = x0.00(限时免费)。
CN_FALLBACK_MODELS: list[dict] = [
    {"id": "auto", "name": "Auto", "maxInputTokens": 168000, "maxOutputTokens": 32000},
    {"id": "hy3", "name": "Hy3", "maxInputTokens": 192000, "maxOutputTokens": 64000, "credits": "x0.00", "badges": ["badge:限时免费:#FF0000"]},
    {"id": "hy3-x", "name": "Hy3 x", "maxInputTokens": 192000, "maxOutputTokens": 64000, "credits": "x0.05"},
    {"id": "hy4-preview", "name": "Hy4 preview", "maxInputTokens": 1000000, "maxOutputTokens": 64000, "credits": "x0.00", "badges": ["badge:限时免费:#FF0000"]},
    {"id": "hy4-preview-x", "name": "Hy4 preview x", "maxInputTokens": 1000000, "maxOutputTokens": 64000, "credits": "x0.29"},
    {"id": "default", "name": "Default", "maxInputTokens": 200000, "maxOutputTokens": 24000, "credits": "x2.20"},
    {"id": "glm-5.3", "name": "Glm 5.3", "maxInputTokens": 1000000, "maxOutputTokens": 48000, "credits": "x0.79"},
    {"id": "glm-5.3-flash", "name": "Glm 5.3 flash", "maxInputTokens": 1000000, "maxOutputTokens": 32000, "credits": "x0.06"},
    {"id": "glm-5.2", "name": "Glm 5.2", "maxInputTokens": 1000000, "maxOutputTokens": 48000, "credits": "x0.79"},
    {"id": "glm-5v-turbo", "name": "Glm 5v turbo", "maxInputTokens": 200000, "maxOutputTokens": 64000, "credits": "x0.71"},
    {"id": "deepseek-v4-flash", "name": "Deepseek v4 flash", "maxInputTokens": 1000000, "maxOutputTokens": 50000, "credits": "x0.17"},
    {"id": "deepseek-v4-pro", "name": "Deepseek v4 pro", "maxInputTokens": 1000000, "maxOutputTokens": 50000, "credits": "x0.51"},
    {"id": "kimi-k3-1", "name": "Kimi k3.1", "maxInputTokens": 1000000, "maxOutputTokens": 32000, "credits": "x1.62"},
    {"id": "kimi-k2.7", "name": "Kimi k2.7", "maxInputTokens": 256000, "maxOutputTokens": 32000, "credits": "x0.57"},
    {"id": "minimax-m3", "name": "MiniMax m3", "maxInputTokens": 512000, "maxOutputTokens": 128000, "credits": "x0.25"},
    {"id": "hunyuan-2.0-thinking", "name": "Hunyuan 2.0 thinking", "maxInputTokens": 128000, "maxOutputTokens": 24000, "credits": "x0.04"},
]

# 动态拉取的候选路径(按序尝试)
MODELS_PATH_CANDIDATES = [
    "/console/enterprises/personal/models",
    "/console/enterprises/models",
]


def _parse_credits(raw: Any) -> float | None:
    """从 'x0.79 credits' / 'x0.00' 提取倍率;无则 None。"""
    if not isinstance(raw, str):
        return None
    m = re.search(r"x([\d.]+)", raw)
    return float(m.group(1)) if m else None


def _model_meta(m: dict) -> dict:
    """把后端模型条目规整成目录 meta(含权威定价)。"""
    raw_tags = m.get("tags") if m.get("tags") is not None else m.get("badges") or []
    tags = [str(t) for t in raw_tags if isinstance(t, str)]
    badges = [t for t in tags if t.startswith("badge:") or "免费" in t]
    credits = _parse_credits(m.get("credits"))
    return {
        "id": m.get("id"),
        "name": m.get("name"),
        "maxInputTokens": m.get("maxInputTokens"),
        "maxOutputTokens": m.get("maxOutputTokens"),
        "credits": credits,
        "free": credits == 0.0,
        "badges": badges,
    }


def _filter_usable(models: list[dict], agents: list[dict] | None) -> list[dict]:
    """过滤出 chat 可用模型:非 disabled;若后端给了 agents,只保留 'cli' 用到的。"""
    out: list[dict] = []
    if agents:
        cli_ids = set()
        for ag in agents:
            if isinstance(ag, dict) and ag.get("name") == "cli":
                cli_ids = set(ag.get("models") or [])
                break
        if cli_ids:
            for m in models:
                if m.get("id") in cli_ids and not m.get("disabled"):
                    out.append(m)
            return out
    for m in models:
        if not m.get("disabled"):
            out.append(m)
    return out


class ModelCatalog:
    """按账号名维护 {model_id: meta},提供路由与合并视图。线程安全。"""

    def __init__(self):
        self._lock = threading.Lock()
        self._by_account: dict[str, dict[str, dict]] = {}
        self._free_accounts: dict[str, set[str]] = {}
        self._errors: dict[str, str] = {}
        self._last_sync: float = 0.0

    # ---- 状态 ----

    def account_names(self) -> list[str]:
        with self._lock:
            return list(self._by_account.keys())

    def models_of(self, account: str) -> list[str]:
        with self._lock:
            return list((self._by_account.get(account) or {}).keys())

    def accounts_for(self, model: str) -> list[str]:
        with self._lock:
            return [name for name, m in self._by_account.items() if model in m]

    def has(self, account: str, model: str) -> bool:
        with self._lock:
            return model in (self._by_account.get(account) or {})

    def errors(self) -> dict[str, str]:
        with self._lock:
            return dict(self._errors)

    def last_sync_at(self) -> float:
        with self._lock:
            return self._last_sync

    # ---- 写入 ----

    def set_account_catalog(self, account: str, models: list[dict], error: str | None = None):
        with self._lock:
            meta = {_model_meta(m)["id"]: _model_meta(m) for m in models if m.get("id")}
            if meta:
                self._by_account[account] = meta
            if error and not meta:
                self._errors[account] = error
            self._last_sync = time.time()

    def apply_overrides(self, account: str, model_ids: list) -> None:
        with self._lock:
            cur = self._by_account.setdefault(account, {})
            for mid in model_ids or []:
                if str(mid) not in cur:
                    cur[str(mid)] = {"id": str(mid), "name": str(mid)}

    # ---- 免费 ----

    def mark_free(self, account: str, ids: list) -> None:
        """把账号的某些模型标记为免费(配置种子/覆盖)。"""
        with self._lock:
            self._free_accounts.setdefault(account, set()).update(str(i) for i in (ids or []))

    def is_free(self, account: str, model: str) -> bool:
        with self._lock:
            meta = (self._by_account.get(account) or {}).get(model)
            if meta and meta.get("free"):
                return True
            return model in (self._free_accounts.get(account) or set())

    # ---- 同步 ----

    def sync(self, pool) -> None:
        """对池内每个账号:优先动态拉取,失败用内置回退。账号 key = 池账号名。"""
        for name, cred in pool.ordered:
            models, error = _fetch_account(cred)
            if not models:
                models = _fallback_for(cred)
                if not error:
                    error = "dynamic fetch failed; using fallback"
            self.set_account_catalog(name, models, error=error)

    # ---- 合并视图 ----

    def merged_models(self, with_global_aliases: bool = True) -> list[dict]:
        """统一 OpenAI /v1/models 列表。global 账号模型额外以 'global/<id>' 别名暴露。

        每条带 x_account / x_free / x_credits(倍率)/ badges(角标)。
        """
        out: list[dict] = []
        seen: set[str] = set()
        with self._lock:
            observed = False  # 保留命名占位(未来恢复学习用)
            for account, meta in self._by_account.items():
                is_global = account == "global" or "global" in account
                for mid, m in sorted(meta.items()):
                    item = {
                        "id": mid,
                        "object": "model",
                        "created": 1700000000,
                        "owned_by": "workbuddy",
                        "x_account": account,
                        "x_free": (m.get("free") or mid in (self._free_accounts.get(account) or set())),
                        "x_credits": m.get("credits"),
                        "badges": m.get("badges") or [],
                    }
                    if m.get("maxInputTokens"):
                        item["context_window"] = m["maxInputTokens"]
                    if m.get("maxOutputTokens"):
                        item["max_output_tokens"] = m["maxOutputTokens"]
                    if not is_global:
                        if mid not in seen:
                            out.append(item)
                            seen.add(mid)
                    else:
                        out.append(dict(item, id=mid))
                        out[-1]["id"] = f"global/{mid}"
                        if mid in seen:
                            continue
            # 保证 'auto' 在内
            if "auto" not in {m["id"].split("/")[-1] for m in out}:
                out.insert(0, {"id": "auto", "object": "model", "created": 1700000000, "owned_by": "workbuddy"})
            return out


def _fallback_for(cred: CredentialManager) -> list[dict]:
    if cred.region == "global":
        return GLOBAL_FALLBACK_MODELS
    return CN_FALLBACK_MODELS


def _fetch_account(cred: CredentialManager) -> tuple[list[dict], str | None]:
    """尝试拉取某个账号的动态模型目录。返回 (models, error)。"""
    headers = cred.get_headers()
    last_err = ""
    for path in MODELS_PATH_CANDIDATES:
        try:
            with httpx.Client(timeout=20) as c:
                r = c.get(f"{cred.backend_base}{path}", headers=headers)
            if r.status_code != 200:
                last_err = f"HTTP {r.status_code}"
                continue
            data = r.json()
            inner = data.get("data") or {}
            models = _filter_usable(inner.get("models") or [], inner.get("agents"))
            if models:
                return models, None
            last_err = "empty model list"
        except Exception as e:
            last_err = f"{type(e).__name__}: {e}"
    return [], last_err or "no models endpoint available"