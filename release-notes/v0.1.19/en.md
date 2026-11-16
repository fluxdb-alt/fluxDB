## New features

- **Select all and export query results**: data pages and query results support `⌘A` / `Ctrl+A` to select every cell on the current page and copy it as TSV for pasting into Excel (the shortcut still selects text when focus is in a search box or the SQL editor). A new "Export" action in the query result toolbar reuses the SQL the result actually ran, and the SQL can be edited under the "Condition" tab with refreshed row estimates. The "General" tab selects fields (all by default, with invert) and writes TXT / CSV / JSON / XML; duplicate column names become `id`, `id_2` in the header. Export runs the full SQL in batches regardless of the result page and can be cancelled.
- **Test data generator**: open it from the table data toolbar ("Generate data") or the table context menu ("Generate test data"). Configure a generator and its parameters per column, with 40+ built-in generators for numbers, text, names, contact details, addresses, companies, commerce and dates; incompatible generators are greyed out by column type. Foreign-key columns read a parent-table value pool (physical foreign keys merged with ER logical relationships, at most 1,000 rows), and the preview shares the same value rules as the actual write. Configure rows (up to 200,000), random seed, data language, per-column NULL rate and uniqueness, and batch size (1–5,000); write modes are append and truncate-then-write, and unique conflicts can skip the row or abort. The right side previews data, dialect-specific sample SQL and task logs, and a running task can be cancelled.
- **Query editor split panes**: a tab can hold any number of side-by-side or stacked panes, each with its own editor, run action and results. `⌘\` / `Ctrl+\` splits right and `⇧⌘\` / `Ctrl+Shift+\` splits down. Closing a pane that still has SQL or results asks whether to discard or move it to a new tab, and the tab strip shows a pane-count badge.
- **Foreign-key navigation, relations panel and form view**: the data toolbar adds a "Table | Form" switch. The form lays fields out vertically and reuses the same `EditDataCell` change set and commit semantics. Foreign-key cells and the right-hand relations panel jump to the referenced or referencing row by equality, with inline previews, an ER mini-map and a draggable width; enum and candidate-value columns get an editable dropdown in the form.
- **Unified command palette**: `⇧⌘P` / `Ctrl+Shift+P` searches commands, objects, queries, history and connections in one place. `Tab` / `Shift+Tab` switches scope, arrow keys select and `Enter` runs; command rows show their own shortcuts.
- **Fast tab switching**: hold `Ctrl+Tab` / `Ctrl+Shift+Tab` to open a VS Code-style tab drawer and keep cycling, releasing Ctrl to commit. `⇧⌘E` / `Ctrl+Shift+E` opens a searchable switcher, which scrolls the selection into view. Switching stays within the current connection / database workspace.
- **Numbered tab access and table shortcut groups**: `⌥1`–`⌥9` / `Alt+1`–`Alt+9` switch to the Nth visible tab in the current workspace, following pin and drag order. One shortcut can bind 1–9 tables: a single table opens directly and multiple tables show a keyboard picker, with up to 32 groups. Manage them from the table context menu or settings; conflict detection covers app shortcuts, existing groups and reserved keys. A closed connection is connected automatically before the table opens.
- **Table sizes in the sidebar**: object lists now return size with the existing metadata query. MySQL / MariaDB / TiDB use data plus index length, PostgreSQL / KingbaseES estimate from `pg_class.relpages` and index pages, and SQL Server uses `sys.partitions.used_page_count`. The size appears right of the table name when the sidebar is at least 220 px wide and hides when narrower.

## Improvements

- Added the built-in Flux Amber theme, selectable directly in the theme picker.
- Query result export preserves the SQL as written: the executor's default row cap is not added to the export condition, so no `LIMIT` means no limit and an explicit `LIMIT` is kept as-is.
- Foreign-key targets in the generator are shown as "FK → table.column" badges; long names are elided in the middle with the full target on hover.

## Fixes

- Fixed SQL history being lost at startup because the history loading task was cancelled.
- Fixed history table-name extraction treating tables that start with digits, such as `3d_active_history`, as numeric tokens and skipping them.
- Fixed SQL history quick search (`⌘⇧H` / `Ctrl+Shift+H`) returning results from other databases; it now filters by the current connection and database.

## Known limits

- Query result export has no SQL INSERT format because a result set has no target table. Export runs in read-only mode so writes never take effect, and an interrupted task leaves a `.partial` file behind.
- Test data generation supports SQL connections only; MongoDB and Redis have no entry point. A single run generates at most 200,000 rows, only one generation task runs at a time, and truncate-then-write rolls back only on PostgreSQL / SQLite. Unique conflicts skip rows whose value cannot be made unique by default.
- Table sizes are magnitude hints for the sidebar: PostgreSQL / KingbaseES values are `relpages` estimates and may be 0 (hidden) before ANALYZE or on empty tables; other databases do not provide sizes.
- The command palette, tab switching and numbered tab access work within the current connection workspace only.
- Split panes are per-tab state and are discarded when the tab closes.
