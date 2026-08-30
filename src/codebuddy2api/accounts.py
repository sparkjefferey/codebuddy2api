#!/usr/bin/env python3
"""
accounts.py — 多账号凭据管理层。

- CredentialManager: 单个登录态文件(workbuddy-desktop.info 等)的读取、
  自动刷新与原子回写;按账号 region 提供对应后端主机。
- AccountPool: 扫描 auth 目录里的全部 *.info,按名称注册多个账号;
  提供按名取用、健康名单与按 region 过滤。

凭据 header 方案对齐官方 CodeBuddy CLI 与社区实测通过的参照实现
(Sliverkiss/workbuddy2api):
  X-Product / Origin / Referer / X-Requested-With 缺失是 401/
  ClientApiAuthenticationException 的根因;空字段用 X-No-* 约定表达
  "字段缺失";chat 请求绝不携带 X-Refresh-Token(刷新用它自己的端点)。
"""

from __future__ import annotations

import json
import os
import sys
import threading
import time
from pathlib import Path
from typing import Optional

import httpx

BACKEND_CN = "https://copilot.tencent.com"
BACKEND_GLOBAL = "https://www.workbuddy.ai"
BILLING_CN = "https://www.codebuddy.cn"
BILLING_GLOBAL = "https://www.workbuddy.ai"
GLOBAL_DOMAIN_SUFFIX = ".workbuddy.ai"
DEFAULT_DOMAIN = "www.codebuddy.cn"
# 与官方 CodeBuddy CLI 一致(参照实现实测可用)
USER_AGENT = "CLI/2.63.2 CodeBuddy/2.63.2"


# ---------------------------------------------------------------------------
# 路径定位
# ---------------------------------------------------------------------------

def default_auth_dirs() -> list[Path]:
    env_dir = os.environ.get("CODEBUDDY_AUTH_DIR")
    if env_dir:
        return [Path(env_dir)]
    home = Path.home()
    plat = sys.platform
    if plat == "darwin":
        return [home / "Library" / "Application Support" / "CodeBuddyExtension" / "Data" / "Public" / "auth"]
    if plat == "win32":
        local = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        return [local / "CodeBuddyExtension" / "Data" / "Public" / "auth"]
    xdg = Path(os.environ.get("XDG_DATA_HOME", home / ".local" / "share"))
    return [xdg / "CodeBuddyExtension" / "Data" / "Public" / "auth"]


def find_auth_files(dirs: Optional[list[Path]] = None) -> list[Path]:
    """按目录查找全部 *.info 登录态文件(不排序优先级,交由调用方语义)。"""
    files: list[Path] = []
    for d in dirs or default_auth_dirs():
        if d.is_dir():
            files.extend(sorted(d.glob("*.info")))
    return files


def region_from_domain(domain: str | None) -> str:
    d = str(domain or "").strip().lower()
    return "global" if d.endswith(GLOBAL_DOMAIN_SUFFIX) else "cn"


def backend_for_domain(domain: str | None) -> str:
    return BACKEND_GLOBAL if region_from_domain(domain) == "global" else BACKEND_CN


# ---------------------------------------------------------------------------
# 单账号凭据
# ---------------------------------------------------------------------------

