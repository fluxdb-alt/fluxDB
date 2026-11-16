// 测试数据生成的应用层编排（无 UI 依赖）。
//
// 生成与写入分两步：
// 1. `fluxdb_core::generate_table_rows` 产出确定性数据行；
// 2. 行按目标列写成临时 JSON，交给现有导入管线执行（分批参数化 INSERT、进度、
//    取消、清空语义与类型转换都与「导入表数据」完全一致）。
//
// 外键取值池在这里读取：物理外键与用户自定义 ER 关系在桌面端已归一为
// `ForeignKeyInfo`，应用层只按给定的表/列取少量父表值，不关心关系来源。
// 临时文件只含生成数据，不含连接信息或凭据，结束（含失败/取消）后删除。

/// 生成源信息：目标列与可引用的外键关系，供界面初始化列配置。
#[derive(Clone, Debug, PartialEq)]
pub struct DataGenerationSource {
    pub columns: Vec<DataGenerationSourceColumn>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataGenerationSourceColumn {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
}

impl AppController {
    /// 读取目标表结构与外键关系（不执行写操作）。
    pub fn load_data_generation_source(
        &self,
        object: &ObjectPath,
    ) -> fluxdb_core::Result<DataGenerationSource> {
        let config = self
            .connection_config(object.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let metadata = fluxdb_connectors::connector_for(&config)?
            .load_table_design_metadata(object)?;
        // 唯一性只认单列唯一索引；复合唯一索引不能保证单列取值互不重复。
        let single_unique = metadata
            .indexes
            .iter()
            .filter(|index| index.is_unique && index.columns.len() == 1)
            .flat_map(|index| index.columns.iter().cloned())
            .collect::<BTreeSet<_>>();
        let columns = metadata
            .columns
            .iter()
            .map(|column| DataGenerationSourceColumn {
                name: column.name.clone(),
                type_name: column.type_name.clone().unwrap_or_default(),
                nullable: column.nullable,
                primary_key: column.primary_key,
                unique: column.primary_key || single_unique.contains(&column.name),
            })
            .collect();
        Ok(DataGenerationSource {
            columns,
            foreign_keys: metadata.foreign_keys,
        })
    }

    /// 生成指定行数并写入目标表。
    ///
    /// 调用方必须在后台线程运行；`cancel` 同时作用于生成与写入两个阶段。
    pub fn generate_table_data(
        &self,
        object: &ObjectPath,
        mut spec: fluxdb_core::GenerationSpec,
        mode: fluxdb_core::TableImportMode,
        batch_rows: usize,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(fluxdb_core::TableImportProgress),
    ) -> fluxdb_core::Result<fluxdb_core::GenerationReport> {
        let config = self
            .connection_config(object.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        ensure_read_only_allows(&config, fluxdb_core::WriteIntent::Mutation)?;
        let mut warnings = Vec::new();

        // 外键：按引用目标读取父表已有值；读不到时保留空池，生成阶段回退随机整数。
        for column in spec.columns.iter_mut() {
            let Some(reference) = column.reference.clone() else {
                continue;
            };
            match self.load_generation_reference_pool(object.connection_id, &reference) {
                Ok(pool) => column.reference_pool = pool,
                Err(error) => warnings.push(format!(
                    "{} 的父表 {} 读取失败，回退为随机整数：{}",
                    column.name, reference.label, error.message
                )),
            }
        }

        let mut temp = GenerationTempFile::create()?;
        let report = fluxdb_core::generate_table_rows(
            &spec,
            cancel,
            &mut |event| {
                progress(fluxdb_core::TableImportProgress {
                    stage: event.stage,
                    message: event.message,
                });
            },
            &mut |row| temp.write_row(&spec, row),
        )?;
        // 空池等告警由生成器给出；与上层读取失败合并，避免同一条重复。
        for warning in &report.warnings {
            if !warnings.contains(warning) {
                warnings.push(warning.clone());
            }
        }
        temp.finish()?;

        // 复用导入管线：JSON 会按目标列类型把数字/布尔/NULL 还原成参数。
        let options = fluxdb_core::TableImportOptions {
            encoding: String::new(),
            delimiter: String::new(),
            quote: String::new(),
            has_header: false,
            trim_whitespace: false,
            empty_as_null: false,
            mode,
            batch_rows,
        };
        let import = self.import_table_data(
            object,
            temp.path(),
            options,
            Vec::new(),
            cancel,
            progress,
        )?;
        for warning in import.warnings {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        Ok(fluxdb_core::GenerationReport {
            rows: import.imported_rows,
            // 唯一冲突跳过的行不会写入；保留计数让界面如实说明实际写入量。
            skipped_rows: report.skipped_rows,
            warnings,
        })
    }

    /// 读取父表某一列的前 N 个值，作为外键取值池。
    ///
    /// 预览与真实生成都从这里取池：界面打开弹窗时预取一次，生成前再刷新一次，
    /// 避免预览显示的是「池为空时的随机整数回退」。
    pub fn load_generation_reference_pool(
        &self,
        connection_id: ConnectionId,
        reference: &fluxdb_core::GenerationReference,
    ) -> fluxdb_core::Result<Vec<fluxdb_core::GenerationValue>> {
        let object = ObjectPath {
            connection_id,
            database: reference.database.clone(),
            schema: reference.schema.clone(),
            name: reference.table.clone(),
            kind: ObjectKind::Table,
        };
        let page = self.load_data_page(
            &object,
            Pagination::new(0, fluxdb_core::DATA_GENERATION_REFERENCE_POOL_MAX as u64),
            &[],
            &[],
            None,
        )?;
        let Some(index) = page
            .columns
            .iter()
            .position(|column| column.name == reference.column)
        else {
            return Err(Error::new(
                ErrorKind::Query,
                format!("父表 {} 没有列 {}", reference.table, reference.column),
            ));
        };
        Ok(page
            .rows
            .iter()
            .filter_map(|row| row.values.get(index))
            .filter_map(generation_value_from_cell)
            .collect())
    }
}

fn generation_value_from_cell(value: &CellValue) -> Option<fluxdb_core::GenerationValue> {
    use fluxdb_core::GenerationValue;
    match value {
        CellValue::Null => Some(GenerationValue::Null),
        CellValue::Bool(value) => Some(GenerationValue::Bool(*value)),
        CellValue::I64(value) => Some(GenerationValue::Int(*value)),
        CellValue::F64(value) if value.is_finite() => Some(GenerationValue::Float(*value)),
        CellValue::Text(value) | CellValue::Json(value) => {
            Some(GenerationValue::Text(value.clone()))
        }
        // 二进制/截断文本/大字段没有可直接复用的标量值，跳过而不是写入错误数据。
        _ => None,
    }
}

/// 生成结果临时文件；写入 JSON 数组，结束后删除。
///
/// 用 JSON 而不是 CSV：导入器会按目标列类型把数字/布尔/NULL 还原成参数，
/// 文本列也不会因为分隔符或引号被误切。
struct GenerationTempFile {
    path: std::path::PathBuf,
    file: Option<std::io::BufWriter<std::fs::File>>,
    first: bool,
}

impl GenerationTempFile {
    fn create() -> fluxdb_core::Result<Self> {
        let unique = format!(
            "fluxdb-generate-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        let path = std::env::temp_dir().join(unique);
        let file = std::fs::File::create(&path).map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("无法创建生成数据临时文件：{error}"),
            )
        })?;
        Ok(Self {
            path,
            file: Some(std::io::BufWriter::new(file)),
            first: true,
        })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 逐行写 JSON 对象；跳过列与「数据库默认值」列不写键，导入时自然保留默认值。
    fn write_row(
        &mut self,
        spec: &fluxdb_core::GenerationSpec,
        row: Vec<fluxdb_core::GenerationValue>,
    ) -> fluxdb_core::Result<()> {
        use std::io::Write;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "生成临时文件已关闭"))?;
        if self.first {
            writeln!(file, "[").map_err(generation_write_error)?;
            self.first = false;
        } else {
            writeln!(file, ",").map_err(generation_write_error)?;
        }
        let mut object = serde_json::Map::new();
        for (column, value) in spec.columns.iter().zip(row) {
            if matches!(column.generator.as_str(), "skip" | "default") {
                continue;
            }
            object.insert(column.name.clone(), generation_value_json(value));
        }
        let line = serde_json::to_string(&serde_json::Value::Object(object))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("生成数据序列化失败：{error}"),
                )
            })?;
        write!(file, "{line}").map_err(generation_write_error)
    }

    fn finish(&mut self) -> fluxdb_core::Result<()> {
        use std::io::Write;
        if let Some(mut file) = self.file.take() {
            if self.first {
                writeln!(file, "[").map_err(generation_write_error)?;
            }
            writeln!(file, "\n]").map_err(generation_write_error)?;
            file.flush().map_err(generation_write_error)?;
        }
        Ok(())
    }
}

