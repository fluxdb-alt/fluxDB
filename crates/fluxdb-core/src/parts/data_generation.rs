// 测试数据生成（结构生成器 + 语义生成器）。
//
// 纯逻辑：不连数据库、不写文件、不拼 SQL。输入是一份声明式 `GenerationSpec`，
// 输出按列顺序逐行交给调用方；写入仍复用现有导入管线（分批参数化 INSERT、进度、
// 取消、清空语义），因此这里不引入第二套写库实现。
//
// 确定性：每个单元格的值只由 (seed, 行号, 列序号) 决定，与生成顺序无关；
// 预览、SQL 预览与真实生成对同一行给出同一结果。唯一列在生成过程中用集合去重，
// 超出重试上限时报错，而不是静默写入重复值。

/// 单次生成的行数上限：导入管线会把 JSON 一次性读入内存，这里先给出明确边界。
pub const DATA_GENERATION_MAX_ROWS: u64 = 200_000;
/// 外键取值池上限；由上层读取父表时按此截断。
pub const DATA_GENERATION_REFERENCE_POOL_MAX: usize = 1000;

/// 生成值：与 SQL 参数语义对齐的少量标量类型。
///
/// 十进制用 `Decimal` 保留原始标度（`9.90` 不能变成 `9.9`）；日期时间用
/// `Text`，由导入管线按目标列类型解析。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GenerationValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(String),
    Text(String),
}

impl GenerationValue {
    /// 去重键：唯一列判定使用文本形式，NULL 不参与唯一性（SQL 唯一约束允许多个 NULL）。
    fn uniqueness_key(&self) -> Option<String> {
        match self {
            Self::Null => None,
            Self::Bool(value) => Some(value.to_string()),
            Self::Int(value) => Some(value.to_string()),
            Self::Float(value) => Some(value.to_string()),
            Self::Decimal(value) | Self::Text(value) => Some(value.clone()),
        }
    }
}

/// 外键引用目标（物理外键与用户自定义 ER 关系都归一到这里）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationReference {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: String,
    pub column: String,
    /// 展示用来源标签（如「外键 users.id」「ER 关系 orders→users」）。
    pub label: String,
}

/// 单列生成配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationColumnSpec {
    pub name: String,
    /// 目标列类型文本（如 `decimal(10,2)`），用于推断与参数校验。
    pub type_name: String,
    pub nullable: bool,
    /// 是否要求生成值唯一（主键/唯一约束或用户开关）。
    pub unique: bool,
    /// 生成器 id，见 [`data_generators`]。
    pub generator: String,
    /// 生成器参数（全部以字符串传递，按生成器解释）。
    pub params: std::collections::BTreeMap<String, String>,
    /// 空值比例（0-100）；仅对可空列生效。
    pub null_percent: u8,
    /// 外键引用目标；为 `None` 表示该列不引用其他表。
    pub reference: Option<GenerationReference>,
    /// 父表已有值（由上层读取）；空池时外键生成回退为随机整数并给出警告。
    pub reference_pool: Vec<GenerationValue>,
}

/// 一次生成的完整配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationSpec {
    pub rows: u64,
    pub seed: String,
    /// `zh_cn` / `en`；只影响语义生成器选用的数据集。
    pub locale: String,
    /// 唯一列重试耗尽后的处理方式。
    pub unique_conflict: UniqueConflict,
    pub columns: Vec<GenerationColumnSpec>,
}

/// 唯一冲突策略：跳过整行（默认）或中止并报错。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum UniqueConflict {
    /// 丢弃该行继续生成；最终写入行数会少于请求行数，由报告说明。
    #[default]
    SkipRow,
    /// 立即返回错误，调用方不会写入任何数据。
    Abort,
}

/// 生成阶段进度（与导入的 `TableImportProgress` 形状一致，UI 共用一套日志渲染）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationProgress {
    pub stage: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationReport {
    pub rows: u64,
    /// 唯一冲突被跳过的行数（`UniqueConflict::SkipRow` 时可能 > 0）。
    pub skipped_rows: u64,
    pub warnings: Vec<String>,
}

/// 生成器家族：只用于界面分组与类型兼容提示，不影响取值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratorFamily {
    Numeric,
    Text,
    Person,
    Contact,
    Address,
    Company,
    Commerce,
    Date,
    Relation,
    Advanced,
}

/// 生成器目录项。
#[derive(Clone, Copy, Debug)]
pub struct DataGenerator {
    pub id: &'static str,
    pub family: GeneratorFamily,
}

pub const DATA_GENERATORS: &[DataGenerator] = &[
    DataGenerator { id: "sequence", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.int", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.decimal", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.range", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.boolean", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.digit", family: GeneratorFamily::Numeric },
    DataGenerator { id: "random.uuid", family: GeneratorFamily::Text },
    DataGenerator { id: "random.ulid", family: GeneratorFamily::Text },
    DataGenerator { id: "constant", family: GeneratorFamily::Text },
    DataGenerator { id: "categorical", family: GeneratorFamily::Text },
    DataGenerator { id: "regex", family: GeneratorFamily::Text },
    DataGenerator { id: "format", family: GeneratorFamily::Text },
    DataGenerator { id: "template", family: GeneratorFamily::Text },
    DataGenerator { id: "name.first", family: GeneratorFamily::Person },
    DataGenerator { id: "name.last", family: GeneratorFamily::Person },
    DataGenerator { id: "name.full", family: GeneratorFamily::Person },
    DataGenerator { id: "name.title", family: GeneratorFamily::Person },
    DataGenerator { id: "internet.email", family: GeneratorFamily::Contact },
    DataGenerator { id: "internet.email_safe", family: GeneratorFamily::Contact },
    DataGenerator { id: "internet.username", family: GeneratorFamily::Contact },
    DataGenerator { id: "internet.domain", family: GeneratorFamily::Contact },
    DataGenerator { id: "phone.number", family: GeneratorFamily::Contact },
    DataGenerator { id: "address.city", family: GeneratorFamily::Address },
    DataGenerator { id: "address.country", family: GeneratorFamily::Address },
    DataGenerator { id: "address.state", family: GeneratorFamily::Address },
    DataGenerator { id: "address.street", family: GeneratorFamily::Address },
    DataGenerator { id: "address.postcode", family: GeneratorFamily::Address },
    DataGenerator { id: "company.name", family: GeneratorFamily::Company },
    DataGenerator { id: "company.job", family: GeneratorFamily::Company },
    DataGenerator { id: "company.industry", family: GeneratorFamily::Company },
    DataGenerator { id: "commerce.price", family: GeneratorFamily::Commerce },
    DataGenerator { id: "color.hex", family: GeneratorFamily::Commerce },
    DataGenerator { id: "date.datetime_between", family: GeneratorFamily::Date },
    DataGenerator { id: "date.recent", family: GeneratorFamily::Date },
    DataGenerator { id: "date.unix", family: GeneratorFamily::Date },
    DataGenerator { id: "lorem.word", family: GeneratorFamily::Text },
    DataGenerator { id: "lorem.sentence", family: GeneratorFamily::Text },
    DataGenerator { id: "lorem.paragraph", family: GeneratorFamily::Text },
    DataGenerator { id: "copy_field", family: GeneratorFamily::Advanced },
    DataGenerator { id: "foreign_ref", family: GeneratorFamily::Relation },
    DataGenerator { id: "skip", family: GeneratorFamily::Advanced },
    DataGenerator { id: "default", family: GeneratorFamily::Advanced },
];

pub fn data_generator(id: &str) -> Option<&'static DataGenerator> {
    DATA_GENERATORS.iter().find(|generator| generator.id == id)
}

