#!/usr/bin/env python3
"""
app.py — API Transmitter 的**原生桌面 App**(同进程:网关 + 原生窗口)。

与 `converter.py`(纯 headless 服务端)的区别:
  - converter.py : 只跑 HTTP 网关,界面是浏览器/PWA;
  - app.py       : 网关在**后台线程**跑,主线程开一个原生窗口(WKWebView /
                   WebView2 / WebKitGTK)内嵌同一个 `web/index.html`,
                   macOS 上以**菜单栏常驻**形态运行,关窗不退出。

原生观感的几个关键点(都是“网页界面”做不到的):
  - 真实独立窗口 + 独立 Dock/任务栏图标,不在浏览器里;
  - macOS 菜单栏(NSStatusBar)常驻:关窗隐藏、点图标恢复,菜单里直接
    「打开控制台 / 签到 / 显示配置文件 / 重启 / 退出」;
  - 窗口走 vibrancy 半透明标题栏,内容顶到窗口边缘(无浏览器工具栏/地址栏);
  - 前端通过 `window.pywebview.api` 直接调 Python(剪贴板/打开文件/退出等);
  - 退出时窗口与网关一起干净关闭。

用法:
  python -m codebuddy2api.app                 # 启动 App(网关 + 原生窗口)
  python -m codebuddy2api.app --no-window     # 只跑网关(等价 converter)
  python -m codebuddy2api.app --window-only   # 只开窗口(连接已运行的网关)
依赖: pywebview(可选;缺失时自动降级为 headless)。
"""

from __future__ import annotations

import argparse
import os
import socket
import sys
import tempfile
import threading
import time
import webbrowser
from pathlib import Path
from typing import Any

# 让「直接跑脚本」(`python src/codebuddy2api/app.py`)也能 import 到同包模块。
# `-m codebuddy2api.app` 时 __package__ == "codebuddy2api",路径由 PYTHONPATH 提供;
# 直接跑脚本时 __package__ 为空,需自行把仓库的 src/(本文件的上上级里的 src)挂到
# sys.path 最前面 —— 这里必须是 src/ 而非仓库根,否则 import 不到 codebuddy2api 包。
if not __package__:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import codebuddy2api.config as config_mod  # noqa: E402
import codebuddy2api.converter as gw  # noqa: E402

APP_NAME = "API Transmitter"
APP_TITLE = APP_NAME
WINDOW_WIDTH = 1180
WINDOW_HEIGHT = 800
WINDOW_MIN = (880, 600)

_window = None      # pywebview Window(主线程创建)
_menubar = None     # MenuBar 实例
RUNTIME: dict[str, Any] = {"started_at": 0.0, "window_mode": False}


# ---------------------------------------------------------------------------
# 网关侧:后台线程跑 uvicorn
# ---------------------------------------------------------------------------

def _port_open(host: str, port: int, timeout: float = 0.35) -> bool:
    probe_host = "127.0.0.1" if host in ("0.0.0.0", "::", "") else host
    try:
        with socket.create_connection((probe_host, port), timeout=timeout):
            return True
    except OSError:
        return False


def _wait_port(host: str, port: int, deadline: float = 20.0) -> bool:
    end = time.time() + deadline
    while time.time() < end:
        if _port_open(host, port):
            return True
        time.sleep(0.15)
    return False


def _serve(cfg: config_mod.Config) -> None:
    """阻塞式跑 uvicorn(在后台线程调用)。"""
    import uvicorn

    uvicorn.run(gw.app, host=cfg.host, port=cfg.port, log_level="warning")


def start_gateway(args: argparse.Namespace) -> config_mod.Config:
    """初始化网关状态(账号池/日志/模型目录),返回生效配置。不阻塞。"""
    from codebuddy2api.accounts import AccountPool, find_auth_files
    from codebuddy2api.models_catalog import ModelCatalog

    cfg = gw._effective_config(args)
    cfg.agents = config_mod.merge_agents(cfg.agents)
    gw.STATE["cfg"] = cfg
    gw.STATE["api_key"] = cfg.api_key
    gw.STATE["config_file"] = gw._resolve_config_file(args)

    auth_dir = cfg.auth_dir or None
    if auth_dir and Path(auth_dir).is_dir():
        files = list(Path(auth_dir).glob("*.info"))
    else:
        files = find_auth_files()
    pool = AccountPool.from_files(files, accounts_config=cfg.accounts)
    gw.STATE["pool"] = pool

    gw.setup_log(cfg.log_file, cfg.log_max_bytes, cfg.log_backups)

    gw.STATE["catalog"] = ModelCatalog()
    threading.Thread(target=gw._sync_catalog_now, daemon=True).start()
    threading.Thread(target=gw._catalog_worker, daemon=True).start()

    RUNTIME["started_at"] = time.time()
    gw._log(f"==== app 启动(网关在后台线程)host={cfg.host} port={cfg.port} accounts={pool.names()} ====")
    return cfg


