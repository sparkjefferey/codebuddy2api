#!/usr/bin/env python3
"""
ccswitch.py — CC Switch(v2, com.ccswitch.desktop)一键注册。

不直接读写 CC Switch 的 ~/.cc-switch/cc-switch.db(SQLite),
而是生成其官方支持的 deeplink `ccswitch://v1/import?...`,
交由运行中的 CC Switch 自行落库,对 schema 变化免疫。
格式与本机已有实现一致(spark_api / sub2api 的 ccswitchImport.ts):

    resource=provider&app=claude&name=…&homepage=…&endpoint=…
    &apiKey=…[&model=…]&configFormat=json&usageEnabled=false
"""

from __future__ import annotations

import platform
import subprocess
import sys
import urllib.parse


def build_deeplink(*,
                   endpoint: str,
                   name: str = "WorkBuddy 算力网关",
                   api_key: str = "workbuddy",
                   model: str | None = None,
                   app: str = "claude") -> str:
    """构造 CC Switch 导入 deeplink。endpoint 不含 /v1/messages(由协议拼接)。"""
    params: list[tuple[str, str]] = [
        ("resource", "provider"),
        ("app", app),
        ("name", name),
        ("homepage", endpoint),
        ("endpoint", endpoint),
        ("apiKey", api_key),
    ]
    if model:
        params.insert(2, ("model", model))
    params += [
        ("configFormat", "json"),
        ("usageEnabled", "false"),
    ]
    return "ccswitch://v1/import?" + urllib.parse.urlencode(params)


def open_deeplink(url: str) -> bool:
    """把 deeplink 交给本机:macOS 用 open、Linux 用 xdg-open、Windows 用 start。"""
    if platform.system() == "Darwin":
        cmd = ["open", url]
    elif sys.platform.startswith("linux"):
        cmd = ["xdg-open", url]
    elif platform.system() == "Windows":
        cmd = ["cmd", "/c", "start", "", url]
    else:
        return False
    try:
        subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except Exception:
        return False


if __name__ == "__main__":
    print(build_deeplink(endpoint="http://127.0.0.1:8787", model="hy3", api_key="workbuddy"))