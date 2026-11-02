// MongoDB 领域模型：集合浏览 / 查询 / 元数据的跨层数据结构。
//
// 与 `DataPage` 分开的原因：MongoDB 的最小读取单位是**整个文档**而不是行 × 列，
// 文档还要保留 BSON 类型（ObjectId / Date / Decimal128 …）。用行列表达会丢掉
// 投影、嵌套结构与非关系型类型，因此这里用 Extended JSON 文本承载文档值，
// 由 UI 侧复用现有 JSON 组件渲染与编辑。

/// 单次 MongoDB 查询的文档上限：连接器按此夹紧 `find`/聚合，UI 的「条数」也按它封顶。
///
/// 放在 core 而不是连接器里：这是**产品上限**，输入框校验与请求兜底必须与连接器
/// 用的是同一个数，否则用户能填进去一个执行时被静默改小的值。
pub const MONGO_MAX_DOCUMENTS_PER_QUERY: u64 = 500;

/// 单次查询结果文本的字节上限（8 MiB）。
///
/// 只限条数挡不住内存：MongoDB 单篇文档最大 16 MB，500 篇理论上是 8 GB。
/// 连接器按此预算提前停止读取并标记截断，避免把整段结果留在内存里再交给 UI 排版。
pub const MONGO_MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// 一次 `find` 请求。所有 JSON 字段都是 **Extended JSON 文本**，由连接器解析成 BSON。
///
/// 空字符串等价于未提供：`filter` 为空表示 `{}`，其余为空表示不限制。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoFindRequest {
    /// 目标集合。
    pub collection: String,
    /// 过滤条件（Extended JSON 对象）。
    pub filter: String,
    /// 投影（Extended JSON 对象）。
    pub projection: String,
    /// 排序（Extended JSON 对象，值只能是 1 / -1）。
    pub sort: String,
    /// 排序规则（Extended JSON 对象，如 `{ "locale": "en", "strength": 2 }`）。
    pub collation: String,
    /// 索引提示：Extended JSON 字符串（索引名）或对象（键定义），对应 `hint`。
    pub hint: String,
    /// 跳过文档数。
    pub skip: u64,
    /// 本页最多返回文档数（连接器负责夹到上限）。
    pub limit: u64,
    /// 服务端执行上限（毫秒，对应 `maxTimeMS`）；`None` 表示不限制。
    pub max_time_ms: Option<u64>,
}

impl MongoFindRequest {
    /// 仅按集合名构造默认查询（`{}`，无排序）。
    pub fn new(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            ..Default::default()
        }
    }
}

/// 一条文档：`_id` 的十六进制/字符串表示 + 完整 Relaxed Extended JSON 文本。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoDocument {
    /// `_id` 的可读标识（ObjectId 用 24 位十六进制；其它类型用其 Extended JSON 文本）。
    /// 仅用于展示与选中态定位，不参与查询。
    pub id: String,
    /// Relaxed Extended JSON 文本（已格式化，保留 BSON 类型标注）。
    pub json: String,
}

/// 一页文档结果。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoDocumentsPage {
    pub documents: Vec<MongoDocument>,
    /// 本页返回条数。
    pub returned: u64,
    /// 集合文档总数；仅在用户显式请求统计或结果集不足一页时填充，避免每次翻页都做全表 count。
    pub total: Option<u64>,
    /// 是否还有更多（按 `skip + returned < total` 或"取 limit+1 条"判定）。
    pub has_more: bool,
}

/// 集合类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MongoCollectionKind {
    #[default]
    Collection,
    /// 视图（只读，不能写入）。
    View,
    /// 时间序列集合。
    Timeseries,
}

/// 集合/视图元数据。拿不到的统计保持 `None`，不伪造。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoCollectionInfo {
    pub name: String,
    pub kind: MongoCollectionKind,
    /// `count_documents({})`；视图与权限不足时为 `None`。
    pub document_count: Option<u64>,
    /// `avgObjSize`（字节）。
    pub avg_document_size: Option<u64>,
    /// `size`（未压缩逻辑大小，字节）。
    pub data_size: Option<u64>,
    /// `storageSize`（磁盘占用，字节）。
    pub storage_size: Option<u64>,
    /// 索引数量。
    pub index_count: Option<u64>,
    /// 视图的定义管道（Extended JSON）；仅视图有值。
    pub view_pipeline: Option<String>,
    /// 视图的来源集合。
    pub view_on: Option<String>,
}

/// 索引键定义的一项：字段名与方向/类型（`1`、`-1`、`text`、`2dsphere` 等）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoIndexField {
    pub field: String,
    /// 键值的展示文本；BSON 数值与字符串统一转成文本，无需 UI 再解析 JSON。
    pub value: String,
}

