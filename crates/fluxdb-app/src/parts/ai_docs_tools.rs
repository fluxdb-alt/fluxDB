// AI 通用文档工具：docs.search / docs.read。
//
// 只读内置产品文档，不读取用户文件、不联网、不碰连接或凭据。两个工具共用
// manual_docs 的索引；搜索结果只带短摘要，正文必须再按 section_id 分页读取。

struct AiDocsTool {
    descriptor: fluxdb_core::ToolDescriptor,
}

impl fluxdb_core::AiTool for AiDocsTool {
    fn descriptor(&self) -> &fluxdb_core::ToolDescriptor {
        &self.descriptor
    }

    fn execute(
        &self,
        ctx: fluxdb_core::ToolExecutionContext,
        arguments_json: String,
    ) -> fluxdb_core::AiFuture<'_, fluxdb_core::ToolReceipt> {
        Box::pin(async move {
            let now = ai_chat_now();
            let result = (|| -> fluxdb_core::Result<serde_json::Value> {
                if ctx.cancel.is_cancelled() {
                    return Err(fluxdb_core::Error::new(
                        fluxdb_core::ErrorKind::Internal,
                        "文档读取已取消",
                    ));
                }
                let args: serde_json::Value =
                    serde_json::from_str(&arguments_json).map_err(|_| {
                        fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            "文档参数无效",
                        )
                    })?;
                let source = args
                    .get("source")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let payload = if self.descriptor.id == "docs.search" {
                    let query = args
                        .get("query")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim();
                    if query.is_empty() {
                        return Err(fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            "搜索关键词不能为空",
                        ));
                    }
                    let limit = args
                        .get("limit")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(5) as usize;
                    let hits = docs_search(source, query, limit).map_err(|message| {
                        fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, message)
                    })?;
                    serde_json::json!({
                        "query": query,
                        "count": hits.len(),
                        "results": hits
                            .into_iter()
                            .map(|hit| {
                                serde_json::json!({
                                    "source": hit.source,
                                    "source_title": hit.source_title,
                                    "section_id": hit.section_id,
                                    "title": hit.title,
                                    "chapter": hit.chapter,
                                    "snippet": hit.snippet,
                                })
                            })
                            .collect::<Vec<_>>(),
                    })
                } else {
                    let section_id = args
                        .get("section_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim();
                    if section_id.is_empty() {
                        return Err(fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            "缺少文档小节 ID",
                        ));
                    }
                    let offset = args
                        .get("offset")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as usize;
                    let max_bytes = args
                        .get("max_bytes")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(8192) as usize;
                    let read = docs_read(source, section_id, offset, max_bytes).map_err(
                        |message| {
                            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, message)
                        },
                    )?;
                    serde_json::json!({
                        "source": read.source,
                        "source_title": read.source_title,
                        "section_id": read.section_id,
                        "title": read.title,
                        "chapter": read.chapter,
                        "offset": read.offset,
                        "next_offset": read.next_offset,
                        "truncated": read.truncated,
                        "text": read.text,
                    })
                };
                if ctx.cancel.is_cancelled() {
                    return Err(fluxdb_core::Error::new(
                        fluxdb_core::ErrorKind::Internal,
                        "文档读取已取消",
                    ));
                }
                Ok(payload)
            })();
            let (status, value, error) = match result {
                Ok(value) => (fluxdb_core::ToolReceiptStatus::Succeeded, value, None),
                Err(error) => (
                    if ctx.cancel.is_cancelled() {
                        fluxdb_core::ToolReceiptStatus::Cancelled
                    } else {
                        fluxdb_core::ToolReceiptStatus::Failed
                    },
                    serde_json::json!({"error": error.message}),
                    Some("docs_unavailable".into()),
                ),
            };
            Ok(fluxdb_core::ToolReceipt {
                call_id: ctx.call_id,
                tool_id: self.descriptor.id.clone(),
                status,
                result_json: value.to_string(),
                error_code: error,
                affected_rows: None,
                started_at_ms: now,
                finished_at_ms: ai_chat_now(),
            })
        })
    }
}

/// 装配两个通用文档工具；内置文档是编译期静态资源，始终可用。
pub fn add_ai_docs_tools(catalog: &mut ToolCatalog) -> fluxdb_core::Result<()> {
    let specs = [
        (
            "docs.search",
            "Search bundled FluxDB product documentation by keyword; read-only and local. Use docs.read for full section text.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 200},
                    "source": {"type": "string", "description": "Optional documentation source id; defaults to user-guide"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 10}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            ToolOutputShape::Opaque,
        ),
        (
            "docs.read",
            "Read a bounded section of bundled FluxDB product documentation by section_id; call docs.search first to find the section.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source": {"type": "string", "description": "Optional documentation source id; defaults to user-guide"},
                    "section_id": {"type": "string", "minLength": 1},
                    "offset": {"type": "integer", "minimum": 0},
                    "max_bytes": {"type": "integer", "minimum": 1, "maximum": 16384}
                },
                "required": ["section_id"],
                "additionalProperties": false
            }),
            ToolOutputShape::Read,
        ),
    ];
    for (id, description, input_schema, output_shape) in specs {
        catalog.add_tool_with_shape(
            Box::new(AiDocsTool {
                descriptor: fluxdb_core::ToolDescriptor {
                    id: id.into(),
                    description: description.into(),
                    version: 1,
                    risk: fluxdb_core::ToolRisk::Read,
                    input_schema_json: input_schema.to_string(),
                },
            }),
            output_shape,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod ai_docs_tools_tests {
    use super::*;

    fn context(call_id: &str) -> fluxdb_core::ToolExecutionContext {
        fluxdb_core::ToolExecutionContext {
            automatic_approval: false,
            bounded_write: false,
            session_id: "session".into(),
            turn_id: "turn".into(),
            iteration: 1,
            call_id: call_id.into(),
            cancel: fluxdb_core::CancelFlag::new(),
            progress: None,
        }
    }

    fn run(catalog: &ToolCatalog, id: &str, args: serde_json::Value) -> fluxdb_core::ToolReceipt {
        let tool = catalog.executable(id).expect("tool");
        crate::ai_executor::block_on(tool.execute(context(id), args.to_string()))
            .expect("execute")
            .expect("receipt")
    }

    #[test]
    fn search_and_read_return_bundled_guide_content() {
        let mut catalog = ToolCatalog::new(Vec::new());
        add_ai_docs_tools(&mut catalog).expect("register");

        let search = run(
            &catalog,
            "docs.search",
            serde_json::json!({"query": "MariaDB", "limit": 3}),
        );
        assert!(search.is_success());
        let search: serde_json::Value = serde_json::from_str(&search.result_json).expect("json");
        let section_id = search["results"][0]["section_id"]
            .as_str()
            .expect("section id")
            .to_string();

        let read = run(
            &catalog,
            "docs.read",
            serde_json::json!({"section_id": section_id, "max_bytes": 8192}),
        );
        assert!(read.is_success());
        let read: serde_json::Value = serde_json::from_str(&read.result_json).expect("json");
        assert_eq!(read["source"], USER_GUIDE_SOURCE);
        assert!(!read["text"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn unknown_section_returns_controlled_error() {
        let mut catalog = ToolCatalog::new(Vec::new());
        add_ai_docs_tools(&mut catalog).expect("register");
        let receipt = run(
            &catalog,
            "docs.read",
            serde_json::json!({"section_id": "999.999"}),
        );
        assert!(!receipt.is_success());
        assert_eq!(receipt.error_code.as_deref(), Some("docs_unavailable"));
        assert!(receipt.result_json.contains("未找到文档小节"));
    }
}
