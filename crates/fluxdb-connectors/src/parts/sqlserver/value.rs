use chrono::FixedOffset;

fn sqlserver_query_column(column: &tiberius::Column) -> Column {
    Column {
        name: column.name().to_string(),
        type_name: Some(sqlserver_column_type_name(column.column_type()).to_string()),
        nullable: true,
        primary_key: false,
        comment: None,
    }
}

fn sqlserver_column_type_name(column_type: tiberius::ColumnType) -> &'static str {
    use tiberius::ColumnType;
    match column_type {
        ColumnType::Null => "sql_variant",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Datetime4 | ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Float4 => "real",
        ColumnType::Float8 | ColumnType::Floatn => "float",
        ColumnType::Money | ColumnType::Money4 => "money",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Intn => "int",
        ColumnType::Decimaln | ColumnType::Numericn => "decimal",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => "varbinary",
        ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::Text => "varchar",
        ColumnType::NVarchar | ColumnType::NChar | ColumnType::NText => "nvarchar",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "geometry",
        ColumnType::SSVariant => "sql_variant",
    }
}

/// sql_variant 的内部基础类型由驱动按原始字节返回；这里渲染成可读文本，
/// 不把它伪装成普通二进制列，也不声称已解析内部类型。
fn sqlserver_sql_variant_label(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "<sql_variant: empty>".to_string();
    }
    let preview = bytes
        .iter()
        .take(64)
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    let suffix = if bytes.len() > 64 { "…" } else { "" };
    format!("<sql_variant {} bytes: 0x{preview}{suffix}>", bytes.len())
}

fn sqlserver_row(row: tiberius::Row) -> Row {
    let columns = row.columns().to_vec();
    let values = row
        .into_iter()
        .enumerate()
        .map(|(index, data)| sqlserver_cell_value(data, columns.get(index)))
        .collect::<Vec<_>>();
    Row { values }
}

/// 逐 TDS 类型转换，绝不通过 `row.get` 的 panic 路径猜类型。decimal/money
/// 返回精确文本；空间/层次类型按 tiberius 默认编码无法安全解释，返回明确的
/// 不支持文本而不是崩溃。
fn sqlserver_cell_value(
    data: tiberius::ColumnData<'static>,
    column: Option<&tiberius::Column>,
) -> CellValue {
    use tiberius::ColumnData;
    match data {
        ColumnData::U8(value) => value.map_or(CellValue::Null, |value| CellValue::I64(i64::from(value))),
        ColumnData::I16(value) => value.map_or(CellValue::Null, |value| CellValue::I64(i64::from(value))),
        ColumnData::I32(value) => value.map_or(CellValue::Null, |value| CellValue::I64(i64::from(value))),
        ColumnData::I64(value) => value.map_or(CellValue::Null, CellValue::I64),
        ColumnData::F32(value) => value.map_or(CellValue::Null, |value| {
            if value.is_finite() {
                CellValue::F64(f64::from(value))
            } else {
                CellValue::Text(value.to_string())
            }
        }),
        ColumnData::F64(value) => value.map_or(CellValue::Null, |value| {
            if value.is_finite() {
                CellValue::F64(value)
            } else {
                CellValue::Text(value.to_string())
            }
        }),
        ColumnData::Bit(value) => value.map_or(CellValue::Null, CellValue::Bool),
        ColumnData::String(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(value.into_owned())
        }),
        ColumnData::Guid(value) => value.map_or(CellValue::Null, |value| CellValue::Text(value.to_string())),
        ColumnData::Binary(value) if sqlserver_is_variant_column(column) => {
            value.map_or(CellValue::Null, |value| {
                CellValue::Text(sqlserver_sql_variant_label(value.as_ref()))
            })
        }
        ColumnData::Binary(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Bytes(value.into_owned())
        }),
        ColumnData::Numeric(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(sqlserver_numeric_text(value))
        }),
        ColumnData::Xml(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(value.to_string())
        }),
        ColumnData::DateTime(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(sqlserver_chrono_datetime(sqlserver_datetime_from_tds(value)))
        }),
        ColumnData::SmallDateTime(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(sqlserver_chrono_datetime(sqlserver_small_datetime_from_tds(value)))
        }),
        ColumnData::Date(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(format!("{}", sqlserver_date_from_days(value.days())))
        }),
        ColumnData::Time(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(format!("{}", sqlserver_time_from_tds(value)))
        }),
        ColumnData::DateTime2(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(sqlserver_chrono_datetime(sqlserver_datetime2_from_tds(value)))
        }),
        ColumnData::DateTimeOffset(value) => value.map_or(CellValue::Null, |value| {
            CellValue::Text(sqlserver_datetimeoffset_from_tds(value).to_rfc3339())
        }),
    }
    .with_unsupported_type(column)
}