# ---------------------------------------------------------------------------
# 暴露给前端的原生 API(window.pywebview.api.*)
# ---------------------------------------------------------------------------

class NativeApi:
    """前端通过 `window.pywebview.api.<name>(...)` 调用;返回 Promise。"""

    def __init__(self, cfg: config_mod.Config):
        self._cfg = cfg

    def env(self) -> dict:
        c = gw.cfg()
        return {
            "native": True,
            "app_name": APP_NAME,
            "base": f"http://{gw._display_host(c)}:{c.port}",
            "host": gw._display_host(c),
            "port": c.port,
            "api_key": c.api_key,
            "version": gw._bundle_version(),
            "platform": sys.platform,
            "menubar": (_menubar.ok if _menubar else False),
        }

    def open_external(self, url: str) -> bool:
        try:
            return bool(webbrowser.open(url))
        except Exception:
            return False

    def show(self) -> bool:
        return _show_window()

    def hide(self) -> bool:
        return _hide_window()

    def quit(self) -> bool:
        _quit()
        return True

    def restart(self) -> bool:
        _restart()
        return True

    def checkin(self) -> dict:
        return _checkin_all()

    def copy(self, text: str) -> bool:
        """复制到系统剪贴板;不支持的平台返回 False,由前端兜底。"""
        if sys.platform != "darwin":
            return False
        try:
            from AppKit import NSPasteboard, NSPasteboardTypeString

            pb = NSPasteboard.generalPasteboard()
            pb.clearContents()
            return bool(pb.setString_forType_(text, NSPasteboardTypeString))
        except Exception:
            return False

    def reveal(self, path: str) -> bool:
        return _reveal(path)

    def reveal_config(self) -> bool:
        return _reveal(gw.STATE.get("config_file") or str(Path.cwd() / "config.json"))

    def reveal_log(self) -> bool:
        return _reveal(gw.cfg().log_file or str(Path.cwd() / "gateway.log"))


# ---------------------------------------------------------------------------
# 窗口控制
#
# 线程说明:这些函数既会被菜单栏点击(macOS 主线程)调用,也会被前端 JS
# (pywebview 的桥接线程)调用。pywebview 的 Window.show/hide 内部已各自
# 派发回 GUI 主线程(cocoa 后端用 AppHelper.callAfter,gtk 用 GLib.idle_add),
# 所以这里直接调用即可,不需要再包一层派发。
# ---------------------------------------------------------------------------

def _show_window() -> bool:
    if _window is None:
        return False
    try:
        _window.show()
        _window.restore()
        return True
    except Exception:
        return False


def _hide_window() -> bool:
    if _window is None:
        return False
    try:
        _window.hide()
        return True
    except Exception:
        return False


def _quit() -> None:
    """关窗 + 结束 pywebview 事件循环 → 进程退出(网关随之结束)。"""
    global _window
    win, _window = _window, None
    if win is None:
        os._exit(0)
    try:
        win.destroy()
    except Exception:
        os._exit(0)
    # 兜底:某些后端 destroy 后循环不自退,强制结束。
    threading.Timer(2.0, lambda: os._exit(0)).start()


def _restart() -> None:
    try:
        os.execv(sys.executable, [sys.executable] + sys.argv)
    except Exception as e:
        print(f"[app] 重启失败: {e}", file=sys.stderr)
        _notify("重启失败", str(e))


# ---------------------------------------------------------------------------
# 系统动作
# ---------------------------------------------------------------------------

