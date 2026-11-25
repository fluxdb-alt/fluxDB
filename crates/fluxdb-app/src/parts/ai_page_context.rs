/// 当前页面是一种自动引用；描述与有界预览可落盘，完整正文只由本轮工具持有。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageContextKind {
    SqlEditor,
    TableData,
    TableStructure,
    RedisKey,
    RedisBrowser,
    RedisWorkbench,
    /// 设置页：只暴露当前分区的非敏感值与可修改字段。
    Settings,
    /// ER 关系图：只提供目标定位、选中表/关系的有界摘要、加载状态与版本。
    ErDiagram,
    /// MongoDB 集合页：只提供定位、查询参数与状态；文档值经 `mongo.read_documents` 按需读取。
    MongoCollection,
    /// 表概览：只提供定位、表数量与本地搜索词；表清单与注释经工具按需查询。
    ObjectList,
    Other,
}

impl PageContextKind {
    fn name(self) -> &'static str {
        match self {
            Self::SqlEditor => "sql_editor",
            Self::TableData => "table_data",
            Self::TableStructure => "table_structure",
            Self::RedisKey => "redis_key",
            Self::RedisBrowser => "redis_browser",
            Self::RedisWorkbench => "redis_workbench",
            Self::Settings => "settings",
            Self::ErDiagram => "er_diagram",
            Self::MongoCollection => "mongo_collection",
            Self::ObjectList => "object_list",
            Self::Other => "unsupported_page",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CurrentPageContext {
    pub kind: PageContextKind,
    pub label: String,
    pub snapshot: UiContextSnapshot,
    /// 结构、定位、状态，不包含连接配置或凭据。
    pub details: serde_json::Value,
    pub editor_grant: Option<EditorGrant>,
    pub data_grant: Option<PageDataGrant>,
    /// MongoDB 集合页的文档快照授权：仅当用户在本轮页面显式共享时可读。
    pub mongo_grant: Option<MongoDocumentsGrant>,
}

impl CurrentPageContext {
    pub fn reference_id(&self) -> String {
        format!("page:{}", self.snapshot.tab_id.unwrap_or(0))
    }

    pub fn public_json(&self) -> serde_json::Value {
        let delivery = if self.kind == PageContextKind::Other {
            "unavailable"
        } else if self.snapshot.selected_text.is_some() {
            "selection"
        } else if self.snapshot.excerpt.as_ref().is_some_and(|e| e.truncated) {
            "excerpt"
        } else if self.snapshot.excerpt.is_some() {
            "full_text"
        } else {
            "structured"
        };
        let mut capabilities = Vec::new();
        if self.editor_grant.is_some() {
            capabilities.extend(["editor.search", "editor.read_range"]);
        }
        if self.editor_grant.as_ref().is_some_and(|grant| grant.sql_edit.is_some()) {
            capabilities.push("editor.apply_edit");
        }
        if self.data_grant.is_some() {
            capabilities.push("page.read_rows");
        }
        if self.mongo_grant.is_some() {
            capabilities.push("mongo.read_documents");
        }
        if self.kind == PageContextKind::Settings {
            capabilities.extend([
                "toolbox.search",
                "toolbox.load",
                "toolbox.inspect",
            ]);
        }
        let mut value = serde_json::json!({
            "kind": "current_page",
            "page_kind": self.kind.name(),
            "id": self.reference_id(),
            "label": self.label,
            "target": serde_json::from_str::<serde_json::Value>(&self.snapshot.target.to_json()).unwrap_or_default(),
            "revision": self.snapshot.tab_revision,
            "delivery": delivery,
            "scope": if self.snapshot.selected_text.is_some() && self.editor_grant.is_some() {
                "full_editor_with_selection_preview"
            } else if self.snapshot.selected_text.is_some() { "selected_text" } else { "current_page" },
            "content": self.snapshot.selected_text.as_ref()
                .or_else(|| self.snapshot.excerpt.as_ref().map(|e| &e.text))
                .or(self.snapshot.summary.as_ref()),
            "truncated": self.snapshot.excerpt.as_ref().is_some_and(|e| e.truncated),
            "details": self.details,
            "capabilities": capabilities,
            "untrusted": true,
            "notes": self.snapshot.notes,
        });
        // 预算针对序列化后的真实体积（控制字符转义也计入），不截坏结构化 JSON。
        if value["content"].to_string().len() > 24 * 1024 {
            if let Some(text) = value["content"].as_str() {
                let prefix = text.chars().take(3000).collect::<String>();
                value["content"] =
                    serde_json::json!(format!("{prefix}\n… [序列化输出超限，正文已截断] …"));
                value["truncated"] = serde_json::json!(true);
                value["delivery"] = serde_json::json!("excerpt");
            }
        }
        let mut omitted = Vec::new();
        for field in [
            "value",
            "indexes",
            "foreign_keys",
            "results",
            "executions",
            "columns",
            "filter_rules",
            "sort_rules",
        ] {
            if value.to_string().len() <= 48 * 1024 {
                break;
            }
            if value["details"]
                .as_object_mut()
                .is_some_and(|details| details.remove(field).is_some())
            {
                omitted.push(field);
            }
        }
        if !omitted.is_empty() {
            value["omitted_details"] = serde_json::json!(omitted);
            value["truncated"] = serde_json::json!(true);
        }
        value
    }
}

/// 从共享 Rope 快照构造首次发送的预览，不在 UI 线程物化整个大文档。
pub fn capture_editor_snapshot(
    tab_id: u64,
    target: TargetRef,
    text: &fluxdb_editor_core::BufferSnapshot,
    selection: fluxdb_editor_core::Range,
) -> UiContextSnapshot {
    let limits = ContextLimits::default();
    let has_selection = selection.start < selection.end;
    let start = text.clamp_to_char_boundary(selection.start.min(text.len()));
    let end = text.clamp_to_char_boundary(selection.end.min(text.len()));
    let (preview, truncated) = if has_selection {
        let end =
            text.clamp_to_char_boundary(end.min(start.saturating_add(limits.max_excerpt_bytes)));
        (
            text.text_in_range(fluxdb_editor_core::Range::new(start, end)),
            end < selection.end,
        )
    } else if text.len() <= limits.max_excerpt_bytes
        && text.line_count() <= limits.max_excerpt_lines
    {
        (text.to_string(), false)
    } else {
        let head_bytes = (limits.max_excerpt_bytes as f64 * limits.head_ratio) as usize;
        let head_rows = (limits.max_excerpt_lines as f64 * limits.head_ratio) as usize;
        let head_line_end = if text.line_count() > head_rows {
            text.line_start(head_rows)
        } else {
            text.len()
        };
        let head_end = text.clamp_to_char_boundary(head_line_end.min(head_bytes));
        let tail_start = text.clamp_to_char_boundary(
            text.line_start(
                text.line_count()
                    .saturating_sub(limits.max_excerpt_lines - head_rows),
            )
            .max(
                text.len()
                    .saturating_sub(limits.max_excerpt_bytes - head_bytes),
            ),
        );
        let head = text.text_in_range(fluxdb_editor_core::Range::new(0, head_end));
        let tail = text.text_in_range(fluxdb_editor_core::Range::new(
            tail_start.max(head_end),
            text.len(),
        ));
        (
            format!(
                "{head}\n… [内容省略；用 editor.search 定位，再用 editor.read_range 读取] …\n{tail}"
            ),
            true,
        )
    };
    UiContextSnapshot {
        tab_id: Some(tab_id),
        tab_revision: text.version(),
        target,
        selected_text: (has_selection && !truncated).then(|| preview.clone()),
        excerpt: (!has_selection || truncated).then_some(BoundedExcerpt {
            text: preview,
            truncated,
            omitted_lines: 0,
            next_offset: 0,
        }),
        statement_at_cursor: None,
        summary: None,
        last_error: None,
        result_ref: None,
        notes: Vec::new(),
    }
}

/// 结构摘要不包含行数据；长名称/注释与过多列均显式截断，可通过元数据工具继续读取。
pub fn page_columns_json(columns: &[fluxdb_core::Column]) -> serde_json::Value {
    serde_json::json!({
        "total":columns.len(),"truncated":columns.len()>32,
        "items":columns.iter().take(32).map(|c|serde_json::json!({
            "name":c.name.chars().take(128).collect::<String>(),
            "type":c.type_name.as_ref().map(|s|s.chars().take(128).collect::<String>()),
            "nullable":c.nullable,"primary_key":c.primary_key,
            "comment":c.comment.as_ref().map(|s|s.chars().take(128).collect::<String>()),
            "text_truncated":c.name.chars().count()>128
                || c.type_name.as_ref().is_some_and(|s|s.chars().count()>128)
                || c.comment.as_ref().is_some_and(|s|s.chars().count()>128)
        })).collect::<Vec<_>>()
    })
}

#[derive(Clone, Debug)]
pub struct PageDataGrant {
    pub reference_id: String,
    pub target: TargetRef,
    pub page: Arc<fluxdb_core::DataPage>,
}

/// MongoDB 文档快照授权：文档值不在页面摘录里，只能经本轮工具按需读取。
#[derive(Clone, Debug)]
pub struct MongoDocumentsGrant {
    pub reference_id: String,
    pub target: TargetRef,
    /// 集合名（`_id` 之外的唯一定位）。
    pub collection: String,
    /// 发送时已加载的文档（`_id` 可读标识 + Extended JSON 文本），不重新查询数据库。
    pub documents: Arc<Vec<(String, String)>>,
    /// 该快照是按什么过滤条件取的；模型据此判断结果是否能代表全集。
    pub filter: String,
}

fn page_data_registry() -> &'static Mutex<HashMap<String, PageDataGrant>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, PageDataGrant>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mongo_documents_registry() -> &'static Mutex<HashMap<String, MongoDocumentsGrant>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, MongoDocumentsGrant>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn take_page_data_grant(turn_id: &str) -> Option<PageDataGrant> {
    page_data_registry().lock().ok()?.remove(turn_id)
}

pub fn take_mongo_documents_grant(turn_id: &str) -> Option<MongoDocumentsGrant> {
    mongo_documents_registry().lock().ok()?.remove(turn_id)
}

pub fn clear_current_page_grants(turn_id: &str) {
    clear_editor_grant(turn_id);
    take_page_data_grant(turn_id);
    take_mongo_documents_grant(turn_id);
}

struct PageReadRowsTool {
    descriptor: fluxdb_core::ToolDescriptor,
    grant: PageDataGrant,
    session_id: String,
    turn_id: String,
}

impl fluxdb_core::AiTool for PageReadRowsTool {
    fn descriptor(&self) -> &fluxdb_core::ToolDescriptor {
        &self.descriptor
    }
    fn execute(
        &self,
        ctx: ToolExecutionContext,
        arguments: String,
    ) -> fluxdb_core::AiFuture<'_, ToolReceipt> {
        Box::pin(async move {
            let now = now_ms();
            let result = (|| -> fluxdb_core::Result<serde_json::Value> {
                if ctx.cancel.is_cancelled() {
                    return Err(editor_error("页面读取已取消"));
                }
                if ctx.session_id != self.session_id || ctx.turn_id != self.turn_id {
                    return Err(editor_error("页面快照不属于本轮任务"));
                }
                let args: serde_json::Value =
                    serde_json::from_str(&arguments).map_err(|_| editor_error("参数无效"))?;
                if args["reference_id"].as_str() != Some(self.grant.reference_id.as_str()) {
                    return Err(editor_error("页面引用未授权"));
                }
                let page = &self.grant.page;
                let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0))
                    .map_err(|_| editor_error("偏移超限"))?;
                if offset > page.rows.len() {
                    return Err(editor_error("偏移超出已加载快照"));
                }
                let limit = args["limit"].as_u64().unwrap_or(10).clamp(1, 100) as usize;
                let column_indices = if let Some(columns) = args["columns"].as_array() {
                    let indices = columns
                        .iter()
                        .map(|c| c.as_u64().and_then(|v| usize::try_from(v).ok()))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| editor_error("列索引无效"))?;
                    if indices.iter().any(|i| *i >= page.columns.len()) {
                        return Err(editor_error("列索引超出快照"));
                    }
                    indices
                } else {
                    (0..page.columns.len().min(32)).collect::<Vec<_>>()
                };
                let mut rows = Vec::new();
                let mut bytes = 0;
                for row in page.rows.iter().skip(offset).take(limit) {
                    let values: Vec<_> = column_indices.iter().filter_map(|i|row.values.get(*i)).map(|cell| match cell {
                        fluxdb_core::CellValue::Bytes(data) => serde_json::json!({"binary_bytes": data.len()}),
                        fluxdb_core::CellValue::BinarySummary(data) => serde_json::json!({"binary_bytes":data.byte_length,"is_null":data.is_null}),
                        fluxdb_core::CellValue::Text(text) | fluxdb_core::CellValue::Json(text) =>
                            serde_json::json!({"text": text.chars().take(1024).collect::<String>(), "truncated": text.chars().take(1025).count() > 1024}),
                        // 数据页里只有预览：明确标出总量，避免 AI 把片段当完整值引用。
                        fluxdb_core::CellValue::TruncatedText { preview, total_chars } =>
                            serde_json::json!({"text": preview.chars().take(1024).collect::<String>(), "truncated": true, "total_chars": total_chars}),
                        value => serde_json::to_value(value).unwrap_or_default(),
                    }).collect();
                    let row = serde_json::json!({"values": values, "columns_truncated": column_indices.len()<row.values.len()});
                    let size = row.to_string().len();
                    if size > 16 * 1024 && rows.is_empty() {
                        return Err(editor_error("单行超过输出预算，请用 columns 指定更少的列"));
                    }
                    if bytes + size > 16 * 1024 {
                        break;
                    }
                    bytes += size;
                    rows.push(row);
                }
                let next = offset + rows.len();
                Ok(serde_json::json!({
                    "reference_id": self.grant.reference_id, "rows": rows,
                    "column_indices":column_indices,
                    "next_offset": next, "has_more": next < page.rows.len(),
                    "total_loaded_rows": page.rows.len(), "database_has_more": page.has_more,
                    "scope": "用户明确开启共享的发送时已加载页快照，不重新查询数据库", "untrusted": true,
                }))
            })();
            let (status, value, code) = match result {
                Ok(value) => (ToolReceiptStatus::Succeeded, value, None),
                Err(error) => (
                    if ctx.cancel.is_cancelled() {
                        ToolReceiptStatus::Cancelled
                    } else {
                        ToolReceiptStatus::Failed
                    },
                    serde_json::json!({"error": error.message}),
                    Some("page_unavailable".into()),
                ),
            };
            Ok(ToolReceipt {
                call_id: ctx.call_id,
                tool_id: self.descriptor.id.clone(),
                status,
                result_json: value.to_string(),
                error_code: code,
                affected_rows: None,
                started_at_ms: now,
                finished_at_ms: now_ms(),
            })
        })
    }
}

