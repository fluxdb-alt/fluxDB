// 目标授权与风险策略（设计 §25、§29，T020）。
//
// 内核不变量：
// - **deny 优先**：内核拒绝不能被插件 Allow 覆盖；`RequireApproval` 优先于 `Allow`。
// - **批准绑定 plan_hash**：参数/目标/插件摘要任何变化都会换 hash，旧批准自然失效。
// - **fail closed**：强制安全审计无法执行/无法落盘时，一律拒绝执行。
// - `denied` / `unknown` 不是可重试错误（不能诱导模型换工具绕过）。

use fluxdb_core::ApprovalDetails;
use fluxdb_core::ApprovalRisk;

/// 审批范围（写进审批行，供 UI 与执行前复核）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalScope {
    pub session_id: String,
    pub turn_id: String,
    pub tool_id: String,
    pub tool_version: u32,
    pub target_json: String,
    /// 计划影响的资源摘要（连接/库/表/Key 前缀等）。
    pub resource_json: String,
}

impl ApprovalScope {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "session_id": self.session_id,
            "turn_id": self.turn_id,
            "tool_id": self.tool_id,
            "tool_version": self.tool_version,
            "target": self.target_json,
            "resource": self.resource_json,
        })
        .to_string()
    }
}

/// 由 SQL 内核分类生成 SQL 工具的审批展示事实。
///
/// 只作为展示投影，不参与授权判定；行数未实际查询前明确写"未核实"。
pub fn sql_approval_details(sql: &str, policy: &dyn SqlPolicy) -> ApprovalDetails {
    let (guardrails, risk) = sql_approval_guardrails(&analyze_sql(sql, policy));
    ApprovalDetails {
        risk,
        statements: vec![sql.to_string()],
        guardrails,
        impact_basis: "未核实".to_string(),
        estimated_rows: None,
    }
}

/// 由 Redis 内核分类生成 Redis 执行工具的审批展示事实。
pub fn redis_approval_details(argv: &[String]) -> ApprovalDetails {
    let (guardrails, risk) = redis_approval_guardrails(&classify_redis_command(argv));
    ApprovalDetails {
        risk,
        statements: vec![serde_json::to_string_pretty(argv).unwrap_or_default()],
        guardrails,
        impact_basis: "未核实".to_string(),
        estimated_rows: None,
    }
}

fn sql_approval_guardrails(risk: &SqlRisk) -> (Vec<String>, ApprovalRisk) {
    let mut guardrails = Vec::new();
    let level = match risk {
        SqlRisk::ReadOnly => {
            guardrails.push("单语句，不含事务或会话控制".to_string());
            guardrails.push("语句已判定为只读，不修改数据".to_string());
            ApprovalRisk::Low
        }
        SqlRisk::Rejected { reasons } => {
            guardrails.push(format!("内核策略已拒绝该语句（{}）", reasons.join("、")));
            ApprovalRisk::High
        }
        SqlRisk::RequiresAuthorization { reasons } => {
            if !reasons.iter().any(|reason| reason == "unknown_syntax") {
                guardrails.push("单语句，不含事务或会话控制".to_string());
            }
            guardrails.push("未通过只读分类，必须逐次批准".to_string());
            if reasons.iter().any(|reason| {
                reason.starts_with("denied_function:")
                    || reason.starts_with("denied_syntax:")
            }) {
                guardrails.push("语句含已知副作用语法，影响面无法从文本确定".to_string());
            }
            // 副作用语法或无法解析的语句无法判断影响面，一律按高风险提示。
            if reasons.iter().any(|reason| {
                reason.starts_with("denied_function:")
                    || reason.starts_with("denied_syntax:")
                    || reason == "unknown_syntax"
            }) {
                ApprovalRisk::High
            } else {
                ApprovalRisk::Medium
            }
        }
    };
    (guardrails, level)
}

fn redis_approval_guardrails(risk: &RedisRisk) -> (Vec<String>, ApprovalRisk) {
    let mut guardrails = vec!["按 argv 原样传参，不拼接命令文本".to_string()];
    let level = match risk {
        RedisRisk::ReadOnly => {
            guardrails.push("命令在只读白名单内".to_string());
            ApprovalRisk::Low
        }
        RedisRisk::Rejected { reasons } => {
            guardrails.push(format!("内核策略已拒绝该命令（{}）", reasons.join("、")));
            ApprovalRisk::High
        }
        RedisRisk::RequiresAuthorization { reasons } => {
            if reasons
                .iter()
                .any(|reason| reason.starts_with("mutating_or_script:"))
            {
                guardrails.push("写命令或脚本执行，不在只读白名单内".to_string());
            }
            // 未识别命令可能是新命令或改名命令，影响面未知，按高风险提示。
            if reasons
                .iter()
                .any(|reason| reason.starts_with("unknown_command:"))
            {
                guardrails.push("命令未在内核白名单内，影响面未核实".to_string());
                ApprovalRisk::High
            } else {
                ApprovalRisk::Medium
            }
        }
    };
    (guardrails, level)
}

