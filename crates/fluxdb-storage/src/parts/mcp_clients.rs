use std::collections::BTreeMap;

use crate::{FileStorage, Result, sqlite};
use serde::{Deserialize, Serialize};

/// 记录本应用写入客户端的端点信息，用于更新前核对条目归属。
///
/// `authorization` 是应用自己签发的本机回环令牌（客户端配置里已明文存在），
/// 仅用于发现条目过期；不得写入日志或对外事件。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClientLaunch {
    pub url: String,
    pub authorization: String,
}

impl std::fmt::Debug for McpClientLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClientLaunch")
            .field("url", &self.url)
            .field("authorization", &"<redacted>")
            .finish()
    }
}

/// 键名带版本：旧版记录的是 stdio 启动命令，结构已不兼容，直接换键而不迁移。
/// 代价是升级后首次打开会显示「未托管」，用户重新安装即可恢复维护关系。
const MCP_CLIENT_REGISTRATIONS_KEY: &str = "mcp.client_registrations.v2";

impl FileStorage {
    pub fn load_mcp_client_registrations(&self) -> Result<BTreeMap<String, McpClientLaunch>> {
        let conn = self.open_sqlite()?;
        Ok(sqlite::get_json(&conn, MCP_CLIENT_REGISTRATIONS_KEY)?.unwrap_or_default())
    }

    pub fn save_mcp_client_registrations(
        &self,
        registrations: &BTreeMap<String, McpClientLaunch>,
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, MCP_CLIENT_REGISTRATIONS_KEY, registrations)
    }
}
