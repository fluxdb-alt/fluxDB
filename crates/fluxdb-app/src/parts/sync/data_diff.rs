//! 数据差异算法（纯逻辑，不连库）。
//!
//! 核心思路：以「比对键 → 行指纹」为目标端索引，流式扫描源端逐行分类。
//!
//! - 内存保留目标端的键值 + `u64` 指纹；为了在预览里给出字段级新旧值，
//!   额外保留有限条数的目标行（见 `DETAIL_ROW_LIMIT`），源端不落内存。
//! - 键用规范化字节序列编码，复合键做长度前缀，避免拼接歧义。
//! - 指纹覆盖两端同名字段（按目标端字段顺序），不做「按列求差」——UPDATE 时
//!   统一重写非键字段即可，语义更简单，也不会漏掉 NULL 与空串的区别。
//! - 未完整读取的值（`TruncatedText` / `LargeValue` / `BinarySummary`）一律
//!   报错跳过：拿预览片段比对会把「读取不全」伪装成「内容不同」，进而触发
//!   错误的 DELETE/UPDATE。
//!
//! 已知边界：字符串按字节精确比较。源端与目标端若分别受不同大小写敏感排序规则
//! 影响，可能产生多余的 UPDATE；重复执行是幂等的，不会写坏数据。

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use fluxdb_core::{CellValue, Error, ErrorKind};

use super::column_types::{ColumnFamily, coerce_value};
use super::model::{RowDiffAction, RowDiffSample};

/// 单表目标行数上限。超过后直接报错，避免一次同步把进程内存打满。
/// `ponytail:` 目前是 O(目标行数) 的内存索引；超大表后续改成落临时表分片比对。
pub(crate) const MAX_TARGET_ROWS: usize = 2_000_000;

/// 每张表保留的预览样本上限。
pub(crate) const SAMPLE_LIMIT: usize = 100;

/// 保留目标行原文用于「字段级新旧值」预览的行数上限。
///
/// 超过上限只保留键值与指纹，样本降级为整行差异（不带新旧值），
/// 避免大表比对时把整表数据再复制一份进内存。
pub(crate) const DETAIL_ROW_LIMIT: usize = 200_000;

/// 目标行原文缓存的估算字节上限（约 64 MiB）。
///
/// 行数上限挡不住「行少但字段巨大」的表，两个上限同时生效才安全。
pub(crate) const DETAIL_BYTES_LIMIT: usize = 64 * 1024 * 1024;

const TAG_NULL: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_NUMBER: u8 = 2;
const TAG_TEXT: u8 = 3;
const TAG_BYTES: u8 = 4;
const TAG_JSON: u8 = 5;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum KeyPlanError {
    /// 目标表没有主键且未指定比对键。
    NoKey,
    /// 指定的键列在某一端不存在。
    MissingColumn(String),
    /// 两端没有同名字段。
    NoSharedColumns,
}

/// 两端字段映射结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ColumnPlan {
    /// 键列在源端/目标端的实际字段名（大小写以各自表为准）。
    pub key_source_names: Vec<String>,
    pub key_target_names: Vec<String>,
    /// `(源端下标, 目标端下标)`。
    pub key_pairs: Vec<(usize, usize)>,
    /// 参与指纹比对的字段，按目标端字段顺序；不含键列。
    pub compare_pairs: Vec<(usize, usize)>,
    pub only_in_source: Vec<String>,
    pub only_in_target: Vec<String>,
    /// 目标端字段名，按目标端顺序；用于错误提示与类型规范化。
    pub target_names: Vec<String>,
    /// 目标端字段类型族，与 `target_names` 同下标。默认 `Unknown` 表示不转换。
    pub target_families: Vec<ColumnFamily>,
}

