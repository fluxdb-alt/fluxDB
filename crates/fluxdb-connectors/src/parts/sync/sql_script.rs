/// 把数据同步实际执行的修改渲染成可重放的 SQL 字面量脚本。
///
/// 只接受完整值：预览/截断/内容引用值直接报错，绝不把半截数据写进脚本。
/// 标识符按目标方言引用，值和标识符都不经过任何运行时参数绑定——脚本本身就是
/// 独立可执行文本。
fn script_literal(kind: DatabaseKind, value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(value) => {
            // SQL Server / Oracle / 达梦没有布尔字面量；统一按 0/1 写。
            if matches!(
                kind,
                DatabaseKind::SqlServer | DatabaseKind::Oracle | DatabaseKind::Dameng
            ) {
                if *value { "1" } else { "0" }.to_string()
            } else {
                if *value { "TRUE" } else { "FALSE" }.to_string()
            }
        }
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => {
            if !value.is_finite() {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "迁移脚本无法表示非有限浮点数",
                ));
            }
            value.to_string()
        }
        CellValue::Text(value) | CellValue::Json(value) => script_text_literal(kind, value)?,
        CellValue::Bytes(value) => script_bytes_literal(kind, value),
        // 预览值不是权威数据：写成字面量会在重放时覆盖或丢弃真实内容。
        CellValue::TruncatedText { .. }
        | CellValue::LargeValue(_)
        | CellValue::BinarySummary(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "导出迁移脚本需要完整字段值，请先加载完整的大字段后重试",
            ));
        }
    })
}

fn script_text_literal(kind: DatabaseKind, value: &str) -> fluxdb_core::Result<String> {
    if value.contains('\0') {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "迁移脚本无法表示包含 NUL 字符的文本",
        ));
    }
    // MySQL 默认把反斜杠当转义符；其余方言按标准 SQL 只把单引号翻倍。
    let escaped = if matches!(
        kind,
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb
    ) {
        value.replace('\\', "\\\\").replace('\'', "''")
    } else {
        value.replace('\'', "''")
    };
    Ok(format!("'{escaped}'"))
}

fn script_bytes_literal(kind: DatabaseKind, value: &[u8]) -> String {
    let hex = value
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match kind {
        DatabaseKind::Postgres | DatabaseKind::Kingbase => format!("'\\x{hex}'::bytea"),
        DatabaseKind::SqlServer => format!("0x{hex}"),
        DatabaseKind::Oracle | DatabaseKind::Dameng => format!("hextoraw('{hex}')"),
        _ => format!("X'{}'", hex.to_uppercase()),
    }
}

/// 渲染一条 UPDATE；`table` 必须是已按方言限定的表名。
pub fn script_update_statement(
    kind: DatabaseKind,
    table: &str,
    update: &RowUpdate,
) -> fluxdb_core::Result<String> {
    if update.cells.is_empty() {
        return Err(Error::new(ErrorKind::Query, "更新没有需要写入的字段"));
    }
    let dialect = fluxdb_core::sql_dialect(kind);
    let assignments = update
        .cells
        .iter()
        .map(|cell| {
            // 流式值只有占位符，真实内容不在内存里：导出会把占位符当成数据。
            if cell.value.stream().is_some() {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "导出迁移脚本不支持流式字段值，请先关闭流式传输后重试",
                ));
            }
            Ok(format!(
                "{} = {}",
                dialect.quote_identifier(&cell.column),
                script_literal(kind, cell.value.placeholder())?
            ))
        })
        .collect::<fluxdb_core::Result<Vec<_>>>()?
        .join(", ");
    Ok(format!(
        "UPDATE {table} SET {assignments} WHERE {};",
        script_identity_clause(kind, &update.identity)?
    ))
}

/// 渲染一条 DELETE；`table` 必须是已按方言限定的表名。
pub fn script_delete_statement(
    kind: DatabaseKind,
    table: &str,
    identity: &RowIdentity,
) -> fluxdb_core::Result<String> {
    Ok(format!(
        "DELETE FROM {table} WHERE {};",
        script_identity_clause(kind, identity)?
    ))
}

