## New features

- **Structure compare and sync**: open it from a connection's context menu, choose the source and target connection, database and optional schema/table. Base tables are paired by `schema.name` and reported as missing in target, extra in target, structurally different or identical, with side-by-side DDL and per-table column/index differences. Selected actions can create missing tables, drop extra tables and recreate modified tables; sync runs in the background and reports to the task center.
- **Cross-connection data transfer**: read source rows in pages and write them to a table on another connection in batches. Supports append and truncate-then-write modes, and can create the target table from the source DDL when it does not exist. Progress and logs update live, the task can be canceled, and memory usage depends only on the page size rather than table size.
- **Task center**: collects background work such as backup, restore, SQL file execution, table and MongoDB import/export, structure sync and data transfer. It shows running/completed/failed/canceled status, progress and logs, supports cancellation, and persists history locally so it survives a restart.
- **Visual query plans**: the toolbar "Explain" action reuses the selected SQL and parses `EXPLAIN FORMAT=JSON` for MySQL / MariaDB / TiDB, `EXPLAIN (FORMAT JSON)` for PostgreSQL / KingbaseES, `EXPLAIN QUERY PLAN` for SQLite, and hand-written MySQL-style `EXPLAIN` tables into a node tree and inspector. Full table scans, filesorts and missing indexes are flagged; unrecognized output falls back to the raw result table instead of guessing a plan.

## Improvements

- **Connection form refactor** (omitted from the previous release notes): high-frequency fields stay under "Connection info", while MySQL / MariaDB / TiDB charset, collation and URL parameters move to "Advanced"; SQLite hides the empty network tab to avoid irrelevant fields.
- MySQL / MariaDB / TiDB now support SOCKS5 and HTTP CONNECT proxies, both with username/password authentication and combinable with TLS; PostgreSQL reuses the shared proxy dialer. An unreachable proxy, failed handshake or failed authentication reports an error instead of silently falling back to a direct connection; SSH tunnels and proxies remain mutually exclusive.
- Unified connection-profile validation, options write-back, dial endpoints and credential slots across databases, with form fields and profile hydration managed centrally to reduce per-database branches.

## Fixes

- Fixed a false TLS-mode migration for legacy MySQL profiles: the old verify bit is migrated to `verify_ca` only when TLS is enabled, so profiles with TLS disabled are no longer rewritten.
- Fixed "Filter & sort" clearing values unconditionally when the field changes. Values are now checked against the new column type: text/unknown types keep them as-is, numeric/boolean columns keep only parseable values, between/not-between clears both bounds when either is incompatible, and a count is shown when some values are removed.
- Fixed PostgreSQL completion compatibility on KingbaseES and a nested `block_on` in export pagination.
- Added the missing Chinese and English "cancel task" strings for backup and restore.

## Known limits

- Structure compare covers base tables only, not views or routines. Cross-database sync recreates objects from the source DDL, so target-dialect differences may need manual adjustment.
- With "truncate then write", the truncate and subsequent batches are not in the same transaction; a mid-way failure leaves the target table truncated.
- Query plans are parsed only for MySQL / MariaDB / TiDB, PostgreSQL / KingbaseES and SQLite output. Other databases or unrecognized formats fall back to the raw result table, and the "Explain" entry never runs `EXPLAIN ANALYZE`.
- Task center history is stored locally; tasks still running when the process is interrupted are recorded as failed.
