//! 跨类型同步的列类型族与值规范化。
//!
//! 差异比对按目标列的类型族把两端值规范成同一逻辑表示，避免
//! 「MySQL 返回 1、PostgreSQL 返回 bool true」这类同义值被当成差异。
//! 这里只做**可逆、显式**的转换：解析失败就报错，不猜、不截断。
//!
//! 边界：时间值只按文本原样比较，不做时区/格式重排；两端日期格式不同时
//! 仍可能产生多余 UPDATE（语句幂等，不会写坏数据）。

use fluxdb_core::{CellValue, is_binary_type_name, is_json_type_name, is_text_type_name};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ColumnFamily {
    /// 未识别的类型：不做转换，按原值比对，避免误伤方言扩展类型。
    Unknown,
    Bool,
    Integer,
    Decimal,
    Float,
    Text,
    Binary,
    Json,
    Temporal,
}

pub(crate) fn column_family(type_name: Option<&str>) -> ColumnFamily {
    let Some(type_name) = type_name.map(str::trim).filter(|name| !name.is_empty()) else {
        return ColumnFamily::Unknown;
    };
    if is_binary_type_name(type_name) {
        return ColumnFamily::Binary;
    }
    if is_json_type_name(type_name) {
        return ColumnFamily::Json;
    }
    if is_text_type_name(type_name) {
        return ColumnFamily::Text;
    }
    match type_base(type_name).as_str() {
        "bool" | "boolean" => ColumnFamily::Bool,
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
        | "int8" | "serial" | "bigserial" | "smallserial" => ColumnFamily::Integer,
        "numeric" | "decimal" | "dec" | "number" => ColumnFamily::Decimal,
        "real" | "float" | "float4" | "float8" | "double" | "double precision" | "binary_float"
        | "binary_double" => ColumnFamily::Float,
        "date"
        | "time"
        | "datetime"
        | "datetime2"
        | "smalldatetime"
        | "timestamp"
        | "timestamptz"
        | "timetz"
        | "timestamp without time zone"
        | "timestamp with time zone"
        | "datetimeoffset"
        | "year" => ColumnFamily::Temporal,
        _ => ColumnFamily::Unknown,
    }
}

/// 源列能否安全写入目标列。`Unknown` 一律放行，由逐值转换兜底。
pub(crate) fn families_compatible(source: ColumnFamily, target: ColumnFamily) -> bool {
    use ColumnFamily::*;
    if matches!(source, Unknown) || matches!(target, Unknown) {
        return true;
    }
    match target {
        Text => !matches!(source, Binary),
        Binary => matches!(source, Binary | Text),
        Json => matches!(source, Json | Text),
        Bool | Integer | Decimal | Float => {
            matches!(source, Bool | Integer | Decimal | Float | Text)
        }
        Temporal => matches!(source, Temporal | Text),
        Unknown => true,
    }
}

