# FluxDB — Cross-Platform Database GUI & SQL Editor

**An open-source database management tool built with Rust + GPUI for macOS, Windows, and Linux.**

Connect to MySQL, TiDB, PostgreSQL, SQLite, and Redis in one native desktop app for SQL queries, data editing, table management, and key-value operations. Use FluxDB as a MySQL or PostgreSQL GUI client, a SQLite database browser, and a Redis GUI for visual key-value management.

**English | [简体中文](./README.md)**

[![Release](https://img.shields.io/github/v/release/fluxdb-alt/fluxDB)](https://github.com/fluxdb-alt/fluxDB/releases/latest)
[![CI](https://github.com/fluxdb-alt/fluxDB/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxdb-alt/fluxDB/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue)](./LICENSE)
![Platforms](https://img.shields.io/badge/Platforms-macOS%20%7C%20Windows%20%7C%20Linux-blue)

[Download](#download-and-install) · [Features](#features) · [User Guide](./docs/user-guide.md) · [Contributing](#contributing) · [Support & Sponsorship](#support--sponsorship)

![FluxDB main window](docs/screenshots/home_page.png)

> FluxDB is in early development. Features and compatibility are still evolving, and the app has not yet been tested across a broad range of real-world scenarios. Back up important data before making changes.

## Features

- **Multiple databases**: Manage connections and browse databases, schemas, tables, and Redis keys from a shared sidebar.
- **SQL editor**: Syntax highlighting, code folding, keyword/table/column completion, SQL formatting, and query history.
- **Data browsing and editing**: Browse pages, filter rows, edit and write back results, export data, and execute SQL files.
- **Table management**: Inspect columns and DDL, create tables, and modify table structures.
- **Redis workbench**: Browse and edit values, inspect key details, and run commands in an embedded terminal.
- **Native desktop UI**: Built with Rust + GPUI, with light and dark themes and no WebView rendering dependency.

### Database support

Available features vary by database. See the [user guide](./docs/user-guide.md) (in Chinese) for details.

| Database | Main capabilities |
| --- | --- |
| MySQL / TiDB | Connections, SQL queries, table management, data editing, export, and backup |
| PostgreSQL | SQL queries, schema and object browsing, table management, data editing, user and role administration, and native client tools |
| SQLite | Local database connections, SQL queries, table structure browsing, and data editing |
| Redis | Connection overview, key-value browsing and editing, workbench, and command terminal |

### Screenshots

| SQL editor | Data browsing and editing |
| :---: | :---: |
| ![SQL editor](docs/screenshots/sql_editor_adapter.png) | ![Data browsing and editing](docs/screenshots/mysql_data_table_ui.png) |

| Table management | Redis key details |
| :---: | :---: |
| ![Table management](docs/screenshots/mysql_table_info.png) | ![Redis key details](docs/screenshots/redis_detail.png) |

## Download and install

Open the **[latest release](https://github.com/fluxdb-alt/fluxDB/releases/latest)** and choose your platform's package under Assets. See [Releases](https://github.com/fluxdb-alt/fluxDB/releases) for release notes and earlier versions.

| Platform | Architecture | Package |
| --- | --- | --- |
| macOS | Apple Silicon (ARM64) | `FluxDB-<version>-macos-arm64.dmg` |
| macOS | Intel (x64) | `FluxDB-<version>-macos-x64.dmg` |
| Windows | x64 | `FluxDB-<version>-windows-x64-setup.exe` |
| Linux | x64 | `fluxdb_<version>_amd64.deb` |

- **macOS**: Open the DMG and drag the app into Applications. The app is currently ad-hoc signed and not notarized. If macOS blocks the first launch, verify the download source and allow it in System Settings → Privacy & Security.
- **Windows**: Run the installer and follow the prompts. A desktop shortcut is optional.
- **Linux**: The DEB targets Debian-based distributions such as Ubuntu 22.04/24.04, with X11 / Wayland desktops. From the download directory, run `sudo apt install ./fluxdb_<version>_amd64.deb`, replacing `<version>` with the actual version.

Release packages include `.sha256` checksum files. Windows ARM64, Linux ARM64, AppImage, Flatpak, and Snap packages are not currently available. Compatibility with additional distributions and hardware still needs validation.

After installation, click “添加连接” (Add connection), choose a database type, enter connection details, and test the connection. Follow the [user guide](./docs/user-guide.md) for more details; it is currently in Chinese and uses macOS screenshots and shortcuts.

## Run from source

### Prerequisites

Install the latest stable [Rust toolchain](https://rustup.rs) (the project uses Rust edition 2024) and your platform's native build tools:

| Platform | Build dependencies |
| --- | --- |
| macOS | Xcode Command Line Tools, available through `xcode-select --install` |
| Windows | Visual Studio Build Tools with Desktop development with C++ and Windows SDK, using the MSVC toolchain; release builds require the SDK's `fxc.exe` |
| Linux | C/C++ compiler, pkg-config, CMake, Clang, and development libraries for OpenSSL, fonts, X11 / Wayland, and more; see the [CI configuration](./.github/workflows/ci.yml) for the complete list |

### Launch the app

```bash
git clone https://github.com/fluxdb-alt/fluxDB.git
cd fluxDB
cargo run --locked -p fluxdb-desktop
```

The first build takes time to download and compile dependencies.

### Build and package

```bash
cargo build --locked --release -p fluxdb-desktop
```

On macOS, use the bundled script to generate an `.app` and `.dmg` with dynamic libraries included:

```bash
./scripts/package-macos.sh
```

The script supports options such as `PROFILE=debug` and `CREATE_DMG=0`; see the [packaging script](./scripts/package-macos.sh). Windows installer and Linux DEB packaging steps are in the [release workflow](./.github/workflows/release.yml).

## Contributing

Bug reports, feature suggestions, documentation improvements, and code contributions are welcome.

- **Report a bug**: Search [existing issues](https://github.com/fluxdb-alt/fluxDB/issues) first. Include the app version, operating system and architecture, database type and version, reproduction steps, and expected versus actual behavior. Add sanitized logs or screenshots where useful.
- **Suggest a feature**: Describe your use case and the problem in [Issues](https://github.com/fluxdb-alt/fluxDB/issues). For larger changes, discuss the approach before implementation.
- **Contribute code**: Fork the repository, create a branch, and keep changes focused. Explain the reason for the change and validation results in your pull request. See [AGENTS.md](./AGENTS.md) for development conventions and the [UI style guide](./docs/ui-style.md) for interface changes (both in Chinese).

Before submitting Rust changes, run:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
```

Some database integration tests require running database services. Describe your test environment and any unverified areas in the PR. For UI behavior changes, also launch the app and verify the main window and affected interactions.

### Project structure

| Directory | Responsibility |
| --- | --- |
| `apps/fluxdb-desktop` | GPUI desktop UI and interactions |
| `crates/fluxdb-app` | Application state and business orchestration |
| `crates/fluxdb-core` | Domain models, errors, and connector interfaces |
| `crates/fluxdb-connectors` | Database connections and operations |
| `crates/fluxdb-storage` | Configuration and connection persistence |
| `crates/fluxdb-editor-core` | General-purpose editor core |
| `crates/fluxdb-editor-language` | Editor language adapter protocol |

The project's code is generated by AI. Humans define requirements and perform acceptance checks, including reviewing changes, running tests, and inspecting the UI.

## Roadmap

- [ ] Complete the integration of settings that are not yet fully functional.
- [ ] Expand Windows / Linux compatibility testing across distributions and hardware.
- [ ] Add MongoDB support for collection browsing and document editing.
- [ ] Add UI internationalization and language switching.
- [ ] Explore AI features such as natural-language-to-SQL, enhanced completion, and query result explanations.

These are planned directions; unfinished items are not available features of the current release.

## Support & Sponsorship

If FluxDB is useful to you, consider starring the repository, sharing it with others, reporting issues, improving the docs, or contributing code.

You can also sponsor development and AI tooling costs using the QR codes below. Thank you for your support!

| Alipay | WeChat Pay |
| :---: | :---: |
| <img src="docs/sponsor/alipay.png" alt="Alipay sponsorship QR code" width="220"> | <img src="docs/sponsor/wechat.png" alt="WeChat Pay sponsorship QR code" width="220"> |

## Acknowledgments

Thanks to these open-source projects for their foundations and design references:

- [Zed](https://github.com/zed-industries/zed): The GPUI ecosystem, editor design, and native desktop architecture.
- [GPUI Component / gpui-kit](https://github.com/longbridge/gpui-kit): UI components such as inputs and dialogs.
- [dbx](https://github.com/t8y2/dbx): References for database abstractions and connection management.
- [RedisInsight](https://github.com/RedisInsight/RedisInsight): References for Redis visualization and interactions.

## License

FluxDB is licensed under the [GNU GPL v3.0 or later](./LICENSE).