/// 计算最终 plan hash：工具 + 版本 + 目标 + **规范化参数** + 插件摘要。
///
/// 任何一项变化都会改变 hash，从而让旧审计/审批失效（设计 §25.1）。
pub fn plan_hash(
    tool_id: &str,
    tool_version: u32,
    target_json: &str,
    normalized_arguments_json: &str,
    plugin_digest: Option<&str>,
) -> String {
    // 规范化必须在哈希内部完成：serde_json 的 `Map` 顺序会随 `preserve_order` 特性
    // 在不同编译单元/依赖图里变化，同一语义参数会算出不同 hash（跨进程复核会误判失效）。
    let canonical_target = canonical_json_str(target_json);
    let canonical_arguments = canonical_json_str(normalized_arguments_json);
    let material = format!(
        "v1\u{1f}{tool_id}\u{1f}{tool_version}\u{1f}{canonical_target}\u{1f}{canonical_arguments}\u{1f}{}",
        plugin_digest.unwrap_or("-")
    );
    // FNV-1a 64：稳定、无额外依赖；这里只用于"变化即失效"，不作为安全哈希。
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 内核权限策略（连接与 Agent 授权的交集结果，不含用户偏好）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelPolicy {
    /// 目标连接/对象是否允许读取。
    pub read_allowed: bool,
    /// 目标是否允许写入（连接只读时恒为 false）。
    pub write_allowed: bool,
    /// 是否允许管理类操作（默认 false）。
    pub admin_allowed: bool,
}

impl Default for KernelPolicy {
    fn default() -> Self {
        // 保守默认：可读、不可写、不可管理。
        Self {
            read_allowed: true,
            write_allowed: false,
            admin_allowed: false,
        }
    }
}

/// 授权输入。
#[derive(Clone, Debug)]
pub struct AuthorizeInput {
    pub tool_id: String,
    pub tool_version: u32,
    pub risk: fluxdb_core::ToolRisk,
    pub target_json: String,
    pub normalized_arguments_json: String,
    pub plugin_digest: Option<String>,
    /// 全局停用的工具；非空即命中即拒，Agent 与插件都不能重新打开。
    pub globally_disabled_tools: std::collections::BTreeSet<String>,
    /// Agent 允许的工具白名单；空表示不限制。
    pub agent_allowed_tools: Vec<String>,
    pub policy: KernelPolicy,
    /// 强制安全审计的判定（`None` = 未审计；调用方必须先审计，见 `audit_or_fail_closed`）。
    pub audit: Option<AuditOutcome>,
}

/// 审计结果（内核最严格结果优先）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Allow,
    RequireApproval,
    Deny,
}

impl AuditOutcome {
    /// 落库/落片段的稳定机器键；UI 按它上色与文案。
    pub fn key(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireApproval => "require_approval",
            Self::Deny => "deny",
        }
    }
}

/// 授权结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow {
        plan_hash: String,
    },
    RequireApproval {
        plan_hash: String,
        scope: ApprovalScope,
    },
    Deny {
        code: &'static str,
        reason: String,
        /// 权限类拒绝不是可重试错误（不能诱导换工具绕过）。
        retryable: bool,
    },
}

impl PermissionDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, PermissionDecision::Allow { .. })
    }

    pub fn plan_hash(&self) -> Option<&str> {
        match self {
            PermissionDecision::Allow { plan_hash }
            | PermissionDecision::RequireApproval { plan_hash, .. } => Some(plan_hash),
            PermissionDecision::Deny { .. } => None,
        }
    }
}

/// 一次审计的完整事实：结论 + 命中的规则 + 脱敏原因码。
///
/// 事实行要写进 `ai_tool_audits`，因此原因码只放规则标签与对象名（表/列/命令），
/// 不放参数值、SQL 原文或密钥材料。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditFacts {
    pub outcome: AuditOutcome,
    /// 命中的规则 ID（机器键），未命中为空。
    pub rule_ids: Vec<String>,
    /// 可见原因码；只描述规则与对象，不含参数值。
    pub reason_codes: Vec<String>,
    /// 判定时使用的规则集版本，用于区分"旧规则放的"与"新规则放的"。
    pub ruleset_version: String,
    /// 本次调用的工具风险等级（`read`/`write`/`admin`），落库供审计页筛选。
    pub risk: String,
}

