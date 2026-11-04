# ClashFx

A Windows desktop proxy client with a GPUI native interface and a Rust failover engine. ClashFx uses Mihomo directly through its local controller and WebSocket log API. Node.js, React, Tauri and WebView are not required to run or build this application.

## Run

The signed Windows x64 installer installs ClashFx for the current user and creates Start menu and desktop shortcuts by default. It includes the required Visual C++ runtime and geodata. Setup and Uninstall ask running instances to be exited from the system tray before continuing. Uninstall retains subscriptions and settings in the existing data directory.

Keep the bundled `geodata` directory alongside the executables. GeoIP, GeoSite and ASN databases are verified against the official MetaCubeX release digests when packaging and seeded locally before validation. Rule mode can start without downloading those databases; existing valid data is retained and incomplete files are backed up before repair. Validation errors are summarized in the notification, with full core diagnostics retained in the log.

Mihomo processes, including configuration validation, belong to Windows jobs that terminate them when their owner disappears. Before enabling the system proxy, ClashFx starts a separate hidden recovery process and waits for its readiness signal. That process holds a handle to the owner and restores only the matching session's proxy record after a crash. It respects settings subsequently claimed by another proxy application. Closing the window to the tray keeps the owner alive; it does not trigger recovery.

Open `releases/ClashFx/ClashFx.exe`. Keep `mihomo.exe` in the same directory. Import a Clash/Mihomo YAML subscription URL or a local YAML file. The app prepares the core automatically when nodes are available, using the previous node, the first pool node, or the first imported node. Choose another node if desired, then enable **系统代理** on the overview to route applications using Windows proxy settings through ClashFx. For automatic failover, add the desired nodes to the switching pool.

Enabling the system proxy also forwards loopback HTTP/SOCKS proxy endpoints configured in `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY` to the active core. Applications such as Codex can keep their existing local proxy settings. If another proxy application still owns one of those endpoints, activation fails with an actionable error before changing Windows settings. Compatibility listeners and their connections close when the system proxy is disabled or ClashFx exits; external proxy endpoints and environment variables are left unchanged.

While ClashFx owns the system proxy and its core is running, connectivity checks and a local check every 60 seconds also verify the Windows proxy switch. If it was disabled while still pointing to ClashFx, the app re-enables it and records the repair in the log, preserving the original settings for exit recovery. This check continues when automatic node switching is paused or the pool is empty. If the proxy address changes, ClashFx relinquishes ownership and releases compatibility listeners without overwriting the new settings. Disabling the proxy in ClashFx or exiting stops repairs.

The UI is in Chinese. Page headings and the sidebar brand block are omitted because navigation and the native titlebar already identify them. It provides connection status, subscription and node selection, subscription/node ordering, latency results, manual node selection, failover settings, rule/global/direct routing, and searchable application and core diagnostics. Navigation follows 连接 → 自动切换 → 订阅池 → 日志, with Settings at the bottom.

The overview expresses connection state with a native network diagram rather than status headings. The inactive route is static; an active system proxy has solid colored routes, a check indicator and continuously flowing particles. Preparation and failures have distinct symbols. The animation represents operating state, not traffic volume. At the user's request, ClashFx animates independently of Windows client-area animation preferences without changing those preferences. Direct mode also keeps the proxy diagram inactive. Animation stops requesting frames when minimized and is absent from other pages. Node and subscription names, controls and actionable errors remain visible.

Click the current node name on Connection, or the borderless node icon, count and right arrow on a subscription card, to open node selection. The back arrow returns to the originating page. Import and edit open centered dialogs with an orange primary action, a quieter Cancel action, and optional multiline notes. Notes persist through updates, appear as a single-line card preview, and are included in search. Changing notes alone does not reload the core. Keyboard focus stays inside dialogs; Escape cancels, and Ctrl+S submits. Delete is at the lower left of the edit dialog and retains its centered confirmation. Source previews show only the host; copying and editing retain the full URL. A changed URL is downloaded and validated before replacement, with rollback on failure. Local files can be replaced while retaining subscription identity, notes and node choices.