def _checkin_all() -> dict:
    """全部账号签到:直接复用 billing 模块(不经 HTTP,免鉴权/免端口)。"""
    import asyncio

    from codebuddy2api.billing import daily_checkin

    pool = gw.STATE.get("pool")
    if not pool or not pool.names():
        _notify("签到", "没有可用账号")
        return {"ok": 0, "total": 0, "error": "no accounts"}

    async def run():
        return [await daily_checkin(c) for c in pool.all()]

    # 菜单点击发生在 macOS 主线程,该线程没有运行中的事件循环
    # (uvicorn 的 loop 在后台线程),asyncio.run 在这里是安全的。
    res = asyncio.run(run())
    ok = sum(1 for r in res if r.get("ok"))
    _notify("签到", f"已完成 {ok}/{len(res)} 个账号")
    return {"ok": ok, "total": len(res), "results": res}


def _reveal(path: str) -> bool:
    path = os.path.abspath(os.path.expanduser(path or ""))
    if not path or not os.path.exists(path):
        _notify("文件不存在", path or "(空路径)")
        return False
    try:
        import subprocess

        if sys.platform == "darwin":
            subprocess.Popen(["open", "-R", path])
        elif sys.platform.startswith("win"):
            subprocess.Popen(["explorer", "/select,", path])
        else:
            subprocess.Popen(["xdg-open", os.path.dirname(path)])
        return True
    except Exception as e:
        print(f"[app] 打开失败: {e}", file=sys.stderr)
        return False


def _notify(title: str, msg: str) -> None:
    """系统通知(macOS 用 UNUserNotificationCenter;失败退回 stderr)。"""
    print(f"[app] {title}: {msg}", file=sys.stderr)
    if sys.platform != "darwin":
        return
    try:
        from UserNotifications import (UNMutableNotificationContent,
                                       UNNotificationRequest,
                                       UNUserNotificationCenter)

        content = UNMutableNotificationContent.alloc().init()
        content.setTitle_(title)
        content.setBody_(msg)
        req = UNNotificationRequest.requestWithIdentifier_content_trigger_(
            f"wb-{int(time.time() * 1000)}", content, None)
        UNUserNotificationCenter.currentNotificationCenter() \
            .addNotificationRequest_withCompletionHandler_(req, None)
    except Exception:
        pass


def _app_log_path() -> str:
    """App 形态的崩溃/降级日志落点:优先与网关日志同目录,失败退回临时目录。

    网关日志路径在 start_gateway() 之后才有(STATE['cfg'].log_file),
    这里做多重兜底,确保 _fatal 一定能写出一条可见记录。
    """
    try:
        c = gw.cfg()
        if c.log_file:
            d = os.path.dirname(os.path.abspath(os.path.expanduser(c.log_file)))
            if os.path.isdir(d):
                return os.path.join(d, "api-transmitter-app.log")
    except Exception:
        pass
    try:
        cfg_file = gw.STATE.get("config_file")
        if cfg_file:
            d = os.path.dirname(os.path.abspath(cfg_file))
            if os.path.isdir(d):
                return os.path.join(d, "api-transmitter-app.log")
    except Exception:
        pass
    return os.path.join(tempfile.gettempdir(), "api-transmitter-app.log")


def _fatal(msg: str, *, exc: BaseException | None = None) -> None:
    """原生窗口不可用时的「看得见」兜底:写日志文件 + 弹系统错误框。

    背景:App 形态打包为 console=False,任何 Python 异常都不会显示在终端,
    表现为"窗口一闪即退、无任何报错"。这里把错误固定写到日志文件并弹一个
    原生消息框,让用户/反馈者能拿到原因,而不是盲猜。Windows 用 MessageBoxW,
    macOS 用 osascript,Linux 退回 stderr。
    """
    detail = f"{msg}"
    if exc is not None:
        detail += f"\n\n{exc!r}"
    stamp = time.strftime("%Y-%m-%d %H:%M:%S")
    try:
        with open(_app_log_path(), "a", encoding="utf-8") as f:
            f.write(f"[{stamp}] [FATAL] {detail}\n")
    except Exception:
        pass
    sys.stderr.write(f"[app][FATAL] {detail}\n")
    try:
        if sys.platform == "win32":
            import ctypes
            ctypes.windll.user32.MessageBoxW(0, detail, "API Transmitter", 0x10)  # MB_ICONERROR
        elif sys.platform == "darwin":
            os.system(
                f'osascript -e \'display dialog "{(detail[:1000]).replace(chr(34), chr(39))}" '
                f'with title "API Transmitter" buttons {{"OK"}} default button "OK"\''
            )
        else:
            print(f"[app][FATAL] {detail}", file=sys.stderr)
    except Exception:
        pass