impl AuditFacts {
    /// 无规则信息的最小事实（默认 Hook 与失败路径使用）。
    ///
    /// `risk` 先留空：Hook 只看到 `AuditInput`，风险等级由主循环按工具 spec 补齐。
    pub fn bare(outcome: AuditOutcome, ruleset_version: impl Into<String>) -> Self {
        Self {
            outcome,
            rule_ids: Vec::new(),
            reason_codes: Vec::new(),
            ruleset_version: ruleset_version.into(),
            risk: String::new(),
        }
    }
}

/// 强制安全审计：审计无法执行时 **fail closed**（返回 Deny）。
///
/// 返回完整事实供持久化；不可用时的事实是 `Deny` + 固定原因码，
/// 保证"审计挂掉"和"规则拒绝"在审计表里都能查到。
pub fn audit_or_fail_closed(hook: &dyn SecurityAuditHook, input: &AuditInput) -> AuditFacts {
    match hook.audit_facts(input) {
        Ok(facts) => facts,
        // 审计不可用 = 拒绝，绝不"先执行再说"。
        Err(_) => AuditFacts {
            outcome: AuditOutcome::Deny,
            rule_ids: vec!["kernel.audit_unavailable".to_string()],
            reason_codes: vec!["audit_unavailable".to_string()],
            ruleset_version: input.ruleset_version.clone(),
            risk: String::new(),
        },
    }
}

/// 审计输入（不含 API Key / 加密载荷 / 数据样本）。
#[derive(Clone, Debug)]
pub struct AuditInput {
    pub tool_id: String,
    pub tool_version: u32,
    pub plan_hash: String,
    pub target_json: String,
    pub normalized_arguments_json: String,
    pub ruleset_version: String,
}

/// 强制安全审计 Hook（内核注册，插件不能卸载/跳过）。
pub trait SecurityAuditHook: Send + Sync {
    fn audit(&self, input: &AuditInput) -> fluxdb_core::Result<AuditOutcome>;

    /// 带规则详情的审计；默认实现只给出结论，不编造规则信息。
    ///
    /// 需要把规则写入 `ai_tool_audits` 的实现覆写它；只关心结论的调用方用 [`Self::audit`]。
    fn audit_facts(&self, input: &AuditInput) -> fluxdb_core::Result<AuditFacts> {
        self.audit(input)
            .map(|outcome| AuditFacts::bare(outcome, input.ruleset_version.clone()))
    }
}

