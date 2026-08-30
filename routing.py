#!/usr/bin/env python3
"""
routing.py — 请求→账号 路由与故障切换链。

选择优先级:
  1. 显式头 `X-WB-Account: <账号名>` 最优先;
  2. 模型别名 `global/xxx` / `g:xxx` → 强制 global 账号;
  3. 按模型能力: 该模型只存在于某账号后端时自动选它; 重名模型按
     default_account(默认第一个健康账号) 选择;
  4. 'auto'/'default' 或未知模型 → 默认账号;
  5. 兜底: 配置的 default_account 或第一个健康账号。

`candidate_chain()` 返回有序账号名列表,第一个是路由结果,其余用于
上游鉴权失败时的故障切换(一用一降级,不再回环)。
"""

from __future__ import annotations

from references.codebuddy2api.accounts import AccountPool
from references.codebuddy2api.models_catalog import ModelCatalog

GLOBAL_PREFIXES = ("global/", "g:", "wb-global/")
SPECIAL_MODELS = {"auto", "default"}


def split_alias(model: str) -> tuple[str | None, str]:
    """把 'global/glm-5.2' → ('global', 'glm-5.2'); 裸模型 → (None, model)。"""
    model = (model or "").strip()
    lower = model.lower()
    for prefix in GLOBAL_PREFIXES:
        if lower.startswith(prefix):
            return "global", model[len(prefix):]
    return None, model


def pick_account(pool: AccountPool,
                 catalog: ModelCatalog | None,
                 model: str,
                 *,
                 account_header: str | None = None,
                 default_account: str = "") -> str | None:
    chain = candidate_chain(pool, catalog, model, account_header=account_header,
                            default_account=default_account)
    return chain[0] if chain else None


def candidate_chain(pool: AccountPool,
                    catalog: ModelCatalog | None,
                    model: str,
                    *,
                    account_header: str | None = None,
                    default_account: str = "") -> list[str]:
    """返回有序账号名: 首选 + 其余账号(用于故障切换)。"""
    orders = pool.names()
    if not orders:
        return []

    # 1) 显式账号头
    if account_header and pool.get(account_header) is not None:
        return _ordered_with_fallback(orders, account_header)

    model = (model or "").strip()
    region, plain = split_alias(model)

    # 2) 别名强制 region
    if region:
        ra = pool.region_accounts(region)
        if ra:
            return _ordered_with_fallback(orders, ra[0])

    # 3) 按模型能力路由(特异模型只出现在某账号)
    if plain and plain not in SPECIAL_MODELS and catalog is not None:
        owners = [n for n in catalog.accounts_for(plain) if n in pool.by_name]
        if owners:
            primary = default_account if default_account in owners else owners[0]
            return _ordered_with_fallback(orders, primary)

    # 4/5) 默认账号或第一个健康账号
    if default_account and pool.get(default_account) is not None:
        return _ordered_with_fallback(orders, default_account)
    healthy = pool.healthy()
    if healthy:
        return _ordered_with_fallback(orders, healthy[0].name)
    return orders


def _ordered_with_fallback(names: list[str], primary: str) -> list[str]:
    """primary 打头,其余按池顺序,去重。"""
    if primary not in names:
        return names
    return [primary] + [n for n in names if n != primary]