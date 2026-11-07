// 显式 @/附件引用：授权不扩大写权限，文件只读句柄、不接受模型任意路径。
#[derive(Clone, Debug, PartialEq)]
pub struct AiContextReference {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub source: String,
    pub summary: String,
}
impl AiContextReference {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"label":self.label,"kind":self.kind,"source":self.source,"summary":self.summary})
    }
    pub(crate) fn parse(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_str()?.into(),
            label: value["label"].as_str()?.into(),
            kind: value["kind"].as_str()?.into(),
            source: value["source"].as_str()?.into(),
            summary: value["summary"].as_str()?.into(),
        })
    }
}
/// 路径来自用户文件选择器；敏感目录、非文本文件与 symlink 不授权。
pub fn authorize_ai_file(path:&std::path::Path,storage_root:&std::path::Path)->fluxdb_core::Result<AiContextReference> {
    let fail=|reason:&str|fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal,reason);
    if std::fs::symlink_metadata(path).map(|m|m.file_type().is_symlink()).unwrap_or(false) {return Err(fail("不允许链接附件"));}
    let path=path.canonicalize().map_err(|_|fail("附件不存在或无法读取"))?;
    let root=storage_root.canonicalize().unwrap_or_else(|_|storage_root.to_path_buf());
    if path.starts_with(root) || path.components().any(|component| {
        let name=component.as_os_str().to_string_lossy().to_lowercase();
        [".ssh",".aws",".codex",".env","config.toml"].contains(&name.as_str()) || name.contains("credential") || name.contains("secret")
    }) {return Err(fail("配置、凭据和私钥不能作为附件"));}
    let ext=path.extension().and_then(|e|e.to_str()).unwrap_or("").to_lowercase();
    if !["sql","txt","csv","tsv","json","md","log","yaml","yml"].contains(&ext.as_str()) {return Err(fail("仅支持文本附件，不能读取数据库、二进制或密钥文件"));}
    let file=open_ai_authorized_file(&path)?;
    let meta=file.metadata().map_err(|_|fail("无法读取附件信息"))?;
    if !meta.is_file() || meta.len()>16*1024*1024 {return Err(fail("附件必须是文件且不超过 16 MiB"));}
    let label=path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    // 头部取样判定内容类型：二进制附件也要授权并给出类型/大小，读取时才按类型反馈而不是一律当文本硬解。
    let (content_summary,binary)=ai_file_content_summary(open_ai_authorized_file(&path)?,&label)?;
    let mut reference=AiContextReference{id:uuid::Uuid::new_v4().to_string(),label,kind:"file".into(),source:path.to_string_lossy().into_owned(),summary:format!("{} · {} 字节；{}",content_summary,meta.len(),if binary{"仅返回类型与预览"}else{"按需分段读取"})};
    // source 只供宿主：路径、指纹与是否为二进制都只留在宿主侧，运行快照不外泄路径。
    reference.source=serde_json::json!({"path":reference.source,"size":meta.len(),"modified":meta.modified().ok().and_then(|v|v.duration_since(std::time::UNIX_EPOCH).ok()).map(|v|v.as_nanos().to_string()),"binary":binary}).to_string();
    Ok(reference)
}
/// 引用定位解析与身份校验共用同一路径，避免公开上下文和实际执行目标不一致。
fn ai_reference_target(reference: &AiContextReference) -> fluxdb_core::Result<Option<TargetRef>> {
    let invalid = || {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "数据库引用已失效，请移除后重新选择",
        )
    };
    let target = if reference.kind == "connection" {
        let id = reference.source.parse::<u64>().map_err(|_| invalid())?;
        if reference.id != format!("connection:{id}") {
            return Err(invalid());
        }
        TargetRef {
            connection_id: Some(i64::try_from(id).map_err(|_| invalid())?),
            ..Default::default()
        }
    } else if matches!(
        reference.kind.as_str(),
        "database"
            | "schema"
            | "table"
            | "view"
            | "procedure"
            | "function"
            | "column"
            | "index"
            | "collection"
            | "redis_db"
            | "redis_key"
    ) {
        let path: fluxdb_core::ObjectPath =
            serde_json::from_str(&reference.source).map_err(|_| invalid())?;
        let expected_kind = match path.kind {
            fluxdb_core::ObjectKind::Database => "database",
            fluxdb_core::ObjectKind::Schema => "schema",
            fluxdb_core::ObjectKind::Table => "table",
            fluxdb_core::ObjectKind::View => "view",
            fluxdb_core::ObjectKind::Procedure => "procedure",
            fluxdb_core::ObjectKind::Function => "function",
            fluxdb_core::ObjectKind::Column => "column",
            fluxdb_core::ObjectKind::Index => "index",
            fluxdb_core::ObjectKind::Collection => "collection",
            fluxdb_core::ObjectKind::RedisDb => "redis_db",
            fluxdb_core::ObjectKind::RedisKey => "redis_key",
        };
        if reference.kind != expected_kind
            || reference.id != format!("object:{}", source_hash(&[&reference.source]))
        {
            return Err(invalid());
        }
        let database = path.database.clone().or_else(|| {
            (path.kind == fluxdb_core::ObjectKind::Database).then(|| path.name.clone())
        });
        let schema = path
            .schema
            .clone()
            .or_else(|| (path.kind == fluxdb_core::ObjectKind::Schema).then(|| path.name.clone()));
        let redis_db = if matches!(
            path.kind,
            fluxdb_core::ObjectKind::RedisDb | fluxdb_core::ObjectKind::RedisKey
        ) {
            Some(
                database
                    .as_deref()
                    .unwrap_or(&path.name)
                    .parse::<u32>()
                    .map_err(|_| invalid())?,
            )
        } else {
            None
        };
        TargetRef {
            connection_id: Some(i64::try_from(path.connection_id.0).map_err(|_| invalid())?),
            database,
            schema,
            redis_db,
            object: (!matches!(
                path.kind,
                fluxdb_core::ObjectKind::Database
                    | fluxdb_core::ObjectKind::Schema
                    | fluxdb_core::ObjectKind::RedisDb
            ))
            .then_some(path.name),
        }
    } else if reference.kind == "saved_query" {
        let value: serde_json::Value =
            serde_json::from_str(&reference.source).map_err(|_| invalid())?;
        if reference.id != format!("saved_query:{}", source_hash(&[&reference.source])) {
            return Err(invalid());
        }
        let connection_id = value["connection_id"].as_u64().ok_or_else(invalid)?;
        TargetRef {
            connection_id: Some(i64::try_from(connection_id).map_err(|_| invalid())?),
            database: value["database"].as_str().map(str::to_owned),
            schema: value["schema"].as_str().map(str::to_owned),
            ..Default::default()
        }
    } else if matches!(reference.kind.as_str(), "selection" | "result_set") {
        let value: serde_json::Value =
            serde_json::from_str(&reference.source).map_err(|_| invalid())?;
        if reference.id != format!("{}:{}", reference.kind, source_hash(&[&reference.source])) {
            return Err(invalid());
        }
        let target = &value["target"];
        if !target.is_object()
            || (!target["connection_id"].is_null() && target["connection_id"].as_i64().is_none())
            || ["database", "schema", "object"]
                .iter()
                .any(|key| !target[*key].is_null() && !target[*key].is_string())
            || (!target["redis_db"].is_null()
                && target["redis_db"]
                    .as_u64()
                    .and_then(|db| u32::try_from(db).ok())
                    .is_none())
        {
            return Err(invalid());
        }
        TargetRef {
            connection_id: target["connection_id"].as_i64(),
            database: target["database"].as_str().map(str::to_owned),
            schema: target["schema"].as_str().map(str::to_owned),
            object: target["object"].as_str().map(str::to_owned),
            redis_db: target["redis_db"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok()),
        }
    } else {
        return Ok(None);
    };
    Ok((!target.is_empty()).then_some(target))
}
impl AppController {
    /// 显式数据库引用决定本轮目标；文件引用不改变目标，也不授予写权限。
    fn referenced_execution_target(
        &self,
        references: &[AiContextReference],
    ) -> fluxdb_core::Result<Option<TargetRef>> {
        let invalid = || {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Internal,
                "数据库引用已失效，请移除后重新选择",
            )
        };
        let mut targets = Vec::new();
        for reference in references {
            let Some(target) = ai_reference_target(reference)? else {
                continue;
            };
            if !self.state.connections.iter().any(|connection| {
                Some(connection.config.id.0)
                    == target.connection_id.and_then(|id| u64::try_from(id).ok())
            }) {
                return Err(invalid());
            }
            targets.push(target);
        }
        if targets.is_empty() {
            return Ok(None);
        }
        // 不同非空定位值不能合并，否则会悄悄选错连接或扩大数据库范围。
        let merge = |values: Vec<String>| -> fluxdb_core::Result<Option<String>> {
            let values = values
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            if values.len() > 1 {
                return Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Internal,
                    "本轮引用跨多个连接、数据库或 schema；请保留同一执行目标的引用后发送",
                ));
            }
            Ok(values.into_iter().next())
        };
        let connection_id = merge(
            targets
                .iter()
                .filter_map(|target| target.connection_id.map(|id| id.to_string()))
                .collect(),
        )?
        .and_then(|id| id.parse().ok());
        let database = merge(
            targets
                .iter()
                .filter_map(|target| target.database.clone())
                .collect(),
        )?;
        let schema = merge(
            targets
                .iter()
                .filter_map(|target| target.schema.clone())
                .collect(),
        )?;
        let objects = targets
            .iter()
            .filter_map(|target| target.object.clone())
            .collect::<std::collections::BTreeSet<_>>();
        Ok(Some(TargetRef {
            connection_id,
            database,
            schema,
            redis_db: targets.iter().find_map(|target| target.redis_db),
            object: (objects.len() == 1)
                .then(|| objects.into_iter().next())
                .flatten(),
        }))
    }
}