# ---------------------------------------------------------------------------
# macOS 菜单栏(NSStatusBar)
# ---------------------------------------------------------------------------

def _MenuTargetClass():
    """菜单项的 target:持有回调表,转到 Python 函数。"""
    cached = getattr(_MenuTargetClass, "_cls", None)
    if cached is not None:
        return cached

    import objc
    from Foundation import NSObject

    class MenuTarget(NSObject):
        def initWithHandlers_(self, handlers):
            self = objc.super(MenuTarget, self).init()
            if self is None:
                return None
            self._h = handlers or {}
            return self

        # 注意:pyobjc 会把**所有**实例方法当作潜在 selector 校验签名,
        # 因此这里也用 ObjC 风格命名(带尾下划线的 selector)。
        def runKey_(self, key):
            fn = (self._h or {}).get(key)
            if not fn:
                return
            try:
                fn()
            except Exception as e:
                print(f"[app] 菜单动作失败({key}): {e}", file=sys.stderr)

        def onOpen_(self, sender):      self.runKey_("open")
        def onExternal_(self, sender):  self.runKey_("external")
        def onCheckin_(self, sender):   self.runKey_("checkin")
        def onConfig_(self, sender):    self.runKey_("config")
        def onLogs_(self, sender):      self.runKey_("logs")
        def onRestart_(self, sender):   self.runKey_("restart")
        def onQuit_(self, sender):      self.runKey_("quit")

    _MenuTargetClass._cls = MenuTarget
    return MenuTarget


class MenuBar:
    """macOS 菜单栏常驻图标 + 菜单。关窗只隐藏,点图标恢复,退出走菜单。

    仅在 macOS 生效;其它平台 install() 返回 False,退化为普通窗口(关窗即退出)。
    """

    def __init__(self, cfg: config_mod.Config):
        self.cfg = cfg
        self.item = None
        self.ok = False

    # -- 安装 --
    def install(self) -> bool:
        if sys.platform != "darwin":
            return False
        try:
            from AppKit import NSStatusBar, NSVariableStatusItemLength
        except Exception as e:
            print(f"[app] 菜单栏不可用({e}),退化为普通窗口", file=sys.stderr)
            return False
        try:
            bar = NSStatusBar.systemStatusBar()
            self.item = bar.statusItemWithLength_(NSVariableStatusItemLength)
            btn = self.item.button()
            icon = self._icon()
            if icon is not None:
                btn.setImage_(icon)
                btn.setTitle_("")
            else:
                btn.setTitle_("WB")
            btn.setToolTip_(APP_NAME)
            self.item.setMenu_(self._menu())
            self.ok = True
            return True
        except Exception as e:
            print(f"[app] 菜单栏安装失败: {e}", file=sys.stderr)
            return False

    def dispose(self) -> None:
        if self.item is not None and sys.platform == "darwin":
            try:
                from AppKit import NSStatusBar

                NSStatusBar.systemStatusBar().removeStatusItem_(self.item)
            except Exception:
                pass
        self.item, self.ok = None, False

    # -- 图标 --
    def _icon(self):
        """菜单栏图标:必须用**纯 alpha 模板图**(menubar-template.png)。

        关键:不能用彩色的 icon-192.png —— setTemplate_(True) 会丢弃颜色只保留
        alpha,而彩色图标的圆角底 alpha 是满的,结果就是菜单栏上一个实心方块,
        完全看不出图形。模板图只有箭头有 alpha、其余透明,才是正确用法。
        """
        try:
            from AppKit import NSImage

            for cand in _menubar_icon_candidates():
                if cand and os.path.isfile(cand):
                    img = NSImage.alloc().initWithContentsOfFile_(cand)
                    if img:
                        img.setTemplate_(True)   # 跟随系统明暗自动上色
                        img.setSize_((18, 18))
                        return img
        except Exception:
            pass
        return self._drawn_icon()

    def _drawn_icon(self):
        """没有图标文件时画一个闪电(模板模式,跟随系统配色)。"""
        try:
            from AppKit import NSBezierPath, NSColor, NSImage

            img = NSImage.alloc().initWithSize_((18, 18))
            img.lockFocus()
            NSColor.labelColor().set()
            path = NSBezierPath.bezierPath()
            path.moveToPoint_((11.5, 2.5))
            path.lineToPoint_((6.5, 10.0))
            path.lineToPoint_((9.5, 10.0))
            path.lineToPoint_((8.0, 15.5))
            path.lineToPoint_((13.0, 7.5))
            path.lineToPoint_((10.0, 7.5))
            path.closePath()
            path.fill()
            img.unlockFocus()
            img.setTemplate_(True)
            return img
        except Exception:
            return None

    # -- 菜单 --
    def _menu(self):
        from AppKit import NSMenu, NSMenuItem

        c = gw.cfg()
        base = f"http://{gw._display_host(c)}:{c.port}"
        target = _MenuTargetClass().alloc().initWithHandlers_({
            "open": _show_window,
            "external": lambda: webbrowser.open(base),
            "checkin": _checkin_all,
            "config": lambda: _reveal(gw.STATE.get("config_file") or "config.json"),
            "logs": lambda: _reveal(gw.cfg().log_file or "gateway.log"),
            "restart": _restart,
            "quit": _quit,
        })

        menu = NSMenu.alloc().init()

        def item(title: str, sel: str, key: str = ""):
            it = NSMenuItem.alloc().initWithTitle_action_keyEquivalent_(title, sel, key)
            it.setTarget_(target)
            menu.addItem_(it)

        item(f"打开 {APP_NAME}", "onOpen:")
        item("在浏览器中打开", "onExternal:")
        menu.addItem_(NSMenuItem.separatorItem())
        item("全部账号签到", "onCheckin:")
        item("显示配置文件 config.json", "onConfig:")
        item("显示日志文件", "onLogs:")
        menu.addItem_(NSMenuItem.separatorItem())
        item("重启网关", "onRestart:")
        menu.addItem_(NSMenuItem.separatorItem())
        item(f"退出 {APP_NAME}", "onQuit:", "q")
        return menu