/// 解析比对键：优先用户指定，否则用目标表主键。
///
/// 返回的是目标端字段名；源端是否存在由这里一并校验，避免到扫描阶段才失败。
pub(crate) fn resolve_key_columns(
    requested: &[String],
    source_names: &[String],
    target_names: &[String],
    target_primary_key: &[String],
) -> Result<Vec<String>, KeyPlanError> {
    let requested = requested
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if requested.is_empty() {
        if target_primary_key.is_empty() {
            return Err(KeyPlanError::NoKey);
        }
        return Ok(target_primary_key.to_vec());
    }
    let mut keys = Vec::with_capacity(requested.len());
    for name in requested {
        if find_column(source_names, name).is_none() {
            return Err(KeyPlanError::MissingColumn(name.to_string()));
        }
        let Some(target_index) = find_column(target_names, name) else {
            return Err(KeyPlanError::MissingColumn(name.to_string()));
        };
        keys.push(target_names[target_index].clone());
    }
    Ok(keys)
}

/// 建立两端字段映射。键列必须两端都存在；其余同名字段进入指纹比对。
pub(crate) fn plan_columns(
    source_names: &[String],
    target_names: &[String],
    key_columns: &[String],
) -> Result<ColumnPlan, KeyPlanError> {
    let mut key_source_names = Vec::with_capacity(key_columns.len());
    let mut key_target_names = Vec::with_capacity(key_columns.len());
    let mut key_pairs = Vec::with_capacity(key_columns.len());
    for key in key_columns {
        let Some(source_index) = find_column(source_names, key) else {
            return Err(KeyPlanError::MissingColumn(key.clone()));
        };
        let Some(target_index) = find_column(target_names, key) else {
            return Err(KeyPlanError::MissingColumn(key.clone()));
        };
        key_source_names.push(source_names[source_index].clone());
        key_target_names.push(target_names[target_index].clone());
        key_pairs.push((source_index, target_index));
    }

    let mut compare_pairs = Vec::new();
    let mut only_in_source = Vec::new();
    for (source_index, name) in source_names.iter().enumerate() {
        if key_source_names
            .iter()
            .any(|key| key.eq_ignore_ascii_case(name))
        {
            continue;
        }
        match find_column(target_names, name) {
            Some(target_index)
                if !key_target_names
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(&target_names[target_index])) =>
            {
                compare_pairs.push((source_index, target_index));
            }
            _ => only_in_source.push(name.clone()),
        }
    }
    compare_pairs.sort_by_key(|(_, target_index)| *target_index);

    let only_in_target = target_names
        .iter()
        .filter(|name| {
            !key_target_names
                .iter()
                .any(|key| key.eq_ignore_ascii_case(name))
                && find_column(source_names, name).is_none()
        })
        .cloned()
        .collect::<Vec<_>>();

    if key_pairs.is_empty() && compare_pairs.is_empty() {
        return Err(KeyPlanError::NoSharedColumns);
    }
    Ok(ColumnPlan {
        key_source_names,
        key_target_names,
        key_pairs,
        compare_pairs,
        only_in_source,
        only_in_target,
        target_families: vec![ColumnFamily::Unknown; target_names.len()],
        target_names: target_names.to_vec(),
    })
}

impl ColumnPlan {
    /// 注入目标列类型族。长度不足的部分保持 `Unknown`（不转换）。
    pub(crate) fn with_target_families(mut self, families: Vec<ColumnFamily>) -> Self {
        self.target_families = families;
        self.target_families
            .resize(self.target_names.len(), ColumnFamily::Unknown);
        self
    }

    /// 跳过指定字段：仅从比对集合移除，键列不受影响。
    pub(crate) fn skip_columns(mut self, skip: &[String]) -> Self {
        if skip.is_empty() {
            return self;
        }
        self.compare_pairs.retain(|(_, target_index)| {
            self.target_names.get(*target_index).is_none_or(|name| {
                !skip
                    .iter()
                    .any(|skipped| skipped.eq_ignore_ascii_case(name))
            })
        });
        self
    }

