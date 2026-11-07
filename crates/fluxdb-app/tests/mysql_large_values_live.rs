//! 真实 MySQL 大字段读取验证。
//!
//! 仅在显式提供 `FLUXDB_MYSQL_SMOKE=host:port:user:password` 时运行；默认忽略。
//! 依赖 `large_fields_test.large_values`（body/doc/payload 三列共 176 MB 超限值）。
//! 断言只看页的形状：大字段必须变成内容引用 + 有界预览，页总驻留远小于源数据；
//! 拿到引用后还要能按块读回真内容，证明「不丢行」不是只留个标签。

use fluxdb_app::CellContentFileStore;
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellContentStore, CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint,
    MysqlBasicOptions, MysqlConnectionProfile, ObjectKind, ObjectPath, SecretRef,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn smoke_config(database: &str) -> ConnectionConfig {
    let raw = std::env::var("FLUXDB_MYSQL_SMOKE").expect("FLUXDB_MYSQL_SMOKE");
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_MYSQL_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("MySQL port");
    ConnectionConfig {
        id: ConnectionId(9_907),
        name: "MySQL large values".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: parts[0].into(),
                port,
                database: database.into(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
                ..fluxdb_core::MysqlBasicOptions::default()
            },
            ..MysqlConnectionProfile::default()
        }),
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
    .mysql_resolved()
}

