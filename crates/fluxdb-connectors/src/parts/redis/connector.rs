const REDIS_DATABASE_COUNT: u32 = 16;

const REDIS_SCAN_COUNT: &str = "200";

/// 侧边栏展开一个 DB 时最多列出的 Key 数量。
const REDIS_SIDEBAR_KEY_LIMIT: usize = 1000;

#[derive(Clone, Debug, Default)]
pub struct RedisConnector {
    config: Option<ConnectionConfig>,
}

impl RedisConnector {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// 读取连接级运行概览（版本 / 内存 / CPU 采样），供底栏连接状态摘要展示。
    /// 参考 RedisInsight 数据库概览语义，与 key 详情无关。
    pub fn overview(&self) -> fluxdb_core::Result<ConnectionOverview> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 概览需要连接配置上下文",
            ));
        };
        let mut connection = redis_connect(config)?;
        redis_load_overview(&mut connection)
    }

    /// 新建一个 Key（对齐 RedisInsight AddKey）。`object` 是目标库的 `RedisDatabase` 路径，
    /// 其 `database` 字段决定落到哪个库；建 Key 前会 EXISTS 防覆盖，集合类需带至少一个元素，
    /// 应用可选 TTL。错误信息为中文，直接面向 UI 展示。
    pub fn add_key(&self, object: &ObjectPath, request: &RedisAddKeyRequest) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(ErrorKind::Connection, "Redis 新建 Key 需要连接配置上下文"));
        };
        redis_create_key(config, object, request)
    }

    /// 追加 Stream 条目；`maxlen` 非 None 时带上 `MAXLEN ~ n` 做近似裁剪
    /// （用 `~` 而不是精确裁剪：精确裁剪在大流上是 O(N)，会阻塞服务端）。
    pub fn add_stream_entry(
        &self,
        object: &ObjectPath,
        id: &str,
        fields: &[(String, String)],
        maxlen: Option<u64>,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Stream 修改需要连接配置上下文",
            ));
        };
        redis_xadd_stream_entry(config, object, id, fields, maxlen)
    }

    pub fn delete_stream_entry(&self, object: &ObjectPath, entry_id: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Stream 修改需要连接配置上下文",
            ));
        };
        redis_xdel_stream_entry(config, object, entry_id)
    }

    pub fn delete_set_member(&self, object: &ObjectPath, member: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Set 修改需要连接配置上下文",
            ));
        };
        redis_srem_set_member(config, object, member)
    }

    pub fn add_set_member(&self, object: &ObjectPath, member: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Set 修改需要连接配置上下文",
            ));
        };
        redis_sadd_set_member(config, object, member)
    }

    /// 分页查询 Set 成员：单次 SSCAN，游标透传；同时回传 SCARD 总数。
    pub fn load_set_members(
        &self,
        object: &ObjectPath,
        query: &str,
        cursor: &str,
        limit: usize,
    ) -> fluxdb_core::Result<RedisSetMemberPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Set 查询需要连接配置上下文",
            ));
        };
        redis_sscan_set_members(config, object, query, cursor, limit)
    }

    pub fn load_hash_fields(
        &self,
        object: &ObjectPath,
        query: &str,
        cursor: &str,
        limit: usize,
    ) -> fluxdb_core::Result<RedisHashFieldPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 查询需要连接配置上下文",
            ));
        };
        redis_hscan_hash_fields(config, object, query, cursor, limit)
    }

    /// 完整值弹框专用：HGET 拉取单字段完整原始值，不截断（>1MB 的值在表格中被截断标记替换）。
    pub fn load_hash_field_full(
        &self,
        object: &ObjectPath,
        field: &str,
    ) -> fluxdb_core::Result<Option<String>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 查询需要连接配置上下文",
            ));
        };
        if object.kind != ObjectKind::RedisKey {
            return Err(Error::new(ErrorKind::Internal, "Redis Hash 查询缺少 Key"));
        }
        let database = redis_path_database(object)?;
        let mut connection = redis_connect(config)?;
        redis_select(&mut connection, database)?;
        redis_load_exact_hash_field_full(&mut connection, object.name.as_str(), field)
    }

    /// 加载 Redis string / JSON 值详情：`full=false` 只取前 `REDIS_STRING_MAX_LENGTH` 字节预览
    /// （STRLEN + GETRANGE，轻量，避免把大 value 整体拉回内存），`full=true` 取完整值（GET）。
    ///
    /// 服务端真实类型通过 `TYPE` 重新判定：`string` 走 STRLEN/GETRANGE/GET，
    /// `rejson-rl`/`json` 走 `JSON.GET`（JSON 文档体积通常有限，始终整取、`loaded_all` 恒为 true）。
    pub fn load_string_value(
        &self,
        object: &ObjectPath,
        full: bool,
    ) -> fluxdb_core::Result<RedisStringValue> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis String 查询需要连接配置上下文",
            ));
        };
        if object.kind != ObjectKind::RedisKey {
            return Err(Error::new(ErrorKind::Internal, "Redis String 查询缺少 Key"));
        }
        let database = redis_path_database(object)?;
        let mut connection = redis_connect(config)?;
        redis_select(&mut connection, database)?;
        redis_load_string_value(&mut connection, object.name.as_str(), full)
    }

    /// 下载 Redis string / JSON 值，返回原始字节，供导出文件使用。
    ///
    /// 与 [`RedisConnector::load_string_value`] 不同，这里始终整取且返回未做 UTF-8 容错处理
    /// 的原始二进制字节，保证下载文件与 Redis 中存储内容逐字节一致。
    /// 服务端真实类型通过 `TYPE` 重新判定：`string` 走 `GET`，`rejson-rl`/`json` 走 `JSON.GET`。
    pub fn download_string_value(&self, object: &ObjectPath) -> fluxdb_core::Result<Vec<u8>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis String 下载需要连接配置上下文",
            ));
        };
        if object.kind != ObjectKind::RedisKey {
            return Err(Error::new(ErrorKind::Internal, "Redis String 下载缺少 Key"));
        }
        let database = redis_path_database(object)?;
        let mut connection = redis_connect(config)?;
        redis_select(&mut connection, database)?;
        redis_download_string_value(&mut connection, object.name.as_str())
    }

    pub fn load_zset_members(
        &self,
        object: &ObjectPath,
        query: &str,
        cursor: &str,
        limit: usize,
    ) -> fluxdb_core::Result<RedisZSetMemberPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis ZSet 查询需要连接配置上下文",
            ));
        };
        redis_load_zset_members(config, object, query, cursor, limit)
    }

    /// 分页查询 Stream 条目：单次 XREVRANGE（从新到旧），游标为上一页多取出的那条 ID。
    pub fn load_stream_entries(
        &self,
        object: &ObjectPath,
        range: RedisStreamRange,
        cursor: &str,
        limit: usize,
    ) -> fluxdb_core::Result<RedisStreamEntryPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Stream 查询需要连接配置上下文",
            ));
        };
        redis_load_stream_entries(config, object, range, cursor, limit)
    }

    /// 读取 Stream 的消费者组概览（只读）。服务端不支持 XINFO GROUPS 时返回空列表。
    pub fn load_stream_groups(
        &self,
        object: &ObjectPath,
    ) -> fluxdb_core::Result<Vec<RedisStreamGroup>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Stream 查询需要连接配置上下文",
            ));
        };
        redis_load_stream_groups(config, object)
    }

    /// 分页查询 List 元素；`query` 非空时按「下标跳转」读取单个元素（对齐 RedisInsight，见实现说明）。
    pub fn load_list_items(
        &self,
        object: &ObjectPath,
        query: &str,
        cursor: &str,
        limit: usize,
    ) -> fluxdb_core::Result<RedisListItemPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis List 查询需要连接配置上下文",
            ));
        };
        redis_load_list_items(config, object, query, cursor, limit)
    }

    /// 写入 Hash 字段值；`ttl` 决定该字段 TTL 是保留、清除还是改成指定秒数。
    pub fn set_hash_field(
        &self,
        object: &ObjectPath,
        field: &str,
        value: &str,
        ttl: RedisHashFieldTtl,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 修改需要连接配置上下文",
            ));
        };
        redis_hset_hash_field(config, object, field, value, ttl, false)
    }

    /// 写入 Hash 字段值，来源为「完整值弹框」：放行超过 1MB 被截断标记的完整值写回，
    /// 但二进制占位符（U+FFFC）仍拒绝。行内表格编辑走 [`Self::set_hash_field`]（default 拒截断）。
    pub fn set_hash_field_raw(
        &self,
        object: &ObjectPath,
        field: &str,
        value: &str,
        ttl: RedisHashFieldTtl,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 修改需要连接配置上下文",
            ));
        };
        redis_hset_hash_field(config, object, field, value, ttl, true)
    }

    /// 读取 Redis 服务端版本，作为字段级 TTL 等能力开关。
    ///
    /// 发 `INFO server` 并解析 `redis_version`；解析失败返回 `Ok(None)`（能力未知），
    /// 连接/协议错误正常抛错。字段级 TTL（HEXPIRE/HPEXPIRE）自 Redis 7.4 起可用。
    pub fn server_version(&self) -> fluxdb_core::Result<Option<RedisServerVersion>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 版本读取需要连接配置上下文",
            ));
        };
        let mut connection = redis_connect(config)?;
        let version = match connection.command(&["INFO", "server"])? {
            RedisValue::Bulk(Some(bytes)) => redis_info_text_map(&redis_text_from_bytes(bytes))
                .get("redis_version")
                .and_then(|raw| RedisServerVersion::parse(raw)),
            _ => None,
        };
        Ok(version)
    }

    /// 惰性补齐一批键的元信息（类型 / 预览 / 大小 / TTL）。
    /// 配合 [`Connector::load_data`]（RedisDb 分支只返回键名）实现「先键名、后懒加载元信息」的
    /// 两段式，对齐 RedisInsight 的 getMetadata：首屏只拿键名，用户可见/选中某批行后再按需批量补。
    /// `object` 只需带连接与库号（键名走 `keys` 入参，不依赖 object.name）。
    pub fn load_key_metadata(
        &self,
        object: &ObjectPath,
        keys: &[String],
    ) -> fluxdb_core::Result<DataPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Key 元信息懒加载需要连接配置上下文",
            ));
        };
        redis_key_metadata(config, object, keys)
    }

    /// 仅更新指定 Hash 字段的 TTL，不触碰字段值（纯 TTL 命令，Redis 7.4+ 字段级 TTL）。
    ///
    /// 与 [`Self::set_hash_field`] 不同：后者先 `HSET` 写 value 再补 TTL，会重写整段值，
    /// 对 >1MB 被截断的字段是危险的；本方法只发 `HPEXPIRE`/`HPERSIST`，绝不回写 value。
    pub fn set_hash_field_ttl(
        &self,
        object: &ObjectPath,
        field: &str,
        ttl: RedisHashFieldTtl,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash TTL 修改需要连接配置上下文",
            ));
        };
        redis_hset_hash_field_ttl(config, object, field, ttl)
    }

    pub fn rename_hash_field(
        &self,
        object: &ObjectPath,
        old_field: &str,
        new_field: &str,
        value: &str,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 修改需要连接配置上下文",
            ));
        };
        redis_rename_hash_field(config, object, old_field, new_field, value)
    }

    pub fn delete_hash_field(&self, object: &ObjectPath, field: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Hash 修改需要连接配置上下文",
            ));
        };
        redis_hdel_hash_field(config, object, field)
    }

    pub fn add_zset_member(&self, object: &ObjectPath, member: &str, score: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis ZSet 修改需要连接配置上下文",
            ));
        };
        redis_zadd_member(config, object, member, score)
    }

    pub fn update_zset_score(&self, object: &ObjectPath, member: &str, score: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis ZSet 修改需要连接配置上下文",
            ));
        };
        redis_zadd_member(config, object, member, score)
    }

    pub fn delete_zset_member(&self, object: &ObjectPath, member: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis ZSet 修改需要连接配置上下文",
            ));
        };
        redis_zrem_member(config, object, member)
    }

    pub fn push_list_items(&self, object: &ObjectPath, items: &[String], head: bool) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis List 修改需要连接配置上下文",
            ));
        };
        redis_push_list_items(config, object, items, head)
    }

    pub fn set_list_item(
        &self,
        object: &ObjectPath,
        index: usize,
        expected_old: Option<&str>,
        value: &str,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis List 修改需要连接配置上下文",
            ));
        };
        redis_set_list_item(config, object, index, expected_old, value)
    }

    pub fn delete_list_item(
        &self,
        object: &ObjectPath,
        index: usize,
        expected_old: Option<&str>,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis List 修改需要连接配置上下文",
            ));
        };
        redis_delete_list_item(config, object, index, expected_old)
    }

    /// 从 List 头部/尾部按数量弹出元素（LPOP/RPOP，对齐 RedisInsight Remove elements）。
    pub fn pop_list_items(
        &self,
        object: &ObjectPath,
        head: bool,
        count: usize,
    ) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis List 修改需要连接配置上下文",
            ));
        };
        redis_pop_list_items(config, object, head, count)
    }
}

