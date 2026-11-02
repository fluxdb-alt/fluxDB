// MongoDB 集合工作台的标签页状态。
//
// 与 `DataEditorState`（行列模型）分开：MongoDB 的读取单位是整篇文档，且要保留 BSON 类型，
// 因此这里以 Extended JSON 文本承载查询参数与文档值，分节展示（文档 / 聚合 / 索引 / 集合信息）。

/// 集合工作台的分节页签。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MongoSection {
    #[default]
    Documents,
    Aggregation,
    Indexes,
    Info,
}

/// 文档结果的展示方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MongoDisplayMode {
    /// 文档卡片（字段 + BSON 类型徽标，可折叠嵌套）。
    #[default]
    Documents,
    /// 表格（`_id` + 文档 JSON 两列）。
    Table,
    /// 整页 JSON（复用 JSON 组件）。
    Json,
}

/// 查询面板的可编辑文本。对象类字段均为 Extended JSON 文本。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoQueryDraft {
    pub filter: String,
    pub projection: String,
    pub sort: String,
    /// 排序规则（Extended JSON 对象）。
    pub collation: String,
    /// 索引提示：索引名（JSON 字符串）或键定义对象。
    pub hint: String,
    /// 跳过条数（文本形式；空串等同 0）。
    pub skip: String,
    /// 本页条数（文本形式，提交时解析并夹到连接器上限）。
    pub limit: String,
    /// 服务端执行上限毫秒（文本形式；空串表示不限制）。
    pub max_time_ms: String,
}

/// 设置里的「默认查询限制」→ MongoDB 每页条数。
///
/// `0` 表示不限制：MongoDB 单次查询本身有硬上限（`MONGO_MAX_DOCUMENTS_PER_QUERY`），
/// 「不限制」按该上限落地。其余档位（1000）同样封顶到上限，否则用户选了 1000 却只
/// 拿到 500 条，看起来像丢了数据。
pub fn mongo_query_limit(setting: u64) -> u64 {
    let max = fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY;
    if setting == 0 { max } else { setting.min(max) }
}

impl MongoQueryDraft {
    /// 默认草稿：空 Filter 等价 `{}`，每页条数取调用方给的取值（通常是设置里的默认查询限制）。
    ///
    /// 入参按设置原值传入即可：`0`（不限制）与超出连接器上限的档位都由
    /// [`mongo_query_limit`] 归一。
    pub fn new(limit: u64) -> Self {
        Self {
            filter: "{}".to_string(),
            projection: String::new(),
            sort: String::new(),
            collation: String::new(),
            hint: String::new(),
            skip: "0".to_string(),
            limit: mongo_query_limit(limit).to_string(),
            max_time_ms: String::new(),
        }
    }

    /// 起始位置：`skip` 是分页偏移的唯一来源（翻页会把新偏移写回这一格）。
    pub fn skip_value(&self) -> u64 {
        self.skip
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|skip| *skip > 0)
            .unwrap_or(0)
    }

    /// 每个编辑项都能在分析器里逐字段定位错误，因此空文本一律按「未填写」处理。
    pub fn to_request(&self, collection: &str) -> fluxdb_core::MongoFindRequest {
        fluxdb_core::MongoFindRequest {
            collection: collection.to_string(),
            filter: self.filter.clone(),
            projection: self.projection.clone(),
            sort: self.sort.clone(),
            collation: self.collation.clone(),
            hint: self.hint.clone(),
            skip: self.skip_value(),
            limit: mongo_query_limit(
                self.limit
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .unwrap_or(fluxdb_core::Pagination::DEFAULT_LIMIT),
            ),
            // 0 与空串同样按「不限制」处理：服务器对 `maxTimeMS: 0` 的语义是立即超时，
            // 那不是用户填 0 的本意。
            max_time_ms: self
                .max_time_ms
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0),
        }
    }
}

