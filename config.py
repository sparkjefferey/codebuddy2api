#!/usr/bin/env python3
"""
config.py — 网关配置加载与合并。

优先级: CLI 参数 < config.json < 环境变量默认值。
config.json 缺省路径: 当前目录 ./config.json, 或环境变量 WB_CONFIG。
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


DEFAULT_CONFIG_PATH = "config.json"

# 要接入的下游 agent 的默认连接偏好(前端可编辑,存于 config.json 的 "agents" 段)
DEFAULT_AGENTS: dict = {
    "claude-code": {"name": "Claude Code", "protocol": "anthropic", "enabled": True,
                    "account": "", "model": "hy3"},
    "codex": {"name": "Codex CLI", "protocol": "responses", "enabled": True,
              "account": "", "model": "glm-5.3"},
    "openai": {"name": "OpenAI 兼容客户端 (Cherry Studio / ZCode 等)", "protocol": "chat",
               "enabled": True, "account": "", "model": "auto"},
}

_REQ_AGENT_KEYS = {"name", "protocol", "enabled", "account", "model"}


@dataclass
class Config:
    host: str = "127.0.0.1"
    port: int = 8787
    api_key: str = ""
    auth_dir: str = ""
    accounts: list[dict] = field(default_factory=list)   # [{name, auth_file}]
    default_account: str = ""                            # "auto" | 账号名
    desensitize: bool = False
    no_compact: bool = False
    model_sync_interval_hours: int = 24
    model_overrides: dict[str, list] = field(default_factory=dict)  # {account: [models]}
    free_models: dict[str, list] = field(default_factory=dict)      # {account: [models] 免费模型}
    agents: dict = field(default_factory=dict)                      # {agent_id: {...} 连接偏好}
    log_file: str = ""
    log_max_bytes: int = 10 * 1024 * 1024
    log_backups: int = 3

    @classmethod
    def from_dict(cls, d: dict) -> "Config":
        known = {f.name for f in cls.__dataclass_fields__.values()}
        kwargs = {k: v for k, v in d.items() if k in known}
        return cls(**kwargs)

    def to_dict(self) -> dict:
        return {
            "host": self.host, "port": self.port, "api_key": self.api_key,
            "auth_dir": self.auth_dir, "accounts": self.accounts,
            "default_account": self.default_account,
            "desensitize": self.desensitize, "no_compact": self.no_compact,
            "model_sync_interval_hours": self.model_sync_interval_hours,
            "model_overrides": self.model_overrides,
            "free_models": self.free_models,
            "agents": self.agents,
            "log_file": self.log_file, "log_max_bytes": self.log_max_bytes,
            "log_backups": self.log_backups,
        }


def config_path_candidates() -> list[str]:
    env = os.environ.get("WB_CONFIG")
    if env:
        return [env]
    return [DEFAULT_CONFIG_PATH, os.path.expanduser("~/.config/workbuddy2api/config.json")]


def load_config_file() -> dict:
    for p in config_path_candidates():
        if os.path.isfile(p):
            with open(p, "r", encoding="utf-8") as f:
                data = json.load(f)
            if not isinstance(data, dict):
                raise ValueError(f"配置格式错误(应为 JSON 对象): {p}")
            return data
    return {}


def merge_with_cli(config: Config,
                   *,
                   host: str | None = None,
                   port: int | None = None,
                   api_key: str | None = None,
                   auth_dir: str | None = None,
                   desensitize: bool | None = None,
                   no_compact: bool | None = None,
                   log_file: str | None = None,
                   env_key_api: str = "WORKBUDDY2API_KEY",
                   env_key_log: str = "WORKBUDDY2API_LOG") -> Config:
    """将可选的 CLI/环境值并入配置。None 表示 CLI 未给出,沿用配置或环境默认。"""
    cfg = config

    def pick(cli: Any, env: str, default: Any) -> Any:
        if cli is not None:
            return cli
        return os.environ.get(env, default)

    cfg.host = pick(host, "WORKBUDDY2API_HOST", cfg.host)
    cfg.port = int(pick(port, "WORKBUDDY2API_PORT", cfg.port))
    cfg.api_key = pick(api_key, env_key_api, cfg.api_key)
    cfg.auth_dir = pick(auth_dir, "CODEBUDDY_AUTH_DIR", cfg.auth_dir)
    if desensitize is not None:
        cfg.desensitize = desensitize
    if no_compact is not None:
        cfg.no_compact = no_compact
    cfg.log_file = pick(log_file, env_key_log, cfg.log_file)
    return cfg


def merge_agents(raw: dict | None) -> dict:
    """以 DEFAULT_AGENTS 为骨架合并用户配置(过滤未知字段;补默认字段)。"""
    src = raw if isinstance(raw, dict) else {}
    merged: dict = {}
    for key, default in DEFAULT_AGENTS.items():
        merged[key] = dict(default)
        if isinstance(src.get(key), dict):
            merged[key].update({k: v for k, v in src[key].items() if k in _REQ_AGENT_KEYS})
    for key, val in src.items():
        if key not in merged and isinstance(val, dict):
            item = {k: v for k, v in val.items() if k in _REQ_AGENT_KEYS}
            item.setdefault("name", key)
            item.setdefault("protocol", "chat")
            item.setdefault("enabled", True)
            item.setdefault("account", "")
            item.setdefault("model", "auto")
            merged[key] = item
    return merged


def load_config(*, cli_args: Any = None) -> Config:
    """一站式加载: JSON 文件 + CLI 参数合并。cli_args 是可选的已解析 argparse Namespace。"""
    base = Config.from_dict(load_config_file())
    merged = base
    if cli_args is not None:
        merged = merge_with_cli(
            merged,
            host=getattr(cli_args, "host", None),
            port=getattr(cli_args, "port", None),
            api_key=getattr(cli_args, "api_key", None),
            auth_dir=getattr(cli_args, "auth_dir", None),
            desensitize=getattr(cli_args, "desensitize", None),
            no_compact=getattr(cli_args, "no_compact", None),
            log_file=getattr(cli_args, "log", None),
        )
    return merged