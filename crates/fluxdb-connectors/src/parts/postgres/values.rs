// PostgreSQL 值转换与类型矩阵（T09）。
//
// 与 mysql/sqlite 的「逐型尝试」不同，PG 按列的驱动类型名（`Column.type_name`，来自
// RowDescription 的真实 type name）分类投影，遵循设计 7.1 类型矩阵：
// - 整数(bool/int2/int4/int8) → I64/Bool，不把空字符串当 NULL；
// - float4/float8 有限值 → F64，NaN/±Infinity 保留类型化文本（JSON 导出不能生成非法数字）；
// - numeric/decimal/money → 精确十进制文本（不经 f64，避免精度丢失；money 按服务端文本）；
// - text/varchar/char/name/enum/domain/uuid/inet/bit/… → 保真 Text；
// - date/time/timestamp/timestz/interval → 保真文本（时区/BC/无穷不强塞普通日期控件）；
// - json/jsonb → Json 原文本；
// - bytea → Bytes（表浏览走摘要投影，详情才完整读取）。
//
// 复杂类型（数组/range/composite/未知）由数据读 SQL 以 `::text` 投影后按文本解码；
// 这里对驱动原生可解类型直接解码，保证 execute/数据页两类路径都能拿到类型化 CellValue。

// CellValue / Column / is_binary_type_name 已由 crate 根（lib.rs）导入本作用域。
// chrono 为直接依赖：pg 的 date/time/timestamp 以 chrono 解码成保真文本（与设计 7.1 矩阵一致）。
use chrono;

/// PG 类型基名：剥 `(modifiers)` 与数组后缀 `[]`，供分类匹配。
pub(crate) fn pg_type_base(type_name: &str) -> &str {
    let base = type_name.split('(').next().unwrap_or(type_name).trim();
    base.strip_suffix("[]").unwrap_or(base).trim()
}

/// 整数列宽度：绑定参数需按列真实宽度选型（i64→int8，对 int4 列会序列化失败）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PgIntKind {
    Small,
    Integer,
    Big,
}

/// 由列类型基名推断整数宽度；非整数列返回 `None`。
pub(crate) fn pg_int_kind(base: &str) -> Option<PgIntKind> {
    match base {
        "int2" | "smallint" | "smallserial" => Some(PgIntKind::Small),
        "int4" | "integer" | "serial" => Some(PgIntKind::Integer),
        "int8" | "bigint" | "bigserial" => Some(PgIntKind::Big),
        _ => None,
    }
}

/// 取列单元的整数宽度（未知类型按最宽处理，由底层 ToSql 默认）。
pub(crate) fn pg_column_int_kind(column: &Column) -> Option<PgIntKind> {
    let type_name = column.type_name.as_deref().unwrap_or("");
    pg_int_kind(pg_type_base(type_name))
}

/// 是否整数（int2/int4/int8 及其别名）。
fn is_integer_type(base: &str) -> bool {
    matches!(
        base,
        "int2"
            | "int4"
            | "int8"
            | "smallint"
            | "integer"
            | "bigint"
            | "serial"
            | "bigserial"
            | "smallserial"
    )
}

/// 是否非有限浮点：返回类型化文本（保留 NaN/±Infinity，供编辑回写与 SQL 导出）。
fn float_non_finite_text(value: f64, is_float4: bool) -> Option<String> {
    if value.is_finite() {
        None
    } else if value.is_nan() {
        Some("NaN".to_string())
    } else {
        let sign = if value > 0.0 { "" } else { "-" };
        Some(format!(
            "'{sign}Infinity'::{}",
            if is_float4 { "float4" } else { "float8" }
        ))
    }
}