Click a proxy card to select it; this also starts a stopped core. Every imported node is available immediately, independently of pool membership. A compact searchable dropdown filters subscriptions, with wrapped full names in its list; the gauge icon tests latency without changing the route or manual hold. Under **自动切换**, underlined tabs separate **切换池** from **检测规则**. The left pane contains available nodes grouped by subscription; the right pane contains the automatic switching pool. Green right arrows add individual nodes; red minus icons remove them. Group actions retain an icon and text. A bidirectional arrow separates the panes; server icons show node counts. Searches affect their own pane, and bulk actions during a search affect matching nodes only. Drag the right-hand subscription and node handles to set priority, or use Alt+Up/Down; this does not reorder the subscription management page. Tabs and filters remain available during backend operations. Keyboard focus has an outline; clicking a group does not leave a focus box.

The connection page lists pool nodes with current selection, checking/available/unavailable/unmeasured states, health counts and the latest from/to transition. Routine checks stop once the current route is confirmed healthy; spare nodes are tested on demand. Results older than twice the configured check interval (at least 30 seconds) are marked for rechecking, not counted as available. A complete failed pool exposes its next retry time.

New subscriptions start with an empty switching pool. Clearing the pool stops automatic node selection and keeps the current connection. There is no subscription enable switch or additional region filter; membership in the right pane is the complete automatic selection rule.

Settings and cached subscriptions are stored in the Windows local application data directory for ClashFx. A separate directory can be selected with:

```powershell
.\ClashFx.exe --data-dir G:\ClashFxData
```

Each data directory is locked to one instance. The desktop app starts the core in the background when cached nodes are available and prepares it after an import. It does not change the system proxy until explicitly enabled. Enabling the system proxy also starts a stopped core and waits for its controller and selected node to be ready; startup failure leaves the system proxy untouched. Disabling the system proxy restores the previous Windows settings while keeping the core available for latency testing and applications with an explicit local proxy address. Closing the window hides it to the Windows notification area while the core and monitoring continue. Click the tray icon or launch the app again to restore the same window. The tray context menu contains Show and Exit; Exit restores the previous Windows proxy settings, stops the owned core and compatibility listeners, and waits for completion before quitting. If proxy recovery fails, the app stays open with its core available and shows the error. A persisted ownership record also restores an interrupted session's proxy settings on the next launch without overwriting a subsequently changed proxy server.

Settings includes **开机自启**, off by default. Enabling it registers the current executable and data directory for the current Windows user's next login. The switch reads the Windows registration on launch; disabling it removes only the ClashFx entry. Starting at login does not enable the system proxy. Uninstall removes the startup entry when it points to the installation being removed.

## Automatic failover

Unused subscription proxy groups are omitted from the runtime so their automatic health checks do not duplicate ClashFx checks. Groups referenced by proxy chains, including nested groups, remain available. Entry repair only targets the current node after a failed connectivity check; before applying a candidate it rechecks the existing connection and leaves a recovered route untouched.

- Only TCP/UDP connection failure logs attributed to `CLASHFX_AUTO` count toward the consecutive-failure threshold. Successful managed connections reset it. DNS, DIRECT, subscription update and unrelated warnings do not count.
- A threshold breach starts active HTTP 204 checks. A single failed destination never causes an immediate switch. The current node is checked first; if it passes, it is retained.
- Failure-triggered checks prepare standby nodes alongside the current-node check. If the current node fails, the first working standby is selected without waiting for slower peers. Subscriptions retain their priority; explicit selection and optional failback retain node priority. Probe concurrency is bounded.
- Startup retains the last selected node if it is healthy. Explicit detection uses the switching pool's subscription order. Periodic checks of the current node also detect failures when no new connection logs arrive.
- Automatic failback is disabled by default. When enabled, higher-priority nodes must pass consecutive recovery checks after the failback cooldown. That cooldown does not delay emergency failover. Manual-selection hold remains effective until expiry or explicit resume.
- If every pool node fails, ClashFx retains the selected proxy route and retries later. It does not use nodes outside the pool or silently change the route to DIRECT.
- Stopping, pausing or changing candidates cancels pending probes. Results from a superseded probe cannot switch a newly selected route.
- Remote subscriptions are checked daily. Unchanged profiles do not reload the core or restart checks. Failed updates retain the cached profile and retry after an hour. Updates preserve pool membership; new or renamed nodes must be added explicitly. Missing pool nodes remain visible for removal and cannot be selected automatically.