/// 精确渲染 TDS DECIMAL/NUMERIC。
///
/// tiberius 自带 `Display` 对负小数会输出 `-1.-2500`（整数部分和小数部分
/// 各带一次负号），因此必须从原始 i128 与 scale 重建文本，避免负数精度丢失。
fn sqlserver_numeric_text(value: tiberius::numeric::Numeric) -> String {
    let scale = usize::from(value.scale());
    let raw = value.value();
    let digits = raw.unsigned_abs().to_string();
    let mut text = if scale == 0 {
        digits
    } else if digits.len() <= scale {
        format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
    } else {
        let split = digits.len() - scale;
        format!("{}.{}", &digits[..split], &digits[split..])
    };
    if raw < 0 {
        text.insert(0, '-');
    }
    text
}

fn sqlserver_is_variant_column(column: Option<&tiberius::Column>) -> bool {
    column.is_some_and(|column| column.column_type() == tiberius::ColumnType::SSVariant)
}

/// 空间/UDT 在 tiberius 中以原始二进制返回；当前连接器没有空间协议解码器，
/// 明确提示为不支持，不让整个结果集失败。
trait SqlServerCellValueExt {
    fn with_unsupported_type(self, column: Option<&tiberius::Column>) -> CellValue;
}

impl SqlServerCellValueExt for CellValue {
    fn with_unsupported_type(self, column: Option<&tiberius::Column>) -> CellValue {
        match column.map(tiberius::Column::column_type) {
            Some(tiberius::ColumnType::Udt) => match self {
                CellValue::Bytes(bytes) => CellValue::Text(format!(
                    "<SQL Server UDT 不支持解码，原始 {} 字节>",
                    bytes.len()
                )),
                other => other,
            },
            _ => self,
        }
    }
}

fn sqlserver_date_from_days(days: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1, 1, 1)
        .and_then(|base| base.checked_add_days(chrono::Days::new(u64::from(days))))
        .unwrap_or(NaiveDate::MIN)
}

fn sqlserver_datetime_from_tds(value: tiberius::time::DateTime) -> NaiveDateTime {
    let date = NaiveDate::from_ymd_opt(1900, 1, 1)
        .and_then(|base| base.checked_add_signed(chrono::Duration::days(i64::from(value.days()))))
        .unwrap_or(NaiveDate::MIN);
    let nanos = i64::from(value.seconds_fragments()) * 1_000_000_000 / 300;
    let time = NaiveTime::MIN + chrono::Duration::nanoseconds(nanos);
    NaiveDateTime::new(date, time)
}

fn sqlserver_small_datetime_from_tds(value: tiberius::time::SmallDateTime) -> NaiveDateTime {
    let date = NaiveDate::from_ymd_opt(1900, 1, 1)
        .and_then(|base| base.checked_add_signed(chrono::Duration::days(i64::from(value.days()))))
        .unwrap_or(NaiveDate::MIN);
    let minutes = u32::from(value.seconds_fragments());
    let time = NaiveTime::from_num_seconds_from_midnight_opt(minutes * 60, 0).unwrap_or(NaiveTime::MIN);
    NaiveDateTime::new(date, time)
}

fn sqlserver_datetime2_from_tds(value: tiberius::time::DateTime2) -> NaiveDateTime {
    let date = sqlserver_date_from_days(value.date().days());
    let time = sqlserver_time_from_tds(value.time());
    NaiveDateTime::new(date, time)
}

