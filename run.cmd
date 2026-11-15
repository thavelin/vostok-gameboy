@echo off
setlocal
rem Resolve paths relative to this launcher, not the caller's directory.
set "app=%~dp0vostok-gameboy.exe"
if exist "%app%" goto found
rem In a source checkout with Cargo, keep the build current (a quick no-op when
rem nothing changed). Without Cargo, run what is there.
if not exist "%~dp0Cargo.toml" goto built
cargo --version >nul 2>&1
if errorlevel 1 goto built
pushd "%~dp0"
cargo build --release --locked --quiet 2>nul
if errorlevel 1 echo NOTE: Could not rebuild; running the existing build. Close other copies, or run "cargo build --release" here to see why.
popd
:built
set "app=%~dp0target\release\vostok-gameboy.exe"
if exist "%app%" goto found
echo ERROR: Not built. Run setup.cmd (needs Rust from https://rustup.rs). 1>&2
exit /b 1

:found
"%app%" %*
exit /b %errorlevel%
