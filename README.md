# ClashFx

Windows 原生代理客户端，使用 Rust 与 GPUI 构建，基于 Mihomo 内核运行，支持节点和订阅之间的自动故障切换。

[English](docs/README_en.md)

## 下载

[下载 Windows x64 签名安装包](https://github.com/skyfall0582/clashfx/releases/latest)

在发布页下载 `ClashFx-版本号-x64-Setup.exe` 并按提示安装。发布页同时提供 SHA256 校验文件和对应源码。

## 功能

- 导入 Clash / Mihomo YAML 订阅链接或本地文件，管理节点、订阅与备注。
- 自定义自动切换池及优先顺序，连接故障时检测并切换到可用节点。
- 手动选择节点、检测延迟，支持规则、全局和直连模式。
- 管理 Windows 系统代理，退出或异常中断后恢复原有代理设置。
- 支持托盘运行、开机自启、入口地址修复和日志筛选。

## 构建

需要 Windows x64、Rust 1.95.0（MSVC 工具链）、Visual Studio C++ Build Tools 和 Windows SDK。

```powershell
git clone https://github.com/skyfall0582/clashfx.git
cd clashfx
.\native\scripts\build.ps1
```

构建脚本会下载 Mihomo 和地理数据库，并将完整程序输出到 `native/releases/ClashFx`。

Rust 工作区位于 `native/`，开发环境和检查命令见[贡献指南](CONTRIBUTING.md)。

## 使用

运行构建目录中的 `ClashFx.exe`，保留同目录的 `mihomo.exe` 和 `geodata` 文件夹。

1. 在「订阅池」导入包含 `proxies` 列表的 Clash / Mihomo YAML 订阅或本地文件。
2. 选择节点，在「连接」页开启「系统代理」。
3. 需要自动故障切换时，在「自动切换」中将节点加入切换池，并调整优先顺序。

关闭窗口后程序继续在托盘运行；从托盘菜单退出会恢复系统代理并停止内核。订阅与设置保存在本机，卸载时保留。

详细操作见[中文使用说明](native/USAGE.zh-CN.txt)，构建与实现说明见[开发文档](native/README.md)。

## 许可证

本项目使用 [GPL-3.0-only](LICENSE) 许可证。代码来源与第三方组件声明见 [NOTICE.txt](native/NOTICE.txt)。