/// `mongo.read_documents`：只读当前已加载快照，不重新查询，也不越出授权集合。
struct MongoReadDocumentsTool {
    descriptor: fluxdb_core::ToolDescriptor,
    grant: MongoDocumentsGrant,
    session_id: String,
    turn_id: String,
}

impl fluxdb_core::AiTool for MongoReadDocumentsTool {
    fn descriptor(&self) -> &fluxdb_core::ToolDescriptor {
        &self.descriptor
    }
    fn execute(
        &self,
        ctx: ToolExecutionContext,
        arguments: String,
    ) -> fluxdb_core::AiFuture<'_, ToolReceipt> {
        Box::pin(async move {
            let now = now_ms();
            let result = (|| -> fluxdb_core::Result<serde_json::Value> {
                if ctx.cancel.is_cancelled() {
                    return Err(editor_error("文档读取已取消"));
                }
                if ctx.session_id != self.session_id || ctx.turn_id != self.turn_id {
                    return Err(editor_error("文档快照不属于本轮任务"));
                }
                let args: serde_json::Value =
                    serde_json::from_str(&arguments).map_err(|_| editor_error("参数无效"))?;
                if args["reference_id"].as_str() != Some(self.grant.reference_id.as_str()) {
                    return Err(editor_error("页面引用未授权"));
                }
                let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0))
                    .map_err(|_| editor_error("偏移超限"))?;
                if offset > self.grant.documents.len() {
                    return Err(editor_error("偏移超出已加载快照"));
                }
                let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, 20) as usize;
                // 单条文档与整批都设字节预算：文档可以任意大，不能一次把集合灌进上下文。
                let mut documents = Vec::new();
                let mut bytes = 0;
                for (id, json) in self.grant.documents.iter().skip(offset).take(limit) {
                    let entry = serde_json::json!({
                        "id": id,
                        "json": json.chars().take(8 * 1024).collect::<String>(),
                        "truncated": json.chars().count() > 8 * 1024,
                    });
                    let size = entry.to_string().len();
                    if size > 16 * 1024 && documents.is_empty() {
                        return Err(editor_error("单条文档超过输出预算，请改用投影缩小字段"));
                    }
                    if bytes + size > 24 * 1024 {
                        break;
                    }
                    bytes += size;
                    documents.push(entry);
                }
                let next = offset + documents.len();
                Ok(serde_json::json!({
                    "reference_id": self.grant.reference_id,
                    "collection": self.grant.collection,
                    "documents": documents,
                    "next_offset": next,
                    "has_more": next < self.grant.documents.len(),
                    "total_loaded_documents": self.grant.documents.len(),
                    "filter": self.grant.filter,
                    "scope": "用户明确共享的发送时已加载文档快照，不重新查询数据库；过滤条件外的文档不可见",
                    "untrusted": true,
                }))
            })();
            let (status, value, code) = match result {
                Ok(value) => (ToolReceiptStatus::Succeeded, value, None),
                Err(error) => (
                    if ctx.cancel.is_cancelled() {
                        ToolReceiptStatus::Cancelled
                    } else {
                        ToolReceiptStatus::Failed
                    },
                    serde_json::json!({"error": error.message}),
                    Some("page_unavailable".into()),
                ),
            };
            Ok(ToolReceipt {
                call_id: ctx.call_id,
                tool_id: self.descriptor.id.clone(),
                status,
                result_json: value.to_string(),
                error_code: code,
                affected_rows: None,
                started_at_ms: now,
                finished_at_ms: now_ms(),
            })
        })
    }
}