/// 单列值转换入口：按列的真实类型名分类解码。
pub(crate) fn pg_projected_cell_value(
    row: &tokio_postgres::Row,
    index: usize,
    column: &Column,
) -> CellValue {
    if let Ok(value) = row.try_get::<_, Option<String>>(index) {
        return pg_text_value(value.as_deref(), column).unwrap_or_else(|error| {
            tracing::error!(target: "fluxdb_connectors", column = %column.name, "PostgreSQL 值转换失败");
            CellValue::Text(format!("[{}]", error.message))
        });
    }
    let type_name = column.type_name.as_deref().unwrap_or("");
    // 数组/未知类型：按文本投影（数据读 SQL 已 ::text）。须在剥 `[]` 之前判断。
    if type_name.contains('[') || type_name.starts_with('_') {
        return text_cell(row, index);
    }
    let base = pg_type_base(type_name);

    if type_name == "bytea" || is_binary_type_name(type_name) {
        return row
            .try_get::<_, Option<Vec<u8>>>(index)
            .map(|v| v.map(CellValue::Bytes).unwrap_or(CellValue::Null))
            .unwrap_or(CellValue::Null);
    }

    if is_integer_type(base) {
        return int_cell(row, index);
    }

    match base {
        "bool" | "boolean" => row
            .try_get::<_, Option<bool>>(index)
            .map(|v| v.map(CellValue::Bool).unwrap_or(CellValue::Null))
            .unwrap_or(CellValue::Null),
        "float4" | "real" => float_cell::<f32>(row, index, true),
        "float8" | "double precision" => float_cell::<f64>(row, index, false),
        // numeric/decimal/money：精确十进制文本。tokio-postgres 无本地 decimal 解码，
        // 由数据读 SQL 对这三类列统一 `::text` 投影后按文本读取（保精度、money 按服务端 locale）。
        "numeric" | "decimal" | "money" => text_cell(row, index),
        "json" | "jsonb" => row
            .try_get::<_, Option<String>>(index)
            .map(|v| v.map(CellValue::Json).unwrap_or(CellValue::Null))
            .unwrap_or_else(|_| text_cell(row, index)),
        // 时间家族：以 chrono 保真文本；无法解码时回退原始文本（如 BC、infinity）。
        "date" => row
            .try_get::<_, Option<chrono::NaiveDate>>(index)
            .map(|v| {
                v.map(|d| CellValue::Text(d.format("%Y-%m-%d").to_string()))
                    .unwrap_or(CellValue::Null)
            })
            .unwrap_or_else(|_| text_cell(row, index)),
        "time" => row
            .try_get::<_, Option<chrono::NaiveTime>>(index)
            .map(|v| {
                v.map(|t| CellValue::Text(t.format("%H:%M:%S%.6f").to_string()))
                    .unwrap_or(CellValue::Null)
            })
            .unwrap_or_else(|_| text_cell(row, index)),
        "timestamp" => row
            .try_get::<_, Option<chrono::NaiveDateTime>>(index)
            .map(|v| {
                v.map(|t| {
                    CellValue::Text(t.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
                })
                .unwrap_or(CellValue::Null)
            })
            .unwrap_or_else(|_| text_cell(row, index)),
        "timestamptz" => row
            .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index)
            .map(|v| {
                v.map(|t| {
                    CellValue::Text(t.format("%Y-%m-%d %H:%M:%S%.6f%:z").to_string())
                })
                .unwrap_or(CellValue::Null)
            })
            .unwrap_or_else(|_| text_cell(row, index)),
        // 其余（text/varchar/char/name/enum/domain/uuid/inet/cidr/macaddr/bit/varbit/xml/
        // interval/timetz/range/composite/…）按服务端文本保真投影。
        _ => text_cell(row, index),
    }
}

fn text_cell(row: &tokio_postgres::Row, index: usize) -> CellValue {
    row.try_get::<_, Option<String>>(index)
        .map(|v| v.map(CellValue::Text).unwrap_or(CellValue::Null))
        .unwrap_or_else(|_| {
            tracing::error!(target: "fluxdb_connectors", index, "PostgreSQL 未支持的二进制解码，禁止伪装 NULL");
            CellValue::Text("[无法解码，请刷新后重试]".to_string())
        })
}

