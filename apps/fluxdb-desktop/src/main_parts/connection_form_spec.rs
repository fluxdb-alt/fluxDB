// 连接表单的数据库扩展点。
//
// 表单的字段清单、结构化档案的组装与回填此前按数据库类型散落在
// `connection_fields`、`new_connection_draft`、`from_config` 三处。
// 本模块把它们收敛到 `ConnectionFormSpec`：每种数据库一个实现，
// 新增数据库只需登记一个实现，不必再改上述三处。
//
// 字段控件本身的渲染仍由 `connection_dialog.rs` 负责；这里只描述
// “有哪些字段”和“档案怎么在表单与配置之间搬运”。

/// 一种数据库的连接表单行为。
pub(crate) trait ConnectionFormSpec: Send + Sync {
    /// 「连接信息」页展示的字段顺序。
    fn connection_fields(&self) -> &'static [ConnectionField];

    /// 把当前表单值组装进连接草稿的对应档案字段。
    ///
    /// 未启用结构化档案的数据库（如 SQLite）保持草稿原样。
    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft);

    /// 用已保存的连接回填表单。
    ///
    /// 结构化档案是事实来源；历史连接缺档案时按扁平 options 迁移。
    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm);
}

/// MySQL / TiDB / MariaDB 共用同一套表单与档案。
struct MysqlFormSpec;

impl ConnectionFormSpec for MysqlFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        draft.mysql_profile = Some(form.build_mysql_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.mysql_profile.clone().or_else(|| {
            Some(fluxdb_core::MysqlConnectionProfile::from_options(
                &config.options,
            ))
        });
        if let Some(profile) = profile {
            form.apply_mysql_profile(&profile);
        }
    }
}

struct PostgresFormSpec;

impl ConnectionFormSpec for PostgresFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::PgDefaultSchema,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        // PostgreSQL 连接器要求档案存在，缺它无法拨号。
        draft.postgres_profile = Some(form.build_postgres_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.postgres_profile.clone().or_else(|| {
            Some(fluxdb_core::PostgresConnectionProfile::from_options(
                &config.options,
            ))
        });
        if let Some(profile) = profile {
            form.apply_postgres_profile(&profile);
        }
    }
}

struct RedisFormSpec;

impl ConnectionFormSpec for RedisFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        // URL 参数只由 MySQL 系连接器消费，Redis 不提供入口。
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        draft.redis_profile = Some(form.build_redis_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.redis_profile.clone().or_else(|| {
            Some(fluxdb_core::RedisConnectionProfile::from_options(
                &config.options,
            ))
        });
        let Some(profile) = profile else {
            return;
        };
        form.apply_profile(&profile);
        // 档案是事实来源，其派生的 host/port/账号覆盖扁平 endpoint 值。
        if !profile.basic.host.is_empty() {
            form.host = profile.basic.host.clone();
        }
        form.port = profile.basic.port.to_string();
        form.force_standalone = profile.topology.force_standalone;
        form.ai_key_prefixes = config
            .options
            .get("ai_key_prefixes")
            .cloned()
            .unwrap_or_default();
        // 旧档案把哨兵/集群目标放在节点列表里，界面只剩单个主机/端口：
        // 用实际拨号端点回填，保证显示的地址就是真正会连的地址。
        if !matches!(
            profile.topology.kind,
            fluxdb_core::RedisTopologyKind::Standalone
        ) {
            let (dial_host, dial_port) = profile.dial_endpoint();
            if !dial_host.trim().is_empty() {
                form.host = dial_host;
                form.port = dial_port.to_string();
            }
        }
    }
}

struct DamengFormSpec;

impl ConnectionFormSpec for DamengFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::DamengSchema,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        draft.dameng_profile = Some(form.build_dameng_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.dameng_profile.clone().unwrap_or_else(|| {
            fluxdb_core::DamengConnectionProfile::from_options(&config.options)
        });
        form.dameng_schema = profile.basic.schema.clone();
        // 高级项必须随档案回填：缺一回填，编辑保存就会重置超时/应用名。
        form.dameng_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        form.dameng_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        form.dameng_application_name = profile.advanced.application_name.clone();
        form.dameng_encoding = profile.advanced.encoding.clone();
        form.apply_ssh_tunnel_profile(&profile.ssh);
    }
}

struct SqlServerFormSpec;

