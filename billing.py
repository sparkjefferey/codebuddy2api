#!/usr/bin/env python3
"""
billing.py — 积分余额查询与每日签到(对齐参照实现 Sliverkiss/workbuddy2api)。

- 余额:  POST {billing_base}/v2/billing/meter/get-user-resource
- 签到:  POST {billing_base}/v2/billing/meter/daily-checkin
billing_base: CN=https://www.codebuddy.cn,GLOBAL=https://www.workbuddy.ai
"""

from __future__ import annotations

import time

import httpx

from references.codebuddy2api.accounts import CredentialManager

PRODUCT_CODE = "p_tcaca"


def _billing_headers(cred: CredentialManager) -> dict:
    h = cred.get_headers()
    h["Accept"] = "application/json"
    return h


async def query_credits(cred: CredentialManager, *, client: httpx.AsyncClient | None = None) -> dict:
    """查询账号当前可花费积分余额。失败返回 error,不影响主流程。"""
    url = f"{cred.billing_base}/v2/billing/meter/get-user-resource"
    now = time.time()
    body = {
        "PageNumber": 1,
        "PageSize": 100,
        "ProductCode": PRODUCT_CODE,
        "Status": [0, 3],
        "PackageEndTimeRangeBegin": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(now)),
        "PackageEndTimeRangeEnd": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(now + 365 * 101 * 86400)),
    }
    meta = {"name": cred.name, "region": cred.region, "backend": cred.billing_base}
    own = client is None
    c = client or httpx.AsyncClient(timeout=15)
    try:
        r = await c.post(url, headers=_billing_headers(cred), json=body)
    finally:
        if own:
            await c.aclose()
    if r.status_code != 200:
        return {**meta, "error": f"HTTP {r.status_code}"}
    d = r.json()
    if d.get("code") != 0:
        return {**meta, "error": f"code={d.get('code')} msg={d.get('msg') or d.get('message') or ''}"}
    accounts = (d.get("data") or {}).get("Response", {}).get("Data", {}).get("Accounts", [])
    remain = 0
    raw = []
    for a in accounts:
        cap_remain = a.get("CapacityRemain") or 0
        cycle_size = a.get("CycleCapacitySize") or 0
        cycle_remain = a.get("CycleCapacityRemain") or 0
        cycle_used = a.get("CycleCapacityUsed") or 0
        if cycle_size > 0:
            r_val = cycle_remain
        elif cycle_remain > 0 or cycle_used > 0:
            r_val = cycle_remain
        else:
            r_val = cap_remain
        r_val = max(int(r_val or 0), 0)
        remain += r_val
        raw.append({
            "package": a.get("PackageName"),
            "capacity_remain": a.get("CapacityRemain"),
            "cycle_capacity_remain": a.get("CycleCapacityRemain"),
        })
    return {**meta, "credits_remaining": remain, "packages": raw}


async def daily_checkin(cred: CredentialManager, *, client: httpx.AsyncClient | None = None) -> dict:
    """执行每日签到。返回成功/已签到/错误信息。

    注意: 签名与额度类似,后端对"已签到"返回 HTTP 400 + code!=0,body 仍可解析。
    """
    url = f"{cred.billing_base}/v2/billing/meter/daily-checkin"
    meta = {"name": cred.name, "region": cred.region}
    own = client is None
    c = client or httpx.AsyncClient(timeout=15)
    try:
        r = await c.post(url, headers=_billing_headers(cred), json={})
    finally:
        if own:
            await c.aclose()
    try:
        d = r.json()
    except Exception:
        return {**meta, "ok": False, "error": f"HTTP {r.status_code}: {r.text[:120]}"}
    code = d.get("code")
    if code == 0:
        return {**meta, "ok": True, "message": "签到成功"}
    return {**meta, "ok": False, "message": d.get("msg") or f"已签到(code={code})", "code": code}