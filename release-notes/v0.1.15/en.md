## New features

- Added MariaDB support: a dedicated connection type reuses the MySQL protocol path and persists the database type. It covers object browsing, database create/drop, SQL queries and completion, table design, data editing, import/export, backup and restore, users and privileges, and AI page context with controlled execution.
- MariaDB has its own read-only guard and AI SQL policy. Account DDL uses `IDENTIFIED VIA ... USING PASSWORD` and `SET DEFAULT ROLE ... FOR`, roles are treated as host-less accounts, backup and restore prefer `mariadb-dump` / `mariadb`, and TLS uses `--ssl`.
- Added connection-level read-only mode: the advanced connection settings include a read-only switch persisted as `options["read_only"]`. SQL writes are blocked by statement analysis, Redis writes by a command allowlist, and MongoDB writes by a read-only command allowlist; SQLite files also open read-only at the driver level, and the sidebar marks read-only connections.
- In read-only connections the Redis CLI now connects to a local RESP proxy: read commands run through the existing connector, write commands return `READONLY` without reaching the server, the proxy listens only on `127.0.0.1`, and `REDISCLI_AUTH` carries a random token.
- Added `⌘P` / `Ctrl+P` quick open: fuzzy-search tables and views already loaded in the sidebar, with highlighting, keyboard selection and direct open. When no tab is active it selects an available SQL connection automatically.
- Added built-in AI documentation tools: `docs.search` searches product documentation compiled into the app, and `docs.read` pages through a section by `section_id`. Both are read-only over local static content and never access user files, databases, credentials or the network.
- Added a complete Chinese user guide covering connections, the SQL editor, data editing, schema management, import/export, backup and restore, permissions, Redis, MongoDB, settings, shortcuts and troubleshooting, with real screenshots.
- The MongoDB command-line terminal and database-level command workbench now support `insertOne` / `insertMany` / `insert`, `updateOne` / `updateMany`, `replaceOne`, `deleteOne` / `deleteMany` / `remove`, index create/drop, `db.createCollection` and collection drop. Aggregation still rejects `$out` / `$merge` write-back.

## Improvements

- The settings database list now reflects the real connector state for MySQL, MariaDB, PostgreSQL, SQL Server, Oracle, SQLite, TiDB, Dameng DM8, MongoDB and Redis, replacing stale mock labels.
- Quitting with unsaved changes or running queries now asks for confirmation, covering keyboard shortcuts, the tray and window close.
- Connection dialogs, the object tree, data editing, import, restore, database/object create-drop, and Redis / MongoDB write entries share the read-only guard, and the UI and AI tools report the same read-only state.
- The AI SQL policy adds a MariaDB dialect and read-only checks; MongoDB write methods, the native terminal and the command workbench share one read-only boundary.
- SQL completion gained missing derived-table, CTE, set-operation, JSON table function, qualifier and case-handling cases across MySQL, PostgreSQL, Oracle and SQL Server.
- Redis Workbench execution failures now render as records like successes, so a failure remains visible when earlier records exist.

## Fixes

- Fixed MariaDB table validation incorrectly applying the MySQL `CHECK ... NOT ENFORCED` restriction.
- Fixed completion not triggering after a plain `schema.` qualifier; quoted identifiers no longer depend on a fallback path.
- Fixed MySQL completion gaps for CTE projection columns, set-operation derived columns, named windows, `JSON_TABLE` column lists, schema-qualified UDFs and cross-database table metadata.
- Fixed PostgreSQL completion gaps for `LATERAL` table-function column aliases, duplicate `()` insertion when a call already has parentheses, and quoted columns losing the `d.` qualifier.
- Fixed Oracle completion gaps for unquoted lowercase table names, CTE / derived-table aliases, `UNION` projections and `JSON_TABLE` column inference; leading comments are now stripped before execution to avoid `ORA-00900`.
- Fixed SQL Server completion inference for `OPENJSON ... WITH` column lists and `PIVOT` output columns.
- Fixed MongoDB completion reuse producing `db.db["customers"].` when a collection candidate is chosen after more typing: reused ranges are now shifted by the newly typed suffix.
- Fixed dirty cells not highlighting and deleted rows not dimming after an in-place primary-key edit; row identity now also considers the new value from the current edit.
- Fixed Redis Workbench not recording a failed execution and error messages being hidden by earlier records.
- Fixed the old function-signature popover remaining after a mouse click or drag moved the cursor out of a call.
- Fixed the update dialog not closing immediately after cancelling a download, and the cancel result being reported twice.
- Fixed Redis Pub/Sub publishing still being queued in read-only connections; the publish entry now rejects it.

## Known limits

- Connection read-only mode is a client-side guard, not a replacement for database account permissions. External clients or connections that bypass FluxDB remain governed by server-side grants.
- MariaDB reuses the MySQL protocol path and does not implement every MariaDB-specific server feature. Backup and restore require a local `mariadb-dump` / `mariadb` or compatible MySQL client.
- The AI documentation tools only cover documentation bundled with the app; they do not read online docs, user files or external knowledge bases.
- MongoDB write methods cover only the commands listed for the terminal and command workbench; unimplemented methods return a clear message instead of reporting success.

## Validation

- Passed `cargo fmt --all -- --check`, `cargo check --workspace --locked` and `cargo clippy --workspace --all-targets --locked`.
- Passed `cargo test -p fluxdb-app --lib`, documentation parsing and search tests, and the tool-catalog coverage test.
- Added live-database completion cases for MySQL, PostgreSQL, Oracle and SQL Server; they are gated by environment variables and skip automatically when unset.
- The release passed the GitHub Actions three-platform (macOS, Windows, Linux) build preflight.