/// 生成器产出值的类型：界面据此标注每项，并灰掉与列类型不兼容的生成器。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratorOutput {
    /// 跟随列/源列类型（常量、复制、外键等），任何列都可选。
    Any,
    Int,
    Float,
    Text,
    Bool,
    Date,
}

/// 生成器产出类型；未列出的生成器都产出文本。
pub fn generator_output(id: &str) -> GeneratorOutput {
    match id {
        "sequence" | "random.int" | "random.digit" | "date.unix" => GeneratorOutput::Int,
        "random.decimal" | "random.range" | "commerce.price" => GeneratorOutput::Float,
        "random.boolean" => GeneratorOutput::Bool,
        "date.datetime_between" | "date.recent" => GeneratorOutput::Date,
        "constant" | "copy_field" | "foreign_ref" | "skip" | "default" => GeneratorOutput::Any,
        _ => GeneratorOutput::Text,
    }
}

/// 列声明类型对应的值族；无法识别时按文本处理。
pub fn column_output(type_name: &str) -> GeneratorOutput {
    let base = type_name
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if matches!(
        base.as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
            | "int8" | "serial" | "bigserial" | "smallserial" | "number"
    ) {
        return GeneratorOutput::Int;
    }
    if matches!(
        base.as_str(),
        "numeric" | "decimal" | "real" | "float4" | "float8" | "double" | "double precision" | "float"
    ) {
        return GeneratorOutput::Float;
    }
    if base.contains("date") || base.contains("time") || base == "timestamp" {
        return GeneratorOutput::Date;
    }
    if matches!(base.as_str(), "boolean" | "bool" | "bit") {
        return GeneratorOutput::Bool;
    }
    GeneratorOutput::Text
}

/// 生成器能否写入该类型的列：整数与小数互认，其余同族才兼容。
pub fn generator_compatible_with_type(id: &str, type_name: &str) -> bool {
    use GeneratorOutput::*;
    let generator = generator_output(id);
    if generator == Any {
        return true;
    }
    let column = column_output(type_name);
    generator == column || matches!((generator, column), (Int | Float, Int | Float))
}

/// 按列名与类型推断默认生成器；查询不出来时退回 `constant`。
pub fn infer_generator(column_name: &str, type_name: &str) -> &'static str {
    let name = column_name.to_ascii_lowercase();
    let base = type_name
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let is_int = matches!(
        base.as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
            | "int8" | "serial" | "bigserial" | "smallserial" | "number"
    );
    let is_float = matches!(
        base.as_str(),
        "numeric" | "decimal" | "real" | "float4" | "float8" | "double" | "double precision" | "float"
    );
    let is_date = base.contains("date") || base.contains("time") || base == "timestamp";
    let is_bool = matches!(base.as_str(), "boolean" | "bool" | "bit");

    if name == "id" || name.ends_with("_id") {
        return "sequence";
    }
    if name.contains("email") || name.contains("mail") {
        return "internet.email_safe";
    }
    if name.contains("phone") || name.contains("mobile") || name.contains("tel") {
        return "phone.number";
    }
    if name.contains("name") {
        return "name.full";
    }
    if name.contains("city") {
        return "address.city";
    }
    if name.contains("country") {
        return "address.country";
    }
    if name.contains("address") {
        return "address.street";
    }
    if name.contains("status") || name.contains("type") || name.contains("level") {
        return "categorical";
    }
    if is_bool {
        return "random.boolean";
    }
    if is_date {
        return "date.datetime_between";
    }
    if is_int {
        return "random.int";
    }
    if is_float {
        return "random.decimal";
    }
    "lorem.word"
}

/// 生成入口：按行回调，调用方负责落盘或写入。
///
/// - 取消：每 256 行检查一次，取消返回 `Cancelled`。
/// - 进度：每 5000 行回调一次「生成」阶段。
/// - 唯一列：值重复时最多重试 64 次；仍冲突按 `spec.unique_conflict` 跳过整行或报错。
pub fn generate_table_rows(
    spec: &GenerationSpec,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(GenerationProgress),
    sink: &mut dyn FnMut(Vec<GenerationValue>) -> Result<()>,
) -> Result<GenerationReport> {
    use std::sync::atomic::Ordering;

    validate_spec(spec)?;
    let mut warnings = Vec::new();
    let mut tracker = UniqueTracker {
        used: spec
            .columns
            .iter()
            .map(|_| std::collections::HashSet::new())
            .collect(),
        reserved: Vec::new(),
    };
    let mut report = GenerationReport::default();
    let reference_pool = reference_pools(spec, &mut warnings);

    progress(GenerationProgress {
        stage: "准备".into(),
        message: format!(
            "正在生成 {} 行（{} 列，种子 {}）",
            spec.rows,
            spec.columns.len(),
            spec.seed
        ),
    });

    for row_index in 0..spec.rows {
        if row_index % 256 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(Error::new(ErrorKind::Cancelled, "已取消数据生成"));
        }
        let mut row = Vec::with_capacity(spec.columns.len());
        // 本行已占用的唯一键：跳过整行时必须回滚，否则会虚占容量、提前耗尽重试。
        tracker.reserved.clear();
        let mut conflict_column = None;
        for (column_index, column) in spec.columns.iter().enumerate() {
            match generate_cell(
                spec,
                column,
                column_index,
                row_index,
                &row,
                &reference_pool[column_index],
                &mut tracker,
            )? {
                Some(value) => row.push(value),
                None => {
                    conflict_column = Some(column.name.as_str());
                    break;
                }
            }
        }
        if let Some(column_name) = conflict_column {
            for (column_index, key) in std::mem::take(&mut tracker.reserved) {
                tracker.used[column_index].remove(&key);
            }
            if spec.unique_conflict == UniqueConflict::Abort {
                return Err(Error::new(
                    ErrorKind::Query,
                    format!("{column_name} 无法生成不重复的值，请更换生成器或减少行数"),
                ));
            }
            report.skipped_rows += 1;
            continue;
        }
        sink(row)?;
        report.rows += 1;
        if (row_index + 1) % 5000 == 0 {
            progress(GenerationProgress {
                stage: "生成".into(),
                message: format!("已生成 {}/{} 行", row_index + 1, spec.rows),
            });
        }
    }

    if report.skipped_rows > 0 {
        warnings.push(format!(
            "有 {} 行因唯一冲突被跳过，实际生成 {} 行",
            report.skipped_rows, report.rows
        ));
    }
    report.warnings = warnings;
    progress(GenerationProgress {
        stage: "完成".into(),
        message: format!("已生成 {} 行", report.rows),
    });
    Ok(report)
}