/// `@` 候选上限：浮层只渲染有限列表，关键字越具体结果越准确。
pub const AI_REFERENCE_CANDIDATE_LIMIT: usize = 20;

fn ai_database_kind_label(kind: fluxdb_core::DatabaseKind) -> &'static str {
    match kind {
        fluxdb_core::DatabaseKind::MySql => "MySQL",
        fluxdb_core::DatabaseKind::TiDb => "TiDB",
        fluxdb_core::DatabaseKind::MariaDb => "MariaDB",
        fluxdb_core::DatabaseKind::Sqlite => "SQLite",
        fluxdb_core::DatabaseKind::MongoDb => "MongoDB",
        fluxdb_core::DatabaseKind::Redis => "Redis",
        fluxdb_core::DatabaseKind::Postgres => "PostgreSQL",
        fluxdb_core::DatabaseKind::Kingbase => "人大金仓 KingbaseES",
        fluxdb_core::DatabaseKind::Dameng => "达梦 DM8",
        fluxdb_core::DatabaseKind::SqlServer => "SQL Server",
        fluxdb_core::DatabaseKind::Oracle => "Oracle",
    }
}

fn ai_object_kind_label(kind: fluxdb_core::ObjectKind) -> &'static str {
    match kind {
        fluxdb_core::ObjectKind::Database => "数据库",
        fluxdb_core::ObjectKind::Schema => "Schema",
        fluxdb_core::ObjectKind::Table => "表",
        fluxdb_core::ObjectKind::View => "视图",
        fluxdb_core::ObjectKind::Procedure => "存储过程",
        fluxdb_core::ObjectKind::Function => "函数",
        fluxdb_core::ObjectKind::Column => "列",
        fluxdb_core::ObjectKind::Index => "索引",
        fluxdb_core::ObjectKind::Collection => "集合",
        fluxdb_core::ObjectKind::RedisDb => "Redis DB",
        fluxdb_core::ObjectKind::RedisKey => "Redis Key",
    }
}