pub fn add_ai_mongo_documents_tool(
    catalog: &mut ToolCatalog,
    grant: Option<MongoDocumentsGrant>,
    job: &AiConversationJob,
) -> fluxdb_core::Result<()> {
    let Some(grant) = grant else {
        return Ok(());
    };
    catalog.add_tool_with_shape(Box::new(MongoReadDocumentsTool {
        descriptor: fluxdb_core::ToolDescriptor {
            id: "mongo.read_documents".into(), version: 1, risk: fluxdb_core::ToolRisk::Read,
            description: "按位置读取用户明确共享的当前 MongoDB 集合页已加载文档快照；不查询数据库，也不读取授权集合之外的文档".into(),
            input_schema_json: serde_json::json!({"type":"object","properties":{
                "reference_id":{"type":"string"},"offset":{"type":"integer","minimum":0},
                "limit":{"type":"integer","minimum":1,"maximum":20}
            },"required":["reference_id"],"additionalProperties":false}).to_string(),
        },
        grant, session_id: job.session_id.clone(), turn_id: job.turn_id.clone(),
    }), ToolOutputShape::Read)
}

pub fn add_ai_page_tools(
    catalog: &mut ToolCatalog,
    grant: Option<PageDataGrant>,
    job: &AiConversationJob,
) -> fluxdb_core::Result<()> {
    let Some(grant) = grant else {
        return Ok(());
    };
    catalog.add_tool_with_shape(Box::new(PageReadRowsTool {
        descriptor: fluxdb_core::ToolDescriptor {
            id: "page.read_rows".into(), version: 1, risk: fluxdb_core::ToolRisk::Read,
            description: "按行读取用户明确开启共享的当前页面已加载数据快照；不查询数据库，也不读取未授权页面".into(),
            input_schema_json: serde_json::json!({"type":"object","properties":{
                "reference_id":{"type":"string"},"offset":{"type":"integer","minimum":0},
                "limit":{"type":"integer","minimum":1,"maximum":100},
                "columns":{"type":"array","items":{"type":"integer","minimum":0},"minItems":1,"maxItems":100}
            },"required":["reference_id"],"additionalProperties":false}).to_string(),
        }, grant, session_id:job.session_id.clone(), turn_id:job.turn_id.clone(),
    }), ToolOutputShape::Read)
}

