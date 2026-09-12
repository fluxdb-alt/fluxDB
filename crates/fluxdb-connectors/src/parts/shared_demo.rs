fn mysql_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database_error) = &error
        && database_error.code().as_deref() == Some("1045")
    {
        return Error::new(ErrorKind::Authentication, database_error.message());
    }

    Error::new(ErrorKind::Connection, error.to_string())
}

fn sqlite_error(error: sqlx::Error) -> Error {
    Error::new(ErrorKind::Connection, error.to_string())
}

fn mock_objects(connection_id: ConnectionId) -> Vec<ObjectSummary> {
    // demo 元数据集（T081）：4 张关联表，供补全 fixture 覆盖表/列/FK/排序等场景。
    vec![
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Product".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(504),
            modified_at: None,
            comment: Some("Products sold or used in manufacturing.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "ProductCategory".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(4),
            modified_at: None,
            comment: Some("High-level product categories.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Order".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(1200),
            modified_at: None,
            comment: Some("Sales orders referencing products and customers.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Customer".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(88),
            modified_at: None,
            comment: Some("Customers who placed orders.".to_string()),
        },
    ]
}

fn mock_data_page(offset: u64, limit: u64) -> DataPage {
    let pagination = Pagination::new(offset, limit);

    DataPage {
        columns: vec![
            Column {
                name: "id".to_string(),
                type_name: Some("INTEGER".to_string()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "name".to_string(),
                type_name: Some("TEXT".to_string()),
                nullable: false,
                primary_key: false,
                comment: Some("Product display name".to_string()),
            },
        ],
        rows: vec![
            Row {
                values: vec![CellValue::I64(1), CellValue::Text("Road Bike".to_string())],
            },
            Row {
                values: vec![CellValue::I64(2), CellValue::Text("Helmet".to_string())],
            },
        ],
        offset: pagination.offset,
        limit: pagination.limit,
        has_more: false,
    }
}

/// T081 demo 各表的补全列（表名区分，供列/FK/类型排序 fixture 使用）。
fn mock_completion_columns(table: &str) -> Vec<Column> {
    let cols = |pairs: &[(&str, &str, bool, bool, Option<&str>)]| -> Vec<Column> {
        pairs
            .iter()
            .map(|(name, type_name, nullable, primary_key, comment)| Column {
                name: (*name).to_string(),
                type_name: Some((*type_name).to_string()),
                nullable: *nullable,
                primary_key: *primary_key,
                comment: comment.map(str::to_string),
            })
            .collect()
    };
    match table {
        "Product" | "product" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Product display name")),
            ("category_id", "INTEGER", true, false, Some("FK to ProductCategory.id")),
            ("price", "REAL", true, false, None),
            ("active", "INTEGER", true, false, Some("1 if sellable")),
            ("created_at", "TEXT", true, false, Some("ISO-8601 timestamp")),
        ]),
        "ProductCategory" | "productcategory" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Category display name")),
            ("sort_order", "INTEGER", true, false, None),
        ]),
        "Order" | "order" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("product_id", "INTEGER", false, false, Some("FK to Product.id")),
            ("quantity", "INTEGER", true, false, None),
            ("total", "REAL", true, false, None),
            ("customer_id", "INTEGER", false, false, Some("FK to Customer.id")),
            ("created_at", "TEXT", true, false, Some("ISO-8601 timestamp")),
        ]),
        "Customer" | "customer" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, None),
            ("email", "TEXT", true, false, Some("Customer email")),
            ("city", "TEXT", true, false, None),
        ]),
        // 未知表（含大小写变体）兜底：沿用通用 id+name，保证既有断言不破坏。
        _ => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Display name")),
        ]),
    }
}

/// T081 demo 各表外键（P2.13 FK JOIN 建议）。复合外键以同名多条 ForeignKeyInfo 表达，
/// fk_join_completion_items 会按 name 分组生成 `ON a.x = b.x AND a.y = b.y`。
fn mock_completion_foreign_keys(table: &str) -> Vec<ForeignKeyInfo> {
    match table {
        // Product.category_id -> ProductCategory.id
        "Product" | "product" => vec![ForeignKeyInfo {
            name: "fk_product_category".to_string(),
            column: "category_id".to_string(),
            ref_schema: None,
            ref_table: "ProductCategory".to_string(),
            ref_column: "id".to_string(),
        }],
        // Order 的表连接：product_id + customer_id 独立 FK。
        "Order" | "order" => vec![
            ForeignKeyInfo {
                name: "fk_order_product".to_string(),
                column: "product_id".to_string(),
                ref_schema: None,
                ref_table: "Product".to_string(),
                ref_column: "id".to_string(),
            },
            ForeignKeyInfo {
                name: "fk_order_customer".to_string(),
                column: "customer_id".to_string(),
                ref_schema: None,
                ref_table: "Customer".to_string(),
                ref_column: "id".to_string(),
            },
        ],
        _ => Vec::new(),
    }
}
