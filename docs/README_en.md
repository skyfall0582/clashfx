# ClashFx

A native Windows proxy client built with Rust and GPUI, powered by Mihomo, with automatic node and subscription failover.

[简体中文](../README.md)

## Features

- Import Clash / Mihomo YAML subscription URLs or local files, and manage nodes, subscriptions and notes.
- Choose an automatic switching pool and its priority order; check failed connections and switch to an available node.
- Select nodes manually, test latency, and use Rule, Global or Direct routing.
- Manage the Windows system proxy and restore its previous settings on exit or after a crash.
- Run in the system tray, start at login, repair entry addresses and filter logs.

## Build

Requires Windows x64, Rust 1.95.0 with the MSVC toolchain, Visual Studio C++ Build Tools and a Windows SDK.

```powershell
git clone https://github.com/skyfall0582/clashfx.git
cd clashfx
.\native\scripts\build.ps1
```

The script downloads Mihomo and geodata and writes the complete application to `native/releases/ClashFx`.

The Rust workspace is in `native/`. See [CONTRIBUTING.md](../CONTRIBUTING.md) for development setup and checks.

## Use

Run `ClashFx.exe` from the build output. Keep `mihomo.exe` and the `geodata` directory alongside it.

1. Import a Clash / Mihomo YAML subscription or local file with an explicit `proxies` list on the subscriptions page (订阅池).
2. Select a node and enable the system proxy (系统代理) on the connection page (连接).
3. For automatic failover, add nodes to the switching pool on the automatic switching page (自动切换) and set their priority.

Closing the window keeps ClashFx running in the system tray. Exiting through the tray restores the system proxy and stops the core. Subscriptions and settings stay on the local device and are retained on uninstall.

See the [developer documentation](../native/README.md) for detailed behavior and build instructions.

## License

[GPL-3.0-only](../LICENSE). Source attribution and third-party component notices are in [NOTICE.txt](../native/NOTICE.txt).