/// 内核授权：把工具风险、Agent 白名单、连接权限与审计结果求交集。
///
/// 顺序（严格度递增）：Agent 白名单 → 连接/内核权限 → 风险 → 审计。
/// 任一 Deny 立即返回 Deny，插件 Allow 无法覆盖。
pub fn authorize(input: &AuthorizeInput) -> PermissionDecision {
    let hash = plan_hash(
        &input.tool_id,
        input.tool_version,
        &input.target_json,
        &input.normalized_arguments_json,
        input.plugin_digest.as_deref(),
    );
    let deny = |code: &'static str, reason: String| PermissionDecision::Deny {
        code,
        reason,
        retryable: false,
    };

    // 1) 全局工具启停：用户关掉的工具最先拒绝，Agent 白名单与插件都不能覆盖。
    if input
        .globally_disabled_tools
        .contains(&input.tool_id)
    {
        return deny(
            "tool_globally_disabled",
            format!("工具 {} 已在工具目录中全局关闭", input.tool_id),
        );
    }

    // 2) Agent 白名单（空 = 不限制）。
    if !input.agent_allowed_tools.is_empty()
        && !agent_allows_tool(
            &input.agent_allowed_tools,
            &input.tool_id,
            input.risk,
        )
        && !dynamic_tool_capability(&input.tool_id)
            .is_some_and(|capability| {
                input
                    .agent_allowed_tools
                    .iter()
                    .any(|allowed| allowed == capability)
            })
    {
        return deny(
            "tool_not_allowed",
            format!("Agent 未允许工具 {}", input.tool_id),
        );
    }

    // 3) 连接/内核权限：与风险取交集。
    match input.risk {
        fluxdb_core::ToolRisk::Read => {
            if !input.policy.read_allowed {
                return deny("read_not_allowed", "目标不允许读取".to_string());
            }
        }
        fluxdb_core::ToolRisk::Write => {
            if !input.policy.write_allowed {
                return deny("write_not_allowed", "目标不允许写入".to_string());
            }
        }
        fluxdb_core::ToolRisk::Admin => {
            if !input.policy.admin_allowed {
                return deny("admin_not_allowed", "管理类操作默认拒绝".to_string());
            }
        }
    }

    // 4) 强制审计（未审计视为不可执行；fail closed）。
    let audit = match input.audit {
        Some(AuditOutcome::Deny) => {
            return deny("audit_denied", "安全审计拒绝该调用".to_string());
        }
        Some(AuditOutcome::RequireApproval) => AuditOutcome::RequireApproval,
        Some(AuditOutcome::Allow) => AuditOutcome::Allow,
        // 没有任何审计结论时按"需要审批"处理，而不是直接放行。
        None => AuditOutcome::RequireApproval,
    };

    // 5) 默认写审批。显式执行策略只能在前面的硬性授权与审计放行后改变审批方式。
    let needs_approval =
        audit == AuditOutcome::RequireApproval || input.risk != fluxdb_core::ToolRisk::Read;
    if needs_approval {
        return PermissionDecision::RequireApproval {
            plan_hash: hash,
            scope: ApprovalScope {
                session_id: String::new(),
                turn_id: String::new(),
                tool_id: input.tool_id.clone(),
                tool_version: input.tool_version,
                target_json: input.target_json.clone(),
                resource_json: input.target_json.clone(),
            },
        };
    }
    PermissionDecision::Allow { plan_hash: hash }
}

fn agent_allows_tool(
    allowed: &[String],
    tool_id: &str,
    _risk: fluxdb_core::ToolRisk,
) -> bool {
    allowed.iter().any(|id| id == tool_id)
}

/// 动态加载的配置动作继承领域能力，避免 Agent 白名单随每个 action 膨胀。
fn dynamic_tool_capability(tool_id: &str) -> Option<&'static str> {
    if tool_id.starts_with("config.") {
        return Some(crate::CONFIG_PROPOSE_CAPABILITY);
    }
    None
}

#[cfg(test)]
mod db_run_permission_tests {
    use super::*;

    #[test]
    fn retired_sql_allowlist_ids_are_not_implicit_permissions() {
        assert!(!agent_allows_tool(
            &["db.query".to_string()],
            "db.run",
            fluxdb_core::ToolRisk::Read
        ));
        assert!(agent_allows_tool(
            &["db.run".to_string()],
            "db.run",
            fluxdb_core::ToolRisk::Write
        ));
    }

    #[test]
    fn configuration_actions_inherit_the_domain_capability() {
        assert_eq!(
            dynamic_tool_capability("config.model.create"),
            Some("config.propose")
        );
        assert_eq!(dynamic_tool_capability("db.run"), None);
    }
}

/// 已批准后复核：批准必须与**当前 plan_hash** 完全一致，且状态仍是 approved 且未过期。
pub fn approval_matches(
    approval_plan_hash: &str,
    current_plan_hash: &str,
    status: fluxdb_storage::ai_platform::audits::ApprovalStatus,
    expires_at_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    use fluxdb_storage::ai_platform::audits::ApprovalStatus;
    approval_plan_hash == current_plan_hash
        && status == ApprovalStatus::Approved
        && expires_at_ms.is_none_or(|expiry| expiry > now_ms)
}

/// 无规则集的内核审计 Hook：恒 `Allow`。
///
/// 供测试、示例与尚未接入配置的调用方使用；**规则集为空不等于跳过审计链**，
/// 审计仍在必经位置执行，只是没有规则命中。生产路径使用
/// [`ConfiguredAuditHook`]（携带从插件配置装载的规则集）。
pub struct KernelAuditHook;

impl SecurityAuditHook for KernelAuditHook {
    fn audit(&self, _input: &AuditInput) -> fluxdb_core::Result<AuditOutcome> {
        Ok(AuditOutcome::Allow)
    }
}

/// 携带规则集的内核审计 Hook。
///
/// 规则集是**快照**：在每轮（含审批恢复）开始时从「调用前安全审计」插件的配置读一次，
/// 因此规则变更作用于下一个工具调用边界，不会在调用进行中被半途改写。
/// Hook 自身不访问存储，避免在已开启的事务内另开连接。
pub struct ConfiguredAuditHook {
    rules: AuditRules,
}

