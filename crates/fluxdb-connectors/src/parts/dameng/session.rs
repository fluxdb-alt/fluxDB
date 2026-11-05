use dm_pure::{ClientConfig, DmClient, DmError, ExecuteResult, Param, Value};

/// 达梦连接句柄：驱动 client + 可能存在的 SSH 隧道。
///
/// 隧道必须与 client 同生命周期存活；Deref 让既有 helper（接收 `&mut DmClient`）
/// 无需改动即可继续使用。
struct DmSession {
    client: DmClient,
    _tunnel: Option<SshTunnel>,
}

impl std::ops::Deref for DmSession {
    type Target = DmClient;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl std::ops::DerefMut for DmSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

/// 建立一条 DM8 普通连接。驱动是同步实现，调用方按需在短生命周期线程中执行。
fn dm_connect(config: &ConnectionConfig) -> fluxdb_core::Result<DmSession> {
    let profile = dm_profile(config)?;
    let (mut host, mut port) = profile.dial_endpoint();
    // SSH 隧道：先建本地转发，再把拨号目标改成 127.0.0.1:local_port。
    let tunnel = if profile.ssh.enabled {
        let opened = open_tunnel_for_profile(
            &profile.ssh,
            (&host, port),
            profile.advanced.connect_timeout_secs,
        )?;
        host = "127.0.0.1".into();
        port = opened.local_port;
        Some(opened)
    } else {
        None
    };
    let connect_timeout = Duration::from_secs(u64::from(
        profile.advanced.connect_timeout_secs.max(1),
    ));
    let io_timeout = if profile.advanced.query_timeout_secs > 0 {
        Duration::from_secs(u64::from(profile.advanced.query_timeout_secs))
    } else {
        connect_timeout
    };
    let mut client = DmClient::with_config(ClientConfig {
        host,
        port,
        user: profile.basic.username.trim().to_string(),
        password: profile.password().unwrap_or("").to_string(),
        app_name: if profile.advanced.application_name.trim().is_empty() {
            "FluxDB".into()
        } else {
            profile.advanced.application_name.trim().to_string()
        },
        timeout: io_timeout,
        encoding: (!profile.advanced.encoding.trim().is_empty())
            .then(|| profile.advanced.encoding.trim().to_string()),
        ..ClientConfig::default()
    });
    client
        .connect()
        .map_err(|error| dm_error("建立达梦连接失败", error))?;
    // DM8 的 SET SCHEMA / ALTER SESSION 在当前驱动连接上会触发服务端资源错误；
    // 所有业务 SQL 已显式使用 schema 限定，因此不在建连阶段切换会话 schema。
    Ok(DmSession {
        client,
        _tunnel: tunnel,
    })
}

fn dm_profile(config: &ConnectionConfig) -> fluxdb_core::Result<&fluxdb_core::DamengConnectionProfile> {
    if config.kind != DatabaseKind::Dameng {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    config
        .dameng_profile
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "达梦连接档案缺失"))
}

fn dm_error(prefix: &str, error: DmError) -> Error {
    let kind = if matches!(error.code, -6852 | -6001) {
        ErrorKind::Connection
    } else {
        ErrorKind::Query
    };
    Error::new(kind, format!("{prefix}：{error}"))
}

fn dm_execute(
    client: &mut DmClient,
    sql: &str,
    params: &[Param],
) -> fluxdb_core::Result<ExecuteResult> {
    client
        .execute(sql, params)
        .map_err(|error| dm_error("达梦执行失败", error))
}

fn dm_value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::TinyInt(value) => Some(value.to_string()),
        Value::SmallInt(value) => Some(value.to_string()),
        Value::Int(value) => Some(value.to_string()),
        Value::BigInt(value) => Some(value.to_string()),
        Value::Real(value) => Some(value.to_string()),
        Value::Double(value) => Some(value.to_string()),
        Value::Decimal(value) | Value::Str(value) | Value::Interval(value) => Some(value.clone()),
        Value::Date(value) => Some(value.to_string()),
        Value::Time(value) => Some(value.to_string()),
        Value::DateTime(value) => Some(value.to_string()),
        Value::Bytes(_) | Value::Raw(_) => None,
    }
}

fn dm_value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::TinyInt(value) => Some(i64::from(*value)),
        Value::SmallInt(value) => Some(i64::from(*value)),
        Value::Int(value) => Some(i64::from(*value)),
        Value::BigInt(value) => Some(*value),
        Value::Decimal(value) | Value::Str(value) => value.parse().ok(),
        _ => value.as_i64(),
    }
}

