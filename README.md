# nvim-speaks Windows Server

Minimal stdio speech server for `nvim-speaks` on Windows.

It reads the existing newline-delimited JSON protocol from stdin, speaks
`speech.text`, and writes optional JSON status replies to stdout. Logs go to
stderr so stdout remains protocol-safe.

Named earcons are intentionally not advertised by this server unless real audio
assets are added. The Neovim plugin keeps the spoken text in the same envelope,
so unsupported earcons fall back to speech instead of hard-coded beep tables.

## Run

```powershell
.\nvim-speaks-windows-server.exe --backend auto
```

Backend choices:

- `auto`: try NVDA first, then SAPI, then log-only.
- `nvda`: use `nvdaControllerClient.dll`.
- `sapi`: use Windows SAPI through PowerShell/.NET `System.Speech`.
- `log`: print speech requests to stderr without speaking.

NVDA example:

```powershell
.\nvim-speaks-windows-server.exe --backend nvda --dll .\nvdaControllerClient.dll
```

## NVDA Controller DLL

The NVDA backend loads `nvdaControllerClient.dll` at runtime. The DLL must match
the server executable architecture.

For a normal 64-bit Windows build, use:

```text
nvda_2026.1_controllerClient\x64\nvdaControllerClient.dll
```

Recommended development layout:

```text
nvim-speaks-windows-server.exe
nvdaControllerClient.dll
```

Then run:

```powershell
.\nvim-speaks-windows-server.exe --backend nvda
```

You can also keep the DLL somewhere else and pass it explicitly:

```powershell
.\nvim-speaks-windows-server.exe --backend nvda --dll C:\path\to\nvdaControllerClient.dll
```

The server does not link against the NVDA DLL at compile time. It uses dynamic
loading so the DLL only needs to be present when running the `nvda` backend.

Neovim config:

```lua
require("nvim-speaks").setup({
  command = { "C:\\path\\to\\nvim-speaks-windows-server.exe" },
})
```

## Build

Native Windows:

```powershell
cargo build --release
```

Linux cross-compile, once the target and linker are installed:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
# or debug
cargo build --target x86_64-pc-windows-gnu
```