impl Drop for GenerationTempFile {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn generation_value_json(value: fluxdb_core::GenerationValue) -> serde_json::Value {
    use fluxdb_core::GenerationValue;
    match value {
        GenerationValue::Null => serde_json::Value::Null,
        GenerationValue::Bool(value) => serde_json::Value::Bool(value),
        GenerationValue::Int(value) => serde_json::Value::Number(value.into()),
        GenerationValue::Float(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        GenerationValue::Decimal(value) | GenerationValue::Text(value) => {
            serde_json::Value::String(value)
        }
    }
}

fn generation_write_error(error: std::io::Error) -> Error {
    Error::new(
        ErrorKind::Internal,
        format!("写入生成数据临时文件失败：{error}"),
    )
}

#[cfg(test)]
mod data_generation_tests {
    use super::*;
    use fluxdb_core::{GenerationColumnSpec, GenerationSpec, GenerationValue};

    fn column(name: &str, generator: &str) -> GenerationColumnSpec {
        GenerationColumnSpec {
            name: name.to_string(),
            type_name: "text".to_string(),
            nullable: true,
            unique: false,
            generator: generator.to_string(),
            params: BTreeMap::new(),
            null_percent: 0,
            reference: None,
            reference_pool: Vec::new(),
        }
    }

    #[test]
    fn temp_file_writes_json_array_and_omits_skipped_columns() {
        let spec = GenerationSpec {
            rows: 1,
            seed: "s".to_string(),
            locale: "en".to_string(),
            unique_conflict: fluxdb_core::UniqueConflict::Abort,
            columns: vec![column("id", "sequence"), column("note", "skip")],
        };
        let mut temp = GenerationTempFile::create().expect("create temp file");
        let path = temp.path().to_path_buf();
        temp.write_row(&spec, vec![GenerationValue::Int(1), GenerationValue::Null])
            .expect("write row");
        temp.finish().expect("finish");
        let text = std::fs::read_to_string(&path).expect("read temp file");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed[0]["id"], 1);
        // 跳过列不写键，导入时不会覆盖数据库默认值。
        assert!(parsed[0].get("note").is_none());
        drop(temp);
        assert!(!path.exists(), "临时文件应在生成结束后删除");
    }
}