fn dm_value_u64(value: &Value) -> Option<u64> {
    dm_value_i64(value).and_then(|value| u64::try_from(value).ok())
}

/// 元数据视图里的 NUMBER 列常以文本返回（如 `DATA_LENGTH='80'`），
/// 不应依赖驱动是否为该列选择了整数编码。
fn dm_meta_u64(value: &Value) -> Option<u64> {
    dm_value_u64(value).or_else(|| dm_value_text(value)?.trim().parse().ok())
}

fn dm_meta_i64(value: &Value) -> Option<i64> {
    dm_value_i64(value).or_else(|| dm_value_text(value)?.trim().parse().ok())
}

/// 驱动值转单元格。按值消费：`Value::Bytes` 动辄几十 MB，clone 一次等于把峰值翻倍。
fn dm_cell_value(value: Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(value) => CellValue::Bool(value),
        Value::TinyInt(value) => CellValue::I64(i64::from(value)),
        Value::SmallInt(value) => CellValue::I64(i64::from(value)),
        Value::Int(value) => CellValue::I64(i64::from(value)),
        Value::BigInt(value) => CellValue::I64(value),
        Value::Real(value) => {
            if value.is_finite() {
                CellValue::F64(f64::from(value))
            } else {
                CellValue::Text(value.to_string())
            }
        }
        Value::Double(value) => {
            if value.is_finite() {
                CellValue::F64(value)
            } else {
                CellValue::Text(value.to_string())
            }
        }
        Value::Decimal(value) | Value::Str(value) | Value::Interval(value) => {
            CellValue::Text(value)
        }
        Value::Bytes(value) | Value::Raw(value) => CellValue::Bytes(value),
        Value::Date(value) => CellValue::Text(value.to_string()),
        Value::Time(value) => CellValue::Text(value.to_string()),
        Value::DateTime(value) => CellValue::Text(value.to_string()),
    }
}

fn dm_param(value: &CellValue) -> Param {
    match value {
        CellValue::Null => Param::Null,
        CellValue::Bool(value) => Param::Bool(*value),
        CellValue::I64(value) => Param::Int(*value),
        CellValue::F64(value) => Param::Float(*value),
        CellValue::Text(value) | CellValue::Json(value) => Param::Str(value.clone()),
        // 截断文本只作筛选前缀；写入前已由 `validate_data_changes` 拒绝。
        CellValue::TruncatedText { preview, .. } => Param::Str(preview.clone()),
        // 大字段只按预览前缀参与筛选；写入前已由 `validate_data_changes` 拒绝。
        CellValue::LargeValue(large) => Param::Str(large.preview.clone()),
        CellValue::Bytes(value) => Param::Bytes(value.clone()),
        CellValue::BinarySummary(_) => Param::Null,
    }
}

fn dm_column(value: &dm_pure::Column, primary_key: bool) -> Column {
    let type_name = if value.prec > 0 {
        match value.type_name.to_ascii_uppercase().as_str() {
            "VARCHAR" | "VARCHAR2" | "CHAR" => {
                Some(format!("{}({})", value.type_name, value.prec))
            }
            "DECIMAL" | "NUMERIC" => Some(format!(
                "{}({},{})",
                value.type_name, value.prec, value.scale
            )),
            _ => Some(value.type_name.clone()),
        }
    } else {
        Some(value.type_name.clone())
    };
    Column {
        name: value.name.clone(),
        type_name,
        nullable: value.nullable,
        primary_key,
        comment: None,
    }
}

/// 结果行转单元格，并按列宽补 `NULL`：驱动返回的行长度可能短于描述列数。
///
/// 按值消费：`Value::Bytes` 动辄几十 MB，clone 一次等于把峰值翻倍。
fn dm_page_values(row: Vec<Value>, width: usize) -> Vec<CellValue> {
    let mut values = row
        .into_iter()
        .take(width)
        .map(dm_cell_value)
        .collect::<Vec<_>>();
    values.resize(width, CellValue::Null);
    values
}

fn dm_qualified_table(schema: &str, table: &str) -> String {
    format!(
        "{}.{}",
        fluxdb_core::DamengDialect.quote_identifier(schema),
        fluxdb_core::DamengDialect.quote_identifier(table)
    )
}

fn dm_request_schema(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    path.schema
        .clone()
        .filter(|schema| !schema.trim().is_empty())
        .or_else(|| {
            config
                .dameng_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .filter(|schema| !schema.trim().is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少达梦 schema 上下文"))
}