/// MongoDB 命令工作台标签页：库级（跨集合）的 shell 语句执行台。
///
/// 与 `MongoCollectionState`（单集合的文档/聚合页）和 `MongoCliState`（真实终端 REPL）分开：
/// 这里只做「输入一条语句 → 追加一条执行记录」，形态对齐 Redis Workbench。
/// 语句语法与只读边界完全复用连接器的 `mongo_shell`，不在这里另写一套解析。
#[derive(Clone, Debug, PartialEq)]
pub struct MongoWorkbenchState {
    pub connection_id: ConnectionId,
    /// 当前库；`use <db>` 只改这里，不写回连接配置。
    pub database: String,
    /// 顶部输入框草稿。执行后清空，语句文本进入 `executions`。
    pub text: String,
    pub running: bool,
    /// 本次会话内的执行记录，按执行顺序追加（一条语句一条记录）。
    pub executions: Vec<MongoWorkbenchExecution>,
    pub error: Option<UserFacingError>,
    /// 最近一次已执行文本的指纹，用于标签页脏标记。
    pub saved_fingerprint: Option<QueryFingerprint>,
    /// 为 `executions` 分配 `id` 的单调递增计数器。
    pub next_execution_id: u64,
    /// 折叠态记录 ID 集合（仅 UI 展示，会话内有效）。
    pub collapsed: std::collections::BTreeSet<u64>,
    /// 当前语句开始执行的时刻，用于在记录里落真实耗时；没有执行时为 `None`。
    pub running_started_at: Option<std::time::Instant>,
}

impl MongoWorkbenchState {
    pub fn new(connection_id: ConnectionId, database: String) -> Self {
        Self {
            connection_id,
            database,
            text: String::new(),
            running: false,
            executions: Vec::new(),
            error: None,
            saved_fingerprint: None,
            next_execution_id: 1,
            collapsed: std::collections::BTreeSet::new(),
            running_started_at: None,
        }
    }

    /// 是否有未执行（未保存）的语句改动；判定口径与 Redis Workbench 一致。
    pub fn has_unsaved_text(&self) -> bool {
        match self.saved_fingerprint {
            Some(saved) => saved != QueryFingerprint::for_text(&self.text),
            None => !self.text.trim().is_empty(),
        }
    }
}

/// 工作台里的一条执行记录。
#[derive(Clone, Debug, PartialEq)]
pub struct MongoWorkbenchExecution {
    /// App 层分配的记录 ID（重跑 / 删除按它定位）。
    pub id: u64,
    /// 实际执行的语句原文。
    pub text: String,
    /// 执行时所在的库（`use` 之后的值）。
    pub database: String,
    /// 成功时连接器返回的回显文本（含 ANSI 颜色，按 shell 风格排版）；
    /// 失败时为 `None`，错误原因见 `error`。
    pub output: Option<String>,
    pub error: Option<UserFacingError>,
    /// 返回的文档数（仅 find / aggregate 这类文档结果有值）。
    pub document_count: Option<u64>,
    /// 结果是否因条数或字节预算被截断。
    pub truncated: bool,
    /// 结构化的文档结果；为空时界面按纯文本展示（`show`、`count`、错误等）。
    pub documents: Vec<fluxdb_core::MongoDocument>,
    /// 执行耗时（毫秒）。
    pub elapsed_ms: u64,
    /// 执行时刻（Unix 秒）。
    pub started_at_unix_secs: u64,
}

impl MongoWorkbenchExecution {
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// 集合工作台标签页。
#[derive(Clone, Debug, PartialEq)]
pub struct MongoCollectionState {
    pub object: ObjectPath,
    /// 只读对象（视图）：禁止写入类操作，UI 据此隐藏/禁用入口。
    pub read_only: bool,
    pub section: MongoSection,
    pub display: MongoDisplayMode,
    /// 已生效的查询：刷新与输入框尚未创建时据此还原面板。
    pub applied_query: MongoQueryDraft,
    pub page: Option<fluxdb_core::MongoDocumentsPage>,
    pub pagination: Pagination,
    /// 每次查询/翻页递增；后台结果只回写与当前令牌一致的加载。
    pub load_generation: u64,
    pub loading: bool,
    pub error: Option<UserFacingError>,
    /// 右侧详情抽屉选中的文档下标（`None` 表示关闭）。
    pub selected_document: Option<usize>,
    pub info: LoadState<fluxdb_core::MongoCollectionInfo>,
    pub indexes: LoadState<Vec<fluxdb_core::MongoIndexInfo>>,
    /// 聚合管道文本（Extended JSON 数组）；空表示未编辑。
    pub aggregation_pipeline: String,
    /// 聚合结果（与文档页同样的分页结构，便于复用文档/表格/JSON 三种展示）。
    pub aggregation: LoadState<fluxdb_core::MongoDocumentsPage>,
    /// 聚合请求令牌：只回写与当前令牌一致的结果。
    pub aggregation_generation: u64,
    /// 补全用的字段目录：抽样结果 + 已加载页面的字段，按集合标签页缓存。
    pub field_catalog: MongoFieldCatalog,
}

impl MongoCollectionState {
    pub fn new(object: ObjectPath, read_only: bool, page_size: u64) -> Self {
        Self {
            object,
            read_only,
            section: MongoSection::Documents,
            display: MongoDisplayMode::Documents,
            applied_query: MongoQueryDraft::new(page_size),
            page: None,
            pagination: Pagination::new(0, page_size),
            load_generation: 0,
            loading: false,
            error: None,
            selected_document: None,
            info: LoadState::NotLoaded,
            indexes: LoadState::NotLoaded,
            aggregation_pipeline: String::new(),
            aggregation: LoadState::NotLoaded,
            aggregation_generation: 0,
            field_catalog: MongoFieldCatalog::default(),
        }
    }