/// 按目标列类型族规范化值。错误文案直接展示给用户，必须说明转换失败。
pub(crate) fn coerce_value(
    value: &CellValue,
    family: ColumnFamily,
) -> Result<CellValue, &'static str> {
    use ColumnFamily::*;
    match value {
        CellValue::Null
        | CellValue::BinarySummary(_)
        | CellValue::TruncatedText { .. }
        | CellValue::LargeValue(_) => return Ok(value.clone()),
        _ => {}
    }
    if family == Unknown {
        return Ok(value.clone());
    }
    match (family, value) {
        (Bool, CellValue::Bool(_)) => Ok(value.clone()),
        (Bool, CellValue::I64(number)) => number_to_bool(*number).map(CellValue::Bool),
        (Bool, CellValue::F64(number)) => float_to_bool(*number).map(CellValue::Bool),
        (Bool, CellValue::Text(text) | CellValue::Json(text)) => {
            parse_bool(text).map(CellValue::Bool)
        }

        (Integer, CellValue::I64(_)) => Ok(value.clone()),
        (Integer, CellValue::F64(number)) => float_to_i64(*number).map(CellValue::I64),
        (Integer, CellValue::Bool(flag)) => Ok(CellValue::I64(i64::from(*flag))),
        (Integer, CellValue::Text(text) | CellValue::Json(text)) => text
            .trim()
            .parse::<i64>()
            .map(CellValue::I64)
            .map_err(|_| "不是有效的整数"),

        (Decimal, CellValue::I64(number)) => Ok(CellValue::Text(number.to_string())),
        (Decimal, CellValue::F64(number)) => decimal_from_f64(*number).map(CellValue::Text),
        (Decimal, CellValue::Bool(flag)) => Ok(CellValue::Text(i64::from(*flag).to_string())),
        (Decimal, CellValue::Text(text) | CellValue::Json(text)) => canonical_decimal(text)
            .map(CellValue::Text)
            .ok_or("不是有效的十进制数"),

        (Float, CellValue::F64(number)) if number.is_finite() => Ok(value.clone()),
        (Float, CellValue::I64(number)) => Ok(CellValue::F64(*number as f64)),
        (Float, CellValue::Bool(flag)) => Ok(CellValue::F64(if *flag { 1.0 } else { 0.0 })),
        (Float, CellValue::Text(text) | CellValue::Json(text)) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(CellValue::F64)
            .ok_or("不是有效的浮点数"),
        (Float, CellValue::F64(_)) => Err("不是有效的浮点数"),

        (Text, CellValue::Text(_)) => Ok(value.clone()),
        (Text, CellValue::Json(text)) => Ok(CellValue::Text(text.clone())),
        (Text, CellValue::I64(number)) => Ok(CellValue::Text(number.to_string())),
        (Text, CellValue::F64(number)) => Ok(CellValue::Text(number.to_string())),
        (Text, CellValue::Bool(flag)) => Ok(CellValue::Text(flag.to_string())),
        (Text, CellValue::Bytes(_)) => Err("二进制值不能写入文本列"),

        (Binary, CellValue::Bytes(_)) => Ok(value.clone()),
        (Binary, CellValue::Text(text) | CellValue::Json(text)) => decode_hex(text)
            .map(CellValue::Bytes)
            .ok_or("不是有效的十六进制二进制值"),

        (Json, CellValue::Json(_)) => Ok(value.clone()),
        (Json, CellValue::Text(text)) => Ok(CellValue::Json(text.clone())),

        (Temporal, CellValue::Text(text) | CellValue::Json(text)) => {
            Ok(CellValue::Text(text.trim().to_string()))
        }

        _ => Err("类型不兼容"),
    }
}

fn type_base(type_name: &str) -> String {
    type_name
        .trim()
        .split('(')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn parse_bool(text: &str) -> Result<bool, &'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "t" | "true" | "y" | "yes" => Ok(true),
        "0" | "f" | "false" | "n" | "no" => Ok(false),
        _ => Err("不是有效的布尔值"),
    }
}

fn number_to_bool(number: i64) -> Result<bool, &'static str> {
    match number {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("布尔列只接受 0/1"),
    }
}

fn float_to_bool(number: f64) -> Result<bool, &'static str> {
    if number == 0.0 {
        Ok(false)
    } else if number == 1.0 {
        Ok(true)
    } else {
        Err("布尔列只接受 0/1")
    }
}

fn float_to_i64(number: f64) -> Result<i64, &'static str> {
    if !number.is_finite()
        || number.fract() != 0.0
        || number < i64::MIN as f64
        || number > i64::MAX as f64
    {
        return Err("不是有效的整数");
    }
    Ok(number as i64)
}

fn decimal_from_f64(number: f64) -> Result<String, &'static str> {
    if !number.is_finite() {
        return Err("不是有效的十进制数");
    }
    canonical_decimal(&number.to_string()).ok_or("不是有效的十进制数")
}