    /// 把源行中参与同步的字段按目标类型族规范化；未映射字段原样保留。
    pub(crate) fn coerce_source_row(
        &self,
        row: &[CellValue],
    ) -> fluxdb_core::Result<Vec<CellValue>> {
        let mut normalized = row.to_vec();
        for (source_index, target_index) in self.key_pairs.iter().chain(&self.compare_pairs) {
            let Some(value) = row.get(*source_index) else {
                return Err(Error::new(ErrorKind::Internal, "行值与字段数量不一致"));
            };
            normalized[*source_index] = self.coerce(value, *target_index)?;
        }
        Ok(normalized)
    }

    fn coerce(&self, value: &CellValue, target_index: usize) -> fluxdb_core::Result<CellValue> {
        let family = self
            .target_families
            .get(target_index)
            .copied()
            .unwrap_or(ColumnFamily::Unknown);
        coerce_value(value, family).map_err(|reason| {
            let name = self
                .target_names
                .get(target_index)
                .map(String::as_str)
                .unwrap_or("?");
            Error::new(ErrorKind::Unsupported, format!("字段 {name} {reason}"))
        })
    }
}

fn find_column(columns: &[String], name: &str) -> Option<usize> {
    columns
        .iter()
        .position(|column| column == name)
        .or_else(|| {
            columns
                .iter()
                .position(|column| column.eq_ignore_ascii_case(name))
        })
}

/// 源端一行相对目标端的判定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceRowAction {
    Insert,
    Update,
    Unchanged,
}

struct TargetEntry {
    fingerprint: u64,
    key_values: Vec<CellValue>,
    /// 目标行原文，仅在前 `DETAIL_ROW_LIMIT` 行保留；用于给出变更字段的新旧值。
    row: Option<Vec<CellValue>>,
}

/// 单表差异状态机。目标端先灌入，源端逐行喂入，最后收尾取删除行。
pub(crate) struct TableDiff {
    plan: ColumnPlan,
    target: HashMap<Vec<u8>, TargetEntry>,
    samples: Vec<RowDiffSample>,
    /// 已缓存的目标行原文估算字节数，用于限制 `DETAIL_BYTES_LIMIT`。
    detail_bytes: usize,
    inserted: u64,
    updated: u64,
    unchanged: u64,
}

impl TableDiff {
    pub(crate) fn new(plan: ColumnPlan) -> Self {
        Self {
            plan,
            target: HashMap::new(),
            samples: Vec::new(),
            detail_bytes: 0,
            inserted: 0,
            updated: 0,
            unchanged: 0,
        }
    }