fn validate_spec(spec: &GenerationSpec) -> Result<()> {
    if spec.rows == 0 {
        return Err(Error::new(ErrorKind::Query, "生成行数必须大于 0"));
    }
    if spec.rows > DATA_GENERATION_MAX_ROWS {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("单次最多生成 {DATA_GENERATION_MAX_ROWS} 行"),
        ));
    }
    if spec.columns.is_empty() {
        return Err(Error::new(ErrorKind::Query, "没有可写入的列"));
    }
    let mut names = std::collections::BTreeSet::new();
    for column in &spec.columns {
        if column.name.trim().is_empty() {
            return Err(Error::new(ErrorKind::Query, "列名不能为空"));
        }
        if !names.insert(column.name.as_str()) {
            return Err(Error::new(
                ErrorKind::Query,
                format!("列名重复：{}", column.name),
            ));
        }
        if data_generator(&column.generator).is_none() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("未知生成器：{}", column.generator),
            ));
        }
        if column.null_percent > 100 {
            return Err(Error::new(ErrorKind::Query, "空值比例不能超过 100"));
        }
    }
    Ok(())
}

/// 每列的外键取值池；空池回退随机整数时只警告一次。
fn reference_pools(
    spec: &GenerationSpec,
    warnings: &mut Vec<String>,
) -> Vec<Vec<GenerationValue>> {
    spec.columns
        .iter()
        .map(|column| {
            if column.generator != "foreign_ref" {
                return Vec::new();
            }
            if column.reference_pool.is_empty() {
                warnings.push(format!(
                    "{} 的父表没有可读数据，回退为随机整数",
                    column.name
                ));
            }
            column.reference_pool.clone()
        })
        .collect()
}

/// 唯一性去重状态：每列已用值 + 本行已占用键（跳过整行时回滚）。
struct UniqueTracker {
    used: Vec<std::collections::HashSet<String>>,
    reserved: Vec<(usize, String)>,
}

fn generate_cell(
    spec: &GenerationSpec,
    column: &GenerationColumnSpec,
    column_index: usize,
    row_index: u64,
    row: &[GenerationValue],
    pool: &[GenerationValue],
    tracker: &mut UniqueTracker,
) -> Result<Option<GenerationValue>> {
    // 随机序列在重试之间必须继续推进；若每次重试都重新播种，
    // 唯一列会反复拿到同一个值，重试 64 次后必然失败。
    let mut rng = SplitMix64::seeded(&spec.seed, row_index, column_index);
    for _ in 0..64 {
        if column.nullable
            && column.null_percent > 0
            && rng.below(100) < u64::from(column.null_percent)
        {
            return Ok(Some(GenerationValue::Null));
        }
        let value = generate_value(spec, column, row_index, row, pool, &mut rng)?;
        if !column.unique {
            return Ok(Some(value));
        }
        match value.uniqueness_key() {
            None => return Ok(Some(value)),
            Some(key) => {
                if tracker.used[column_index].insert(key.clone()) {
                    tracker.reserved.push((column_index, key));
                    return Ok(Some(value));
                }
            }
        }
    }
    // None 表示重试耗尽：是否跳过行由调用方按 `unique_conflict` 决定。
    Ok(None)
}

fn param<'a>(column: &'a GenerationColumnSpec, key: &str, default: &'a str) -> &'a str {
    column
        .params
        .get(key)
        .map(String::as_str)
        .unwrap_or(default)
}

fn param_i64(column: &GenerationColumnSpec, key: &str, default: i64) -> i64 {
    param(column, key, "")
        .trim()
        .parse()
        .unwrap_or(default)
}

fn param_u64(column: &GenerationColumnSpec, key: &str, default: u64) -> u64 {
    param(column, key, "")
        .trim()
        .parse()
        .unwrap_or(default)
}