/// 一个索引。
///
/// 统计类字段（`size_bytes` / `usage_count` / `usage_since`）来自 `collStats` 与
/// `$indexStats`；命令失败或权限不足时保持 `None`，不用 0 冒充。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoIndexInfo {
    pub name: String,
    /// 键定义（Relaxed Extended JSON 文本，如 `{"_id": {"$numberInt":"1"}}`）。
    pub keys_json: String,
    /// 键定义的有序字段列表，供展开行展示与复合索引判定使用。
    pub fields: Vec<MongoIndexField>,
    /// 索引类型：`regular` / `geospatial` / `hashed` / `text` / `wildcard` /
    /// `columnstore` / `clustered`，与 Compass 的 IndexDefinition.type 同一口径。
    pub index_type: String,
    pub unique: bool,
    pub sparse: bool,
    /// 部分索引的过滤条件（Extended JSON）。
    pub partial_filter: Option<String>,
    /// TTL 索引的过期秒数。
    pub expire_after_seconds: Option<i64>,
    /// 是否带 collation。
    pub has_collation: bool,
    /// 是否被隐藏（`hidden: true`）。
    pub hidden: bool,
    /// `collStats.indexSizes` 里该索引的大小（字节）。
    pub size_bytes: Option<u64>,
    /// `$indexStats.accesses.ops`。
    pub usage_count: Option<u64>,
    /// `$indexStats.accesses.since`（epoch 毫秒）。
    pub usage_since: Option<i64>,
    /// 索引是否已建好（`$indexStats.building` 为 true 时才是 false）。
    pub ready: bool,
}

/// 集群拓扑类型（来自 `hello` 的 `setName` / `msg` / `isWritablePrimary`）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MongoTopology {
    /// 无法判定（命令失败或字段缺失）。
    #[default]
    Unknown,
    Standalone,
    ReplicaSet,
    /// `msg: "isdbgrid"`（mongos）。
    Sharded,
}

/// 连接信息弹框的数据（对齐 Compass 的 Stats/Topology/Cluster 三块）。
///
/// 只汇总服务端真实返回的字段：`hello` 拿拓扑与成员，`buildInfo` 拿版本与发行版，
/// `listDatabases` 拿库大小。任一命令失败只留空对应字段，不编造值。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoServerInfo {
    /// `buildInfo.version`。
    pub version: Option<String>,
    /// `buildInfo.modules` 含 `enterprise` 时为 `Enterprise`，否则 `Community`。
    pub edition: Option<String>,
    pub topology: MongoTopology,
    /// 副本集名称（`hello.setName`）。
    pub replica_set_name: Option<String>,
    /// 当前主节点地址（`hello.primary`）。
    pub primary: Option<String>,
    /// 集群成员地址（`hello.hosts`）；独立部署为空。
    pub hosts: Vec<String>,
    /// 协议版本（`hello.maxWireVersion`）。
    pub max_wire_version: Option<i32>,
    /// 服务端运行秒数（`hello.uptime`）。
    pub uptime_secs: Option<u64>,
    /// 可见数据库及其磁盘占用（`listDatabases`）。
    pub databases: Vec<MongoDatabaseStat>,
}

/// MongoDB shell 一次执行的结果。
///
/// shell 语句可能切换当前库（`use <db>`），因此结果除了要回显的文本，还要带回切换后的库名：
/// 库只属于这个终端会话，不能写回连接配置。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoShellResult {
    /// 终端回显文本（已按 shell 风格排版，可能含 ANSI 颜色；换行用 `\n`）。
    pub output: String,
    /// 语句切换后的当前库；`None` 表示库未变化。
    pub database: Option<String>,
    /// 返回的文档数（仅 `find` / `aggregate` 这类文档结果有值）。
    ///
    /// 用于工作台卡片头展示「N 条」，也让「0 条」能和「没有输出」区分开。
    pub document_count: Option<u64>,
    /// 结果是否因条数或字节预算被截断；`true` 时 UI 要提示用户只看到了一部分。
    pub truncated: bool,
    /// 结构化的文档结果（`find` / `findOne` / `aggregate`）。
    ///
    /// 工作台据此渲染字段卡片；非文档语句（`show`、`count` 等）为空，UI 退回文本展示。
    /// 与 `output` 是同一批文档的两种形态：文本供 CLI 逐行回显与复制，结构化数据供卡片视图。
    pub documents: Vec<MongoDocument>,
}

/// 连接信息里的单个数据库条目。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MongoDatabaseStat {
    pub name: String,
    /// `sizeOnDisk`（字节）；权限不足时为 `None`。
    pub size_on_disk: Option<u64>,
    /// 库内是否没有数据。
    pub empty: bool,
}
