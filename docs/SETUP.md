# Development setup (Windows, no admin)

This project targets Windows with the **MSVC** toolchain. Every step here installs
**per-user, without administrator rights**.

## 1. Rust toolchain

Rust installs under your user profile (`%USERPROFILE%\.cargo`, `.rustup`) — no admin.

Download and run `rustup-init.exe` from <https://win.rustup.rs/x86_64>:

```powershell
& "$env:TEMP\rustup-init.exe" -y --default-toolchain stable --default-host x86_64-pc-windows-msvc --profile default
```

Verify (open a **new** shell so `%USERPROFILE%\.cargo\bin` is on `PATH`):

```powershell
cargo --version
rustc --version
```

`rustc` compiles, but producing an `.exe` needs a **linker**. For the MSVC target
that linker is `link.exe`, from the MSVC Build Tools (next step). Without it you
get `error: linker 'link.exe' not found`.

## 2. MSVC build tools — portable, no admin

The official VS Build Tools installer needs admin. Instead use
[**PortableBuildTools**](https://github.com/Data-Oriented-House/PortableBuildTools)
(open-source), which downloads the MSVC compiler + Windows SDK from Microsoft and
extracts them to a user folder, setting user-scope environment variables.

1. Download `PortableBuildTools.exe` (v2.10.2) from the project's GitHub releases.
2. Install non-interactively into a **user-writable** folder (under your profile
   avoids needing admin for the `C:\` root):

   ```powershell
   & .\PortableBuildTools.exe accept_license env=user target=x64 host=x64 path="%USERPROFILE%\BuildTools"
   ```

   - `accept_license` — no prompts (fully headless).
   - `env=user` — writes `INCLUDE`, `LIB`, and `Path` to `HKEY_CURRENT_USER`
     (no admin). Persistent, effective in new sessions after you log out/in.
   - `msvc=` / `sdk=` default to the latest versions.
   - `list` (instead of the flags) prints the available MSVC/SDK versions.

3. For the **current** shell (before logging out), load the environment from the
   generated script, then build:

   ```powershell
   . "$env:USERPROFILE\BuildTools\devcmd.ps1"
   cargo build
   ```

## 3. Verify

```powershell
cargo build      # links successfully now
cargo test       # runs the suite — see docs/TESTING.md
```

## Fallback: GNU toolchain (no MSVC, no admin)

If MSVC is unavailable, the GNU toolchain bundles its own linker and builds M1
(pure Rust, no ONNX) immediately:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
cargo build
```

Note: ONNX Runtime (`ort`, milestone M2) links best against MSVC — switch back to
`stable-x86_64-pc-windows-msvc` before M2.

## Notes

- No component here requires administrator rights.
- `git` must be available for cloning and committing.
