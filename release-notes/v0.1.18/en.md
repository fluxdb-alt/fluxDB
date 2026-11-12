## New features

- **Connection-level server panel**: open the new "Server" tab from a connection's context menu. SQL connections (MySQL / MariaDB / TiDB, PostgreSQL / KingbaseES, SQL Server, Oracle, Dameng) show an overview, sessions and lock waits, and can terminate queries or sessions and run maintenance such as VACUUM / ANALYZE / OPTIMIZE TABLE / REINDEX. Redis shows an overview, clients and slow log, with client termination and slow-log clearing. MongoDB shows an overview, current operations (slow-operation filter, killOp) and an in-session performance trend. The panel supports manual refresh, 5-second auto refresh and a "my sessions only" filter; read-only connections reject termination and maintenance in the UI, the app layer and the connector, and panel errors stay inside the panel.
- **Switch query target database and schema from the toolbar**: the connection name stays fixed while database and schema dropdowns switch the target per connection type; switching is disabled while a query runs. Switching database closes the old session and clears results (open transactions roll back), while switching schema keeps the session. PostgreSQL / KingbaseES send `RESET search_path` when returning to "default", and SQL Server resets to the toolbar database before every execution. SQLite, DM8 and Oracle expose these options read-only according to capability; qualify names in SQL when needed.
- **Custom themes and font settings**: create, edit and delete custom themes, adjusting common tokens and every SQL highlight category, with files written to the user data directory's `themes/`. Zed and VS Code themes (including JSONC) can be imported, syntax highlighting is inferred from scopes, and light/dark variants switch the theme mode automatically. UI font/size and code font/size/line-height are configured separately; the UI size scales menus, labels, tables and dialogs with live preview while dragging.

## Improvements

- The local MCP server moved from a standalone stdio process to an in-app Streamable HTTP endpoint (default `http://127.0.0.1:47821/mcp`). It listens on loopback only and validates a Bearer token on every request; client configuration now writes a URL plus `Authorization` header and takes over old stdio slots. Turning the switch off stops the endpoint and closes sessions, and turning it back on keeps the same address and token.
- Added centralized local SQLite migrations gated by `PRAGMA user_version`: each upgrade takes a consistent `VACUUM INTO` backup (the newest 3 are kept), each step and its version bump commit in one transaction, and a database written by a newer version fails closed instead of being modified by an older app.
- MCP `connections_list` now returns a database hint, and `search_objects` lists databases when no database context is given. Target/scope errors include the expected format and a correction hint.
- Unified database boundary semantics: SQLite table browsing keeps real value types to match the query editor, SQL Server renders negative DECIMAL values exactly, and the TiDB read-only path rolls back its transaction and rejects non-query statements.
- Added cross-engine SQL conformance tests and scripts: SQLite runs by default, while PostgreSQL, MySQL, MariaDB, TiDB, SQL Server, DM8, Kingbase and Oracle are enabled through environment variables, with documentation for container and remote runs.
- The English README is now the default and the Chinese version moved to `README.zh-CN.md`; the local gate script now rejects hard-coded px text sizes so UI text follows the rem scale.

## Fixes

- Fixed Redis databases sorting as strings in the sidebar, which put 10 before 2; they now sort numerically ascending.
- Fixed SQL Server negative DECIMAL values rendering as `-1.-2500`.

## Known limits

- Server panel session/client lists read at most 500 rows. SQL Server has no "cancel query" command and Oracle is limited by its driver, so both only support terminating sessions. The MongoDB performance trend is sampled in-session for about 5 minutes and is not persisted.
- The read-only connection is a client-side guard, not a replacement for a read-only server account; stored procedure side effects and high-privilege accounts can still bypass it.
- Custom themes do not support `.tmTheme` or other plist themes yet. Deleted themes move to `themes/.trash/` and do not free disk immediately. Imported themes infer highlighting from scopes, so some languages may need manual adjustment.
- Switching database closes the old session and rolls back uncommitted transactions. SQLite has no "current database", the DM8 driver cannot switch schema, and an Oracle instance has a single database, so qualify names in SQL in those cases.
- The MCP endpoint is local-only and is not reachable remotely. Turning MCP Server off does not delete client configurations already written.
- Local database upgrades create a `VACUUM INTO` backup (the newest 3 are kept), which uses extra disk. Databases upgraded by a newer version are refused by older apps, which ask the user to upgrade.