#[cfg(test)]
mod er_page_context_tests {
    use super::*;

    fn er_page_context() -> CurrentPageContext {
        CurrentPageContext {
            kind: PageContextKind::ErDiagram,
            label: "ER-销售库".into(),
            snapshot: UiContextSnapshot {
                tab_id: Some(7),
                tab_revision: 3,
                target: TargetRef {
                    connection_id: Some(1),
                    database: Some("sales".into()),
                    schema: None,
                    object: Some("orders".into()),
                    redis_db: None,
                },
                selected_text: None,
                excerpt: None,
                statement_at_cursor: None,
                summary: None,
                last_error: None,
                result_ref: None,
                notes: vec!["关系目录与画布可见范围无关".into()],
            },
            details: serde_json::json!({
                "scope": { "connection_id": 1, "database": "sales", "schema": null },
                "selected_table": { "name": "orders", "column_count": 2 },
                "load_status": { "table_count": 12, "foreign_keys": "Loaded" },
                "version": 3,
            }),
            editor_grant: None,
            data_grant: None,
            mongo_grant: None,
        }
    }

    #[test]
    fn er_page_is_addressable_but_grants_no_data_capability() {
        let value = er_page_context().public_json();

        assert_eq!(value["kind"], "current_page");
        assert_eq!(value["page_kind"], "er_diagram");
        assert_eq!(value["target"]["database"], "sales");
        assert_eq!(value["revision"], 3);
        assert_eq!(value["details"]["version"], 3);
        assert_eq!(value["details"]["selected_table"]["name"], "orders");
        assert_eq!(
            value["capabilities"],
            serde_json::json!([]),
            "ER 页面不授予任何数据读取能力：关系详情只经授权的关系工具按需读取"
        );
        assert_eq!(value["untrusted"], true);
    }

