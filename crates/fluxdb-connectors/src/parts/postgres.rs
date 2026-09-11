// Included in crate-root scope by ../lib.rs; PostgreSQL connector code is split by responsibility.

include!("postgres/tls.rs");
include!("postgres/proxy.rs");
include!("postgres/connection.rs");
include!("postgres/metadata.rs");
include!("postgres/ddl.rs");
include!("postgres/index_items.rs");
include!("postgres/table_info.rs");
include!("postgres/executor.rs");
include!("postgres/values.rs");
include!("postgres/data.rs");
include!("postgres/apply_changes.rs");
include!("postgres/connector.rs");