fn sqlserver_time_from_tds(value: tiberius::time::Time) -> NaiveTime {
    let scale = u32::from(value.scale());
    let nanos = value.increments().saturating_mul(10u64.pow(9u32.saturating_sub(scale)));
    let seconds = nanos / 1_000_000_000;
    let sub_nanos = (nanos % 1_000_000_000) as u32;
    NaiveTime::from_num_seconds_from_midnight_opt(seconds as u32, sub_nanos).unwrap_or(NaiveTime::MIN)
}

fn sqlserver_datetimeoffset_from_tds(value: tiberius::time::DateTimeOffset) -> DateTime<FixedOffset> {
    let naive = sqlserver_datetime2_from_tds(value.datetime2());
    let offset = FixedOffset::east_opt(i32::from(value.offset()) * 60).unwrap_or(FixedOffset::east_opt(0).expect("zero offset"));
    DateTime::from_naive_utc_and_offset(naive, offset)
}

fn sqlserver_chrono_datetime(value: NaiveDateTime) -> String {
    value.format("%Y-%m-%d %H:%M:%S%.f").to_string()
}

/// 按 CellValue 的真实类型绑定写入参数。`BinarySummary` 不是完整值，
/// 只能用于定位（NULL 或已加载的预览），不能用于写入语义。
fn sqlserver_bind_cell(query: &mut tiberius::Query<'_>, value: &CellValue) -> fluxdb_core::Result<()> {
    match value {
        CellValue::Null => query.bind(Option::<&str>::None),
        CellValue::Bool(value) => query.bind(*value),
        CellValue::I64(value) => query.bind(*value),
        CellValue::F64(value) if value.is_finite() => query.bind(*value),
        CellValue::F64(_) => {
            return Err(Error::new(ErrorKind::Query, "非有限浮点数不能写入 SQL Server"));
        }
        CellValue::Text(value) | CellValue::Json(value) => query.bind(value.clone()),
        // 预览不是完整值：绑进去等于用片段覆盖真值。
        CellValue::TruncatedText { .. } => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "文本只加载了预览（截断的 CLOB），不能作为写入值",
            ));
        }
        // 大字段只有预览：绑进去等于用片段覆盖真值。
        CellValue::LargeValue(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "超大字段只加载了预览，不能作为写入值；请先按内容引用读取完整值",
            ));
        }
        CellValue::Bytes(value) => query.bind(value.clone()),
        CellValue::BinarySummary(summary) if summary.is_null => query.bind(Option::<&str>::None),
        CellValue::BinarySummary(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "二进制字段尚未完整加载，不能作为写入值",
            ));
        }
    }
    Ok(())
}

fn sqlserver_bound_query<'a>(
    sql: &'a str,
    values: &'a [CellValue],
) -> fluxdb_core::Result<tiberius::Query<'a>> {
    let mut query = tiberius::Query::new(sql);
    for value in values {
        sqlserver_bind_cell(&mut query, value)?;
    }
    Ok(query)
}

#[cfg(test)]
mod sqlserver_numeric_tests {
    use super::sqlserver_numeric_text;
    use tiberius::numeric::Numeric;

    #[test]
    fn negative_decimal_keeps_scale_and_single_sign() {
        assert_eq!(
            sqlserver_numeric_text(Numeric::new_with_scale(-12_500, 4)),
            "-1.2500"
        );
    }

    #[test]
    fn decimal_smaller_than_one_is_zero_padded() {
        assert_eq!(
            sqlserver_numeric_text(Numeric::new_with_scale(5, 2)),
            "0.05"
        );
        assert_eq!(
            sqlserver_numeric_text(Numeric::new_with_scale(0, 4)),
            "0.0000"
        );
    }

    #[test]
    fn scale_zero_stays_integer() {
        assert_eq!(
            sqlserver_numeric_text(Numeric::new_with_scale(-42, 0)),
            "-42"
        );
    }
}