# ---------------------------------------------------------------------------
# 资源定位
# ---------------------------------------------------------------------------

def _resource_root() -> Path:
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        return Path(meipass)
    return Path(__file__).resolve().parents[2]


def _menubar_icon_candidates() -> list[str]:
    """菜单栏模板图标候选(纯 alpha,setTemplate_ 后只显示图形轮廓)。"""
    root = _resource_root()
    return [
        str(root / "web" / "icons" / "menubar-template.png"),
        str(root / "icons" / "menubar-template.png"),
    ]


def _icon_candidates() -> list[str]:
    """窗口 / Dock 图标候选(彩色完整图标)。"""
    root = _resource_root()
    return [
        str(root / "web" / "icons" / "icon-192.png"),
        str(root / "web" / "icons" / "icon-512.png"),
        str(root / "icons" / "icon-192.png"),
        str(root / "icon.png"),
    ]


def _window_icon() -> str | None:
    for c in _icon_candidates():
        if os.path.isfile(c):
            return c
    return None


# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------

def build_args() -> argparse.Namespace:
    """converter 的全部参数 + App 专属参数。"""
    ap = argparse.ArgumentParser(description=f"{APP_NAME}(原生桌面 App)")
    ap.add_argument("--config", default=None, help="config.json 路径")
    ap.add_argument("--host", default=None)
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--api-key", default=None)
    ap.add_argument("--auth-dir", dest="auth_dir", default=None)
    ap.add_argument("--log", default=None, metavar="PATH")
    ap.add_argument("--desensitize", action="store_true")
    ap.add_argument("--no-compact", action="store_true")
    ap.add_argument("--skip-check", action="store_true")
    ap.add_argument("--no-window", action="store_true",
                    help="不开原生窗口,只跑网关(等价 python -m codebuddy2api.converter)")
    ap.add_argument("--window-only", action="store_true",
                    help="只开原生窗口,不启动网关(连接已运行的网关)")
    ap.add_argument("--no-menubar", action="store_true", help="禁用 macOS 菜单栏常驻")
    ap.add_argument("--debug", action="store_true", help="pywebview 调试模式(可右键检查元素)")
    return ap.parse_args()


