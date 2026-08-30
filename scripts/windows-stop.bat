@echo off
REM ---------------------------------------------------------------
REM  windows-stop.bat — 停止并移除 WorkBuddy 网关自启任务(如需)
REM
REM  环境变量:
REM    MODE=gateway(默认) | app —— 必须与 install-windows.bat 安装时一致,
REM                                两个模式用不同的任务名,互不干扰。
REM
REM  用法:
REM    windows-stop.bat         停止运行 + 禁止登录自启
REM    windows-stop.bat off     仅停止本次运行,保留自启
REM ---------------------------------------------------------------
setlocal
if "%MODE%"=="" set MODE=gateway
if "%MODE%"=="app" (set TASK=ApiTransmitterApp) else (set TASK=ApiTransmitter)
set ROOT=%~dp0\..

if /i "%~1"=="off" goto stop_only

echo [1/2] 移除自启任务 %TASK%...
schtasks /delete /tn "%TASK%" /f >nul 2>nul
if errorlevel 1 echo   (自启任务不存在,忽略)

:stop_only
echo [2/2] 结束网关进程...
schtasks /end /tn "%TASK%" >nul 2>nul
taskkill /im python.exe /fi "WINDOWTITLE eq ApiTransmitter*" /f >nul 2>nul

echo 已停止(默认也已移除自启; 用 windows-stop.bat off 仅停本次)。
echo 重新启动: scripts\install-windows.bat (MODE=%MODE%)
endlocal