fn int_cell(row: &tokio_postgres::Row, index: usize) -> CellValue {
    let value: Option<i64> = row
        .try_get(index)
        .or_else(|_| row.try_get::<_, Option<i32>>(index).map(|v| v.map(|x| x as i64)))
        .or_else(|_| {
            row.try_get::<_, Option<i16>>(index)
                .map(|v| v.map(|x| x as i64))
        })
        .unwrap_or(None);
    value.map(CellValue::I64).unwrap_or(CellValue::Null)
}

fn float_cell<T>(row: &tokio_postgres::Row, index: usize, is_float4: bool) -> CellValue
where
    T: Copy + Into<f64> + tokio_postgres::types::FromSqlOwned,
{
    // 按驱动原生类型解码（float4→f32、float8→f64），统一转为 f64 再判有限性。
    let value: Option<f64> = if is_float4 {
        row.try_get::<_, Option<f32>>(index)
            .map(|v| v.map(|x| x as f64))
            .unwrap_or(None)
    } else {
        row.try_get::<_, Option<f64>>(index).unwrap_or(None)
    };
    match value {
        Some(v) => match float_non_finite_text(v, is_float4) {
            Some(text) => CellValue::Text(text),
            None => CellValue::F64(v),
        },
        None => CellValue::Null,
    }
}

/// 文本协议与显式 ::text 投影共用转换；精确小数、时间、数组等保留服务器原文。
fn pg_text_value(value: Option<&str>, column: &Column) -> fluxdb_core::Result<CellValue> {
    let Some(value) = value else { return Ok(CellValue::Null); };
    let type_name = column.type_name.as_deref().unwrap_or("");
    let invalid = || Error::new(ErrorKind::Query, format!("列 {} 的 {} 值无法解码", column.name, type_name));
    if type_name.starts_with('_') || type_name.contains('[') {
        return Ok(CellValue::Text(value.to_string()));
    }
    Ok(match pg_type_base(type_name) {
        "int2" | "int4" | "int8" | "smallint" | "integer" | "bigint" | "serial" | "bigserial" | "smallserial" => CellValue::I64(value.parse().map_err(|_| invalid())?),
        "bool" | "boolean" => CellValue::Bool(match value { "t" | "true" => true, "f" | "false" => false, _ => return Err(invalid()) }),
        "float4" | "float8" | "real" | "double precision" => {
            let number: f64 = value.parse().map_err(|_| invalid())?;
            if number.is_finite() { CellValue::F64(number) } else { CellValue::Text(value.to_string()) }
        }
        "json" | "jsonb" => CellValue::Json(value.to_string()),
        "bytea" => CellValue::Bytes(pg_decode_bytea(value).ok_or_else(invalid)?),
        _ => CellValue::Text(value.to_string()),
    })
}

fn pg_decode_bytea(value: &str) -> Option<Vec<u8>> {
    if let Some(hex) = value.strip_prefix("\\x") {
        if hex.len() % 2 != 0 { return None; }
        return hex.as_bytes().chunks_exact(2).map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        }).collect();
    }
    // bytea_output=escape：支持双反斜杠与三位八进制，不把客户端设置当固定常量。
    let bytes = value.as_bytes(); let mut out = Vec::new(); let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' { out.push(bytes[i]); i += 1; }
        else if bytes.get(i+1) == Some(&b'\\') { out.push(b'\\'); i += 2; }
        else {
            let digits = bytes.get(i+1..i+4)?;
            if !(b'0'..=b'3').contains(&digits[0]) || !digits[1..].iter().all(|d| (b'0'..=b'7').contains(d)) { return None; }
            out.push((digits[0]-b'0') * 64 + (digits[1]-b'0') * 8 + (digits[2]-b'0')); i += 4;
        }
    }
    Some(out)
}
