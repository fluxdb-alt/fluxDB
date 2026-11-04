/// 内置 Agent 工具；数据库/Redis 工具同时供 MCP 使用，产物工具不对外暴露。
/// 第二列是稳定分组码，第三列是未知工具 id 的英文兜底；已知 id 走 `ai_tool_text` 翻译。
pub const AI_AGENT_TOOL_CATALOG: &[(&str, &str, &str, bool)] = &[
    ("page.find_table", "common", "Find tables by name, table comment, or column name across saved SQL connections without reading rows", false),
    ("page.browse_objects", "common", "Expand configured SQL connections in the sidebar and discover real objects", false),
    ("page.open_table", "common", "Open a discovered table without querying rows", false),
    ("page.query_table", "common", "Fill visible filters and optionally query the table page once", false),
    (
        "editor.apply_edit",
        "common",
        "Replace the explicitly authorized SQL editor range without executing SQL",
        false,
    ),
    (
        "editor.search",
        "common",
        "Search the authorized editor snapshot",
        false,
    ),
    (
        "editor.read_range",
        "common",
        "Read a bounded range of the authorized editor",
        false,
    ),
    (
        "page.read_rows",
        "common",
        "Read bounded rows from the authorized page snapshot",
        false,
    ),
    (
        "file.search",
        "common",
        "Find authorized attachments",
        false,
    ),
    (
        "file.search_content",
        "common",
        "Search authorized attachment content",
        false,
    ),
    (
        "file.read",
        "common",
        "Read an authorized attachment",
        false,
    ),
    (
        "context.read",
        "common",
        "Read an authorized reference",
        false,
    ),
    (
        "docs.search",
        "common",
        "Search bundled FluxDB product documentation",
        false,
    ),
    (
        "docs.read",
        "common",
        "Read a bounded section of bundled FluxDB product documentation",
        false,
    ),
    (
        "plan.update",
        "common",
        "Update the public execution plan",
        false,
    ),
    (
        "connections.list",
        "common",
        "List authorized connection resources",
        false,
    ),
    (
        "toolbox.inspect",
        "common",
        "Inspect live non-sensitive host state",
        false,
    ),
    (
        "toolbox.search",
        "common",
        "Search long-tail operations without loading their schemas",
        false,
    ),
    (
        "toolbox.load",
        "common",
        "Load selected operation schemas for the current turn",
        false,
    ),
    (
        "config.propose",
        "common",
        "Allow toolbox-loaded configuration actions; persistence still requires confirmation",
        true,
    ),
    (
        "db.search_objects",
        "sql",
        "Incrementally search tables, views, and columns",
        false,
    ),
    (
        "db.describe_object",
        "sql",
        "Read real columns, indexes, and relationships",
        false,
    ),
    (
        "db.get_relationships",
        "relationship",
        "Read registered relationships (physical foreign keys plus confirmed logical relationships), with availability and required filters",
        false,
    ),
    (
        "db.find_join_paths",
        "relationship",
        "Find join paths over available relationships and return a structured plan (without generating SQL)",
        false,
    ),
    (
        "er.propose_relationships",
        "relationship",
        "Generate relationship candidates for individual review; adoption requires user confirmation and does not execute database DDL",
        false,
    ),
    (
        "db.run",
        "sql",
        "Single SQL entry: classify read/write, approve writes, optional plan",
        true,
    ),
    (
        "redis.scan",
        "redis",
        "Incremental scan; never a full KEYS",
        false,
    ),
    (
        "redis.inspect",
        "redis",
        "Bounded type-aware value preview and TTL",
        false,
    ),
    (
        "redis.execute",
        "redis",
        "Controlled command that inspects all keys and risks",
        true,
    ),
    (
        "redis.subscribe",
        "redis",
        "Bounded subscription with time, count, and byte limits; cancellable",
        false,
    ),
    (
        "redis.server_info",
        "redis",
        "Bounded version and diagnostic summary",
        false,
    ),
    (
        "mongo.read_documents",
        "mongo",
        "Read bounded documents from the authorized MongoDB collection page snapshot; never re-queries the database",
        false,
    ),
    (
        "result.read_page",
        "common",
        "Read a result snapshot already obtained",
        false,
    ),
    (
        "diagnostics.read_turns",
        "common",
        "Read redacted failure facts from recent turns (terminal state, stage, error code, timing, failing tool)",
        false,
    ),
    (
        "artifact.create",
        "artifact",
        "Create a report draft; built-in rendering requires the BI plugin",
        true,
    ),
    (
        "artifact.add_block",
        "artifact",
        "Append charts, tables, or notes to a report without modifying the database",
        true,
    ),
    (
        "artifact.finalize",
        "artifact",
        "Validate sources and generate a report snapshot without modifying the database",
        true,
    ),
];

/// 仅 MCP 提供的工具仍受全局开关控制，不进入内置 Agent 的配置。
///
/// 关系/路径工具对外复用同名内部 ID（`db.get_relationships` / `db.find_join_paths`），
/// 因此它们的全局开关与内置 AI 是同一份，无需在此重复列出。
pub const AI_MCP_ONLY_TOOL_CATALOG: &[(&str, &str, &str, bool)] = &[(
    "operation.status",
    "common",
    "Query the current caller's operation status, including unknown results after interruption",
    false,
)];

pub fn is_known_ai_tool(id: &str) -> bool {
    AI_AGENT_TOOL_CATALOG
        .iter()
        .chain(AI_MCP_ONLY_TOOL_CATALOG)
        .any(|item| item.0 == id)
}

#[cfg(test)]
mod ai_tool_management_tests {
    use super::*;

    #[test]
    fn management_catalog_covers_conditional_tools_without_duplicates() {
        let ids: Vec<_> = AI_AGENT_TOOL_CATALOG
            .iter()
            .chain(AI_MCP_ONLY_TOOL_CATALOG)
            .map(|entry| entry.0)
            .collect();
        assert_eq!(
            ids.len(),
            ids.iter().copied().collect::<BTreeSet<_>>().len()
        );
        for id in [
            "toolbox.inspect",
            "toolbox.search",
            "toolbox.load",
            "config.propose",
            "db.run",
            "editor.read_range",
            "editor.search",
            "page.read_rows",
            "mongo.read_documents",
            "file.read",
            "file.search",
            "file.search_content",
            "context.read",
            "docs.search",
            "docs.read",
            "plan.update",
        ] {
            assert!(ids.contains(&id), "{id}");
        }
        assert!(!is_known_ai_tool("page.prefill_form"));
        for spec in standard_sql_tool_specs()
            .expect("SQL specs")
            .into_iter()
            .chain(standard_redis_tool_specs().expect("Redis specs"))
        {
            assert!(ids.contains(&spec.id.as_str()), "missing {}", spec.id);
        }
    }
}