fn generate_value(
    spec: &GenerationSpec,
    column: &GenerationColumnSpec,
    row_index: u64,
    row: &[GenerationValue],
    pool: &[GenerationValue],
    rng: &mut SplitMix64,
) -> Result<GenerationValue> {
    let zh = spec.locale.starts_with("zh");
    let value = match column.generator.as_str() {
        "sequence" => {
            let start = param_i64(column, "start", 1);
            let step = param_i64(column, "step", 1);
            GenerationValue::Int(start.saturating_add((row_index as i64).saturating_mul(step)))
        }
        "random.int" => {
            let min = param_i64(column, "min", 0);
            let max = param_i64(column, "max", 1000);
            GenerationValue::Int(rng.between(min, max))
        }
        "random.decimal" => {
            let min = param(column, "min", "0.00").parse::<f64>().unwrap_or(0.0);
            let max = param(column, "max", "100.00").parse::<f64>().unwrap_or(100.0);
            let scale = param_u64(column, "scale", 2).min(6) as usize;
            let raw = min + rng.unit() * (max - min).max(0.0);
            GenerationValue::Decimal(format!("{raw:.scale$}"))
        }
        "random.range" => {
            let low = param_i64(column, "low", 0);
            let high = param_i64(column, "high", 100);
            let step = param_i64(column, "step", 1).max(1);
            let span = ((high - low).max(0) / step) + 1;
            GenerationValue::Int(low + rng.below(span as u64) as i64 * step)
        }
        "random.boolean" => {
            let ratio = param_u64(column, "ratio", 50).min(100);
            GenerationValue::Bool(rng.below(100) < ratio)
        }
        "random.digit" => {
            let count = param_u64(column, "count", 1).clamp(1, 18) as usize;
            let mut text = String::with_capacity(count);
            for _ in 0..count {
                text.push((b'0' + rng.below(10) as u8) as char);
            }
            GenerationValue::Text(text)
        }
        "random.uuid" => GenerationValue::Text(uuid_v4(rng)),
        "random.ulid" => GenerationValue::Text(ulid(rng)),
        "constant" => GenerationValue::Text(param(column, "value", "").to_string()),
        "categorical" => {
            let values = categorical_values(column);
            match values.get(rng.below(values.len() as u64) as usize) {
                Some(value) => GenerationValue::Text(value.clone()),
                None => GenerationValue::Null,
            }
        }
        "regex" => GenerationValue::Text(expand_regex(param(column, "pattern", ""), rng)),
        "format" => GenerationValue::Text(expand_format(param(column, "pattern", ""), rng)),
        "template" => GenerationValue::Text(expand_template(spec, column, row_index, row, rng)),
        "name.first" => GenerationValue::Text(pick(if zh { GIVEN_ZH } else { GIVEN_EN }, rng).to_string()),
        "name.last" => GenerationValue::Text(pick(if zh { SURNAMES_ZH } else { LAST_EN }, rng).to_string()),
        "name.full" => GenerationValue::Text(if zh {
            format!(
                "{}{}",
                pick(SURNAMES_ZH, rng),
                pick(GIVEN_ZH, rng)
            )
        } else {
            format!("{} {}", pick(GIVEN_EN, rng), pick(LAST_EN, rng))
        }),
        "name.title" => GenerationValue::Text(if zh {
            format!(
                "{}{}{}",
                pick(SURNAMES_ZH, rng),
                pick(GIVEN_ZH, rng),
                pick(TITLES_ZH, rng)
            )
        } else {
            format!("{} {}", pick(TITLES_EN, rng), pick(LAST_EN, rng))
        }),
        "internet.email" => {
            let domain = pick(if zh { DOMAINS_ZH } else { DOMAINS_EN }, rng);
            GenerationValue::Text(format!(
                "{}.{}{}@{}",
                pick(LAST_EN, rng).to_ascii_lowercase(),
                pick(GIVEN_EN, rng).to_ascii_lowercase(),
                rng.below(1000),
                domain
            ))
        }
        "internet.email_safe" => {
            let domain = param(column, "domain", "example.com");
            GenerationValue::Text(format!(
                "{}.{}{}@{}",
                pick(LAST_EN, rng).to_ascii_lowercase(),
                pick(GIVEN_EN, rng).to_ascii_lowercase(),
                rng.below(10000),
                domain
            ))
        }
        "internet.username" => GenerationValue::Text(format!(
            "{}_{}{}",
            pick(GIVEN_EN, rng).to_ascii_lowercase(),
            pick(LAST_EN, rng).to_ascii_lowercase(),
            rng.below(100)
        )),
        "internet.domain" => GenerationValue::Text(
            pick(if zh { DOMAINS_ZH } else { DOMAINS_EN }, rng).to_string(),
        ),
        "phone.number" => GenerationValue::Text(format!(
            "1{}{:08}",
            rng.below(10),
            rng.below(100_000_000)
        )),
        "address.city" => GenerationValue::Text(pick(if zh { CITIES_ZH } else { CITIES_EN }, rng).to_string()),
        "address.country" => GenerationValue::Text(pick(if zh { COUNTRIES_ZH } else { COUNTRIES_EN }, rng).to_string()),
        "address.state" => GenerationValue::Text(pick(if zh { STATES_ZH } else { STATES_EN }, rng).to_string()),
        "address.street" => GenerationValue::Text(format!(
            "{}{}号",
            pick(if zh { STREETS_ZH } else { STREETS_EN }, rng),
            rng.below(999) + 1
        )),
        "address.postcode" => GenerationValue::Text(format!("{:06}", rng.below(1_000_000))),
        "company.name" => GenerationValue::Text(if zh {
            format!(
                "{}{}{}",
                pick(CITY_PREFIX_ZH, rng),
                pick(COMPANY_WORDS_ZH, rng),
                pick(COMPANY_SUFFIX_ZH, rng)
            )
        } else {
            format!(
                "{} {} {}",
                pick(CITY_PREFIX_ZH, rng),
                pick(COMPANY_WORDS_ZH, rng),
                "Ltd."
            )
        }),
        "company.job" => GenerationValue::Text(pick(if zh { JOBS_ZH } else { JOBS_EN }, rng).to_string()),
        "company.industry" => GenerationValue::Text(pick(if zh { INDUSTRIES_ZH } else { INDUSTRIES_EN }, rng).to_string()),
        "commerce.price" => {
            let min = param(column, "min", "9.90").parse::<f64>().unwrap_or(9.9);
            let max = param(column, "max", "9999.00").parse::<f64>().unwrap_or(9999.0);
            let raw = min + rng.unit() * (max - min).max(0.0);
            GenerationValue::Decimal(format!("{raw:.2}"))
        }
        "color.hex" => GenerationValue::Text(format!("#{:06x}", rng.below(0x1000000))),
        "date.datetime_between" => {
            let from = parse_date(param(column, "from", "2025-01-01"))?;
            let to = parse_date(param(column, "to", "2026-12-31"))?;
            let days = (to - from).max(0) as u64;
            let day = from + rng.below(days + 1) as i64;
            let seconds = rng.below(86_400);
            GenerationValue::Text(format!(
                "{} {:02}:{:02}:{:02}",
                format_date(day)?,
                seconds / 3600,
                (seconds % 3600) / 60,
                seconds % 60
            ))
        }
        "date.recent" => {
            let days = param_i64(column, "days", 365).max(1) as u64;
            let day = today_days() - rng.below(days) as i64;
            GenerationValue::Text(format_date(day)?)
        }
        "date.unix" => GenerationValue::Int(
            today_days() * 86_400 - rng.below(365 * 86_400) as i64,
        ),
        "lorem.word" => {
            let count = param_u64(column, "count", 1).clamp(1, 50) as usize;
            let words = if zh { LOREM_ZH } else { LOREM_EN };
            let out = (0..count)
                .map(|_| pick(words, rng))
                .collect::<Vec<_>>()
                .join(if zh { "" } else { " " });
            GenerationValue::Text(out)
        }
        "lorem.sentence" => {
            let count = param_u64(column, "count", 8).clamp(1, 100) as usize;
            let words = if zh { LOREM_ZH } else { LOREM_EN };
            let mut text = (0..count)
                .map(|_| pick(words, rng))
                .collect::<Vec<_>>()
                .join(if zh { "" } else { " " });
            if zh {
                text.push('。');
            } else {
                text.push('.');
            }
            GenerationValue::Text(text)
        }
        "lorem.paragraph" => {
            let sentences = param_u64(column, "count", 3).clamp(1, 20) as usize;
            let words = if zh { LOREM_ZH } else { LOREM_EN };
            let mut text = String::new();
            for _ in 0..sentences {
                let sentence = (0..8)
                    .map(|_| pick(words, rng))
                    .collect::<Vec<_>>()
                    .join(if zh { "" } else { " " });
                text.push_str(&sentence);
                text.push(if zh { '。' } else { '.' });
            }
            GenerationValue::Text(text)
        }
        "copy_field" => {
            let source = param(column, "source", "");
            spec.columns
                .iter()
                .position(|candidate| candidate.name == source)
                .and_then(|index| row.get(index).cloned())
                .unwrap_or(GenerationValue::Null)
        }
        "foreign_ref" => {
            if pool.is_empty() {
                GenerationValue::Int(rng.between(1, 2000))
            } else if param(column, "pick", "random") == "sequence" {
                pool[(row_index as usize) % pool.len()].clone()
            } else {
                pool[rng.below(pool.len() as u64) as usize].clone()
            }
        }
        "skip" | "default" => GenerationValue::Null,
        other => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("未知生成器：{other}"),
            ))
        }
    };
    Ok(value)
}