    /// 灌入目标端一行。键重复说明「主键并不唯一」，此时删除/更新都不可判定，
    /// 必须报错而不是覆盖索引项。
    pub(crate) fn absorb_target(&mut self, row: &[CellValue]) -> fluxdb_core::Result<()> {
        if self.target.len() >= MAX_TARGET_ROWS {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("目标行数超过 {MAX_TARGET_ROWS}，暂不支持在内存中比对"),
            ));
        }
        let (key, key_values) = self.encode_key(row, true)?;
        let fingerprint = self.fingerprint(row, true)?;
        // 大字段同行数都要设上限：只按行数限制会让「少行超大字段」的表把内存打满。
        let weight = row_weight(row);
        let detail_row = if self.target.len() < DETAIL_ROW_LIMIT
            && self.detail_bytes + weight <= DETAIL_BYTES_LIMIT
        {
            self.detail_bytes += weight;
            Some(row.to_vec())
        } else {
            None
        };
        if self
            .target
            .insert(
                key,
                TargetEntry {
                    fingerprint,
                    key_values,
                    row: detail_row,
                },
            )
            .is_some()
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "目标表存在重复的比对键，无法安全同步",
            ));
        }
        Ok(())
    }

    /// 灌入源端一行，返回该行应执行的动作。
    pub(crate) fn absorb_source(
        &mut self,
        row: &[CellValue],
    ) -> fluxdb_core::Result<SourceRowAction> {
        let (key, key_values) = self.encode_key(row, false)?;
        let fingerprint = self.fingerprint(row, false)?;
        let action = match self.target.remove(&key) {
            None => {
                self.inserted += 1;
                self.push_sample(RowDiffAction::Insert, &key_values, None);
                SourceRowAction::Insert
            }
            Some(entry) if entry.fingerprint == fingerprint => {
                self.unchanged += 1;
                SourceRowAction::Unchanged
            }
            Some(entry) => {
                self.updated += 1;
                // 样本用满后不再逐字段比对，避免大表为展示细节多付一轮开销。
                if self.samples.len() < SAMPLE_LIMIT {
                    let changes = self.changed_fields(row, entry.row.as_deref())?;
                    self.push_update_samples(&key_values, &changes);
                }
                SourceRowAction::Update
            }
        };
        Ok(action)
    }

    /// 收尾：目标端剩余行即「源端不存在」，返回其键值供 DELETE 使用。
    pub(crate) fn finish(mut self) -> TableDiffOutcome {
        let mut remaining = self
            .target
            .drain()
            .map(|(_, entry)| entry.key_values)
            .collect::<Vec<_>>();
        // 输出顺序固定，便于测试与脚本预览稳定。
        remaining.sort_by(|left, right| compare_key_values(left, right));
        for key_values in remaining
            .iter()
            .take(SAMPLE_LIMIT.saturating_sub(self.samples.len()))
        {
            self.push_sample(RowDiffAction::Delete, key_values, None);
        }
        TableDiffOutcome {
            inserted: self.inserted,
            updated: self.updated,
            unchanged: self.unchanged,
            deleted: remaining.len() as u64,
            deleted_keys: remaining,
            samples: self.samples,
            columns_only_in_source: self.plan.only_in_source.clone(),
            columns_only_in_target: self.plan.only_in_target.clone(),
            key_columns: self.plan.key_target_names.clone(),
            column_count: self.plan.key_pairs.len() + self.plan.compare_pairs.len(),
        }
    }

    /// 逐字段比较源行与目标行，返回 `(字段名, 源值, 目标值)`。
    ///
    /// 目标行未保留（超出 `DETAIL_ROW_LIMIT`）时返回空，调用方降级为整行样本。
    fn changed_fields(
        &self,
        source_row: &[CellValue],
        target_row: Option<&[CellValue]>,
    ) -> fluxdb_core::Result<Vec<(String, String, String)>> {
        let Some(target_row) = target_row else {
            return Ok(Vec::new());
        };
        let mut changes = Vec::new();
        for (source_index, target_index) in &self.plan.compare_pairs {
            let source_value = self.row_value(source_row, *source_index, *target_index)?;
            let target_value = self.row_value(target_row, *target_index, *target_index)?;
            if canonical_bytes(&source_value)? == canonical_bytes(&target_value)? {
                continue;
            }
            let column = self
                .plan
                .target_names
                .get(*target_index)
                .cloned()
                .unwrap_or_default();
            changes.push((
                column,
                sample_label(&source_value),
                sample_label(&target_value),
            ));
        }
        Ok(changes)
    }

    fn row_value(
        &self,
        row: &[CellValue],
        index: usize,
        target_index: usize,
    ) -> fluxdb_core::Result<CellValue> {
        let value = row
            .get(index)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "行值与字段数量不一致"))?;
        self.plan.coerce(value, target_index)
    }

    /// 按字段展开更新样本：一个变更字段一行，便于详情表逐行对照新旧值。
    fn push_update_samples(
        &mut self,
        key_values: &[CellValue],
        changes: &[(String, String, String)],
    ) {
        if changes.is_empty() {
            self.push_sample(RowDiffAction::Update, key_values, None);
            return;
        }
        for (column, source, target) in changes {
            if self.samples.len() >= SAMPLE_LIMIT {
                return;
            }
            self.push_sample(
                RowDiffAction::Update,
                key_values,
                Some((column.clone(), source.clone(), target.clone())),
            );
        }
    }

    fn push_sample(
        &mut self,
        action: RowDiffAction,
        key_values: &[CellValue],
        change: Option<(String, String, String)>,
    ) {
        if self.samples.len() >= SAMPLE_LIMIT {
            return;
        }
        let (column, source_value, target_value) = match change {
            Some((column, source, target)) => (Some(column), Some(source), Some(target)),
            None => (None, None, None),
        };
        self.samples.push(RowDiffSample {
            action,
            key_values: self
                .plan
                .key_target_names
                .iter()
                .cloned()
                .zip(key_values.iter().map(sample_label))
                .collect(),
            column,
            source_value,
            target_value,
        });
    }

    fn fingerprint(&self, row: &[CellValue], target_side: bool) -> fluxdb_core::Result<u64> {
        let mut hasher = DefaultHasher::new();
        for (source_index, target_index) in &self.plan.compare_pairs {
            let index = if target_side {
                *target_index
            } else {
                *source_index
            };
            let value = self.plan.coerce(
                row.get(index)
                    .ok_or_else(|| Error::new(ErrorKind::Internal, "行值与字段数量不一致"))?,
                *target_index,
            )?;
            write_canonical(&mut hasher, &value).map_err(|_| unsupported_value(index))?;
        }
        Ok(hasher.finish())
    }

    /// 键编码：复合键逐列做「标签 + 长度前缀」，避免 `1|23` 与 `12|3` 撞键。
    fn encode_key(
        &self,
        row: &[CellValue],
        target_side: bool,
    ) -> fluxdb_core::Result<(Vec<u8>, Vec<CellValue>)> {
        let mut bytes = Vec::new();
        let mut values = Vec::with_capacity(self.plan.key_pairs.len());
        for (source_index, target_index) in &self.plan.key_pairs {
            let index = if target_side {
                *target_index
            } else {
                *source_index
            };
            let value = row
                .get(index)
                .ok_or_else(|| Error::new(ErrorKind::Internal, "行值与字段数量不一致"))?;
            let normalized = self.plan.coerce(value, *target_index)?;
            write_canonical(&mut bytes, &normalized).map_err(|_| unsupported_value(index))?;
            // 保留目标行原值供 DELETE 使用；规范化只用于键匹配，不改变身份参数。
            values.push(value.clone());
        }
        Ok((bytes, values))
    }
}

