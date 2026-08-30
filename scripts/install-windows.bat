@echo off
REM ---------------------------------------------------------------
REM  install-windows.bat — Windows 一键安装 API Transmitter
REM
REM  环境变量:
REM    MODE=gateway(默认) 无头网关,界面在浏览器/PWA 打开
REM    MODE=app          原生桌面 App(网关同进程 + 原生窗口,WebView2)
REM
REM  用法: 双击运行 或 install-windows.bat
REM  卸载/停止用同目录的 windows-stop.bat
REM ---------------------------------------------------------------
setlocal
cd /d "%~dp0\.."
set ROOT=%CD%
if "%MODE%"=="" set MODE=gateway
if "%MODE%"=="app" (set TASK=ApiTransmitterApp) else (set TASK=ApiTransmitter)

where py >nul 2>nul && set "PY=py -3" || set "PY=python"

echo [1/4] 准备原生环境(.venv)...
if not exist ".venv\Scripts\python.exe" (
  %PY% -m venv .venv || (echo [X] 创建 venv 失败,先装 Python 3.12 && exit /b 1)
)

echo [2/4] 安装依赖...
".venv\Scripts\python.exe" -m pip install --disable-pip-version-check -q -r requirements.txt || (echo [X] 依赖安装失败 && exit /b 1)
if "%MODE%"=="app" (
  echo      (原生 App: 追加 pywebview / WebView2 依赖)
  ".venv\Scripts\python.exe" -m pip install --disable-pip-version-check -q -r requirements-app.txt
)

REM 生成启动器(内含 PYTHONPATH 指向 src),供 schtasks 登录自启调用
if "%MODE%"=="app" (
  set "MODNAME=codebuddy2api.app"
  set "LAUNCHER=%ROOT%\scripts\run-gateway-app-win.bat"
) else (
  set "MODNAME=codebuddy2api.converter"
  set "LAUNCHER=%ROOT%\scripts\run-gateway-win.bat"
)
> "%LAUNCHER%" (
  echo @echo off
  echo cd /d "%ROOT%"
  echo set PYTHONPATH=%ROOT%\src
  echo ".venv\Scripts\python.exe" -m %MODNAME% --desensitize --skip-check
)

echo [3/4] 注册登录自启任务 "%TASK%" ...
schtasks /create /tn "%TASK%" /tr "\"%LAUNCHER%\"" /sc onlogon /rl limited /f >nul
if errorlevel 1 (echo [W] schtasks 注册失败,仍可手动启动)

echo [4/4] 立即启动网关...
start "ApiTransmitter" /min cmd /c ""%LAUNCHER%""

echo.
echo 完成: 网关已启动(MODE=%MODE%),控制台 http://127.0.0.1:8787
if "%MODE%"=="app" (
  echo   原生窗口会自动打开;关窗即退出。
) else (
  echo   控制台请在浏览器打开上述地址。
)
echo   自启任务已被注册(登录时自动运行)。
echo   停止/卸载: scripts\windows-stop.bat
endlocal