def _fallback_headless(cfg: config_mod.Config, reason: str) -> None:
    msg = (f"原生窗口不可用（{reason}），已降级为浏览器控制台。\n\n"
           f"请在浏览器打开: http://{gw._display_host(cfg)}:{cfg.port}")
    sys.stderr.write(f"\n⚠️  {msg}\n\n")
    # App 形态 console=False,stderr 不可见;这里把降级原因也落到日志+弹框,
    # 否则用户只会看到"窗口没出现"。注意:先 _serve 起网关再弹框,弹框只提示。
    try:
        with open(_app_log_path(), "a", encoding="utf-8") as f:
            f.write(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] [HEADLESS] {reason}\n")
    except Exception:
        pass
    if sys.platform == "win32":
        try:
            import ctypes
            # 非阻塞:另起线程弹框,不挡网关线程(MessageBoxW 自带消息循环)。
            import threading
            threading.Thread(
                target=lambda: ctypes.windll.user32.MessageBoxW(
                    0, f"{msg}\n\n(日志见 api-transmitter-app.log)", "API Transmitter", 0x40),
                daemon=True,
            ).start()
        except Exception:
            pass
    _serve(cfg)


def run_app(args: argparse.Namespace) -> int:
    global _window, _menubar

    cfg = start_gateway(args)
    host, port = gw._display_host(cfg), cfg.port

    if not args.skip_check:
        gw.preflight(gw.STATE["pool"])

    if args.no_window:
        _serve(cfg)
        return 0

    try:
        import webview
    except Exception as e:
        _fallback_headless(cfg, f"缺少 pywebview({e})")
        return 0

    if args.window_only:
        if not _port_open(cfg.host, port):
            sys.stderr.write(f"⚠️  --window-only 但 {host}:{port} 无服务,界面会显示“网关离线”。\n")
    elif _port_open(cfg.host, port):
        sys.stderr.write(f"⚠️  端口 {port} 已被占用,假定已有网关在跑(可用 --window-only 明确)。\n")
    else:
        threading.Thread(target=_serve, args=(cfg,), daemon=True).start()
        if not _wait_port(cfg.host, port):
            _fallback_headless(cfg, f"网关未能在 {host}:{port} 就绪")
            return 1

    url = f"http://{host}:{port}/?native=1"
    RUNTIME["window_mode"] = True

    if not args.no_menubar:
        _menubar = MenuBar(cfg)

    try:
        _window = webview.create_window(
            APP_TITLE,
            url=url,
            js_api=NativeApi(cfg),
            width=WINDOW_WIDTH,
            height=WINDOW_HEIGHT,
            min_size=WINDOW_MIN,
            # 原生观感:半透明标题栏 + 内容铺满(vibrancy 仅 macOS 生效)
            vibrancy=(sys.platform == "darwin"),
            background_color="#0E1116" if sys.platform == "darwin" else "#FFFFFF",
            text_select=True,
            confirm_close=False,
        )
    except Exception as e:
        # 创建窗口即失败(后端不可用等):写明原因 + 弹框,再降级 headless,
        # 不再静默 return 让进程"闪退"。
        _fatal(f"原生窗口创建失败,降级为浏览器控制台。原因: {e}", exc=e)
        _fallback_headless(cfg, f"原生窗口创建失败({e}),降级为 headless 模式")
        return 1

    if _menubar is not None:
        # 关窗 → 隐藏到菜单栏(仅菜单栏安装成功时);否则关窗即退出。
        def _on_closing() -> bool:
            return _hide_window() if _menubar.ok else False

        _window.events.closing += _on_closing

        def _install_menubar() -> None:
            """NSStatusBar 只能在**主线程**操作,而 `loaded` 事件跑在别的线程。"""
            if sys.platform == "darwin":
                try:
                    from PyObjCTools.AppHelper import callAfter

                    callAfter(_menubar.install)
                    return
                except Exception:
                    pass
            _menubar.install()

        _window.events.loaded += _install_menubar

    sys.stderr.write(f"\n✅ {APP_NAME} 已启动: http://{host}:{port} (原生窗口)\n")
    sys.stderr.write("   关窗:隐藏到菜单栏;退出:菜单栏 → 退出(或 Cmd+Q)。\n\n")

    try:
        webview.start(debug=bool(args.debug), icon=_window_icon())
    except Exception as e:
        # webview.start 在事件循环里崩溃(典型:Windows 缺 WebView2 Runtime/
        # interop DLL)。App 形态 console=False 会吞掉异常,这里显式暴露。
        _fatal(f"原生窗口事件循环崩溃,网关仍在后台运行,请用浏览器打开控制台。原因: {e}", exc=e)
        # 网关是 daemon 线程,start() 异常退出后进程也会结束;这里不再重复起 HTTP,
        # 直接返回,但已通过 _fatal 把原因写到日志+弹框。
        return 1
    return 0


def main() -> int:
    return run_app(build_args())


if __name__ == "__main__":
    sys.exit(main())