pub(crate) struct TableDiffOutcome {
    pub inserted: u64,
    pub updated: u64,
    pub unchanged: u64,
    pub deleted: u64,
    /// 目标端剩余键值，执行阶段据此生成 DELETE；顺序稳定。
    pub deleted_keys: Vec<Vec<CellValue>>,
    pub samples: Vec<RowDiffSample>,
    pub columns_only_in_source: Vec<String>,
    pub columns_only_in_target: Vec<String>,
    pub key_columns: Vec<String>,
    /// 参与同步的字段总数（键列 + 比对列）。
    pub column_count: usize,
}

fn unsupported_value(index: usize) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("第 {} 个字段含未完整读取的值，无法安全比对", index + 1),
    )
}

/// 行的估算内存占用：字符串/二进制按长度计，其余按 8 字节计。
fn row_weight(row: &[CellValue]) -> usize {
    row.iter()
        .map(|value| match value {
            CellValue::Text(text) | CellValue::Json(text) => text.len(),
            CellValue::Bytes(bytes) => bytes.len(),
            CellValue::BinarySummary(summary) => summary.byte_length as usize,
            _ => 8,
        })
        .sum()
}

/// 单值规范化字节序列，用于逐字段判断是否真的发生变化。
fn canonical_bytes(value: &CellValue) -> fluxdb_core::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    write_canonical(&mut buffer, value).map_err(|_| {
        Error::new(
            ErrorKind::Unsupported,
            "字段含未完整读取的值，无法给出字段级差异",
        )
    })?;
    Ok(buffer)
}

