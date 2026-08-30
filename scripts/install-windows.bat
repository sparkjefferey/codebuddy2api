@echo off
REM ---------------------------------------------------------------
REM  install-windows.bat — Windows 一键安装 WorkBuddy 算力网关
REM  功能: 建 .venv + 装依赖 + 注册登录自启(schtasks) + 立即启动
REM  用法: 双击运行 或 install-windows.bat
REM  卸载/停止用同目录的 windows-stop.bat
REM ---------------------------------------------------------------
setlocal
cd /d "%~dp0\.."
set ROOT=%CD%
set TASK=WorkBuddyGateway

where py >nul 2>nul && set "PY=py -3" || set "PY=python"

echo [1/4] 准备原生环境(.venv)...
if not exist ".venv\Scripts\python.exe" (
  %PY% -m venv .venv || (echo [X] 创建 venv 失败,先装 Python 3.12 && exit /b 1)
)

echo [2/4] 安装依赖...
".venv\Scripts\python.exe" -m pip install --disable-pip-version-check -q -r requirements.txt || (echo [X] 依赖安装失败 && exit /b 1)

REM 生成启动器(内含 PYTHONPATH 指向 src),供 schtasks 登录自启调用
set "LAUNCHER=%ROOT%\scripts\run-gateway-win.bat"
> "%LAUNCHER%" (
  echo @echo off
  echo cd /d "%ROOT%"
  echo set PYTHONPATH=%ROOT%\src
  echo ".venv\Scripts\python.exe" -m codebuddy2api.converter --desensitize --skip-check
)

echo [3/4] 注册登录自启任务 "%TASK%" ...
schtasks /create /tn "%TASK%" /tr "\"%LAUNCHER%\"" /sc onlogon /rl limited /f >nul
if errorlevel 1 (echo [W] schtasks 注册失败,仍可手动启动)

echo [4/4] 立即启动网关...
start "WorkBuddyGateway" /min cmd /c ""%LAUNCHER%""

echo.
echo 完成: 网关已启动,控制台 http://127.0.0.1:8787
echo   自启任务已被注册(登录时自动运行)。
echo   停止/卸载: scripts\windows-stop.bat
endlocal