/// 十进制文本归一：去正号与无意义的前导/尾随零，保留精度。
/// 指数写法不猜精度，直接判为无效，由调用方报错。
fn canonical_decimal(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(['e', 'E']) {
        return None;
    }
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (integer, fraction) = match digits.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (digits, ""),
    };
    if (integer.is_empty() && fraction.is_empty())
        || !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    // 只有小数点的写法（".5" / "5."）允许，按数字部分补零。
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let mut canonical = String::new();
    if negative && (integer != "0" || !fraction.is_empty()) {
        canonical.push('-');
    }
    canonical.push_str(integer);
    if !fraction.is_empty() {
        canonical.push('.');
        canonical.push_str(fraction);
    }
    Some(canonical)
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let hex = text
        .trim()
        .strip_prefix("0x")
        .or_else(|| text.trim().strip_prefix("0X"))
        .unwrap_or(text.trim())
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if hex.is_empty() || hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_dialect_types_to_families() {
        assert_eq!(column_family(Some("bigint")), ColumnFamily::Integer);
        assert_eq!(column_family(Some("numeric(10,2)")), ColumnFamily::Decimal);
        assert_eq!(column_family(Some("double precision")), ColumnFamily::Float);
        assert_eq!(column_family(Some("boolean")), ColumnFamily::Bool);
        assert_eq!(column_family(Some("varchar(255)")), ColumnFamily::Text);
        assert_eq!(column_family(Some("jsonb")), ColumnFamily::Json);
        assert_eq!(column_family(Some("bytea")), ColumnFamily::Binary);
        assert_eq!(
            column_family(Some("timestamp with time zone")),
            ColumnFamily::Temporal
        );
        assert_eq!(column_family(Some("enum('a','b')")), ColumnFamily::Unknown);
        assert_eq!(column_family(None), ColumnFamily::Unknown);
    }

    #[test]
    fn rejects_conflicting_families_but_allows_unknown() {
        use ColumnFamily::*;
        assert!(families_compatible(Integer, Text));
        assert!(families_compatible(Text, Integer));
        assert!(families_compatible(Bool, Integer));
        assert!(families_compatible(Json, Text));
        assert!(!families_compatible(Binary, Integer));
        assert!(!families_compatible(Json, Integer));
        assert!(!families_compatible(Integer, Binary));
        assert!(families_compatible(Unknown, Binary));
        assert!(families_compatible(Binary, Unknown));
    }

    #[test]
    fn coerces_values_to_target_family() {
        assert_eq!(
            coerce_value(&CellValue::Text("42".into()), ColumnFamily::Integer),
            Ok(CellValue::I64(42))
        );
        assert_eq!(
            coerce_value(&CellValue::Bool(true), ColumnFamily::Integer),
            Ok(CellValue::I64(1))
        );
        assert_eq!(
            coerce_value(&CellValue::I64(0), ColumnFamily::Bool),
            Ok(CellValue::Bool(false))
        );
        assert_eq!(
            coerce_value(&CellValue::Text("true".into()), ColumnFamily::Bool),
            Ok(CellValue::Bool(true))
        );
        assert_eq!(
            coerce_value(&CellValue::Text("1.500".into()), ColumnFamily::Decimal),
            Ok(CellValue::Text("1.5".into()))
        );
        assert_eq!(
            coerce_value(&CellValue::Text("-000.00".into()), ColumnFamily::Decimal),
            Ok(CellValue::Text("0".into()))
        );
        assert_eq!(
            coerce_value(&CellValue::Text("1.5".into()), ColumnFamily::Integer),
            Err("不是有效的整数")
        );
        assert_eq!(
            coerce_value(&CellValue::Text("abc".into()), ColumnFamily::Bool),
            Err("不是有效的布尔值")
        );
        assert_eq!(
            coerce_value(&CellValue::Text("0x0AFF".into()), ColumnFamily::Binary),
            Ok(CellValue::Bytes(vec![0x0A, 0xFF]))
        );
        assert_eq!(
            coerce_value(&CellValue::Text("[1]".into()), ColumnFamily::Json),
            Ok(CellValue::Json("[1]".into()))
        );
    }

    #[test]
    fn keeps_null_and_unknown_values_untouched() {
        assert_eq!(
            coerce_value(&CellValue::Null, ColumnFamily::Integer),
            Ok(CellValue::Null)
        );
        let value = CellValue::Text("x".into());
        assert_eq!(
            coerce_value(&value, ColumnFamily::Unknown),
            Ok(value.clone())
        );
    }
}
