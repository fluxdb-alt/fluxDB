## New features

- Added KingbaseES (Renmin University Kingbase) as a data source in PG compatibility mode. It reuses the PostgreSQL connector, wire protocol, `pg_catalog` and double-quoted identifier dialect, and supports connection tests, database/schema and object browsing, SQL queries and completion, data viewing and editing, table structure viewing and design, and the users and privileges entry.
- The connection form reuses the PostgreSQL profile with defaults `127.0.0.1:54321` / `system` / `test`, and supports TLS, SSH tunnels, proxies, schema scope, timeouts and `postgres://` connection-string import. KingbaseES is listed in the sidebar, settings and connection-kind list.
- Read-only mode, AI page context with controlled execution, filtering and sorting, import/export, table creation and ER diagrams are wired through the PostgreSQL family.

## Improvements

- PostgreSQL-related branches now share `is_postgres_family()`, so a new PG-family database no longer needs a `DatabaseKind::Postgres` patch at every dialect, completion, filter or AI policy site.
- The settings connector list and the connection-kind rail now report 11 built-in connectors.

## Fixes

- Fixed a panic when opening "table properties → DDL" on KingbaseES. Kingbase declares `pg_sequence.seqstart` / `seqincrement` / `seqmin` / `seqmax` as `int2` while the values are int8, so decoding them as `i64` failed; the query now casts to `::bigint`, which leaves PostgreSQL behavior unchanged.
- Fixed the connection-kind rail still showing "10 built-in drivers".

## Known limits

- PG compatibility mode only; Oracle compatibility mode is not supported.
- Native backup/restore is not supported yet. Use `sys_dump` / `ksql` from the command line when needed; the backup entry reports that the database is not supported.
- No `psql` / `ksql` external tool entries.
- The users and privileges page reuses the PostgreSQL implementation and has not been separately regression-tested on KingbaseES.
- Table DDL covers sequences, constraints, indexes, triggers, partitions and views. If a Kingbase-specific statement is not covered, please report the failing SQL.
