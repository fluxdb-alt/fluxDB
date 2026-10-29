# FluxDB

**A native, cross-platform database client and SQL editor designed for high performance and low overhead: query, edit data, manage schemas and debug Redis in one window.**

FluxDB is built with Rust and GPUI, without Electron or a WebView. It brings MySQL, PostgreSQL, SQL Server, SQLite, TiDB, Dameng DM8, MongoDB and Redis into one desktop workflow, with a permission-aware AI Agent.

**English | [简体中文](./README.md)**

[![Release](https://img.shields.io/github/v/release/fluxdb-alt/fluxDB)](https://github.com/fluxdb-alt/fluxDB/releases/latest)
[![CI](https://img.shields.io/github/actions/workflow/status/fluxdb-alt/fluxDB/ci.yml)](https://github.com/fluxdb-alt/fluxDB/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue)](./LICENSE)
![Platforms](https://img.shields.io/badge/Platforms-macOS%20%7C%20Windows%20%7C%20Linux-blue)

[Download the latest release](https://github.com/fluxdb-alt/fluxDB/releases/latest) · [Latest changes](#latest-changes) · [Features](#core-capabilities) · [Database support](#database-support) · [User guide](./docs/user-guide.md) · [Report an issue](https://github.com/fluxdb-alt/fluxDB/issues)

![FluxDB SQL editor with schema completion](docs/screenshots/sql_editor_adapter.png)



## Latest changes

### v0.1.14

- **MongoDB support**: connection-string import with SRV discovery, database/collection/view browsing, a collection workspace (documents, aggregation, indexes, info), filters projection and sort, a visual condition builder, SQL-style query completion, document create/edit/duplicate/delete, JSON / JSONL / CSV import and export, collection rename and drop, a command-line terminal, and AI page context with controlled execution.
- **MongoDB first-release limits**: no database-level backup/restore (`mongodump` / `mongorestore`) and no performance-metrics panel; no cross-database collection moves, bulk delete, in-table row editing, transactions or change streams. See [Database support](#database-support) and [MongoDB support](./docs/mongodb.md#7-已知限制) (Chinese).
- **Oversized values no longer drop rows**: when a cell exceeds the result-page budget its body is offloaded to local controlled storage and only a preview plus size is kept, viewable in chunks; upload replacement now streams in chunks. Applies to MySQL, SQLite, PostgreSQL, SQL Server and Dameng DM8.
- **MongoDB query performance**: the document area now uses a virtualized list with whole-page parse caching, building only the cards in view each frame.
- **Stability fixes**: fixed rows with large values failing to update or delete in PostgreSQL and in tables without primary keys, SQL Server dropping rows over budget, crashes when importing non-ASCII values into MongoDB, and filter/sort popovers not anchored to the control that opened them.

[Full release notes](./release-notes/v0.1.14/en.md) · [All releases](https://github.com/fluxdb-alt/fluxDB/releases)

### v0.1.13

- **Oracle support**: Service Name / SID connections, schema/object browsing, SQL queries with dialect-aware completion, table design, data editing, filtering and sorting, user and permission management, local logical export, and AI page context with controlled execution.
- **Oracle first-release limits**: no server-side Data Pump backup; export is local logical export (DDL plus paged data), which is neither a native server backup nor a cross-page consistent snapshot. Grant details of other accounts are unreadable and are not drawn as revocable grants. See [Database support](#database-support).
- **Five-tab user administration**: General / Roles / System privileges / Object privileges / SQL preview, each opening with a type-locked grant editor. Oracle "New schema" now opens the create-account form, and Dameng reuses the same panel.
- **Stability fixes**: fixed Oracle import `ORA-01861`, table copy not reading the source DDL, Explain sending statements that always fail, and account changes not reporting how many statements already ran.

[Full release notes](./release-notes/v0.1.13/en.md) · [All releases](https://github.com/fluxdb-alt/fluxDB/releases)

## Why FluxDB

- **One workspace for multiple data sources**: manage relational databases, MongoDB and Redis side by side.
- **From query to write**: SQL editing, editable result sets, table design, import/export, backup/restore and ER diagrams.
- **Native architecture, performance first**: built with Rust + GPUI, without Electron or a WebView, with low overhead and responsive interaction as design goals.
- **AI with explicit boundaries**: bounded reads, authorized editor edits and approval for write or high-risk tools.

## Core capabilities

### SQL and data

- Syntax highlighting, code folding, keyword/table/column/function completion, SQL formatting and query history.
- Paginated browsing, filters, sorting, row details and cell details.
- Editable result sets with writes back to the database and safeguards for dangerous actions.
- Import TXT, CSV, JSON and XML.
- Export SQL, TXT, CSV, JSON and XML.
- Execute existing `.sql` files.

### Schema and ER diagrams

- Inspect columns, indexes, foreign keys, triggers and DDL.
- Create and modify tables visually, with DDL generated for the target database dialect.
- Layout ER diagrams from real foreign keys, with optional local logical relationships.
- Local logical relationships never create or modify database foreign keys.
- Export ER diagrams as JSON, DBML, Mermaid or SVG.

### MongoDB workbench

- Connection-string import with SRV discovery, browsing by database / collection / view, and connection info with topology.
- The documents tab supports filter, projection, sort, collation, paging and index hint, with card, table and JSON display modes.
- The condition builder and completion cover filter, projection, sort and aggregation pipelines; aggregation, indexes and collection info each have their own tab.
- Create, edit, duplicate and delete documents; import JSON / JSONL / CSV and export JSON / JSONL / CSV.
- Collection context menu covers pin, copy name, refresh, export, rename, import and drop; databases can create collections and be dropped.

### Redis workbench

- Browse the keyspace and inspect or edit common value types, TTL and key details.
- Run commands with command history in the Workbench.
- Use the embedded Redis CLI terminal.
- Observe channel messages with Pub/Sub subscriptions.
- Connect with TLS, SSH tunnels, Sentinel, Cluster, URI or Cloud configuration.

### Operations, security and updates

- Connection passwords and model API keys use controlled storage and stay out of ordinary exports, logs, event snapshots and AI context.
- In-app updates verify the signed manifest and package checksum.
- Release packages include SHA-256 checksums.
- The UI supports English and Simplified Chinese with runtime switching.

## Demo

**Full demo** · 2 min 46 sec · MP4, about 8.6 MB

https://github.com/user-attachments/assets/f6075999-a87f-4644-9d14-dbda49a7981d

## Database support

Capabilities vary by database. See the [user guide](./docs/user-guide.md) for details.

| Database | Current capabilities | Backup / restore | Connections and limits | Not yet supported |
| --- | --- | --- | --- | --- |
| MySQL | SQL queries, object browsing, table management, data editing, import/export, native client tool entry | `mysqldump` logical backup; SQL restore | TLS/SSL, SSH tunnel, proxy, timeouts, charset | - |
| TiDB | Reuses the MySQL protocol path with TiDB SQL dialect, completion and schema management | Reuses MySQL logical backup | Same connection features as MySQL | - |
| PostgreSQL | SQL queries, schema/object browsing, table management, data editing, users and roles, psql tool entry | `pg_dump` plain SQL; `psql` restore | TLS, SSH, proxy, schema scope, timeouts | - |
| SQL Server | T-SQL queries, database/schema/object browsing, table design and data editing, table import/export, logins, database users, role membership and explicit permissions, object-level logical backup/restore, AI page context and controlled execution | Object-level logical SQL backup and restore | SQL Server authentication, TLS, explicit ports; named instances require the actual port | Windows integrated authentication, Azure AD, contained database users, column-level permission editing, user-defined server roles, SQL Browser/named-instance auto-discovery, and database-level physical `BACKUP DATABASE`. Object-level logical backup and restore is not a substitute for database-level disaster recovery. |
| SQLite | Local file connections, SQL queries, object browsing, table management, data editing | Consistent `VACUUM INTO` snapshot; restore to a new file with integrity and foreign-key checks | Local file path | - |
| Dameng DM8 | Schema/object browsing, SQL queries, table design and management, data editing, table import | Logical backup and SQL restore; restore currently targets a single schema | Instance and schema connection settings | - |
| MongoDB | Connection-string import (`mongodb://` / `mongodb+srv://`), database/collection/view browsing, collection workspace (document filter/projection/sort, condition builder, aggregation, indexes, collection info), document create/edit/duplicate/delete, JSON / JSONL / CSV import and export, collection rename and drop, command-line terminal, SQL-style query completion, AI page context and controlled execution | No database-level backup/restore; export is document-level logical export (JSON / JSONL / CSV) | `mongodb+srv://` discovery, SCRAM-SHA-1 / SCRAM-SHA-256 / MONGODB-X509, TLS options via connection-string import; at most 500 documents per query | No `mongodump` / `mongorestore` backup and restore, no performance-metrics panel, no SSH tunnel, no bulk delete (and no undo), no cross-database collection moves, no view rename/drop, no time-series or capped collection creation, no in-table row editing, no transactions or change streams; CSV import supports scalar fields only and must be UTF-8; new documents always get an ObjectId `_id`. |
| Redis | Keyspace browsing, value editing, TTL, Workbench, CLI, Pub/Sub | No database-level backup/restore | TLS, SSH, Sentinel, Cluster, URI/Cloud; Cluster uses DB 0 only | - |

MariaDB is not currently supported. It is a roadmap candidate, not a current capability.

## AI Agent

FluxDB includes an optional AI Agent for understanding schemas, writing SQL, investigating data and generating reports. It is not an unrestricted desktop assistant.

- **Schema discovery**: search connections, databases, schemas, tables, views and columns without reading whole tables by default.
- **Bounded reads**: read only the rows or content allowed by each tool contract.
- **Authorized editor actions**: edit only an explicitly authorized SQL range and never execute it implicitly.
- **Controlled SQL and Redis execution**: write or high-risk tools such as `db.run` and `redis.execute` require approval.
- **Scoped context**: read only authorized attachments, page references or editor snapshots.
- **BI and reports**: generate report drafts, charts, tables and explanations as saveable artifacts.
- **MCP and plugins**: configure MCP Client/Server, model bindings, plugins, tool permissions and usage diagnostics.
- **User-provided models**: configure your own model service and API key. Secrets use controlled storage and do not enter ordinary logs, exports or AI context.

## Download and install

Open the [latest release](https://github.com/fluxdb-alt/fluxDB/releases/latest) and choose the package for your platform.

| Platform | Architecture | Package |
| --- | --- | --- |
| macOS | Apple Silicon (ARM64) | `FluxDB-<version>-macos-arm64.dmg` |
| macOS | Intel (x64) | `FluxDB-<version>-macos-x64.dmg` |
| Windows | x64 | `FluxDB-<version>-windows-x64-setup.exe` |
| Linux | x64 | `fluxdb_<version>_amd64.deb` |

- **macOS**: open the DMG and drag FluxDB into Applications. The app is currently ad-hoc signed and not notarized. If macOS blocks the first launch, verify the download source and allow it in System Settings → Privacy & Security.
- **Windows**: run the installer and optionally create a desktop shortcut.
- **Linux**: targets Debian-based distributions such as Ubuntu 22.04/24.04 with X11 or Wayland. Run `sudo apt install ./fluxdb_<version>_amd64.deb`.

Release packages include `.sha256` checksums. Windows ARM64, Linux ARM64, AppImage, Flatpak and Snap are not currently available.

MySQL and PostgreSQL backup/restore depend on the local `mysqldump`/`mysql` and `pg_dump`/`psql` clients. Basic queries and editing do not require them.

After installation, choose “Add connection”, select a database type, enter the connection details and test the connection. See the [user guide](./docs/user-guide.md); it currently uses Chinese screenshots and macOS shortcuts.

## Run from source

<details>
<summary>Build dependencies, running and packaging</summary>

### Prerequisites

Install the latest stable [Rust toolchain](https://rustup.rs) (the project uses Rust edition 2024) and the native build tools for your platform:

| Platform | Build dependencies |
| --- | --- |
| macOS | Xcode Command Line Tools, available through `xcode-select --install` |
| Windows | Visual Studio Build Tools with Desktop development with C++ and the Windows SDK, using the MSVC toolchain; release builds require the SDK's `fxc.exe` |
| Linux | C/C++ compiler, pkg-config, CMake, Clang, OpenSSL, font and X11/Wayland development libraries; see the [CI configuration](./.github/workflows/ci.yml) for the complete list |

### Run

```bash
git clone https://github.com/fluxdb-alt/fluxDB.git
cd fluxDB
cargo run --locked -p fluxdb-desktop
```

The first build downloads and compiles dependencies and may take a while.

### Build and package

```bash
cargo build --locked --release -p fluxdb-desktop
```

On macOS, generate an `.app` and `.dmg` containing the required dynamic libraries:

```bash
./scripts/package-macos.sh
```

The script supports options such as `PROFILE=debug` and `CREATE_DMG=0`; see the [packaging script](./scripts/package-macos.sh). Windows installer and Linux DEB steps are defined in the [release workflow](./.github/workflows/release.yml).

</details>

## Contributing

Issues, feature proposals, documentation improvements and code contributions are welcome.

- **Report a bug**: search [existing issues](https://github.com/fluxdb-alt/fluxDB/issues) first, then include the app version, operating system and architecture, database type and version, reproduction steps, expected and actual results, and sanitized logs or screenshots where useful.
- **Suggest a feature**: describe the use case and the problem it solves. Discuss larger changes before implementation.
- **Contribute code**: fork the repository, create a focused branch and explain the reason and validation results in the pull request. See [AGENTS.md](./AGENTS.md) for development conventions and the [UI style guide](./docs/ui-style.md) for interface changes (both in Chinese).

Run the local quality gate before submitting:

```bash
./scripts/check.sh
```

Cross-platform build differences still require CI verification before release.

AI is used extensively in the development workflow. Humans own requirements, review, validation and release approval.

## Roadmap

Current priorities are real-world coverage for existing databases, stability and the cross-platform release experience. Candidate directions include:

- **SQL Server follow-up**: Windows integrated authentication and Azure AD, contained database users, column-level permissions, user-defined server roles, named-instance auto-discovery, and database-level physical backup/restore.
- **MongoDB follow-up**: database-level backup/restore, performance metrics, bulk delete and bulk writes, in-table row editing, transactions and change streams.
- **MariaDB**: independent dialect, system tables and backup parameter support.
- **More databases**: evaluate ClickHouse, DuckDB, Elasticsearch/OpenSearch and others based on community demand.

The roadmap does not imply current support.

## Community

Scan the QR code to join the FluxDB WeChat group, share your experience, report problems and suggest improvements. Please also submit bugs and feature requests to [Issues](https://github.com/fluxdb-alt/fluxDB/issues) so they can be tracked.

<img src="docs/screenshots/weichatgroup.png" alt="FluxDB WeChat group QR code" width="200">

## Support and sponsorship

If FluxDB is useful to you, consider starring the repository, sharing it with others, or contributing through issues, documentation and code.

You can also sponsor ongoing development and AI tooling costs.

| Alipay | WeChat |
| :---: | :---: |
| <img src="docs/sponsor/alipay.png" alt="Alipay sponsorship QR code" width="160"> | <img src="docs/sponsor/wechat.png" alt="WeChat sponsorship QR code" width="160"> |

## Acknowledgements

- [Zed](https://github.com/zed-industries/zed): GPUI ecosystem, editor and native desktop architecture.
- [GPUI Component / gpui-kit](https://github.com/longbridge/gpui-kit): reusable UI components.
- [dbx](https://github.com/t8y2/dbx): multi-database abstraction and connection-management design reference.
- [RedisInsight](https://github.com/RedisInsight/RedisInsight): Redis visualization and interaction design reference.

## License

FluxDB is licensed under the [GNU GPL v3.0 or later](./LICENSE).

See [NOTICE](./NOTICE) for copyright and attribution details.
