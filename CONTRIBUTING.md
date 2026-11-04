# Contributing to ClashFx

ClashFx is a Windows x64 proxy client built with Rust, GPUI and Mihomo. Its independent Cargo workspace is in `native/`.

## Development

Install Rust 1.95.0 with the MSVC toolchain, Visual Studio C++ Build Tools and a Windows SDK. `native/rust-toolchain.toml` selects the Rust toolchain and required components.

From the repository root, build the complete application with:

```powershell
.\native\scripts\build.ps1
```

The script downloads Mihomo and geodata when missing and writes the application to `native/releases/ClashFx`. Run `ClashFx.exe` from that directory, or use `run-clashfx.ps1` from the repository root.

## Validation

Run Cargo commands from `native`:

```powershell
cd native
cargo fmt --all -- --check
cargo test --locked -p clashfx-core
cargo clippy --locked --workspace --all-targets -- -D warnings
```

The GitHub workflow runs these checks and builds the application on Windows. Integration tests requiring Mihomo are ignored by default; see [native/README.md](native/README.md) for the commands and their scope. After changing the application or packaged files, run `scripts/build.ps1` from `native` to refresh the complete application output.

## Contributions

Describe the problem, scope and approach in your own words. A direct project-owner request is sufficient scope; link an existing issue when relevant. Keep changes focused and validate the affected behavior. Add tests only when they reproduce a regression or guard behavior that would otherwise go unnoticed.

Use English for code, comments, commit messages and pull requests. Commit subjects follow Conventional Commits, and commits must be signed. Follow the [pull request template](.github/PULL_REQUEST_TEMPLATE.md). Agents must follow [AGENTS.md](AGENTS.md), including its AI assistance disclosure requirement for pull requests and issues.

Preserve the GPL license and the source and component attribution in `native/NOTICE.txt` and `native/LICENSE-APACHE`.