fn categorical_values(column: &GenerationColumnSpec) -> Vec<String> {
    param(column, "values", "a,b,c")
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn pick<'a>(values: &'a [&'a str], rng: &mut SplitMix64) -> &'a str {
    values[rng.below(values.len() as u64) as usize]
}

/// 模板：`${生成器id}` 就地求值；未知 token 原样保留，避免静默丢字。
fn expand_template(
    spec: &GenerationSpec,
    column: &GenerationColumnSpec,
    row_index: u64,
    row: &[GenerationValue],
    rng: &mut SplitMix64,
) -> String {
    let template = param(column, "template", "");
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$'
            && chars.get(index + 1) == Some(&'{')
            && let Some(end) = chars[index + 2..].iter().position(|ch| *ch == '}')
        {
                let token: String = chars[index + 2..index + 2 + end].iter().collect();
                let nested = GenerationColumnSpec {
                    name: column.name.clone(),
                    type_name: column.type_name.clone(),
                    nullable: false,
                    unique: false,
                    generator: token.clone(),
                    params: std::collections::BTreeMap::new(),
                    null_percent: 0,
                    reference: None,
                    reference_pool: Vec::new(),
                };
                match generate_value(spec, &nested, row_index, row, &[], rng) {
                    Ok(value) => out.push_str(&display_value(&value)),
                    Err(_) => {
                        out.push_str("${");
                        out.push_str(&token);
                        out.push('}');
                    }
                }
                index += 2 + end + 1;
                continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn display_value(value: &GenerationValue) -> String {
    match value {
        GenerationValue::Null => String::new(),
        GenerationValue::Bool(value) => value.to_string(),
        GenerationValue::Int(value) => value.to_string(),
        GenerationValue::Float(value) => value.to_string(),
        GenerationValue::Decimal(value) | GenerationValue::Text(value) => value.clone(),
    }
}

/// 迷你正则：支持字面量、`\d`/`\w`、`[a-z]` 区间、`{n}` 量词。
/// 不支持分组与回溯——生成测试数据不需要完整正则引擎。
fn expand_regex(pattern: &str, rng: &mut SplitMix64) -> String {
    #[derive(Clone, Copy)]
    enum Token {
        Digit,
        Word,
        Range(u8, u8),
        Literal(char),
    }

    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let token = match chars[index] {
            '\\' => {
                let next = chars.get(index + 1).copied().unwrap_or('\\');
                index += 2;
                match next {
                    'd' => Token::Digit,
                    'w' => Token::Word,
                    other => Token::Literal(other),
                }
            }
            '[' => {
                let mut from = None;
                let mut to = None;
                let mut cursor = index + 1;
                while cursor < chars.len() && chars[cursor] != ']' {
                    if chars[cursor] != '-' {
                        if from.is_none() {
                            from = Some(chars[cursor] as u8);
                        } else if to.is_none() {
                            to = Some(chars[cursor] as u8);
                        }
                    }
                    cursor += 1;
                }
                index = cursor + 1;
                match (from, to) {
                    (Some(from), Some(to)) => Token::Range(from, to),
                    _ => continue,
                }
            }
            ' ' | '\t' => {
                // 正则里的空白默认忽略，与常见写法 `ORD-2026\d{5}` 一致。
                index += 1;
                continue;
            }
            other => {
                index += 1;
                Token::Literal(other)
            }
        };

        // 紧邻的 `{n}` 量词只作用于前一个 token。
        let mut repeat = 1u64;
        if chars.get(index) == Some(&'{')
            && let Some(end) = chars[index..].iter().position(|ch| *ch == '}')
        {
                let digits: String = chars[index + 1..index + end].iter().collect();
                if let Ok(count) = digits.trim().parse::<u64>() {
                    repeat = count.min(4096);
                }
                index += end + 1;
        }

        for _ in 0..repeat {
            out.push(match token {
                Token::Digit => (b'0' + rng.below(10) as u8) as char,
                Token::Word => {
                    const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
                    ALNUM[rng.below(ALNUM.len() as u64) as usize] as char
                }
                Token::Range(from, to) => {
                    if to <= from {
                        from as char
                    } else {
                        (from + rng.below((to - from + 1) as u64) as u8) as char
                    }
                }
                Token::Literal(ch) => ch,
            });
        }
    }
    out
}

fn expand_format(pattern: &str, rng: &mut SplitMix64) -> String {
    pattern
        .chars()
        .map(|ch| {
            if ch == '#' {
                (b'0' + rng.below(10) as u8) as char
            } else {
                ch
            }
        })
        .collect()
}

pub(crate) fn uuid_v4(rng: &mut SplitMix64) -> String {
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        chunk.copy_from_slice(&rng.next_u64().to_be_bytes()[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn ulid(rng: &mut SplitMix64) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut out = String::with_capacity(26);
    for _ in 0..26 {
        out.push(ALPHABET[rng.below(32) as usize] as char);
    }
    out
}

fn parse_date(text: &str) -> Result<i64> {
    use chrono::{Datelike, NaiveDate};
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map(|date| date.num_days_from_ce() as i64)
        .map_err(|_| Error::new(ErrorKind::Query, format!("日期格式无效：{text}")))
}

fn format_date(days: i64) -> Result<String> {
    use chrono::NaiveDate;
    NaiveDate::from_num_days_from_ce_opt(days as i32)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "日期换算失败"))
}

/// 当前日期（UTC 天数）。测试数据只需要稳定、单调的“今天”，不引入时钟依赖。
fn today_days() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 86_400)
        .unwrap_or(20_000)) as i64
        + 719_163
}