    #[test]
    fn settings_page_exposes_toolbox_without_a_prefill_capability() {
        let mut page = er_page_context();
        page.kind = PageContextKind::Settings;
        page.details = serde_json::json!({});
        assert_eq!(
            page.public_json()["capabilities"],
            serde_json::json!(["toolbox.search", "toolbox.load", "toolbox.inspect"])
        );
    }

    #[test]
    fn mongo_page_offers_document_reads_only_when_a_snapshot_is_shared() {
        let mut page = er_page_context();
        page.kind = PageContextKind::MongoCollection;
        page.details = serde_json::json!({ "section": "documents" });

        assert_eq!(
            page.public_json()["capabilities"],
            serde_json::json!([]),
            "集合页默认只暴露定位与状态，文档值必须等用户发送时才登记"
        );

        page.mongo_grant = Some(MongoDocumentsGrant {
            reference_id: page.reference_id(),
            target: page.snapshot.target.clone(),
            collection: "orders".into(),
            documents: Arc::new(vec![("1".into(), "{\"a\":1}".into())]),
            filter: "{}".into(),
        });
        assert_eq!(
            page.public_json()["capabilities"],
            serde_json::json!(["mongo.read_documents"]),
            "只有本轮登记了文档快照才声明读取能力"
        );
        assert!(
            page.public_json()["details"]["section"] == "documents",
            "文档值不进页面详情，只经工具按需读取"
        );
    }