    /// 默认聚合管道：统计集合总量。用户可改写后再执行，不做隐式字段猜测。
    pub fn default_aggregation_pipeline() -> String {
        "[\n  { \"$group\": { \"_id\": null, \"count\": { \"$sum\": 1 } } }\n]".to_string()
    }

    /// 开始一次聚合，返回本次令牌（与文档加载互不干扰）。
    pub fn begin_aggregation(&mut self) -> u64 {
        self.aggregation_generation = self.aggregation_generation.wrapping_add(1);
        self.aggregation = LoadState::Loading;
        self.aggregation_generation
    }

    pub fn accepts_aggregation(&self, generation: u64) -> bool {
        self.aggregation_generation == generation
    }

    /// 开始一次新的加载，返回本次令牌（旧结果据此丢弃）。
    pub fn begin_load(&mut self) -> u64 {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.load_generation
    }

    pub fn accepts_load(&self, generation: u64) -> bool {
        self.load_generation == generation
    }

    /// 当前页条数上限（与 `pagination.limit` 同源，避免查询草稿与分页器打架）。
    pub fn page_limit(&self) -> u64 {
        self.pagination.limit
    }
}

#[cfg(test)]
mod mongo_query_limit_tests {
    use super::*;

    #[test]
    fn query_limit_follows_setting_and_caps_at_connector_max() {
        let max = fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY;
        // 「不限制」按连接器上限落地：0 直接透传到请求会变成默认页，看起来像忽略设置。
        assert_eq!(mongo_query_limit(0), max);
        assert_eq!(mongo_query_limit(100), 100);
        // 1000 档超过连接器上限，必须在这里就压到 max：否则用户选了 1000 却只拿到 500 条。
        assert_eq!(mongo_query_limit(1000), max);
    }

    #[test]
    fn draft_and_request_share_the_same_limit() {
        let draft = MongoQueryDraft::new(0);
        assert_eq!(draft.limit, fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY.to_string());
        // 手填超过上限的文本（AI/恢复路径可能直接写入）也不该原样进请求。
        let draft = MongoQueryDraft {
            filter: "{}".into(),
            limit: "100000".into(),
            ..Default::default()
        };
        assert_eq!(
            draft.to_request("orders").limit,
            fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY
        );
    }

    #[test]
    fn skip_is_the_single_source_of_paging_offset() {
        // 起点只存在 skip 这一格：空串按 0 处理（清空即从头查），不能回落到上一页偏移。
        let draft = MongoQueryDraft {
            skip: "  ".into(),
            ..MongoQueryDraft::new(0)
        };
        assert_eq!(draft.skip_value(), 0);
        assert_eq!(draft.to_request("orders").skip, 0);

        let draft = MongoQueryDraft {
            skip: "120".into(),
            ..MongoQueryDraft::new(0)
        };
        assert_eq!(draft.to_request("orders").skip, 120);
    }

    #[test]
    fn max_time_zero_means_unlimited() {
        // 服务器对 `maxTimeMS: 0` 的语义是「立即超时」，那不是用户清空/填 0 的本意，
        // 所以这里必须落成 None，而不是把 0 发出去。
        for text in ["", "0", "  "] {
            let draft = MongoQueryDraft {
                max_time_ms: text.into(),
                ..MongoQueryDraft::new(0)
            };
            assert_eq!(draft.to_request("orders").max_time_ms, None, "{text:?}");
        }
        let draft = MongoQueryDraft {
            max_time_ms: "1500".into(),
            ..MongoQueryDraft::new(0)
        };
        assert_eq!(draft.to_request("orders").max_time_ms, Some(1500));
    }
}
