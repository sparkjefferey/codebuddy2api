# -*- mode: python ; coding: utf-8 -*-
# api-transmitter PyInstaller 单 spec(OS / TARGET 环境变量区分平台与产物)。
#
# 用法:
#   OS=mac|win|linux TARGET=gateway pyinstaller packaging/gateway.spec --noconfirm
#   OS=mac|win|linux TARGET=app      pyinstaller packaging/gateway.spec --noconfirm
#
#   TARGET=gateway(默认): 无头网关服务端,配浏览器/PWA 使用 → dist/gateway
#   TARGET=app          : 原生桌面 App(网关同进程 + 原生窗口)→ dist/API Transmitter(.app)
#
# 数据落在 bundle 的 web/ 目录,由 converter._resource_root() 从 sys._MEIPASS 解析。
import os

from PyInstaller.utils.hooks import collect_submodules

OS = os.environ.get("OS", "").lower()          # mac | win | linux
TARGET = os.environ.get("TARGET", "gateway").lower()   # gateway | app
IS_APP = TARGET == "app"

# SPECPATH 即 spec 所在目录(packaging/)本身;取父目录即仓库根。
SPEC_DIR = os.path.abspath(SPECPATH)
ROOT = os.path.dirname(SPEC_DIR)  # 仓库根

# uvicorn 用 importlib 惰性加载 loop/protocol 实现,静态分析会漏,collect_submodules 兜底。
hiddenimports = collect_submodules("uvicorn")

if IS_APP:
    # pywebview 各平台后端同样是运行时按 gui 字符串导入的,必须显式收集。
    hiddenimports += collect_submodules("webview")
    # macOS 走 Cocoa/WKWebView;Windows 走 EdgeChromium(pythonnet);
    # Linux 走 GTK(需在目标机装 libwebkit2gtk-4.1 + gir1.2-webkit2-4.1)。
    hiddenimports += ["webview.platforms.cocoa",
                      "webview.platforms.winforms",
                      "webview.platforms.edgechromium",
                      "webview.platforms.gtk"]

    # pywebview 的 Windows 后端(edgechromium)通过 pythonnet 的 clr.AddReference
    # 加载 webview/lib/ 下的 WebView2 interop DLL(Microsoft.Web.WebView2.*.dll,
    # WebBrowserInterop.x*.dll),并依赖 runtimes/win-*/native/WebView2Loader.dll。
    # collect_submodules 只收集 .py,**不会**把这些 DLL 打进 bundle,而
    # interop_dll_path() 在冻结后只去 _MEIPASS / exe 同目录找它们 —— 缺失时
    # clr.AddReference 抛 FileNotFoundException,webview.start() 崩溃,又因为 App
    # 形态 console=False,异常被吞掉 → 表现为"窗口一闪即退、无任何报错"。
    # 这里把整目录连同 runtimes 一并作为 datas 拷进 bundle 的根(_MEIPASS)。
    _webview_pkg = None
    try:
        import webview as _wv
        _webview_pkg = os.path.dirname(_wv.__file__)
    except Exception:
        pass
    if _webview_pkg:
        _lib = os.path.join(_webview_pkg, "lib")
        if os.path.isdir(_lib):
            for _root, _dirs, _files in os.walk(_lib):
                for _fn in _files:
                    if _fn.lower().endswith(".dll"):
                        _full = os.path.join(_root, _fn)
                        # 落点保持相对 lib/ 的结构,使 interop_dll_path 的
                        # _MEIPASS/lib/... 与 runtimes/... 解析命中。
                        _rel = os.path.relpath(_full, _webview_pkg)
                        datas.append((_full, os.path.join(_rel)))

# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------
if IS_APP:
    app_entry = os.path.join(ROOT, "src", "codebuddy2api", "app.py")
    app_name = "ApiTransmitter"
else:
    app_entry = os.path.join(ROOT, "src", "codebuddy2api", "converter.py")
    app_name = "gateway"

datas = []

# 前端资源:整目录拷进 bundle 的 web/(PWA 的 sw.js / manifest / icons 都要,
# 否则打包后 /manifest.webmanifest、/sw.js 会 404,浏览器里“安装为应用”失效)。
_web_src = os.path.join(ROOT, "web")
for _dirpath, _dirnames, _filenames in os.walk(_web_src):
    for _fn in _filenames:
        if _fn.endswith(".pyc") or "__pycache__" in _dirpath:
            continue
        _full = os.path.join(_dirpath, _fn)
        _rel = os.path.relpath(_dirpath, _web_src)
        datas.append((_full, os.path.join("web", "" if _rel == "." else _rel)))
# 构建时(workflow)若在仓库根生成 VERSION,则随包带上供 /health 上报版本。
if os.path.isfile(os.path.join(ROOT, "VERSION")):
    datas.append((os.path.join(ROOT, "VERSION"), "."))
# macOS .app 内置“首次运行安装/自启”脚本,落在 Contents/Resources/ 顶层
# (bundle 数据根即 Contents/Resources,故 dest 用 "."),
# 与 launchagent-first-run.sh 里 $HERE/../../MacOS/<exe> 的取径一致。
if OS == "mac":
    datas.append((os.path.join(ROOT, "packaging", "macos", "launchagent-first-run.sh"), "."))

a = Analysis(
    [app_entry],
    pathex=[os.path.join(ROOT, "src")],
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=None)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name=app_name,
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    # App 需要窗口,一律隐藏控制台;gateway 保留 console 便于调试(win/linux)。
    console=(False if IS_APP else (OS != "mac")),
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name=app_name,
)

if OS == "mac":
    app = BUNDLE(
        coll,
        name=("API Transmitter.app" if IS_APP else "gateway.app"),
        icon=None,
        bundle_identifier=("com.apitransmitter.gateway.app" if IS_APP
                           else "com.apitransmitter.gateway"),
        info_plist={
            "CFBundleName": ("API Transmitter" if IS_APP else "API Transmitter (headless)"),
            "CFBundleDisplayName": "API Transmitter",
            "CFBundleIdentifier": ("com.apitransmitter.gateway.app" if IS_APP
                                   else "com.apitransmitter.gateway"),
            # gateway: 纯守护进程 —— 无 Dock 图标、无菜单栏
            # app    : 原生 GUI App —— 需要 Dock 图标(菜单栏常驻由 NSStatusBar 自建)
            "LSUIElement": (not IS_APP),
        },
    )
