/// 备份记录：备份真实数据在磁盘，这里只存备份的元数据记录。
/// 一条记录对应一个实际格式决定后缀的备份文件。
///
/// - `connection_id` / `database`：归属的连接与库，备份列表按此筛选。
/// - `output_path`：备份真实文件的完整磁盘路径，用于删除定位与路径列展示。
/// - `created_unix` / `size`：备份时间与文件大小（原读文件系统，现存记录，避免依赖磁盘）。
/// - `tables` / `include_views` / `note`：备份表清单、视图开关、备注。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupRecord {
    pub connection_id: ConnectionId,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub created_unix: i64,
    #[serde(default)]
    pub size: u64,
    /// Some(空数组) = 整库备份；None = 无表清单记录。
    #[serde(default)]
    pub tables: Option<Vec<String>>,
    #[serde(default)]
    pub include_views: bool,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub manifest: Option<fluxdb_core::BackupManifest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreRecord {
    pub source: String,
    pub connection_id: ConnectionId,
    pub target: String,
    pub finished_unix: u64,
    pub success: bool,
    pub canceled: bool,
    pub result: String,
}

impl FileStorage {
    pub fn load_restore_records(&self) -> Result<Vec<RestoreRecord>> {
        let conn = self.open_sqlite()?;
        Ok(sqlite::get_json::<Vec<RestoreRecord>>(&conn, "restore_records")?.unwrap_or_default())
    }
    pub fn save_restore_records(&self, records: &[RestoreRecord]) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, "restore_records", &records.to_vec())
    }
}