impl AppController {
    /// `@` 候选检索：连接、库/schema、表/视图、保存查询、选区、结果集、已授权附件。
    ///
    /// 只返回**摘要元数据**（稳定 ID、标签、来源、简述）；对象正文/文件内容
    /// 一律不在这里展开，详情走受控工具按需读取。查询为空时返回各类前若干条。
    pub fn ai_reference_candidates(
        &self,
        query: &str,
        ui_context: Option<&UiContextSnapshot>,
    ) -> fluxdb_core::Result<Vec<AiContextReference>> {
        let query = query.trim_start_matches('@').to_lowercase();
        let mut candidates = Vec::new();
        for connection in &self.state.connections {
            candidates.push(AiContextReference {
                id: format!("connection:{}", connection.config.id.0),
                label: connection.config.name.clone(),
                kind: "connection".into(),
                source: connection.config.id.0.to_string(),
                summary: format!(
                    "{} 连接 · 只读上下文，不授予写权限",
                    ai_database_kind_label(connection.config.kind)
                ),
            });
            for object in &connection.objects {
                let kind = match object.path.kind {
                    fluxdb_core::ObjectKind::Database => "database",
                    fluxdb_core::ObjectKind::Schema => "schema",
                    fluxdb_core::ObjectKind::Table => "table",
                    fluxdb_core::ObjectKind::View => "view",
                    fluxdb_core::ObjectKind::Procedure => "procedure",
                    fluxdb_core::ObjectKind::Function => "function",
                    fluxdb_core::ObjectKind::RedisDb => "redis_db",
                    fluxdb_core::ObjectKind::RedisKey => "redis_key",
                    fluxdb_core::ObjectKind::Collection => "collection",
                    fluxdb_core::ObjectKind::Column => "column",
                    fluxdb_core::ObjectKind::Index => "index",
                };
                let source = serde_json::to_string(&object.path).unwrap_or_default();
                // 同名对象靠 连接 + 库 + schema + 名称 全路径区分，不靠裸名。
                candidates.push(AiContextReference {
                    id: format!("object:{}", source_hash(&[&source])),
                    label: object.path.name.clone(),
                    kind: kind.into(),
                    source,
                    summary: {
                        let mut location = connection.config.name.clone();
                        if let Some(database) = object.path.database.as_deref() {
                            location.push_str(" / ");
                            location.push_str(database);
                        }
                        if let Some(schema) = object.path.schema.as_deref() {
                            location.push_str(" / ");
                            location.push_str(schema);
                        }
                        let mut summary = format!(
                            "{} · {} / {}",
                            ai_object_kind_label(object.path.kind),
                            location,
                            object.path.name
                        );
                        if let Some(rows) = object.rows {
                            summary.push_str(&format!(" · {rows} 行"));
                        }
                        if let Some(comment) = object.comment.as_deref().filter(|v| !v.is_empty()) {
                            summary.push_str(&format!(" · {comment}"));
                        }
                        summary
                    },
                });
            }
        }

        let storage = self.ai_storage()?;
        for saved in storage.load_saved_queries()? {
            let source = serde_json::json!({
                "query_id": saved.id, "connection_id": saved.connection_id.0,
                "database": saved.database, "schema": saved.schema,
                "fingerprint": source_hash(&[&saved.text]),
            })
            .to_string();
            let connection = self
                .state
                .connections
                .iter()
                .find(|c| c.config.id == saved.connection_id)
                .map(|c| c.config.name.as_str())
                .unwrap_or("连接已移除");
            candidates.push(AiContextReference {
                id: format!("saved_query:{}", source_hash(&[&source])),
                label: saved.name,
                kind: "saved_query".into(),
                source,
                summary: format!(
                    "保存查询 · {} / {} / {} · {} 字符",
                    connection,
                    saved.database.as_deref().unwrap_or("默认库"),
                    saved.schema.as_deref().unwrap_or("默认 Schema"),
                    saved.text.chars().count()
                ),
            });
        }
        if let Some(context) = ui_context {
            if let Some(selection) = context
                .selected_text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                let excerpt = reference_excerpt(selection);
                let source = serde_json::json!({"target": serde_json::from_str::<serde_json::Value>(&context.target.to_json()).map_err(reference_json_error)?,
                    "tab_id": context.tab_id, "text": excerpt.text, "truncated": excerpt.truncated,
                    "fingerprint": source_hash(&[selection])}).to_string();
                candidates.push(AiContextReference {
                    id: format!("selection:{}", source_hash(&[&source])),
                    label: "当前选区".into(),
                    kind: "selection".into(),
                    source,
                    summary: format!(
                        "编辑器选区 · {} 字符 · 详情按需读取",
                        selection.chars().count()
                    ),
                });
            }
            if let Some(tab) = context
                .tab_id
                .and_then(|id| self.state.tabs.iter().find(|tab| tab.id.0 == id))
            {
                let pages: Vec<(&fluxdb_core::DataPage, usize)> = match &tab.kind {
                    TabKind::DataEditor(editor) if !editor.loading => {
                        editor.page.iter().map(|page| (page, 0)).collect()
                    }
                    TabKind::QueryEditor(editor) if !editor.running => editor
                        .results
                        .iter()
                        .enumerate()
                        .map(|(index, page)| (page, index))
                        .collect(),
                    _ => Vec::new(),
                };
                for (page, index) in pages {
                    let (page_text, page_truncated) = reference_page_excerpt(page);
                    let excerpt = reference_excerpt(&page_text);
                    let source = serde_json::json!({"target": serde_json::from_str::<serde_json::Value>(&context.target.to_json()).map_err(reference_json_error)?,
                        "tab_id": tab.id.0, "index": index, "text": excerpt.text, "truncated": excerpt.truncated || page_truncated,
                        "fingerprint": source_hash(&[&page_text])}).to_string();
                    candidates.push(AiContextReference {
                        id: format!("result_set:{}", source_hash(&[&source])),
                        label: format!("当前结果集 {}", index + 1),
                        kind: "result_set".into(),
                        source,
                        summary: format!(
                            "已加载页快照 · {} 列 / {} 行 · offset {} · {}",
                            page.columns.len(),
                            page.rows.len(),
                            page.offset,
                            if excerpt.truncated || page.has_more {
                                "有截断/后续页"
                            } else {
                                "完整当前页"
                            }
                        ),
                    });
                }
            }
        }