fn script_identity_clause(
    kind: DatabaseKind,
    identity: &RowIdentity,
) -> fluxdb_core::Result<String> {
    if identity.values.is_empty() {
        return Err(Error::new(
            ErrorKind::Query,
            "缺少行身份条件，无法生成迁移脚本",
        ));
    }
    let dialect = fluxdb_core::sql_dialect(kind);
    identity
        .values
        .iter()
        .map(|(column, value)| {
            Ok(format!(
                "{} = {}",
                dialect.quote_identifier(column),
                script_literal(kind, value)?
            ))
        })
        .collect::<fluxdb_core::Result<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

#[cfg(test)]
mod sql_script_tests {
    use super::*;
    use fluxdb_core::CellUpdate;
    use std::collections::BTreeMap;

    fn identity(column: &str, value: CellValue) -> RowIdentity {
        RowIdentity {
            values: BTreeMap::from([(column.to_string(), value)]),
        }
    }

    #[test]
    fn text_literals_escape_per_dialect() {
        assert_eq!(
            script_literal(DatabaseKind::Sqlite, &CellValue::Text("it's".into())).expect("sqlite"),
            "'it''s'"
        );
        assert_eq!(
            script_literal(DatabaseKind::MySql, &CellValue::Text("a\\b'c".into())).expect("mysql"),
            "'a\\\\b''c'"
        );
        assert!(
            script_literal(DatabaseKind::Postgres, &CellValue::Text("a\0b".into())).is_err(),
            "NUL 无法写成字面量"
        );
    }

    #[test]
    fn bytes_literals_follow_dialect() {
        let bytes = CellValue::Bytes(vec![0xde, 0xad]);
        assert_eq!(
            script_literal(DatabaseKind::Postgres, &bytes).expect("pg"),
            "'\\xdead'::bytea"
        );
        assert_eq!(
            script_literal(DatabaseKind::SqlServer, &bytes).expect("mssql"),
            "0xdead"
        );
        assert_eq!(
            script_literal(DatabaseKind::Oracle, &bytes).expect("oracle"),
            "hextoraw('dead')"
        );
        assert_eq!(
            script_literal(DatabaseKind::Sqlite, &bytes).expect("sqlite"),
            "X'DEAD'"
        );
    }

    #[test]
    fn preview_and_stream_values_are_rejected() {
        assert!(
            script_literal(
                DatabaseKind::Sqlite,
                &CellValue::TruncatedText {
                    preview: "half".into(),
                    total_chars: 100,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn update_and_delete_render_identity() {
        let update = RowUpdate {
            identity: identity("id", CellValue::I64(7)),
            cells: vec![CellUpdate::new("name", CellValue::Text("x'y".into()))],
        };
        assert_eq!(
            script_update_statement(DatabaseKind::Sqlite, "\"items\"", &update).expect("update"),
            "UPDATE \"items\" SET \"name\" = 'x''y' WHERE \"id\" = 7;"
        );
        assert_eq!(
            script_delete_statement(
                DatabaseKind::Sqlite,
                "\"items\"",
                &identity("id", CellValue::I64(7))
            )
            .expect("delete"),
            "DELETE FROM \"items\" WHERE \"id\" = 7;"
        );
    }

    #[test]
    fn empty_identity_and_cells_are_rejected() {
        let update = RowUpdate {
            identity: RowIdentity {
                values: BTreeMap::new(),
            },
            cells: vec![CellUpdate::new("name", CellValue::Text("x".into()))],
        };
        assert!(script_update_statement(DatabaseKind::Sqlite, "items", &update).is_err());
        let update = RowUpdate {
            identity: identity("id", CellValue::I64(1)),
            cells: Vec::new(),
        };
        assert!(script_update_statement(DatabaseKind::Sqlite, "items", &update).is_err());
    }
}