fn sample_label(value: &CellValue) -> String {
    let label = value.display_label();
    if label.chars().count() <= 48 {
        label
    } else {
        let mut truncated = label.chars().take(45).collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

fn compare_key_values(left: &[CellValue], right: &[CellValue]) -> std::cmp::Ordering {
    left.iter()
        .map(CellValue::display_label)
        .cmp(right.iter().map(CellValue::display_label))
}

/// 把值写入规范化字节序列。数字统一：整数值的浮点与整数编码一致，
/// 避免 `1` 与 `1.0` 被当成不同内容反复 UPDATE。
fn write_canonical<H: CanonicalSink>(
    sink: &mut H,
    value: &CellValue,
) -> Result<(), UnsupportedValue> {
    match value {
        CellValue::Null => sink.write(TAG_NULL, &[]),
        CellValue::Bool(flag) => sink.write(TAG_BOOL, &[u8::from(*flag)]),
        CellValue::I64(number) => sink.write(TAG_NUMBER, &number.to_be_bytes()),
        CellValue::F64(number) => write_float(sink, *number),
        CellValue::Text(text) => sink.write(TAG_TEXT, text.as_bytes()),
        CellValue::Json(text) => sink.write(TAG_JSON, text.as_bytes()),
        CellValue::Bytes(bytes) => sink.write(TAG_BYTES, bytes),
        CellValue::BinarySummary(_)
        | CellValue::TruncatedText { .. }
        | CellValue::LargeValue(_) => {
            return Err(UnsupportedValue);
        }
    }
    Ok(())
}

fn write_float<H: CanonicalSink>(sink: &mut H, number: f64) {
    // -0.0 与 0.0 在 SQL 里相等；NaN 统一成同一个位模式。
    let normalized = if number == 0.0 {
        0.0
    } else if number.is_nan() {
        f64::NAN
    } else {
        number
    };
    if normalized.fract() == 0.0 && normalized >= i64::MIN as f64 && normalized <= i64::MAX as f64 {
        sink.write(TAG_NUMBER, &(normalized as i64).to_be_bytes());
    } else {
        sink.write(TAG_NUMBER, &normalized.to_bits().to_be_bytes());
    }
}

struct UnsupportedValue;

/// 同时服务「字节键」与「指纹」两种输出：前者带长度前缀，后者只累加。
trait CanonicalSink {
    fn write(&mut self, tag: u8, payload: &[u8]);
}

impl CanonicalSink for Vec<u8> {
    fn write(&mut self, tag: u8, payload: &[u8]) {
        self.push(tag);
        self.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        self.extend_from_slice(payload);
    }
}

impl CanonicalSink for DefaultHasher {
    fn write(&mut self, tag: u8, payload: &[u8]) {
        self.write_u8(tag);
        self.write_u64(payload.len() as u64);
        Hasher::write(self, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(source: &[&str], target: &[&str], keys: &[&str]) -> ColumnPlan {
        plan_columns(
            &source
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            &target
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            &keys.iter().map(|name| name.to_string()).collect::<Vec<_>>(),
        )
        .expect("plan")
    }

    fn row(values: Vec<CellValue>) -> Vec<CellValue> {
        values
    }

    #[test]
    fn plan_maps_columns_by_name_ignoring_case() {
        let plan = plan(&["ID", "name", "extra"], &["Name", "id", "note"], &["id"]);
        assert_eq!(plan.key_pairs, vec![(0, 1)]);
        assert_eq!(plan.compare_pairs, vec![(1, 0)]);
        assert_eq!(plan.only_in_source, vec!["extra"]);
        assert_eq!(plan.only_in_target, vec!["note"]);
    }

    #[test]
    fn classifies_insert_update_delete() {
        let mut diff = TableDiff::new(plan(&["id", "name"], &["id", "name"], &["id"]));
        diff.absorb_target(&row(vec![CellValue::I64(2), CellValue::Text("b".into())]))
            .expect("target 2");
        diff.absorb_target(&row(vec![CellValue::I64(3), CellValue::Text("old".into())]))
            .expect("target 3");
        diff.absorb_target(&row(vec![
            CellValue::I64(4),
            CellValue::Text("gone".into()),
        ]))
        .expect("target 4");

        assert_eq!(
            diff.absorb_source(&row(vec![CellValue::I64(1), CellValue::Text("new".into())]))
                .expect("source 1"),
            SourceRowAction::Insert
        );
        assert_eq!(
            diff.absorb_source(&row(vec![CellValue::I64(2), CellValue::Text("b".into())]))
                .expect("source 2"),
            SourceRowAction::Unchanged
        );
        assert_eq!(
            diff.absorb_source(&row(vec![CellValue::I64(3), CellValue::Text("new".into())]))
                .expect("source 3"),
            SourceRowAction::Update
        );

        let outcome = diff.finish();
        assert_eq!(outcome.inserted, 1);
        assert_eq!(outcome.updated, 1);
        assert_eq!(outcome.unchanged, 1);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.key_columns, vec!["id".to_string()]);
    }

    #[test]
    fn update_samples_carry_changed_columns() {
        let mut diff = TableDiff::new(plan(
            &["id", "name", "note"],
            &["id", "name", "note"],
            &["id"],
        ));
        diff.absorb_target(&row(vec![
            CellValue::I64(1),
            CellValue::Text("a".into()),
            CellValue::Text("x".into()),
        ]))
        .expect("target");
        diff.absorb_source(&row(vec![
            CellValue::I64(1),
            CellValue::Text("b".into()),
            CellValue::Text("x".into()),
        ]))
        .expect("source");

        let outcome = diff.finish();
        assert_eq!(outcome.samples.len(), 1);
        let sample = &outcome.samples[0];
        assert_eq!(sample.action, RowDiffAction::Update);
        assert_eq!(sample.column.as_deref(), Some("name"));
        assert_eq!(sample.source_value.as_deref(), Some("b"));
        assert_eq!(sample.target_value.as_deref(), Some("a"));
        assert_eq!(outcome.column_count, 3);
    }

    #[test]
    fn insert_samples_have_no_field_detail() {
        let mut diff = TableDiff::new(plan(&["id", "name"], &["id", "name"], &["id"]));
        diff.absorb_source(&row(vec![CellValue::I64(1), CellValue::Text("a".into())]))
            .expect("source");

        let outcome = diff.finish();
        assert_eq!(outcome.samples.len(), 1);
        let sample = &outcome.samples[0];
        assert_eq!(sample.action, RowDiffAction::Insert);
        assert!(sample.column.is_none());
        assert!(sample.source_value.is_none());
        assert!(sample.target_value.is_none());
    }

    #[test]
    fn integer_and_integral_float_are_equal() {
        let mut diff = TableDiff::new(plan(&["id", "amount"], &["id", "amount"], &["id"]));
        diff.absorb_target(&row(vec![CellValue::I64(1), CellValue::I64(10)]))
            .expect("target");
        assert_eq!(
            diff.absorb_source(&row(vec![CellValue::I64(1), CellValue::F64(10.0)]))
                .expect("source"),
            SourceRowAction::Unchanged
        );
        let outcome = diff.finish();
        assert_eq!(outcome.updated, 0);
        assert_eq!(outcome.unchanged, 1);
    }

    #[test]
    fn null_differs_from_empty_text() {
        let mut diff = TableDiff::new(plan(&["id", "name"], &["id", "name"], &["id"]));
        diff.absorb_target(&row(vec![CellValue::I64(1), CellValue::Null]))
            .expect("target");
        assert_eq!(
            diff.absorb_source(&row(vec![
                CellValue::I64(1),
                CellValue::Text(String::new())
            ]))
            .expect("source"),
            SourceRowAction::Update
        );
    }

    #[test]
    fn composite_key_components_do_not_collide() {
        let mut diff = TableDiff::new(plan(&["a", "b"], &["a", "b"], &["a", "b"]));
        diff.absorb_target(&row(vec![
            CellValue::Text("a".into()),
            CellValue::Text("bc".into()),
        ]))
        .expect("target");
        assert_eq!(
            diff.absorb_source(&row(vec![
                CellValue::Text("ab".into()),
                CellValue::Text("c".into()),
            ]))
            .expect("source"),
            SourceRowAction::Insert
        );
    }

    #[test]
    fn truncated_values_reject_the_table() {
        let mut diff = TableDiff::new(plan(&["id", "body"], &["id", "body"], &["id"]));
        let error = diff
            .absorb_target(&row(vec![
                CellValue::I64(1),
                CellValue::TruncatedText {
                    preview: "head".into(),
                    total_chars: 10_000,
                },
            ]))
            .expect_err("truncated value must fail");
        assert_eq!(error.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn duplicate_target_key_is_rejected() {
        let mut diff = TableDiff::new(plan(&["id", "name"], &["id", "name"], &["id"]));
        diff.absorb_target(&row(vec![CellValue::I64(1), CellValue::Text("a".into())]))
            .expect("first");
        let error = diff
            .absorb_target(&row(vec![CellValue::I64(1), CellValue::Text("b".into())]))
            .expect_err("duplicate key must fail");
        assert_eq!(error.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn cross_type_values_compare_after_target_coercion() {
        let plan = plan(&["id", "amount"], &["id", "amount"], &["id"])
            .with_target_families(vec![ColumnFamily::Integer, ColumnFamily::Integer]);
        let mut diff = TableDiff::new(plan);
        diff.absorb_target(&row(vec![CellValue::I64(1), CellValue::I64(10)]))
            .expect("target");
        // 源端驱动把整数读成文本时，仍按目标列类型归一后视为相同。
        assert_eq!(
            diff.absorb_source(&row(vec![
                CellValue::Text("1".into()),
                CellValue::Text("10".into()),
            ]))
            .expect("source"),
            SourceRowAction::Unchanged
        );
    }

    #[test]
    fn skipped_columns_do_not_trigger_updates() {
        let plan = plan(&["id", "amount"], &["id", "amount"], &["id"])
            .with_target_families(vec![ColumnFamily::Integer, ColumnFamily::Integer])
            .skip_columns(&["amount".into()]);
        let mut diff = TableDiff::new(plan);
        diff.absorb_target(&row(vec![CellValue::I64(1), CellValue::I64(10)]))
            .expect("target");
        assert_eq!(
            diff.absorb_source(&row(vec![CellValue::I64(1), CellValue::I64(999)]))
                .expect("source"),
            SourceRowAction::Unchanged
        );
    }

    #[test]
    fn coerce_source_row_uses_target_types() {
        let plan = plan(&["id", "amount"], &["id", "amount"], &["id"])
            .with_target_families(vec![ColumnFamily::Integer, ColumnFamily::Bool]);
        let normalized = plan
            .coerce_source_row(&row(vec![CellValue::Text("1".into()), CellValue::I64(1)]))
            .expect("coerce");
        assert_eq!(normalized[0], CellValue::I64(1));
        assert_eq!(normalized[1], CellValue::Bool(true));
    }

    #[test]
    fn missing_primary_key_requires_explicit_key() {
        let source = vec!["id".to_string(), "name".to_string()];
        let target = source.clone();
        assert_eq!(
            resolve_key_columns(&[], &source, &target, &[]).expect_err("no key"),
            KeyPlanError::NoKey
        );
        assert_eq!(
            resolve_key_columns(&["name".into()], &source, &target, &[]).expect("explicit key"),
            vec!["name".to_string()]
        );
    }
}