        if with_ai_storage(&storage, ai_files_enabled)? {
            candidates.extend(
                self.state
                    .ai_conversation
                    .context_references
                    .iter()
                    .filter(|reference| reference.kind == "file")
                    .cloned(),
            );
        }

        let candidates = candidates
            .into_iter()
            .filter(|reference| {
                format!(
                    "{} {} {}",
                    reference.label, reference.kind, reference.summary
                )
                .to_lowercase()
                .contains(&query)
            })
            .take(AI_REFERENCE_CANDIDATE_LIMIT)
            .collect();
        Ok(candidates)
    }

    /// 只做内存身份/连接检查。未加载元数据不等于对象不存在，不在每帧触发 I/O。
    pub fn ai_reference_is_valid(&self, reference: &AiContextReference) -> bool {
        match ai_reference_target(reference) {
            Err(_) => false,
            Ok(None) => true,
            Ok(Some(target)) => target.connection_id.is_none_or(|id| {
                self.state
                    .connections
                    .iter()
                    .any(|connection| i64::try_from(connection.config.id.0).ok() == Some(id))
            }),
        }
    }
}

/// 从根目录逐级安全打开：任一中间目录或末端文件被换成链接即拒绝，避免授权后路径被改指到凭据文件。
fn open_ai_authorized_file(path:&std::path::Path)->fluxdb_core::Result<std::fs::File> {
    let fail=|e:std::io::Error|fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal,format!("附件安全打开失败：{e}"));
    #[cfg(unix)] {
        use std::os::{fd::{AsRawFd,FromRawFd},unix::ffi::OsStrExt};
        let mut dir=std::fs::File::open("/").map_err(fail)?;
        let parts=path.components().filter_map(|part|if let std::path::Component::Normal(v)=part{Some(v)}else{None}).collect::<Vec<_>>();
        if !path.is_absolute() || parts.is_empty() {return Err(fail(std::io::Error::other("需要绝对文件路径")));}
        for (index,part) in parts.iter().enumerate() {
            let name=std::ffi::CString::new(part.as_bytes()).map_err(|_|fail(std::io::Error::other("路径含空字符")))?;
            let mut flags=libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_CLOEXEC;
            if index+1<parts.len(){flags|=libc::O_DIRECTORY;}
            // 持有父目录 fd：并发改名/替换 symlink 不能把授权路径转向另一个目录。
            let fd=unsafe{libc::openat(dir.as_raw_fd(),name.as_ptr(),flags)};
            if fd<0 {return Err(fail(std::io::Error::last_os_error()));}
            dir=unsafe{std::fs::File::from_raw_fd(fd)};
        }
        Ok(dir)
    }
    #[cfg(windows)] {
        use std::os::windows::{ffi::OsStrExt,io::FromRawHandle};
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{GENERIC_READ,HANDLE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW,GetFileInformationByHandle,GetFinalPathNameByHandleW,BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_DIRECTORY,FILE_ATTRIBUTE_REPARSE_POINT,FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT,FILE_SHARE_DELETE,FILE_SHARE_READ,FILE_SHARE_WRITE,OPEN_EXISTING,VOLUME_NAME_DOS,
        };
        let fail_msg=|reason:&str|fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal,format!("附件安全打开失败：{reason}"));
        // HANDLE 无 Drop：所有放弃句柄的路径都必须经此转成 File 才会关闭。
        let into_file=|handle:HANDLE|unsafe{std::fs::File::from_raw_handle(handle.0 as *mut std::ffi::c_void)};
        let part=|part:&std::ffi::OsStr|->fluxdb_core::Result<Vec<u16>> {
            let wide=part.encode_wide().collect::<Vec<_>>();
            // 路径含内嵌空字符会被 CreateFileW 截断成授权点之前的部分，必须拒绝。
            if wide.contains(&0) {return Err(fail(std::io::Error::other("路径含空字符")));}
            Ok(wide)
        };
        // OPEN_REPARSE_POINT 只禁止跟随、不禁止打开：必须再按句柄属性显式拒绝 reparse point。
        // 与 Unix 逐级 O_NOFOLLOW 同构——每一级都是「打开后确认不是链接」，而非依赖跟随语义。
        let open=|path:&[u16],directory:bool,readable:bool|->fluxdb_core::Result<HANDLE> {
            let mut flags=FILE_FLAG_OPEN_REPARSE_POINT;
            if directory {flags|=FILE_FLAG_BACKUP_SEMANTICS|FILE_ATTRIBUTE_DIRECTORY;}
            let mut name=path.to_vec();
            name.push(0);
            let handle=unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    // 只有末端需要读内容；中间目录要 GENERIC_READ 会因目录 ACL 被拒。
                    if readable {GENERIC_READ.0} else {0},
                    FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,
                    None,
                    OPEN_EXISTING,
                    flags,
                    None,
                )
            }
            .map_err(|error| fail(std::io::Error::other(error.to_string())))?;
            let mut info=BY_HANDLE_FILE_INFORMATION::default();
            let queried=unsafe{GetFileInformationByHandle(handle,&mut info)}.is_ok();
            let is_reparse=info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
            let is_directory=info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
            let verdict=(!queried)
                .then_some("无法读取句柄信息")
                .or(is_reparse.then_some("不允许链接附件"))
                // 目录位校验：任何一级都不是预期类型即拒绝（如把文件当中间目录）。
                .or((is_directory!=directory).then_some("附件路径层级不正确"));
            if let Some(reason)=verdict {
                // 校验失败不能泄漏句柄：HANDLE 无 Drop，必须显式关闭。
                drop(into_file(handle));
                return Err(fail_msg(reason));
            }
            Ok(handle)
        };
        let parts=path.components().filter_map(|component|if let std::path::Component::Normal(value)=component{Some(value)}else{None}).collect::<Vec<_>>();
        if !path.is_absolute() || parts.is_empty() {return Err(fail(std::io::Error::other("需要绝对文件路径")));}
        // 逐级下钻并逐级校验祖先不是 reparse point：授权后目录被换成 junction 时后续解析不会落到别处。
        let mut accumulated=part(path.components().next().expect("已确认是绝对路径").as_os_str())?;
        let mut current=open(&accumulated,true,false)?;
        let mut error=None;
        let total=parts.len();
        for (index,name) in parts.iter().enumerate() {
            let name=match part(name) {
                Ok(name)=>name,
                Err(err)=>{error=Some(err);break;}
            };
            accumulated.push(u16::from(b'\\'));
            accumulated.extend_from_slice(&name);
            // 中间目录需要 BACKUP_SEMANTICS，且只申请 0 访问权：目录 ACL 通常不含 GENERIC_READ。
            let directory=index+1<total;
            let next=open(&accumulated,directory,!directory);
            drop(into_file(current));
            match next {
                Ok(handle)=>current=handle,
                Err(err)=>{error=Some(err);break;}
            }
        }
        if let Some(err)=error {return Err(err);}
        // Win32 没有 openat，下一级 CreateFileW 只接受路径字符串、无法接收父句柄，
        // 因此除逐级校验外再核对末端句柄真实落点，覆盖下钻途中目录被换成 junction 的竞态。
        let mut buffer=vec![0u16;1024];
        // 用与 std `canonicalize` 相同的 VOLUME_NAME_DOS 形式，保证与授权路径同形可比。
        let length=unsafe{GetFinalPathNameByHandleW(current,&mut buffer,VOLUME_NAME_DOS)};
        if length==0 || length as usize>=buffer.len() {
            drop(into_file(current));
            return Err(fail_msg("无法解析附件真实路径"));
        }
        // 授权路径来自 `canonicalize`，Windows 上本身就是 `\\?\C:\…` 形式，与本返回值同形，可直接比对；
        // 大小写按不敏感处理（盘符与短名可能被系统规范化成不同大小写）。
        let resolved=&buffer[..length as usize];
        let requested=path.as_os_str().encode_wide().collect::<Vec<_>>();
        // 只有 ASCII 区间有大小写变体；其余（含中文文件名）按原值比对。
        let fold=|value:u16|if (0x41..=0x5A).contains(&value){value+0x20}else{value};
        let same=resolved.len()==requested.len() && resolved.iter().zip(&requested).all(|(left,right)|fold(*left)==fold(*right));
        if !same {
            drop(into_file(current));
            return Err(fail_msg("附件路径已指向其它位置"));
        }
        // 末端已确认是普通文件且以 GENERIC_READ 打开：交给上层按 offset/limit 读取。
        Ok(into_file(current))
    }
    #[cfg(not(any(unix, windows)))] {
        let _=path;
        Err(fail(std::io::Error::other("此平台暂未实现防目录链接替换的安全文件句柄，附件读取不可用")))
    }
}