/// 确定性 PRNG（splitmix64）：同一种子/行/列必得同一序列，预览与写入一致。
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn seeded(seed: &str, row: u64, column: usize) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in seed.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= row.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        hash ^= (column as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let mut rng = Self { state: hash };
        rng.next_u64();
        rng
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }

    fn between(&mut self, min: i64, max: i64) -> i64 {
        if max <= min {
            return min;
        }
        min + self.below((max - min + 1) as u64) as i64
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const SURNAMES_ZH: &[&str] = &[
    "王", "李", "张", "刘", "陈", "杨", "赵", "黄", "周", "吴", "徐", "孙", "马", "朱", "胡",
    "郭", "何", "林", "高", "罗",
];
const GIVEN_ZH: &[&str] = &[
    "伟", "芳", "娜", "敏", "静", "强", "磊", "洋", "艳", "杰", "军", "涛", "明", "超", "秀英",
    "霞", "平", "刚", "桂英", "丽",
];
const TITLES_ZH: &[&str] = &["先生", "女士"];
const GIVEN_EN: &[&str] = &[
    "James", "Mary", "Robert", "Patricia", "John", "Jennifer", "Michael", "Linda", "David",
    "Elizabeth",
];
const LAST_EN: &[&str] = &[
    "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Wilson",
    "Moore",
];
const TITLES_EN: &[&str] = &["Mr.", "Ms.", "Dr."];
const DOMAINS_ZH: &[&str] = &["example.cn", "test.cn", "corp.cn", "mail.cn"];
const DOMAINS_EN: &[&str] = &["example.com", "test.org", "corp.net", "mail.com"];
const CITIES_ZH: &[&str] = &[
    "北京", "上海", "广州", "深圳", "杭州", "成都", "南京", "武汉", "西安", "苏州",
];
const CITIES_EN: &[&str] = &[
    "Beijing", "Shanghai", "Guangzhou", "Shenzhen", "Hangzhou", "Chengdu",
];
const COUNTRIES_ZH: &[&str] = &["中国", "美国", "日本", "德国", "英国", "法国", "新加坡"];
const COUNTRIES_EN: &[&str] = &["China", "United States", "Japan", "Germany", "United Kingdom"];
const STATES_ZH: &[&str] = &[
    "北京市", "上海市", "广东省", "浙江省", "江苏省", "四川省", "湖北省",
];
const STATES_EN: &[&str] = &["California", "Texas", "New York", "Washington", "Florida"];
const STREETS_ZH: &[&str] = &[
    "中山路", "人民路", "建设路", "长江路", "解放路", "文化路", "南京路",
];
const STREETS_EN: &[&str] = &["Main St", "Oak Ave", "Pine Rd", "Maple Dr", "Cedar Ln"];
const CITY_PREFIX_ZH: &[&str] = &["北京", "上海", "深圳", "杭州", "成都"];
const COMPANY_WORDS_ZH: &[&str] = &["云图", "锐视", "启明", "恒信", "博远", "星联"];
const COMPANY_SUFFIX_ZH: &[&str] = &["科技有限公司", "贸易有限公司", "信息技术有限公司"];
const JOBS_ZH: &[&str] = &["产品经理", "后端工程师", "数据分析师", "设计师", "销售经理"];
const JOBS_EN: &[&str] = &["Engineer", "Designer", "Analyst", "Manager", "Consultant"];
const INDUSTRIES_ZH: &[&str] = &["制造业", "零售", "医疗", "教育", "金融", "物流"];
const INDUSTRIES_EN: &[&str] = &["Manufacturing", "Retail", "Healthcare", "Education", "Finance"];
const LOREM_ZH: &[&str] = &[
    "测试", "数据", "示例", "订单", "客户", "产品", "记录", "状态", "备注", "生成",
];
const LOREM_EN: &[&str] = &[
    "lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit",
];

/// 生成 SQL 预览：按目标库方言输出前 3 行示例 INSERT，只读展示，不执行。
///
/// 实际的写库仍由导入管线参数化执行；这里只让用户确认列顺序与取值形态，
/// 因此不包含凭据、不访问数据库，也不代表最终绑定参数。
pub fn preview_generation_sql(
    spec: &GenerationSpec,
    kind: DatabaseKind,
    object: &ObjectPath,
    mode: TableImportMode,
) -> Result<String> {
    let dialect = sql_preview_dialect(kind).ok_or_else(|| {
        Error::new(ErrorKind::Unsupported, "当前数据库不支持 SQL 预览")
    })?;
    let mut preview = spec.clone();
    preview.rows = spec.rows.clamp(1, 3);
    // 预览不能因为唯一冲突失败；真实生成仍按用户选择的策略执行。
    preview.unique_conflict = UniqueConflict::SkipRow;
    let mut rows = Vec::new();
    generate_table_rows(
        &preview,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
        &mut |row| {
            rows.push(row);
            Ok(())
        },
    )?;

    // 跳过列与「数据库默认值」列不会写入，与导入临时文件的列集合保持一致。
    let indexes = spec
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| !matches!(column.generator.as_str(), "skip" | "default"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if indexes.is_empty() {
        return Err(Error::new(ErrorKind::Query, "没有可写入的列"));
    }
    let table = sql_preview_table(kind, object)?;
    let column_list = indexes
        .iter()
        .map(|index| dialect(spec.columns[*index].name.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = String::new();
    if mode == TableImportMode::Truncate {
        // 与导入管线一致：SQLite 没有 TRUNCATE，用 DELETE 替代。
        let keyword = if kind == DatabaseKind::Sqlite {
            "DELETE FROM"
        } else {
            "TRUNCATE TABLE"
        };
        sql.push_str(&format!("{keyword} {table};\n\n"));
    }
    sql.push_str(&format!("INSERT INTO {table} ({column_list})\nVALUES\n"));
    let values = rows
        .iter()
        .map(|row| {
            let items = indexes
                .iter()
                .map(|index| {
                    sql_preview_literal(row.get(*index).unwrap_or(&GenerationValue::Null))
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("  ({items})")
        })
        .collect::<Vec<_>>();
    sql.push_str(&values.join(",\n"));
    sql.push(';');
    Ok(sql)
}

/// 返回标识符引用函数；非 SQL 数据库返回 `None`。
fn sql_preview_dialect(
    kind: DatabaseKind,
) -> Option<fn(&str) -> String> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            Some(|name| MySqlDialect.quote_identifier(name))
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            Some(|name| PostgresDialect.quote_identifier(name))
        }
        DatabaseKind::Sqlite => Some(|name| SqliteDialect.quote_identifier(name)),
        DatabaseKind::Dameng => Some(|name| DamengDialect.quote_identifier(name)),
        DatabaseKind::SqlServer => Some(|name| SqlServerDialect.quote_identifier(name)),
        DatabaseKind::Oracle => Some(|name| OracleDialect.quote_identifier(name)),
        DatabaseKind::MongoDb | DatabaseKind::Redis => None,
    }
}

fn sql_preview_table(kind: DatabaseKind, object: &ObjectPath) -> Result<String> {
    let quote = sql_preview_dialect(kind)
        .ok_or_else(|| Error::new(ErrorKind::Unsupported, "当前数据库不支持 SQL 预览"))?;
    let table = quote(&object.name);
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            let database = object
                .database
                .as_deref()
                .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
            Ok(format!("{}.{}", quote(database), table))
        }
        DatabaseKind::Sqlite => Ok(table),
        DatabaseKind::Postgres | DatabaseKind::Kingbase => Ok(format!(
            "{}.{}",
            quote(object.schema.as_deref().unwrap_or("public")),
            table
        )),
        DatabaseKind::Dameng | DatabaseKind::SqlServer | DatabaseKind::Oracle => {
            let schema = object
                .schema
                .as_deref()
                .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 schema 上下文"))?;
            Ok(format!("{}.{}", quote(schema), table))
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => unreachable!("已在上面排除"),
    }
}

/// 示例值转 SQL 字面量：数字原样、布尔用 TRUE/FALSE、文本转义单引号。
fn sql_preview_literal(value: &GenerationValue) -> String {
    match value {
        GenerationValue::Null => "NULL".to_string(),
        GenerationValue::Bool(value) => {
            if *value {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        GenerationValue::Int(value) => value.to_string(),
        GenerationValue::Float(value) => value.to_string(),
        GenerationValue::Decimal(value) if value.parse::<f64>().is_ok() => value.clone(),
        GenerationValue::Decimal(value) | GenerationValue::Text(value) => {
            // 文本统一转义；数值型列上出现无法解析的十进制时也只是展示，不参与绑定。
            format!("'{}'", value.replace('\'', "''"))
        }
    }
}

#[cfg(test)]
mod data_generation_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;

    fn column(name: &str, generator: &str, type_name: &str) -> GenerationColumnSpec {
        GenerationColumnSpec {
            name: name.into(),
            type_name: type_name.into(),
            nullable: false,
            unique: false,
            generator: generator.into(),
            params: BTreeMap::new(),
            null_percent: 0,
            reference: None,
            reference_pool: Vec::new(),
        }
    }

    fn collect(spec: &GenerationSpec) -> Vec<Vec<GenerationValue>> {
        let mut rows = Vec::new();
        generate_table_rows(
            spec,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |row| {
                rows.push(row);
                Ok(())
            },
        )
        .expect("generate");
        rows
    }

    #[test]
    fn same_seed_returns_same_values() {
        let spec = GenerationSpec {
            rows: 5,
            seed: "20261116".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![
                column("name", "name.full", "varchar(64)"),
                column("amount", "random.decimal", "decimal(10,2)"),
                column("code", "regex", "varchar(32)"),
            ],
        };
        let first = collect(&spec);
        let second = collect(&spec);
        assert_eq!(first, second);
        assert_eq!(first.len(), 5);
        assert!(matches!(&first[0][1], GenerationValue::Decimal(text) if text.contains('.')));
    }

    #[test]
    fn sequence_advances_by_step() {
        let mut id = column("id", "sequence", "bigint");
        id.unique = true;
        id.params.insert("start".into(), "10246".into());
        id.params.insert("step".into(), "1".into());
        let spec = GenerationSpec {
            rows: 3,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![id],
        };
        assert_eq!(
            collect(&spec),
            vec![
                vec![GenerationValue::Int(10246)],
                vec![GenerationValue::Int(10247)],
                vec![GenerationValue::Int(10248)],
            ]
        );
    }

    #[test]
    fn unique_column_reports_exhausted_capacity() {
        let mut code = column("code", "categorical", "varchar(8)");
        code.unique = true;
        code.params.insert("values".into(), "a,b".into());
        let spec = GenerationSpec {
            rows: 3,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![code],
        };
        let error = generate_table_rows(
            &spec,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |_| Ok(()),
        )
        .expect_err("capacity exhausted");
        assert!(error.message.contains("不重复"));
    }

    #[test]
    fn preview_sql_quotes_identifiers_skips_columns_and_escapes_text() {
        let mut name = column("name", "constant", "text");
        name.params
            .insert("value".to_string(), "O'Hara".to_string());
        let spec = GenerationSpec {
            rows: 10,
            seed: "sql".into(),
            locale: "en".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![
                column("id", "sequence", "bigint"),
                name,
                column("ignored", "skip", "text"),
            ],
        };
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".into()),
            schema: None,
            name: "orders".into(),
            kind: ObjectKind::Table,
        };
        let sql = preview_generation_sql(
            &spec,
            DatabaseKind::MySql,
            &object,
            TableImportMode::Truncate,
        )
        .expect("preview sql");
        assert!(sql.starts_with("TRUNCATE TABLE `shop`.`orders`;"), "{sql}");
        assert!(
            sql.contains("INSERT INTO `shop`.`orders` (`id`, `name`)"),
            "{sql}"
        );
        // 跳过列不进入列清单。
        assert!(!sql.contains("ignored"), "{sql}");
        // 文本按 SQL 字面量转义；预览固定 3 行。
        assert!(sql.contains("'O''Hara'"), "{sql}");
        assert_eq!(sql.matches("  (").count(), 3, "{sql}");
    }

    #[test]
    fn unique_conflict_skip_policy_drops_rows_and_reports_them() {
        let mut code = column("code", "categorical", "varchar(8)");
        code.unique = true;
        code.params.insert("values".into(), "a,b".into());
        let spec = GenerationSpec {
            rows: 3,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::SkipRow,
            columns: vec![code],
        };
        let mut rows = Vec::new();
        let report = generate_table_rows(
            &spec,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |row| {
                rows.push(row);
                Ok(())
            },
        )
        .expect("skip policy should not fail");
        // 两种取值只能写出两行，第三行按策略跳过并记入报告。
        assert_eq!(report.rows, 2);
        assert_eq!(report.skipped_rows, 1);
        assert!(report.warnings.iter().any(|warning| warning.contains("跳过")));
        let values = rows
            .into_iter()
            .filter_map(|row| match row.into_iter().next() {
                Some(GenerationValue::Text(value)) => Some(value),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert_ne!(values[0], values[1]);
    }

    #[test]
    fn regex_expands_literals_and_digit_repeats() {
        let mut column = column("order_no", "regex", "varchar(32)");
        column
            .params
            .insert("pattern".into(), "ORD-2026\\d{5}".into());
        let spec = GenerationSpec {
            rows: 1,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column],
        };
        let rows = collect(&spec);
        let GenerationValue::Text(text) = &rows[0][0] else {
            panic!("expected text");
        };
        assert_eq!(text.len(), 13, "{text}");
        assert!(text.starts_with("ORD-2026"), "{text}");
        assert!(text[8..].chars().all(|ch| ch.is_ascii_digit()), "{text}");
    }

    #[test]
    fn foreign_ref_falls_back_and_warns_when_pool_empty() {
        let spec = GenerationSpec {
            rows: 2,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column("user_id", "foreign_ref", "bigint")],
        };
        let mut warnings = Vec::new();
        let pool = reference_pools(&spec, &mut warnings);
        assert!(pool[0].is_empty());
        assert_eq!(warnings.len(), 1);
        let rows = collect(&spec);
        assert!(matches!(rows[0][0], GenerationValue::Int(_)));
    }

    #[test]
    fn foreign_ref_uses_reference_pool() {
        let mut user_id = column("user_id", "foreign_ref", "bigint");
        user_id.reference_pool = vec![
            GenerationValue::Int(1422),
            GenerationValue::Int(1723),
            GenerationValue::Int(2001),
        ];
        let spec = GenerationSpec {
            rows: 6,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![user_id],
        };
        let rows = collect(&spec);
        for row in &rows {
            let GenerationValue::Int(value) = row[0] else {
                panic!("expected int");
            };
            assert!([1422, 1723, 2001].contains(&value));
        }
    }

    #[test]
    fn nullable_column_honours_null_percent() {
        let mut remark = column("remark", "lorem.word", "text");
        remark.nullable = true;
        remark.null_percent = 100;
        let spec = GenerationSpec {
            rows: 3,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![remark],
        };
        for row in collect(&spec) {
            assert!(matches!(row[0], GenerationValue::Null));
        }
    }

    #[test]
    fn copy_field_reads_previous_column() {
        let mut copy = column("order_copy", "copy_field", "varchar(32)");
        copy.params.insert("source".into(), "order_no".into());
        let spec = GenerationSpec {
            rows: 2,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column("order_no", "format", "varchar(32)"), copy],
        };
        for row in collect(&spec) {
            assert_eq!(row[0], row[1]);
        }
    }

    #[test]
    fn cancel_stops_generation() {
        let cancel = AtomicBool::new(true);
        let spec = GenerationSpec {
            rows: 1000,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column("name", "name.full", "varchar(32)")],
        };
        let error = generate_table_rows(&spec, &cancel, &mut |_| {}, &mut |_| Ok(()))
            .expect_err("cancelled");
        assert_eq!(error.kind, ErrorKind::Cancelled);
    }

    #[test]
    fn rows_are_capped() {
        let spec = GenerationSpec {
            rows: DATA_GENERATION_MAX_ROWS + 1,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column("name", "name.full", "varchar(32)")],
        };
        let error =
            generate_table_rows(&spec, &AtomicBool::new(false), &mut |_| {}, &mut |_| Ok(()))
                .expect_err("too many rows");
        assert_eq!(error.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn unknown_generator_is_rejected() {
        let spec = GenerationSpec {
            rows: 1,
            seed: "s".into(),
            locale: "zh_cn".into(),
            unique_conflict: UniqueConflict::Abort,
            columns: vec![column("x", "no.such", "text")],
        };
        let error =
            generate_table_rows(&spec, &AtomicBool::new(false), &mut |_| {}, &mut |_| Ok(()))
                .expect_err("unknown generator");
        assert_eq!(error.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn inference_matches_column_meaning() {
        assert_eq!(infer_generator("id", "bigint"), "sequence");
        assert_eq!(infer_generator("user_id", "bigint"), "sequence");
        assert_eq!(infer_generator("buyer_email", "varchar(128)"), "internet.email_safe");
        assert_eq!(infer_generator("created_at", "datetime"), "date.datetime_between");
        assert_eq!(infer_generator("status", "varchar(16)"), "categorical");
        assert_eq!(infer_generator("amount", "decimal(10,2)"), "random.decimal");
    }

    #[test]
    fn generator_compatibility_follows_column_family() {
        // 数值族互认；文本列只接受文本/任意生成器。
        assert!(generator_compatible_with_type("random.int", "bigint"));
        assert!(generator_compatible_with_type("random.decimal", "int"));
        assert!(generator_compatible_with_type("random.int", "decimal(10,2)"));
        assert!(!generator_compatible_with_type("random.int", "varchar(16)"));
        assert!(!generator_compatible_with_type("random.uuid", "datetime"));
        assert!(generator_compatible_with_type("random.uuid", "varchar(64)"));
        assert!(generator_compatible_with_type("random.boolean", "bool"));
        assert!(generator_compatible_with_type("date.recent", "timestamp"));
        assert!(!generator_compatible_with_type("random.boolean", "bigint"));
        // 任意族（常量/复制/外键/跳过）对所有列都可选。
        for id in ["constant", "copy_field", "foreign_ref", "skip", "default"] {
            assert!(generator_compatible_with_type(id, "bigint"), "{id}");
            assert!(generator_compatible_with_type(id, "text"), "{id}");
        }
    }
}
