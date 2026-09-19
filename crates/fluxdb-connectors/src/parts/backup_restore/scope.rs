// 备份范围解析（设计文档 §15.1）：执行前把动态 All 范围解析成一次固定对象快照。
// 快照同时用于外部工具参数和 BackupManifest.objects，保证“执行时重新读取”
// 不会破坏还原计划的指纹与文件元数据。

/// 一次执行实际使用的对象快照。
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedBackupScope {
    /// 工具参数与决策键（MySQL 裸名 / PG `schema.name`；视图也在其中）。
    pub(crate) keys: Vec<String>,
    /// 结构化对象身份，写入 manifest 前转 key。
    pub(crate) objects: Vec<BackupObjectRef>,
}

/// 把请求里的 scope 解析成固定快照；空选择在这里被拒绝，绝不按整库兜底。
pub(crate) fn resolve_backup_scope(request: &BackupRequest) -> fluxdb_core::Result<ResolvedBackupScope> {
    match &request.scope {
        BackupScope::SqliteSnapshot => Ok(ResolvedBackupScope::default()),
        BackupScope::Objects(objects) => {
            if objects.is_empty() {
                return Err(task_error("请至少选择一个备份对象"));
            }
            let mut resolved = ResolvedBackupScope::default();
            for object in objects {
                resolved.keys.push(object.key());
                resolved.objects.push(object.clone());
            }
            Ok(resolved)
        }
        BackupScope::All { include_views } => enumerate_backup_objects(request, *include_views),
    }
}

/// 枚举当前库的全部用户表（及视图）。PG 遍历 schema，对象带 schema 身份；
/// 权限不足/连接失败直接报错，不能显示为 0 个对象。
fn enumerate_backup_objects(
    request: &BackupRequest,
    include_views: bool,
) -> fluxdb_core::Result<ResolvedBackupScope> {
    let connector = connector_for(&request.config)?;
    let root = ObjectPath {
        connection_id: request.config.id,
        database: Some(request.database.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let summaries = if request.config.kind == DatabaseKind::Postgres {
        let database = ObjectPath { kind: ObjectKind::Database, ..root.clone() };
        let mut all = Vec::new();
        for schema in connector.list_objects(Some(&database))? {
            all.extend(connector.list_objects(Some(&schema.path))?);
        }
        all
    } else {
        connector.list_objects(Some(&root))?
    };
    let mut resolved = ResolvedBackupScope::default();
    for summary in summaries {
        let kind = match summary.path.kind {
            ObjectKind::Table => BackupObjectKind::Table,
            ObjectKind::View if include_views => BackupObjectKind::View,
            _ => continue,
        };
        // PG 对象必须保留 schema 身份，区分 public.orders 与 archive.orders。
        let schema = if request.config.kind == DatabaseKind::Postgres {
            Some(summary.path.schema.clone().unwrap_or_else(|| "public".into()))
        } else {
            None
        };
        let object = BackupObjectRef { schema, kind, name: summary.path.name.clone() };
        resolved.keys.push(object.key());
        resolved.objects.push(object);
    }
    if resolved.objects.is_empty() {
        return Err(task_error(if include_views {
            "数据库中没有可备份的表或视图"
        } else {
            "数据库中没有可备份的用户表"
        }));
    }
    Ok(resolved)
}
