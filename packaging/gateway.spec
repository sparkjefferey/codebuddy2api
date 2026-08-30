# -*- mode: python ; coding: utf-8 -*-
# workbuddy-gateway PyInstaller 单 spec（OS 环境变量区分平台，避免三份重复）。
# 用法: OS=mac|win|linux pyinstaller packaging/gateway.spec --noconfirm
#  builds: onedir 于 build/，bundle 于 dist/gateway(.app)；数据落在 _internal/web/。
import os

from PyInstaller.utils.hooks import collect_submodules

OS = os.environ.get("OS", "").lower()  # mac | win | linux

# SPECPATH 即 spec 所在目录(packaging/)本身；烤一遍后取父目录即仓库根。
SPEC_DIR = os.path.abspath(SPECPATH)
ROOT = os.path.dirname(SPEC_DIR)  # 仓库根

# uvicorn 用 importlib 惰性加载 loop/protocol 实现，静态分析会漏，collect_submodules 兜底。
hiddenimports = collect_submodules("uvicorn")

datas = [(os.path.join(ROOT, "web", "index.html"), "web")]
# 构建时(workflow)若在仓库根生成 VERSION，则随包带上供 /health 上报版本。
if os.path.isfile(os.path.join(ROOT, "VERSION")):
    datas.append((os.path.join(ROOT, "VERSION"), "."))
# macOS .app 内置“首次运行安装/自启”脚本，落在 Contents/Resources/ 顶层
# （bundle 数据根即 Contents/Resources，故 dest 用 "."），
# 与 launchagent-first-run.sh 里 $HERE/../../MacOS/gateway 的取径一致。
if OS == "mac":
    datas.append((os.path.join(ROOT, "packaging", "macos", "launchagent-first-run.sh"), "."))

a = Analysis(
    [os.path.join(ROOT, "src", "codebuddy2api", "converter.py")],
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
    name="gateway",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=(OS != "mac"),  # mac 用 .app 包裹去掉终端窗口；win/linux 保留 console 便于调试
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="gateway",
)

if OS == "mac":
    app = BUNDLE(
        coll,
        name="gateway.app",
        icon=None,
        bundle_identifier="com.workbuddy.gateway",
        info_plist={
            "CFBundleName": "WorkBuddy Gateway",
            "CFBundleDisplayName": "WorkBuddy Gateway",
            "CFBundleIdentifier": "com.workbuddy.gateway",
            # 守护进程：无 Dock 图标、无菜单栏
            "LSUIElement": True,
        },
    )