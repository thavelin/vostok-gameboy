@echo off
setlocal
cd /d "%~dp0" || exit /b 1
cargo --version >nul 2>&1
if errorlevel 1 (
  echo Rust is required. Install it from https://rustup.rs and reopen this terminal. 1>&2
  exit /b 1
)
cargo build --release --locked
if errorlevel 1 exit /b 1
echo Ready: run.cmd
exit /b 0
