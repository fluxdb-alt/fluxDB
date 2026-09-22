# FluxDB — 跨平台数据库客户端与 SQL 编辑器

**基于 Rust + GPUI 的开源数据库管理工具（Database GUI），支持 macOS、Windows 和 Linux。**

在一个原生桌面应用中连接 MySQL、TiDB、PostgreSQL、SQLite 和 Redis，完成 SQL 查询、数据编辑、表结构管理与键值操作。无论是 MySQL / PostgreSQL 图形化管理、SQLite 数据库浏览，还是 Redis 可视化管理，都可以在同一个工作区中完成。

**简体中文 | [English](./README.en.md)**

[![Release](https://img.shields.io/github/v/release/fluxdb-alt/fluxDB)](https://github.com/fluxdb-alt/fluxDB/releases/latest)
[![CI](https://github.com/fluxdb-alt/fluxDB/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxdb-alt/fluxDB/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue)](./LICENSE)
![Platforms](https://img.shields.io/badge/平台-macOS%20%7C%20Windows%20%7C%20Linux-blue)

[下载安装](#下载安装) · [功能特性](#功能特性) · [操作手册](./docs/user-guide.md) · [参与贡献](#参与贡献) · [支持与赞助](#支持与赞助)

![FluxDB 主界面](docs/screenshots/home_page.png)

> FluxDB 仍处于早期开发阶段，功能与兼容性正在持续完善，尚未经过广泛的真实场景验证。操作重要数据前，请先做好备份。

## 功能特性

- **多数据库管理**：统一管理连接，在侧边栏浏览数据库、Schema、表和 Redis 键。
- **SQL 编辑器**：语法高亮、代码折叠、关键字与表列补全、SQL 格式化和查询历史。
- **数据浏览与编辑**：分页浏览、筛选数据、编辑并回写结果，支持数据导出与 SQL 文件执行。
- **表结构管理**：查看列信息和 DDL，创建、修改数据表。
- **Redis 工作台**：浏览与编辑键值，查看键详情，通过内嵌终端执行命令。
- **原生桌面体验**：使用 Rust + GPUI 构建，无 WebView 渲染依赖，提供明暗主题。

### 数据库支持

各数据库的功能范围有所不同，具体操作见[操作手册](./docs/user-guide.md)。

| 数据库 | 主要能力 |
| --- | --- |
| MySQL / TiDB | 连接管理、SQL 查询、表结构管理、数据编辑、导出与备份 |
| PostgreSQL | SQL 查询、Schema 与对象浏览、表结构管理、数据编辑、用户与角色权限管理、原生客户端工具 |
| SQLite | 本地数据库连接、SQL 查询、表结构浏览与数据编辑 |
| Redis | 连接总览、键值浏览与编辑、工作台与命令终端 |

### 界面预览

| SQL 编辑器 | 数据浏览与编辑 |
| :---: | :---: |
| ![SQL 编辑器](docs/screenshots/sql_editor_adapter.png) | ![数据浏览与编辑](docs/screenshots/mysql_data_table_ui.png) |

| 表结构管理 | Redis 键值详情 |
| :---: | :---: |
| ![表结构管理](docs/screenshots/mysql_table_info.png) | ![Redis 键值详情](docs/screenshots/redis_detail.png) |

## 下载安装

前往 **[最新版本下载页](https://github.com/fluxdb-alt/fluxDB/releases/latest)**，在 Assets 中选择对应平台的安装包。版本变化与更新说明见 [Releases](https://github.com/fluxdb-alt/fluxDB/releases)。

| 平台 | 架构 | 安装包 |
| --- | --- | --- |
| macOS | Apple Silicon（ARM64） | `FluxDB-<版本>-macos-arm64.dmg` |
| macOS | Intel（x64） | `FluxDB-<版本>-macos-x64.dmg` |
| Windows | x64 | `FluxDB-<版本>-windows-x64-setup.exe` |
| Linux | x64 | `fluxdb_<版本>_amd64.deb` |

- **macOS**：打开 DMG，将应用拖入「应用程序」。当前应用采用 ad-hoc 签名，尚未公证；首次打开若被系统拦截，确认下载来源后，可在「系统设置 → 隐私与安全性」中允许打开。
- **Windows**：运行安装器，按提示完成安装，可选择创建桌面快捷方式。
- **Linux**：面向 Ubuntu 22.04/24.04 等 Debian 系发行版，支持 X11 / Wayland 图形桌面。下载后在所在目录执行 `sudo apt install ./fluxdb_<版本>_amd64.deb`，将 `<版本>` 替换为实际版本号。

发布包附带 `.sha256` 校验文件。Windows ARM64、Linux ARM64、AppImage、Flatpak 和 Snap 暂未提供；更多发行版与硬件环境的兼容性仍需持续验证。

安装后，点击「添加连接」，选择数据库类型，填写连接信息并测试连接，即可开始使用。详细步骤见[操作手册](./docs/user-guide.md)（当前以 macOS 界面和快捷键为例）。

## 从源码运行

### 环境要求

安装最新稳定版 [Rust 工具链](https://rustup.rs)（项目使用 Rust edition 2024），并准备对应平台的原生构建工具：

| 平台 | 构建依赖 |
| --- | --- |
| macOS | Xcode Command Line Tools，可通过 `xcode-select --install` 安装 |
| Windows | Visual Studio Build Tools 的 C++ 桌面开发工具与 Windows SDK，使用 MSVC 工具链；release 构建需要 SDK 中的 `fxc.exe` |
| Linux | C/C++ 编译器、pkg-config、CMake、Clang，以及 OpenSSL、字体、X11 / Wayland 等开发库，完整清单见 [CI 配置](./.github/workflows/ci.yml) |

### 启动应用

```bash
git clone https://github.com/fluxdb-alt/fluxDB.git
cd fluxDB
cargo run --locked -p fluxdb-desktop
```

首次编译需要下载并构建依赖，耗时较长。

### 构建与打包

```bash
cargo build --locked --release -p fluxdb-desktop
```

macOS 可使用仓库脚本生成包含动态库的 `.app` 和 `.dmg`：

```bash
./scripts/package-macos.sh
```

脚本支持 `PROFILE=debug`、`CREATE_DMG=0` 等选项，详见[打包脚本](./scripts/package-macos.sh)。Windows 安装器与 Linux DEB 的构建步骤见[发布工作流](./.github/workflows/release.yml)。

## 参与贡献

欢迎报告问题、提出建议、改进文档和提交代码。

- **报告 Bug**：先搜索[已有 Issues](https://github.com/fluxdb-alt/fluxDB/issues)，提交时注明应用版本、操作系统与架构、数据库类型与版本、复现步骤、预期和实际结果；可附脱敏后的日志或截图。
- **提出建议**：在 [Issues](https://github.com/fluxdb-alt/fluxDB/issues) 中描述使用场景和希望解决的问题。较大改动建议先讨论方案。
- **提交代码**：Fork 仓库并创建分支，保持修改聚焦，在 Pull Request 中说明改动原因与验证结果。开发约定见 [AGENTS.md](./AGENTS.md)，界面改动另请参阅 [UI 样式规范](./docs/ui-style.md)。

Rust 修改提交前运行：

```bash
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
```

部分数据库集成测试需要对应的数据库服务，请在 PR 中说明测试环境与未验证范围。UI 行为修改还需启动应用验证主窗口及相关交互。

### 项目结构

| 目录 | 职责 |
| --- | --- |
| `apps/fluxdb-desktop` | GPUI 桌面界面与交互 |
| `crates/fluxdb-app` | 应用状态与业务编排 |
| `crates/fluxdb-core` | 领域模型、错误类型与连接器接口 |
| `crates/fluxdb-connectors` | 各数据库连接与操作实现 |
| `crates/fluxdb-storage` | 配置与连接持久化 |
| `crates/fluxdb-editor-core` | 通用编辑器内核 |
| `crates/fluxdb-editor-language` | 编辑器语言适配协议 |

本项目代码由 AI 生成，人工负责需求定义与验收，包括审阅变更、运行测试和检查界面效果。

## 路线图

- [ ] 完善尚未完整接入的设置项。
- [ ] 扩大 Windows / Linux 的发行版与硬件兼容性验证。
- [ ] 接入 MongoDB，支持集合浏览与文档编辑。
- [ ] 支持界面国际化与多语言切换。
- [ ] 探索自然语言转 SQL、补全增强与查询结果解释等 AI 功能。

以上为规划方向，尚未完成的功能不代表当前版本已经支持。

## 支持与赞助

如果 FluxDB 对你有帮助，欢迎给项目一个 Star，向朋友推荐，或通过反馈问题、完善文档和贡献代码参与维护。

也欢迎通过以下方式赞助，支持持续开发和 AI 工具费用。感谢每一份支持！

| 支付宝 | 微信 |
| :---: | :---: |
| <img src="docs/sponsor/alipay.png" alt="支付宝赞助二维码" width="220"> | <img src="docs/sponsor/wechat.png" alt="微信赞助二维码" width="220"> |

## 致谢

感谢以下开源项目提供的基础能力与设计参考：

- [Zed](https://github.com/zed-industries/zed)：GPUI 生态、编辑器与原生桌面应用架构。
- [GPUI Component / gpui-kit](https://github.com/longbridge/gpui-kit)：输入框、弹窗等基础 UI 组件。
- [dbx](https://github.com/t8y2/dbx)：多数据库抽象与连接管理设计参考。
- [RedisInsight](https://github.com/RedisInsight/RedisInsight)：Redis 可视化交互设计参考。

## 许可证

FluxDB 采用 [GNU GPL v3.0 或更新版本](./LICENSE) 开源许可证。
