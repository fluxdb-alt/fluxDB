#[derive(Clone, Debug, PartialEq)]
pub struct DataChangeSet {
    pub object: ObjectPath,
    pub inserts: Vec<Row>,
    pub updates: Vec<RowUpdate>,
    pub deletes: Vec<RowIdentity>,
}

impl DataChangeSet {
    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.updates.is_empty() && self.deletes.is_empty()
    }

    pub fn dirty_cell_count(&self) -> usize {
        let updated = self
            .updates
            .iter()
            .map(|update| update.cells.len())
            .sum::<usize>();

        self.inserts.len() + updated
    }
}

/// Hash 字段写入时对该字段 TTL 的处置方式。
///
/// Redis 7.4 起 `HSET` 会清掉字段原有的 TTL（等价于不带 KEEPTTL 的 SET），
/// 因此「只改值」必须显式声明 Keep 并由连接器补回，否则 TTL 会被静默抹掉。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RedisHashFieldTtl {
    /// 保留字段原有 TTL：写入前读 HPTTL，写入后用 HPEXPIRE 补回。
    #[default]
    Keep,
    /// 清除 TTL，让字段变为永不过期（HPERSIST）。
    Persist,
    /// 设置为指定秒数（HEXPIRE）。
    Seconds(u64),
}

/// Redis 服务端三元版本号，用于字段级 TTL 等能力开关判定。
///
/// 字段级 TTL（HEXPIRE/HPEXPIRE/HPERSIST/HPTTL）自 Redis 7.4 起才可用，
/// 低版本执行会返回 unknown command 错误，因此 UI 需按版本决定是否放开 TTL 编辑。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct RedisServerVersion {
    /// 主版本号，如 `7`。
    pub major: u64,
    /// 次版本号，如 `4`。
    pub minor: u64,
    /// 修订号，如 `0`。
    pub patch: u64,
}

impl RedisServerVersion {
    /// 解析形如 `7.4.0` / `6.2.7` 的版本串。缺省位按 0 补齐；
    /// patch 前的 `-rc1` 等后缀容错（只取前导数字）。非法输入返回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = [0u64; 3];
        let mut count = 0usize;
        for part in value.split('.') {
            if count >= 3 {
                break;
            }
            // 取前导数字段：`7.4.0-rc1` 的 `0-rc1` 取 0。
            let digits: String = part
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                return None;
            }
            parts[count] = digits.parse().ok()?;
            count += 1;
        }
        if count == 0 {
            return None;
        }
        Some(Self { major: parts[0], minor: parts[1], patch: parts[2] })
    }

    /// 是否不低于指定主/次版本（patch 不参与比较）。
    pub fn at_least(&self, major: u64, minor: u64) -> bool {
        self.major > major || (self.major == major && self.minor >= minor)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowUpdate {
    pub identity: RowIdentity,
    pub cells: Vec<CellUpdate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellUpdate {
    pub column: String,
    pub value: CellValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RowIdentity {
    pub values: BTreeMap<String, CellValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserFacingError {
    pub title: String,
    pub message: String,
    pub detail: Option<String>,
    pub retryable: bool,
}

impl From<Error> for UserFacingError {
    fn from(error: Error) -> Self {
        let title = match error.kind {
            ErrorKind::Connection => "连接失败",
            ErrorKind::Authentication => "认证失败",
            ErrorKind::Permission => "权限不足",
            ErrorKind::Query => "查询失败",
            ErrorKind::Cancelled => "操作已取消",
            ErrorKind::Unsupported => "暂不支持",
            ErrorKind::Internal => "内部错误",
        };

        Self {
            title: title.to_string(),
            message: error.message,
            detail: None,
            retryable: matches!(error.kind, ErrorKind::Connection | ErrorKind::Cancelled),
        }
    }
}