Defaults: three consecutive failures within a 30-second window; three-second total budget per node, shared by all test addresses and any confirmation retry; current-node checks every five minutes; automatic failback disabled (60-second cooldown and three recovery confirmations when enabled); 30-second retry when all candidates fail; five-minute manual hold. Any configured probe URL returning HTTP 204 is sufficient. Existing TCP connections are not migrated across outbound nodes and may need to reconnect. The v4 settings migration replaces the former default four/eight-second timeout with three seconds, preserving custom timeouts and backing up the original settings. Older v1/v2 settings also receive the stable-connection defaults; existing v3 failback and interval choices remain unchanged.

Failed domain-based entries are repaired automatically, including the manually selected node when the switching pool is empty or automatic switching is paused. ClashFx queries the system resolver and multiple independent DNS-over-HTTPS JSON endpoints for IPv4/IPv6 addresses. An isolated Mihomo process checks each candidate using the original proxy protocol and credentials; a reachable TCP port alone is insufficient. Only a verified address is applied, followed by another check in the running core and rollback on failure. The current node and routing mode are preserved. Repairs use exact subscription hostnames, preserve TLS/SNI and unrelated hosts, and do not modify subscription files or Windows hosts. Subscription hostname changes are rediscovered, and failed addresses are retried with fresh DNS results after a 60-second backoff. Each repair attempt is bounded to 60 seconds and 16 candidate addresses; failure leaves the existing route in place.

No subscriber domains or entry IPs are bundled. Address overrides exist only in the current session and its local runtime configuration; restart starts fresh discovery. `settings.json` has an `entry_repair` object with `enabled` (default `true`) and `resolvers` (default AliDNS, Cloudflare and Google JSON API URLs). Resolver URLs can be replaced for another network; HTTPS is required except for loopback test/local resolvers. DNS requests bypass application proxy settings so a broken entry does not prevent lookup. Literal-IP nodes and nodes using `dialer-proxy` retain their original behavior; direct mode and stopping the core cancel repairs.

## Configuration compatibility

Version 0.2 migrates legacy node selections and region filters into explicit pool membership without expanding the previously allowed set. Previously disabled subscriptions become available for manual use but their nodes are not added to the pool. The original settings are backed up once as `settings.v1.backup.json`; use that backup if returning to a legacy version.

The importer accepts UTF-8 Clash/Mihomo YAML with an explicit `proxies` list. Node protocol fields are preserved. Rule mode is the default: the first subscription containing rules supplies DNS, hosts, sniffer settings and routing rules, and Settings allows selecting another rule source. DIRECT/REJECT rules remain intact and proxy rule targets use the managed selector; logical rules, sub-rules and HTTP rule providers are supported. Selecting another node or failing over to another subscription does not change the rule source. If no subscription supplies rules, Settings explicitly reports the fallback: local networks are direct and other traffic uses the selected proxy. Remote rule/geodata files may need to be downloaded by the core. Node names and referenced static proxy groups are namespaced by subscription to prevent collisions.

Settings applies Rule, Global or Direct modes immediately, with validation and rollback on failure. Global mode uses the managed selector so node switching remains effective. Direct mode bypasses nodes and pauses automatic failover. The duplicate system proxy, manual core controls and reduced-motion setting have been removed from Settings; the only system proxy switch is on Connection.

Logs merge application events with Mihomo stdout/stderr, including startup diagnostics before the WebSocket connects. Warning/error filters, keyword search and copying the filtered results are available in a fixed toolbar. Only the records area scrolls, using the wheel or its visible draggable scrollbar. Update failures appear briefly as dismissible notifications and remain in the logs. Up to 2,000 core messages and 500 application events are retained in memory; the current core log also remains in the data directory. WebSocket events independently drive failure detection, so displaying unrelated DNS or DIRECT warnings does not trigger proxy failover.

