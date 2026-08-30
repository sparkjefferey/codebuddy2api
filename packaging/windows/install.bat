@echo off
REM ============================================================
REM  WorkBuddy Gateway - Windows 自启安装脚本（非管理员）
REM  双击运行即把本目录的 gateway.exe 注册为“登录时自动启动”。
REM  卸载: install.bat uninstall
REM ============================================================
setlocal
set "TASK=WorkBuddyGateway"
set "BIN=%~dp0gateway.exe"
set "WORK=%~dp0"

if /i "%~1"=="uninstall" (
  schtasks /Delete /TN "%TASK%" /F >nul 2>&1
  echo 已卸载自启任务 %TASK%。
  exit /b 0
)

if not exist "%BIN%" (
  echo [错误] 找不到 %BIN%  ^(请在解压目录内运行本脚本^)
  exit /b 1
)

REM RunAtLogon 无需管理员，也无需口令；在登录用户上下文内启动。
schtasks /Create /TN "%TASK%" /TR "\"%BIN%\" --desensitize --skip-check --host 127.0.0.1 --port 8787 --log \"%WORK%gateway.log\"" /SC ONLOGON /F

if errorlevel 1 (
  echo [失败] 创建任务失败，请勿修改文件路径后重试。
  exit /b 1
)

echo 已注册自启任务 %TASK%: 登录时启动 %BIN%
echo 现在启动一次…
start "WorkBuddy Gateway" /min "%BIN%" --desensitize --skip-check --host 127.0.0.1 --port 8787 --log "%WORK%gateway.log"
timeout /t 1 /nobreak >nul
echo.
echo 请注意：本安装包未签名，Windows SmartScreen 首次运行会提示，
echo 选择“更多信息 → 仍要运行”即可。浏览器打开 http://127.0.0.1:8787 查看控制台。
echo 卸载自启: install.bat uninstall
endlocal