class CredentialManager:
    """从 auth 文件读取凭据;token 临近过期时自动刷新并回写。"""

    def __init__(self, path: Path):
        self.path = Path(path)
        self.name = self.path.stem
        self._lock = threading.Lock()
        self._cached: dict | None = None
        self._mtime: float = 0.0

    # ---- 读取 ----

    def _read_raw(self) -> dict:
        with open(self.path, "r", encoding="utf-8") as f:
            return json.load(f)

    def _load_if_stale(self):
        try:
            mt = self.path.stat().st_mtime
        except OSError:
            return
        if self._cached is None or mt != self._mtime:
            self._cached = self._read_raw()
            self._mtime = mt

    def _session(self) -> dict:
        self._load_if_stale()
        if self._cached is None:
            raise RuntimeError(f"无法读取 auth 文件：{self.path}")
        return self._cached

    def _session_headers_parts(self) -> tuple[dict, dict]:
        s = self._session()
        return s.get("auth") or {}, s.get("account") or {}

    # ---- 账号元信息 ----

    @property
    def domain(self) -> str:
        auth, _ = self._session_headers_parts()
        return auth.get("domain") or DEFAULT_DOMAIN

    @property
    def region(self) -> str:
        return region_from_domain(self.domain)

    @property
    def backend_base(self) -> str:
        return backend_for_domain(self.domain)

    @property
    def billing_base(self) -> str:
        return BILLING_GLOBAL if self.region == "global" else BILLING_CN

    def is_expired(self) -> bool:
        auth, _ = self._session_headers_parts()
        expires_at = auth.get("expiresAt") or 0
        # 提前 60s 判定过期
        return time.time() * 1000 >= (expires_at - 60_000)

    # ---- 刷新 ----

    def refresh(self):
        """调后端刷新 token，写回 auth 文件与缓存。"""
        auth, account = self._session_headers_parts()
        headers = self._build_headers_from(auth, account)
        headers.pop("Authorization", None)          # 刷新以其 refreshToken 为凭
        headers["X-Refresh-Token"] = auth.get("refreshToken", "")
        headers["X-Auth-Refresh-Source"] = "workbuddy"
        url = f"{self.backend_base}/v2/plugin/auth/token/refresh"
        try:
            with httpx.Client(timeout=15) as c:
                r = c.post(url, headers=headers, json={})
            data = r.json()
        except Exception as e:
            raise RuntimeError(f"刷新 token 网络失败：{e}")
        if data.get("code") != 0 or not data.get("data"):
            raise RuntimeError(f"刷新 token 失败：{data.get('msg', data)}")
        new_auth = data["data"]
        # 继承部分字段
        new_auth["domain"] = new_auth.get("domain") or auth.get("domain")
        new_auth["lastRefreshTime"] = int(time.time() * 1000)
        if not new_auth.get("expiresAt") and new_auth.get("expiresIn"):
            new_auth["expiresAt"] = int(time.time() * 1000) + new_auth["expiresIn"] * 1000
        if not new_auth.get("refreshExpiresAt") and new_auth.get("refreshExpiresIn"):
            new_auth["refreshExpiresAt"] = int(time.time() * 1000) + new_auth["refreshExpiresIn"] * 1000
        s = self._session()
        s["auth"] = new_auth
        # 原子写回
        tmp = self.path.with_suffix(self.path.suffix + ".tmp")
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(s, f, ensure_ascii=False, indent=2)
        os.replace(tmp, self.path)
        self._cached = s
        self._mtime = self.path.stat().st_mtime

    def get_headers(self) -> dict:
        """返回带最新 token 的后端请求 header；必要时先刷新。"""
        with self._lock:
            if self.is_expired():
                self.refresh()
            auth, account = self._session_headers_parts()
            return self._build_headers_from(auth, account)

    def get_headers_for(self, *, with_authorization: bool = True) -> dict:
        """按需取 header;供 billing 之类不想带或想只带授权头的地方复用。"""
        with self._lock:
            auth, account = self._session_headers_parts()
            h = self._build_headers_from(auth, account)
            if not with_authorization:
                h.pop("Authorization", None)
            return h

    def _build_headers_from(self, auth: dict, account: dict) -> dict:
        domain = auth.get("domain") or DEFAULT_DOMAIN
        origin = BACKEND_GLOBAL if region_from_domain(domain) == "global" else DEFAULT_DOMAIN
        h = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/plain, */*",
            "X-Requested-With": "XMLHttpRequest",
            "Origin": f"https://{origin}",
            "Referer": f"https://{origin}/",
            "User-Agent": USER_AGENT,
            "X-Product": "SaaS",
        }
        token = auth.get("accessToken", "")
        if token:
            h["Authorization"] = "Bearer " + token
        else:
            h["X-No-Authorization"] = "1"
        uid = account.get("uid", "")
        if uid:
            h["X-User-Id"] = uid
        else:
            h["X-No-User-Id"] = "1"
        ent = account.get("enterpriseId") or account.get("enterpriseName") or ""
        if ent:
            h["X-Enterprise-Id"] = ent
        else:
            h["X-No-Enterprise-Id"] = "1"
        if domain:
            h["X-Domain"] = domain
        else:
            h["X-No-Department-Info"] = "1"
        return h

    def summary(self) -> dict:
        auth, acct = self._session_headers_parts()
        return {
            "name": self.name,
            "region": self.region,
            "domain": auth.get("domain"),
            "backend": self.backend_base,
            "uid": acct.get("uid"),
            "nickname": acct.get("nickname"),
            "token_expires_at": auth.get("expiresAt", 0),
            "token_expired": self.is_expired(),
        }


# ---------------------------------------------------------------------------
# 账号池
# ---------------------------------------------------------------------------

class AccountPool:
    """持有一组账号(名称 → CredentialManager)),提供选择与健康视图。"""

    def __init__(self, ordered: list[tuple[str, CredentialManager]]):
        self.ordered = list(ordered)
        self.by_name = {name: cred for name, cred in ordered}

    # ---- 构造 ----

    @classmethod
    def from_files(cls, files: list[Path],
                   accounts_config: Optional[list[dict]] = None) -> "AccountPool":
        """扫描 auth 文件构造账号池。

        accounts_config: [{name, auth_file}] —— name 优先于自动命名;
        auth_file 可为空(自动按该配置在 auth 目录中查找)。
        自动命名规则:单账号文件 -> 按 region('cn'/'global');多文件时
        用 '<region>-<stem>' 避免歧义。此处由调用方控制目录优先级。
        """
        items: list[tuple[str, CredentialManager]] = []

        if accounts_config:
            for cfg in accounts_config:
                name = cfg.get("name") or ""
                af = cfg.get("auth_file") or ""
                picked = None
                if af:
                    p = Path(af)
                    if p.is_file():
                        picked = p
                if picked is None:
                    # 在 files 里按 stem 或名字回退查找
                    for f in files:
                        if f.stem == name or f.stem.endswith(name):
                            picked = f
                            break
                    if picked is None and af:
                        picked = Path(af).expanduser()
                if picked is None:
                    continue
                cred = CredentialManager(picked)
                if not name:
                    name = cred.region if len(items) == 0 else cred.region + "-" + cred.name
                items.append((name, cred))
            return cls(items)

        # 没有显式配置：全部文件入池，自动命名
        used_regions: set[str] = set()
        for f in files:
            cred = CredentialManager(f)
            r = cred.region
            if r in used_regions:
                name = f"{r}-{cred.name}"
            else:
                name = r
            used_regions.add(r)
            items.append((name, cred))
        return cls(items)

    # ---- 查询 ----

    def names(self) -> list[str]:
        return [n for n, _ in self.ordered]

    def get(self, name: str) -> CredentialManager | None:
        return self.by_name.get(name)

    def region_accounts(self, region: str) -> list[str]:
        """返回该 region 的账号名(按池顺序)。"""
        return [name for name, cred in self.ordered if cred.region == region]

    def all(self) -> list[CredentialManager]:
        return [cred for _, cred in self.ordered]

    def healthy(self, max_age_refresh: bool = False) -> list[CredentialManager]:
        """未过期且能读出凭据的账号。"""
        out = []
        for _, cred in self.ordered:
            try:
                if not cred.is_expired():
                    out.append(cred)
            except Exception:
                continue
        return out

    def first(self, region: str | None = None) -> CredentialManager | None:
        if region:
            accts = self.region_accounts(region)
        else:
            accts = self.all()
        if not accts:
            return None
        healthy = [c for c in accts if not c.is_expired()] or accts
        return healthy[0]

    def other_than(self, name: str) -> list[CredentialManager]:
        return [cred for nm, cred in self.ordered if nm != name]

    def summary(self) -> list[dict]:
        out = []
        for name, cred in self.ordered:
            try:
                s = cred.summary()
                s["name"] = name            # 池内账号名(路由/头用),覆盖文件 stem
                out.append(s)
            except Exception as e:
                out.append({"name": name, "error": str(e)})
        return out