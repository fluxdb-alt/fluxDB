#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::{Column, SortDirection};
    use sqlx::{ConnectOptions as _, Connection as _, Row as _};
    use std::time::{SystemTime, UNIX_EPOCH};

    include!("tests/helpers.rs");
    include!("tests/system_sqlite.rs");
    include!("tests/system_ast.rs");
    include!("tests/fixture.rs");
    include!("tests/tabs_and_connections.rs");
    include!("tests/table_info_and_sql_format.rs");
    include!("tests/user_admin.rs");
    include!("tests/data_editor.rs");
    include!("tests/sqlite_binary.rs");
    include!("tests/query.rs");
    include!("tests/redis_overview.rs");
    include!("tests/redis_completion.rs");
    include!("tests/terminal_adapter.rs");
    include!("tests/workbench_dispatch.rs");
    include!("tests/workbench_history.rs");
    include!("tests/tree_hover.rs");
}
