@echo off
rem Служебный скрипт: elevated live-тесты движка (Цикл 7).
rem Запускается через Start-Process -Verb RunAs; лог — target\mxds_live_test.log
cd /d C:\Users\MINX\Documents\rust\MXDS
"%USERPROFILE%\.cargo\bin\cargo.exe" test --test engine_live -- --ignored --nocapture --test-threads=1 > target\mxds_live_test.log 2>&1
exit /b %ERRORLEVEL%
