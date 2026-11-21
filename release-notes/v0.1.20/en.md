## New features

- **Structure and data sync completed**: adds sync orchestration, chunked previews and streaming execution, with cross-dialect table creation and type mapping. Structure sync now covers CHECK constraints, foreign-key actions, index types / predicates and generated columns, and SQLite supports native index creation and deletion. The task center lists sync jobs and their execution records. The sync wizard's table list starts unchecked and requires an explicit selection before preview or execution.
- **Data sync export, cross-database writes and resumable runs**: sync can export migration SQL, appending each statement only after it succeeds. Default file names include the table and local time; append mode flushes statement by statement without repeating the file header. Cross-database write sessions add insert-conflict modes, per-row failure policies and LOB summaries. The sync ledger records resume signatures, chunk cursors and failed rows (storage migration v5).
- **Backup and restore enhancements**: MongoDB native archive backup / restore (`mongodump` / `mongorestore`) with credentials passed through a temporary 0600 config file instead of the command line or logs. PostgreSQL logical backup is now available. MySQL / PostgreSQL logical backups use keyset cursor pagination, multi-row `INSERT` batches and consistent snapshots. The restore dialog accepts MongoDB, and backup-list paths can be clicked to copy the full path.
- **Copy with column names**: `⌘⇧C` / `Ctrl+Shift+C` copies the selection with a header row, and the selection context menu offers the same action. A cell selection includes only the selected columns; the row-number column is excluded.
- **Table comments in browsing and search**: hovering a sidebar table row shows its name and comment, and object search matches both. Quick open (`⌘P`) and the command palette can recall tables by comment and show it in the candidate subtitle.
- **Dedicated sync completion status**: success / partially failed / cancelled status bars show table counts and inserted, updated and deleted row totals. The message log keeps execution progress and appends the report summary; the primary action becomes "Done" and closes the wizard.
- **Unified database brand icon tiles**: sidebar connection icons now use a theme-aware rounded tile with a subtle border, also applied to recent connections, the new-query dialog and the new-connection type list. Brand logos are larger and more consistent despite different built-in padding.
- **Structure compare loading state**: while comparing, the right side shows a spinner and "Comparing…" instead of stale results or an empty state. The Schema selector appears only for PostgreSQL / KingbaseES / SQL Server / Oracle / Dameng; MySQL / MariaDB / TiDB / SQLite hide it because the database is the schema.

## Improvements

- Connection-list rows now use a pointing-hand cursor instead of a closed hand.
- Backup-list paths have a hover hint and a copy confirmation.
- Large exports and backups read data in batches instead of loading whole tables into memory.

## Fixes

- **SQL Server object tree error 207**: table sizes now read `used_page_count` and `row_count` from `sys.dm_db_partition_stats`. Low-privilege accounts without `VIEW DATABASE PERFORMANCE STATE` fall back to a size-less query, so the tree still expands.
- **MySQL 9.x backup failure**: `mysqldump` rejects `--connect-timeout`; the option is now passed only to the `mysql` client used for restore imports.
- **Read-only could not be disabled when editing a connection**: saving now removes the old `read_only` option before merging the draft.
- **Stale MySQL table sizes in the sidebar**: object listing now disables the 24-hour `information_schema` statistics cache. Values still depend on InnoDB's own statistics refresh; run `ANALYZE TABLE` if needed.

## Known limits

- Logical backups do not include stored procedures / functions, triggers, events, privileges or sequence values; add them manually after restore. Unreadable large values fail the backup instead of producing a partial file.
- MongoDB archives support whole-database export only, not per-collection selection. `--oplog` is not used, so the consistency flag is false. Restore relies on the original database name stored in the backup record; external archives are rejected for now.
- Oracle / SQL Server / Dameng have no consistent snapshot yet, so their logical backups are explicitly marked as inconsistent.
- Structure / data sync and backup / restore depend on the matching database client tools. Missing tools are reported during preparation rather than silently degrading.
