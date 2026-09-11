// 写入值意图三态 DEFAULT / NULL / 值（T11）。
//
// 数据编辑需区分「写入数据库默认值」（DEFAULT）与「显式写 NULL」，
// 二者在既有 `CellValue` 中无法表达（`Null` 与「省略」冲突）。
// 独立类型表达写入意图，不污染只读的 `CellValue` / `Row.values`。
//
// 旧 MySQL 适配器保留旧语义（NULL 视为省略 → DEFAULT），
// PG 在 `DataChangeSet.insert_intents` 携带时按三态落库。

#[derive(Clone, Debug, PartialEq)]
pub enum WriteValue {
    /// 不写该列，由数据库默认值填充（DEFAULT）。等价于省略列。
    Default,
    /// 显式写 NULL。
    Null,
    /// 写具体值。
    Value(CellValue),
}

/// 直接把只读值转成「写具体值」意图（连接器默认回退路径共用）。
impl From<CellValue> for WriteValue {
    fn from(value: CellValue) -> Self {
        WriteValue::Value(value)
    }
}