impl Connector for RedisConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Redis
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?config.id,
            connection = ?config.name,
            endpoint = ?config.endpoint,
            "Redis 连接测试开始"
        );
        if config.kind != DatabaseKind::Redis {
            return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
        }
        let mut connection = redis_connect(config)?;
        redis_expect_ok(connection.command(&["PING"])?)
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 对象浏览需要连接配置上下文",
            ));
        };
        let mut connection = redis_connect(config)?;

        match path {
            None => redis_list_databases(config.id, &mut connection),
            Some(path) if path.kind == ObjectKind::RedisDb => {
                let database = redis_path_database(path)?;
                redis_select(&mut connection, database)?;
                // 侧边栏只用于浏览，不做无上限全量 SCAN——
                // 百万级 key 的库会把整个界面卡死。超出上限的部分请用 Key 列表搜索。
                let keys = redis_scan_keys(&mut connection, REDIS_SIDEBAR_KEY_LIMIT)?;
                Ok(keys
                    .into_iter()
                    .map(|key| ObjectSummary {
                        path: ObjectPath {
                            connection_id: path.connection_id,
                            database: Some(database.to_string()),
                            schema: None,
                            name: key,
                            kind: ObjectKind::RedisKey,
                        },
                        rows: None,
                        modified_at: None,
                        comment: None,
                    })
                    .collect())
            }
            Some(path) if path.kind == ObjectKind::RedisKey => Ok(Vec::new()),
            Some(_) => Err(Error::new(ErrorKind::Unsupported, "Redis 仅支持 DB 和 Key 浏览")),
        }
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        _sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 数据读取需要连接配置上下文",
            ));
        };
        redis_load_data(config, path, offset, limit, filters)
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 数据修改需要连接配置上下文",
            ));
        };
        redis_apply_changes(config, changes)
    }

    fn execute(&self, _: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持 Redis 命令执行"))
    }
}