impl ConnectionFormSpec for SqlServerFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::SqlServerSchema,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        // SQL Server 档案是拨号、加密模式与 schema 作用域的事实来源；
        // 密码仍在 storage 落盘前按连接 ID 写入加密凭据表。
        draft.sqlserver_profile = Some(form.build_sqlserver_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.sqlserver_profile.clone().unwrap_or_else(|| {
            fluxdb_core::SqlServerConnectionProfile::from_options(&config.options)
        });
        form.apply_sqlserver_profile(&profile);
    }
}

struct MongoFormSpec;

impl ConnectionFormSpec for MongoFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::MongoDefaultDb,
            ConnectionField::MongoAuthDb,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        // MongoDB 档案是拨号事实来源：缺它连接器只能退回 endpoint 拼 URI，会丢 SRV/认证库。
        draft.mongo_profile = Some(form.build_mongo_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config
            .mongo_profile
            .clone()
            .unwrap_or_else(|| fluxdb_core::MongoConnectionProfile::from_options(&config.options));
        form.apply_mongo_profile(&profile);
    }
}

struct OracleFormSpec;

impl ConnectionFormSpec for OracleFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        // Service Name 与 SID 都列出来：由「连接方式」下拉决定哪一个生效，
        // 隐藏另一半反而让用户不清楚自己填的是哪一种寻址。
        &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::OracleServiceName,
            ConnectionField::OracleSid,
            ConnectionField::OracleSchema,
        ]
    }

    fn build_into(&self, form: &NewConnectionForm, draft: &mut ConnectionDraft) {
        // Oracle 档案是拨号目标（Service Name / SID）、登录身份与 Wallet 的事实来源；
        // 密码与 Wallet 口令在落盘前由 storage 按连接 ID 写入加密凭据表。
        draft.oracle_profile = Some(form.build_oracle_profile());
    }

    fn apply_to_form(&self, config: &ConnectionConfig, form: &mut NewConnectionForm) {
        let profile = config.oracle_profile.clone().unwrap_or_else(|| {
            fluxdb_core::OracleConnectionProfile::from_options(&config.options)
        });
        form.apply_oracle_profile(&profile);
    }
}

struct SqliteFormSpec;

impl ConnectionFormSpec for SqliteFormSpec {
    fn connection_fields(&self) -> &'static [ConnectionField] {
        &[ConnectionField::Name, ConnectionField::SqlitePath]
    }

    fn build_into(&self, _form: &NewConnectionForm, _draft: &mut ConnectionDraft) {
        // SQLite 没有结构化档案，路径与只读标记直接落在 endpoint。
    }

    fn apply_to_form(&self, _config: &ConnectionConfig, _form: &mut NewConnectionForm) {
        // SQLite 的表单字段由 endpoint 回填，无需档案。
    }
}

static MYSQL_FORM_SPEC: MysqlFormSpec = MysqlFormSpec;
static POSTGRES_FORM_SPEC: PostgresFormSpec = PostgresFormSpec;
static REDIS_FORM_SPEC: RedisFormSpec = RedisFormSpec;
static DAMENG_FORM_SPEC: DamengFormSpec = DamengFormSpec;
static SQLSERVER_FORM_SPEC: SqlServerFormSpec = SqlServerFormSpec;
static MONGO_FORM_SPEC: MongoFormSpec = MongoFormSpec;
static ORACLE_FORM_SPEC: OracleFormSpec = OracleFormSpec;
static SQLITE_FORM_SPEC: SqliteFormSpec = SqliteFormSpec;

/// 按数据库类型取表单实现；MySQL 家族共用一份。
pub(crate) fn connection_form_spec(kind: DatabaseKind) -> &'static dyn ConnectionFormSpec {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => &MYSQL_FORM_SPEC,
        DatabaseKind::Postgres | DatabaseKind::Kingbase => &POSTGRES_FORM_SPEC,
        DatabaseKind::Redis => &REDIS_FORM_SPEC,
        DatabaseKind::Dameng => &DAMENG_FORM_SPEC,
        DatabaseKind::SqlServer => &SQLSERVER_FORM_SPEC,
        DatabaseKind::MongoDb => &MONGO_FORM_SPEC,
        DatabaseKind::Oracle => &ORACLE_FORM_SPEC,
        DatabaseKind::Sqlite => &SQLITE_FORM_SPEC,
    }
}