    #[test]
    fn mongo_documents_grant_is_bounded_and_single_use() {
        let job = AiConversationJob {
            session_id: "session".into(),
            turn_id: "mongo-turn".into(),
            assistant_message_id: "assistant".into(),
            model_profile_id: "model".into(),
        };
        // 一条 ASCII 超限文档 + 两条正常文档：单条按 8 KiB 截断后仍可读，整批受预算约束。
        let documents = vec![
            ("1".into(), format!("{{\"blob\":\"{}\"}}", "x".repeat(20 * 1024))),
            ("2".into(), "{\"a\":1}".into()),
            ("3".into(), "{\"b\":2}".into()),
        ];
        let grant = MongoDocumentsGrant {
            reference_id: "ref-1".into(),
            target: TargetRef::default(),
            collection: "orders".into(),
            documents: Arc::new(documents),
            filter: "{\"state\":\"open\"}".into(),
        };
        // 宽字符文档：按字符截断后仍超过单条预算，必须给出可执行的下一步而不是静默截断。
        let wide_grant = MongoDocumentsGrant {
            reference_id: "ref-wide".into(),
            target: TargetRef::default(),
            collection: "orders".into(),
            documents: Arc::new(vec![("1".into(), format!("{{\"t\":\"{}\"}}", "语".repeat(9 * 1024)))]),
            filter: "{}".into(),
        };

        let mut catalog = ToolCatalog::new(Vec::new());
        add_ai_mongo_documents_tool(&mut catalog, Some(grant.clone()), &job).expect("register");
        assert!(catalog.describe("mongo.read_documents").is_some());
        // 宽字符快照单独装配一份工具：与 ASCII 快照共用工具 ID，但引用与预算行为不同。
        let mut wide_catalog = ToolCatalog::new(Vec::new());
        add_ai_mongo_documents_tool(&mut wide_catalog, Some(wide_grant), &job).expect("register");

        let ctx = fluxdb_core::ToolExecutionContext {
            automatic_approval: false,
            bounded_write: false,
            session_id: job.session_id.clone(),
            turn_id: job.turn_id.clone(),
            iteration: 1,
            call_id: "call".into(),
            cancel: fluxdb_core::CancelFlag::new(),
            progress: None,
        };
        let run = |catalog: &ToolCatalog, offset: u64, limit: u64, reference: &str| {
            let tool = catalog.executable("mongo.read_documents").expect("tool");
            let arguments = format!(
                "{{\"reference_id\":\"{reference}\",\"offset\":{offset},\"limit\":{limit}}}"
            );
            let future = tool.execute(ctx.clone(), arguments);
            crate::ai_executor::block_on(future).expect("run").expect("receipt")
        };

        // 未授权引用与错轮次都必须拒绝，且不泄漏任何文档内容。
        let wrong_ref: serde_json::Value =
            serde_json::from_str(&run(&catalog, 0, 5, "other-page").result_json).expect("json");
        assert_eq!(wrong_ref["error"].as_str().unwrap(), "页面引用未授权");
        assert!(
            wrong_ref.get("documents").is_none(),
            "拒绝时不能泄漏任何文档内容: {wrong_ref}"
        );

        let batch: serde_json::Value =
            serde_json::from_str(&run(&wide_catalog, 0, 1, "ref-wide").result_json).expect("json");
        assert!(
            batch["error"]
                .as_str()
                .is_some_and(|message| message.contains("投影")),
            "超过单条输出预算必须给出可执行的下一步: {batch}"
        );
        assert!(batch.get("documents").is_none(), "拒绝时不返回内容: {batch}");

        let batch: serde_json::Value =
            serde_json::from_str(&run(&catalog, 1, 20, "ref-1").result_json).expect("json");
        let items = batch["documents"].as_array().unwrap();
        assert_eq!(items.len(), 2, "跳过后仍可读取剩余文档");
        assert_eq!(items[0]["id"], "2");
        assert_eq!(batch["next_offset"], 3);
        assert_eq!(batch["has_more"], false);
        assert_eq!(batch["total_loaded_documents"], 3);
        assert_eq!(batch["filter"], "{\"state\":\"open\"}");
        assert_eq!(batch["untrusted"], true);

        let paged: serde_json::Value =
            serde_json::from_str(&run(&catalog, 1, 1, "ref-1").result_json).expect("json");
        assert_eq!(paged["documents"].as_array().unwrap().len(), 1);
        assert_eq!(paged["next_offset"], 2);
        assert_eq!(paged["has_more"], true, "分页必须能继续读");

        // 大文档不是拒绝读取的理由：按单条预算截断并显式标记，模型可改用投影取全量。
        let truncated: serde_json::Value =
            serde_json::from_str(&run(&catalog, 0, 20, "ref-1").result_json).expect("json");
        let items = truncated["documents"].as_array().unwrap();
        assert_eq!(items.len(), 3, "两条正常文档应随截断后的首条一起返回: {truncated}");
        assert_eq!(items[0]["truncated"], true);
        assert_eq!(items[1]["truncated"], false);
        assert!(
            items[0]["json"].as_str().unwrap().chars().count() <= 8 * 1024,
            "单条必须限制在 8 KiB 字符内"
        );

        let over_offset = serde_json::from_str::<serde_json::Value>(
            &run(&catalog, 99, 5, "ref-1").result_json,
        )
        .expect("json");
        assert!(over_offset["error"].as_str().unwrap().contains("偏移"), "{over_offset}");

        // 快照只在本轮可用：装配取走后登记即清空，下一轮不会读到旧数据。
        mongo_documents_registry()
            .lock()
            .expect("registry")
            .insert(job.turn_id.clone(), grant.clone());
        assert!(take_mongo_documents_grant(&job.turn_id).is_some());
        assert!(
            take_mongo_documents_grant(&job.turn_id).is_none(),
            "同一轮只能取走一次"
        );
        mongo_documents_registry()
            .lock()
            .expect("registry")
            .insert(job.turn_id.clone(), grant);
        clear_current_page_grants(&job.turn_id);
        assert!(take_mongo_documents_grant(&job.turn_id).is_none());
    }

    #[test]
    fn oversized_details_are_trimmed_instead_of_sending_everything() {
        let mut page = er_page_context();
        page.details["value"] = serde_json::json!("x".repeat(64 * 1024));

        let value = page.public_json();

        assert_eq!(value["truncated"], true);
        assert!(value["details"].get("value").is_none());
        assert_eq!(value["omitted_details"], serde_json::json!(["value"]));
    }
}