impl ConfiguredAuditHook {
    pub fn new(rules: AuditRules) -> Self {
        Self { rules }
    }

    /// 读取安全审计插件的规则配置并构造 Hook。
    ///
    /// 配置读不出来时返回 `Err`，由 [`audit_or_fail_closed`] 转成 `Deny`：
    /// 规则集不可信时不得按"没有规则"放行。
    pub fn from_storage(storage: &fluxdb_storage::FileStorage) -> fluxdb_core::Result<Self> {
        Ok(Self::new(load_audit_rules(storage)?))
    }
}

impl SecurityAuditHook for ConfiguredAuditHook {
    fn audit(&self, input: &AuditInput) -> fluxdb_core::Result<AuditOutcome> {
        Ok(self.rules.evaluate(input).outcome)
    }

    fn audit_facts(&self, input: &AuditInput) -> fluxdb_core::Result<AuditFacts> {
        let verdict = self.rules.evaluate(input);
        Ok(AuditFacts {
            outcome: verdict.outcome,
            rule_ids: verdict.rule_id.map(str::to_string).into_iter().collect(),
            reason_codes: verdict.reason_code.into_iter().collect(),
            ruleset_version: self.rules.version(),
            risk: String::new(),
        })
    }
}

/// 读取安全审计插件的规则配置；插件实例缺失或已下线时返回空规则集（不拦截）。
///
/// 空规则集仍然会走完审计链：`authorize` 的工具开关、Agent 白名单、连接权限与
/// 写审批不受这里影响。
pub fn load_audit_rules(
    storage: &fluxdb_storage::FileStorage,
) -> fluxdb_core::Result<AuditRules> {
    let values = storage.with_ai_connection(|conn| {
        fluxdb_storage::ai_platform::plugins::load_plugin_instance_config(
            conn,
            SECURITY_AUDIT_INSTANCE_ID,
        )
        .map(|config| config.values)
        .map_err(Into::into)
    })?;
    Ok(AuditRules::from_config(&values))
}

/// 安全审计插件实例 ID；规则配置存在该实例的 `config_json`。
pub const SECURITY_AUDIT_INSTANCE_ID: &str = "builtin.security-audit";

/// 规则集读不出来时的审计 Hook：`audit` 返回 `Err`。
///
/// 配合 [`audit_or_fail_closed`] 即"审计不可用 = 拒绝"。调用方拿不到可信规则时
/// 不能退回"没有规则"的放行语义。
pub struct UnavailableAuditHook {
    reason: String,
}

impl UnavailableAuditHook {
    pub fn new(reason: String) -> Self {
        Self { reason }
    }
}

impl SecurityAuditHook for UnavailableAuditHook {
    fn audit(&self, _input: &AuditInput) -> fluxdb_core::Result<AuditOutcome> {
        Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Permission,
            self.reason.clone(),
        ))
    }
}

/// 连接是否被标记为只读：SQLite 走 endpoint 标记，其它类型走显式 `options["read_only"]`。
///
/// 判定本身在 `ConnectionConfig::is_read_only`（连接器也要用），这里只保留业务层的入口。
pub fn connection_is_read_only(config: &fluxdb_core::ConnectionConfig) -> bool {
    config.is_read_only()
}

/// 由**本轮固定目标连接**推导内核权限（设计 §25、§29）。
///
/// - 读：允许（真正的资源校验仍在 resolver/连接层，未固定连接时工具会因目标无效失败）；
/// - 写：只有目标连接未被标记只读才具备写能力；是否逐次审批由执行策略决定；
/// - 管理：默认拒绝。
///
/// 没有固定连接时写权限为 false：不能因为"没指定目标"就落到任意连接去写。
pub fn kernel_policy_for_connection(
    config: Option<&fluxdb_core::ConnectionConfig>,
) -> KernelPolicy {
    KernelPolicy {
        read_allowed: true,
        write_allowed: config.is_some_and(|config| !connection_is_read_only(config)),
        admin_allowed: false,
    }
}

/// 规范化 JSON：对象键递归按字典序排序，数组保持顺序；解析失败时原样返回。
///
/// `plan_hash` 的输入必须经过它，否则同一语义的 JSON 只因键顺序不同就会得到不同 hash。
pub fn canonical_json_str(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => canonical_json(&value),
        Err(_) => raw.to_string(),
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        serde_json::Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        other => other.to_string(),
    }
}
