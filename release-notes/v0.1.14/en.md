## New features

- Added MongoDB support: connection-string import (`mongodb://` / `mongodb+srv://`) fills in hosts, SRV, credentials, default database and auth database, and the tree browses connection → database → collection group → collection/view.
- The MongoDB collection workspace has four tabs: documents, aggregation, indexes and info. The documents tab supports filter, projection, sort, collation, skip, limit, max time and index hint, with card, table and JSON display modes.
- The query panel includes a visual condition builder: it is now an expression tree whose root group switches between AND / OR / NOR, with child groups for `$elemMatch` and `$not + $elemMatch` array scopes, up to 24 conditions in total, and lossless parse/write-back. Field and value cells share completion, and the tree scrolls internally when it overflows. Reverse parsing accepts Compass-style relaxed JSON (unquoted keys, single quotes, comments, trailing commas).
- Filter, projection, sort and aggregation pipelines now have completion: a bracket-and-string scan decides key position, value position and field references; keys complete field paths and query operators, strings complete real values of that field, and value positions complete Extended JSON constructor templates.
- Document writes: create, whole-document replace, duplicate as a new document and single-document delete. All four share the same right-hand drawer and JSON editor, `_id` is immutable, and switching away from an unsaved draft asks for confirmation first.
- Document import accepts JSON arrays, JSON Lines and CSV, sniffed from content rather than extension. CSV scans the whole file per column to infer types, marks mixed columns as `mixed` and re-sniffs per cell, and lets you override each field before importing. Writes run in batches of 1000 with stop support and a message log.
- Document export supports JSON / JSONL / CSV with "all documents" or "current filter" scope, showing a read-only Compass-style query preview for filtered exports. Files are written to `.partial` and renamed only on success, so cancelling or failing leaves no half-written file.
- Collection context menu: pin, copy name, refresh, export, rename, import, drop. Views are read-only and do not render write entries. The database menu gains create-collection, drop-database and a command workbench, and the connection menu gains connection info (topology, version, hosts, visible databases) and a command-line terminal.
- The aggregation pipeline is now a Compass-style card view: pipeline text stays the single source of truth, and adding, removing, moving, disabling or changing a stage operator writes back to that text. The two panes are resizable and the right pane keeps a live preview of the current stage.
- The indexes page is rebuilt after Compass: it shows type, size, usage count, average usage frequency, properties and status, with sortable headers and confirmed index drop. `_id_` has no drop entry.
- Document and JSON views support window-level text selection: double-click selects a word, drag selects across lines, a drag never jumps into another document, and collapsed-row placeholders do not enter the clipboard.
- The database-level command workbench accepts shell statements such as `use` / `show` / `find` / `aggregate` and explicitly rejects write methods. Results render as collapsible cards with document/JSON switching and copy, a draggable persisted split, and row count, elapsed time and truncation state.
- MongoDB is wired into AI page context and controlled execution: only location, query conditions and state are exposed; document values are read on demand through bounded read-only tools.

## Improvements

- Oversized values no longer drop the whole row: when a cell body exceeds the result-page budget it is offloaded to local controlled storage (256 MiB per owner, 1 GiB per process, cleaned up on tab close, paging or disconnect) and only a preview plus full size stays in the result, viewable read-only in 256 KiB chunks. Applies to MySQL, SQLite, PostgreSQL, SQL Server and Dameng DM8.
- Upload replacement for large values now streams in chunks (≥16 MiB uses staging plus append), and large text columns get the same upload entry. Placeholder values are previews only: copy, export, edit and AI context all reject them so truncated content is never written back as a real value.
- The MongoDB document area now uses a virtualized list with whole-page parse caching, building only the cards in view each frame; table and JSON views share the same row virtualization and remeasure after folding or expanding.
- Each connection reuses a MongoClient pool keyed by URI fingerprint, so find / sampling / indexes / collection info / commands share the same connections; the cache is released on disconnect, config edit or connection delete.
- When both MongoDB query option layers are collapsed, a badge shows how many conditions are still active, and expansion state is remembered per tab.
- Filter and sort popovers now anchor to the clicked control, staying beside their trigger after wrapping, scrolling or panel resizing without overflowing the window.
- Execution summaries scroll vertically when taller than the panel, so long batch runs no longer hide rows below the clipped area.

## Fixes

- Fixed PostgreSQL rows containing large values failing to update or delete, and array columns (such as `numeric(12,4)[]`) failing to import due to type incompatibility.
- Fixed tables without a primary key silently reporting zero affected rows when a row identity containing large values could not be determined; the operation is now rejected with an explanation.
- Fixed SQL Server dropping whole rows when a result exceeded the budget; large fields are now offloaded into content references first, and paging SQL no longer references original columns outside the `ROW_NUMBER` subquery.
- Fixed Dameng DM8 truncating result pages by row count only; pages are now collected under a byte budget with oversized cells offloaded.
- Fixed a crash when importing non-ASCII values into MongoDB (date sniffing compares byte prefixes and no longer splits inside a UTF-8 character).
- Fixed newly created MongoDB collections not appearing in the sidebar until a manual refresh.
- Fixed MongoDB CSV import picking the majority type and failing minority rows; mixed columns are now sniffed per cell.
- Fixed creating a JSON key against servers without the RedisJSON module failing with `unknown command`: the command probes for `JSON.SET` first, stores JSON text as a plain string when unavailable, and uses the JSON editor for input.
- Fixed the Redis command signature hint staying visible after all arguments are filled; it now hides once `GET key` is complete.
- Fixed a newly created Redis key opening a separate DataEditor tab; it now stays in its database tab and is selected and scrolled into view after the list refreshes.
- Fixed ghost-button hover text disappearing into the background in the light theme.

## Known limits

- MongoDB has no database-level backup/restore (`mongodump` / `mongorestore`); export is document-level logical export. The performance-metrics entry is disabled and not implemented.
- MongoDB does not support SSH tunnels; TLS options come from the connection string, and the TLS tab has no MongoDB form.
- A single MongoDB query returns at most 500 documents; export pages by `skip`, so concurrent writes during an export may skip or repeat documents.
- MongoDB does not support bulk delete (deletes re-query immediately with no undo), cross-database collection moves, view rename/drop, creation of time-series or capped collections, in-table row editing, transactions or change streams.
- MongoDB CSV import supports scalar fields only (use the JSON path for nested objects and arrays) and the file must be UTF-8; new documents always get an ObjectId `_id`.

## Validation

- Passed `cargo fmt --all -- --check`, `cargo check --workspace --locked` and `cargo clippy --workspace --all-targets --locked`.
- Passed unit tests for MongoDB URI parsing and profile round-trips, import format sniffing and type inference, Extended JSON and redacted error text, and collection refresh replacement semantics.
- Passed headless E2E for filter/sort popover anchoring, execution-summary scrolling, query panel expansion state and the condition builder.
- MongoDB live tests live in `crates/fluxdb-connectors/tests/mongo_live.rs` and `apps/fluxdb-desktop/src/main_parts/mongo_sidebar_tests.rs`; they require `FLUXDB_MONGO_URI` and skip automatically when it is unset, so they are outside the automated gate.