Provider-only subscriptions and URI/base64 subscriptions must first be exported as Clash YAML containing nodes. ClashFx supports subscription and node management and Windows system-proxy workflows. TUN/service mode, WebDAV and extension scripting are not implemented.

## Build

Prerequisites: Rust 1.95.0 with the MSVC target, Visual Studio C++ Build Tools and a Windows SDK. The native workspace pins its own toolchain.

```powershell
.\scripts\fetch-core.ps1
.\scripts\build.ps1
```

Run those commands from `native`. The core fetch script selects the pinned official Mihomo release, validates its SHA256 against the release asset digest when available, and records the release URL/hash in `bin/core-version.json`. After every update, run `scripts/build.ps1` to refresh the complete application in `G:\MY_APP_PROJECTS\ClashFx\native\releases\ClashFx`, including when also creating an installer. From the repository root, `run-clashfx.ps1 -Build` builds and launches the native application.

To create the signed installer, run `scripts/build-installer.ps1` from `native`. It uses the locally configured DSignTool `梵星` rule, Inno Setup 6, and the x64 Visual C++ CRT from Visual Studio Build Tools. Pass `-SigningRule`, `-SigningToolPath`, `-CompilerPath` or `-RuntimeDirectory` for another local setup. The app, Mihomo, installer and uninstaller are signed; invalid or untimestamped signatures fail the build. The setup executable and SHA256 file are written to `native/releases/installer`. Only distribution files are staged; cached subscriptions, settings and development backups are excluded.

`THIRD-PARTY-NOTICES.txt` is included in both application and installer output. After dependency changes, regenerate it with cargo-about 0.9.3 using `cargo about generate --locked --workspace --fail --output-file THIRD-PARTY-NOTICES.txt about.hbs` from `native`. A normal build uses the checked-in notices and does not require cargo-about. Public releases must offer the matching ClashFx and Mihomo sources. GeoLite data has separate attribution, update and deletion requirements; see `NOTICE.txt` before redistributing databases or retaining older installer downloads.

## Verification

```powershell
cargo test -p clashfx-core
cargo test -p clashfx-core --test failover -- --ignored --nocapture
cargo test -p clashfx-core --lib forced_parent_exit_kills_core_and_recovers_proxy -- --ignored --nocapture
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The ignored integration tests require the fetched Mihomo executable. They start local HTTP proxy fixtures, send real requests through Mihomo, force failures and observe the actual log stream. They check same-subscription failover, cross-subscription fallback, pool exclusion, recovery confirmation, manual connection outside the pool, latency testing, clearing the pool while connected, manual hold/resume, all-unavailable behavior, pause, persistence and owned-process shutdown. They also verify live routing modes, rule-source selection, startup diagnostics, unrelated warnings and log clearing. Their fixtures never modify the Windows system proxy or use real subscription credentials. Evidence is retained under `native/work/integration-*`.

## Source layout

- `crates/clashfx-core`: persistent settings, subscription parsing, runtime configuration, Mihomo lifecycle/controller, log classification, health checks, failover and Windows system proxy ownership.
- `crates/clashfx-ui`: GPUI views, native text editing/IME, forms and command bindings.
- `scripts`: official core acquisition and Windows build packaging.
- `assets`: original vector logo and navigation icons, embedded in the GPUI interface and Windows executable. The connection diagram is drawn by `ui/network.rs`. To regenerate application PNG/ICO exports after editing the logo, run `node scripts/render-icons.cjs` with `sharp` installed (or set `CLASHFX_SHARP_MODULE` to its module path). Normal Rust builds use the checked-in exports and require no Node.js.

ClashFx uses GPL-3.0-only. See the repository `LICENSE` and `NOTICE.txt` for source attribution and third-party component notices; the adapted GPUI text editing code includes its Apache-2.0 license.