/// 单元格在页里的实际占用：引用只剩预览，原始 `Text`/`Bytes` 是整份内容。
fn page_bytes(value: &CellValue) -> usize {
    match value {
        CellValue::Text(text) | CellValue::Json(text) => text.len(),
        CellValue::Bytes(bytes) => bytes.len(),
        CellValue::LargeValue(_) | CellValue::BinarySummary(_) => 256,
        _ => 16,
    }
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向带 large_fields_test 的 MySQL"]
fn large_cells_become_bounded_references_and_stay_readable() {
    let config = smoke_config("large_fields_test");
    let dir = std::env::temp_dir().join(format!("fluxdb-large-values-{}", std::process::id()));
    let store = Arc::new(CellContentFileStore::new(&dir));
    fluxdb_core::register_cell_content_store(store.clone());

    let path = ObjectPath {
        connection_id: config.id,
        database: Some("large_fields_test".into()),
        schema: None,
        name: "large_values".into(),
        kind: ObjectKind::Table,
    };
    let page = connector_for(&config)
        .expect("connector")
        .load_data(&path, 0, 100, &[], &[])
        .expect("load_data");

    let stored = page
        .rows
        .iter()
        .flat_map(|row| row.values.iter())
        .map(page_bytes)
        .sum::<usize>();
    let references = page
        .rows
        .iter()
        .flat_map(|row| row.values.iter())
        .filter_map(|value| value.large_value().cloned())
        .collect::<Vec<_>>();

    // 大字段（body/doc）转成内容引用；二进制列在表浏览里本来就只投影摘要，
    // 不进内存，也谈不上「丢行」，所以这里只校验被外置的文本/JSON 列。
    assert_eq!(page.rows.len(), 11, "大字段不能让行消失");
    assert!(page.incomplete.is_none(), "预算内的大字段页不该标不完整");
    assert_eq!(references.len(), 5, "五格超阈值字段都应外置");
    // 源数据约 176 MB，页里只留预览：给足余量仍是数量级差距。
    assert!(
        stored < 1024 * 1024,
        "页驻留 {stored} 字节，大字段没有被换成引用"
    );

    let largest = references
        .iter()
        .max_by_key(|reference| reference.byte_length)
        .expect("largest reference");
    assert!(
        largest.byte_length.unwrap_or(0) > 100 * 1024 * 1024,
        "104 MB 字段的总长度必须如实报告"
    );

    for reference in &references {
        assert!(reference.preview_truncated, "预览必须标明只是片段");
        assert_ne!(reference.kind, fluxdb_core::LargeValueKind::Binary);
        assert!(
            reference.preview.len() <= fluxdb_core::CELL_PREVIEW_CHARS * 4,
            "预览必须有界"
        );
        let meta = store.stat(&reference.content_id).expect("内容引用仍可读");
        assert_eq!(Some(meta.byte_length), reference.byte_length);

        // 分段读：首块长度受限于 CELL_CHUNK_BYTES，末尾越界读只返回剩余字节。
        let head = store
            .read(&reference.content_id, 0, fluxdb_core::CELL_CHUNK_BYTES)
            .expect("读首块");
        assert!(!head.is_empty() && head.len() <= fluxdb_core::CELL_CHUNK_BYTES);
        let tail_offset = meta.byte_length.saturating_sub(16);
        let tail = store
            .read(
                &reference.content_id,
                tail_offset,
                fluxdb_core::CELL_CHUNK_BYTES,
            )
            .expect("读尾块");
        assert_eq!(tail.len() as u64, meta.byte_length - tail_offset);
    }

    store.release_owner(&fluxdb_core::table_content_owner(&path));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 查询执行路径同样要外置：任意 SQL 结果没有主键，超限值只能靠内容引用保留。
///
/// 覆盖流式收页本身：`SELECT body` 会把 176 MB 逐行读进来，若还是 `fetch_all`
/// 先物化再收页，这一步就是用户报的那几个 GB 峰值。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向带 large_fields_test 的 MySQL"]
fn query_result_large_cells_are_spilled_while_streaming() {
    let config = smoke_config("large_fields_test");
    let dir = std::env::temp_dir().join(format!("fluxdb-large-query-{}", std::process::id()));
    let store = Arc::new(CellContentFileStore::new(&dir));
    fluxdb_core::register_cell_content_store(store.clone());

    let result = connector_for(&config)
        .expect("connector")
        .execute(&fluxdb_core::QueryRequest {
            connection_id: config.id,
            database: Some("large_fields_test".into()),
            schema: None,
            session_id: None,
            text: "SELECT id, body FROM large_values ORDER BY id".into(),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                page_size: 100,
                ..fluxdb_core::QueryExecutionOptions::default()
            },
        })
        .expect("execute");
    let page = result.results.first().expect("结果页");
    let references = page
        .rows
        .iter()
        .flat_map(|row| row.values.iter())
        .filter_map(|value| value.large_value().cloned())
        .collect::<Vec<_>>();
    let stored = page
        .rows
        .iter()
        .flat_map(|row| row.values.iter())
        .map(page_bytes)
        .sum::<usize>();
    let readable = references
        .iter()
        .all(|reference| store.stat(&reference.content_id).is_some());

    store.release_owner(&fluxdb_core::result_content_owner(None, config.id));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(page.rows.len(), 11, "查询结果也不能丢行");
    // 只选了 body 一列：7.8 MB、360 KB、52 MB、104 MB 四格超阈值。
    assert_eq!(references.len(), 4, "四格超阈值字段都应外置");
    assert!(stored < 1024 * 1024, "查询页驻留 {stored} 字节");
    assert!(readable, "执行结束前内容引用必须可读");
    assert_eq!(page.next_offset(), 11, "翻页从已收下的行继续");
}

/// 多语句脚本：第一条结果集因预算提前停读后，第二条必须照常返回。
///
/// 流式收页会在未收录行处停下，剩下的行还在驱动缓冲里；若没排空，
/// 下一条语句就会读到上一条的残留行。这里用「先大后小」两语句验证。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向带 large_fields_test 的 MySQL"]
fn script_second_statement_survives_early_stop() {
    let config = smoke_config("large_fields_test");
    let dir = std::env::temp_dir().join(format!("fluxdb-large-script-{}", std::process::id()));
    let store = Arc::new(CellContentFileStore::new(&dir));
    fluxdb_core::register_cell_content_store(store.clone());

    let result = connector_for(&config)
        .expect("connector")
        .execute(&fluxdb_core::QueryRequest {
            connection_id: config.id,
            database: Some("large_fields_test".into()),
            schema: None,
            session_id: None,
            // 第一条 11 行只收 2 行（页容量），流里还剩 9 行没读。
            text: "SELECT id, body FROM large_values ORDER BY id; SELECT 42 AS answer".into(),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                page_size: 2,
                ..fluxdb_core::QueryExecutionOptions::default()
            },
        })
        .expect("execute");

    store.release_owner(&fluxdb_core::result_content_owner(None, config.id));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(result.summaries.len(), 2, "两条语句都要执行");
    assert!(result.summaries.iter().all(|summary| summary.success));
    let second = result.results.get(1).expect("第二条结果页");
    assert_eq!(second.rows.len(), 1);
    assert_eq!(
        second.rows[0].values[0],
        CellValue::I64(42),
        "第二条结果不能错位"
    );
}
