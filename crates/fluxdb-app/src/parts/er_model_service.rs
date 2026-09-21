// ER 逻辑关系服务层（fluxdb-app，er-design.md §6.5 ErModelService 的本地落地）。
//
// 本文件实现本地逻辑关系目录的 CRUD / 确认 / 拒绝 / usage 判定，端点绑定校验交给宿主
// （desktop/加载流程）据 CatalogSnapshot 构造 `ErUsageContext`。服务不拼 SQL、不执行 DDL，
// 编辑只改 FluxDB 本地模型（§D3）。全部写操作带 expected_revision 乐观锁，避免 Agent 建议
// 覆盖用户刚改的关系（er-design §6.3）。
//
// 存储经 `ErRelationshipStore` trait 注入：生产用 fluxdb-storage 的 FileStorage 适配，
// 单测用内存 Map，保证无数据库也完整验证调用链。

use fluxdb_core::{ErRelationshipReview, ErReviewState, ErUsage, ErUsageContext};

/// 关系目录存储抽象（供注入测试）。Send+Sync：服务在后台线程调用，且目录读写在服务内
/// Mutex 串行，跨操作不丢更新；测试可用多线程验证。
pub trait ErRelationshipStore: Send + Sync {
    fn load(&self, scope_key: &str) -> fluxdb_core::Result<Vec<ErRelationship>>;
    fn save(&self, scope_key: &str, rels: &[ErRelationship]) -> fluxdb_core::Result<()>;
}

/// 关系写操作/校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErModelError {
    /// 关系目录读写失败；不能降级为空列表或伪造成功。
    Storage(String),
    NotFound,
    /// expected_revision 与当前不匹配（并发/过期编辑，§6.3）。
    RevisionConflict,
    /// column_pairs / required_filters 结构性非法（空、重复、字面量滥用）。
    InvalidDefinition(String),
    /// (left, right) 内 role 重名（§5.3 role 对内唯一）。
    DuplicateRole,
}

impl std::fmt::Display for ErModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErModelError::Storage(reason) => write!(f, "关系目录存储失败：{reason}"),
            ErModelError::NotFound => write!(f, "关系不存在"),
            ErModelError::RevisionConflict => write!(f, "关系已被修改，请刷新后重试"),
            ErModelError::InvalidDefinition(reason) => write!(f, "关系定义无效：{reason}"),
            ErModelError::DuplicateRole => write!(f, "同一对表之间已存在同名角色"),
        }
    }
}

/// 校验关系定义的结构性约束（§5.3）：
/// - column_pairs 非空、同 pair 内左右列都非空、无重复。
/// - required_filters 列非空、op 已知（由类型保证）、字面量是字面量（由 ErLiteral 类型保证）。
/// 端点/列在当前快照是否存在属绑定校验，由宿主做（服务不持有结构快照）。
fn validate_definition(rel: &ErRelationship) -> Result<(), ErModelError> {
    if rel.column_pairs.is_empty() {
        return Err(ErModelError::InvalidDefinition("字段配对不能为空".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in &rel.column_pairs {
        if p.left_column.is_empty() || p.right_column.is_empty() {
            return Err(ErModelError::InvalidDefinition("字段配对含空列".into()));
        }
        if !seen.insert((&p.left_column, &p.right_column)) {
            return Err(ErModelError::InvalidDefinition("字段配对重复".into()));
        }
    }
    for f in &rel.required_filters {
        if f.column_id.is_empty() {
            return Err(ErModelError::InvalidDefinition("过滤条件引用空列".into()));
        }
        if matches!(
            f.op,
            fluxdb_core::ErFilterOp::IsNull | fluxdb_core::ErFilterOp::IsNotNull
        ) && !matches!(f.literal, fluxdb_core::ErLiteral::Null)
        {
            return Err(ErModelError::InvalidDefinition(
                "为空/不为空条件不能携带字面量".into(),
            ));
        }
        // op 是枚举，不可能是任意 SQL 字符串；literal 是 ErLiteral（结构化字面量），
        // 不接受表达式/函数/子查询——类型系统已保证不拼任意 SQL。
        let _ = &f.op;
        let _ = &f.literal;
    }
    Ok(())
}

/// 生产用关系存储适配：把 fluxdb-storage 的 FileStorage 接到 `ErRelationshipStore`。
/// 单测用内存 MemStore（见下），生产用本适配器经 sqlite kv 持久化（§5/§7）。
pub struct FileErRelationshipStore {
    storage: fluxdb_storage::FileStorage,
    scope_key: String,
}

impl FileErRelationshipStore {
    pub fn new(storage: fluxdb_storage::FileStorage, scope_key: impl Into<String>) -> Self {
        Self {
            storage,
            scope_key: scope_key.into(),
        }
    }
}

impl ErRelationshipStore for FileErRelationshipStore {
    fn load(&self, _scope: &str) -> fluxdb_core::Result<Vec<ErRelationship>> {
        self.storage.load_er_relationships(&self.scope_key)
    }
    fn save(&self, _scope: &str, rels: &[ErRelationship]) -> fluxdb_core::Result<()> {
        self.storage.save_er_relationships(&self.scope_key, rels)
    }
}

/// ER 逻辑关系服务（给定 scope 的目录操作）。构造后持有一个可变目录。
///
/// 并发：目录读改写（load→mutate→save）在 `lock` 内整体串行，同进程内多操作不会互相
/// 覆盖（§7：delete/update/confirm 等都不能绕过既有并发保护）。跨进程（多窗口）不在本文
/// 范围（桌面单实例）：`ponytail: session 内 Mutex 串行写，跨进程需 storage 层 CAS 再做`。
pub struct ErModelService {
    scope_key: String,
    store: Box<dyn ErRelationshipStore>,
    lock: std::sync::Mutex<()>,
}

impl ErModelService {
    pub fn new(scope_key: impl Into<String>, store: Box<dyn ErRelationshipStore>) -> Self {
        Self {
            scope_key: scope_key.into(),
            store,
            lock: std::sync::Mutex::new(()),
        }
    }

    fn load_all(&self) -> Result<Vec<ErRelationship>, ErModelError> {
        self.store
            .load(&self.scope_key)
            .map_err(|error| ErModelError::Storage(error.to_string()))
    }

    fn save_all(&self, rels: &[ErRelationship]) -> fluxdb_core::Result<()> {
        self.store.save(&self.scope_key, rels)
    }

    /// 列出当前目录全部关系（含 proposed/rejected；usage 需另查）。
    pub fn list(&self) -> Result<Vec<ErRelationship>, ErModelError> {
        self.load_all()
    }

    /// 新建（proposed 或按 origin 置 review）。带结构校验 + role 唯一（§5.3）。
    pub fn create(&self, mut rel: ErRelationship) -> Result<ErRelationship, ErModelError> {
        let _guard = self.lock.lock().unwrap();
        validate_definition(&rel)?;
        let mut all = self.load_all()?;
        // role 在 (left,right) 同一对实体内有向唯一（§5.3）；自关联两个方向也算同一对。
        let dup = all.iter().any(|r| {
            let same_pair = (r.left_entity == rel.left_entity && r.right_entity == rel.right_entity)
                || (r.left_entity == rel.right_entity && r.right_entity == rel.left_entity);
            same_pair && r.role == rel.role
        });
        if dup {
            return Err(ErModelError::DuplicateRole);
        }
        if rel.revision == 0 {
            rel.revision = 1;
        }
        if rel.review.state == ErReviewState::Confirmed && rel.review.confirmed_revision.is_none() {
            rel.review.confirmed_revision = Some(rel.revision);
        }
        all.push(rel.clone());
        self.save_all(&all)
            .map_err(|_| ErModelError::InvalidDefinition("保存失败".into()))?;
        Ok(rel)
    }

    /// 按 id 找（可变借用以构造 next revision）。
    fn index_of(all: &[ErRelationship], id: &str) -> Option<usize> {
        all.iter().position(|r| r.id == id)
    }

    /// 更新：expected_revision 不匹配 → RevisionConflict（§6.3 乐观锁）；闭包返回新定义。
    fn update_inner(
        &self,
        id: &str,
        expected_revision: u64,
        f: impl FnOnce(&mut ErRelationship) -> Result<(), ErModelError>,
    ) -> Result<ErRelationship, ErModelError> {
        let _guard = self.lock.lock().unwrap();
        let mut all = self.load_all()?;
        let i = Self::index_of(&all, id).ok_or(ErModelError::NotFound)?;
        if all[i].revision != expected_revision {
            return Err(ErModelError::RevisionConflict);
        }
        f(&mut all[i])?;
        validate_definition(&all[i])?;
        let duplicate_role = all.iter().enumerate().any(|(index, other)| {
            index != i
                && ((other.left_entity == all[i].left_entity
                    && other.right_entity == all[i].right_entity)
                    || (other.left_entity == all[i].right_entity
                        && other.right_entity == all[i].left_entity))
                && other.role == all[i].role
        });
        if duplicate_role {
            return Err(ErModelError::DuplicateRole);
        }
        all[i].revision += 1;
        let out = all[i].clone();
        self.save_all(&all)
            .map_err(|_| ErModelError::InvalidDefinition("保存失败".into()))?;
        Ok(out)
    }

    /// 更新定义（字段配对/端点/基数/过滤）：生成新修订并使旧确认失效（§5.2/§6.3）。
    /// 确认/拒绝走 `confirm`/`reject`（不经过此失效，因为正在设置确认状态本身）。
    pub fn update(
        &self,
        id: &str,
        expected_revision: u64,
        patch: impl FnOnce(&mut ErRelationship) -> Result<(), ErModelError>,
    ) -> Result<ErRelationship, ErModelError> {
        self.update_inner(id, expected_revision, |r| {
            let was_confirmed = r.review.state == ErReviewState::Confirmed;
            patch(r)?;
            if was_confirmed {
                // 定义被编辑后，旧确认不再代表当前状态：回到 proposed，需重新确认（§5.2）。
                // 只清 confirmed_revision 而仍留 Confirmed 会让 usage 的「确认过期」检查
                // 因 None 走不到 outdated 分支而误判为可候选，故必须降级回 proposed。
                r.review.state = ErReviewState::Proposed;
                r.review.confirmed_revision = None;
                r.review.confirmed_by = None;
            }
            Ok(())
        })
    }

    /// 人工确认（§6.3）：标记 confirmed 并记录当前 revision。
    pub fn confirm(
        &self,
        id: &str,
        expected_revision: u64,
        by: &str,
    ) -> Result<ErRelationship, ErModelError> {
        self.update_inner(id, expected_revision, |r| {
            r.review = ErRelationshipReview {
                state: ErReviewState::Confirmed,
                confirmed_revision: Some(r.revision + 1),
                confirmed_by: Some(by.to_string()),
            };
            Ok(())
        })
    }

    /// 拒绝（不删除，保留定义与证据，§5.2）。
    pub fn reject(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<ErRelationship, ErModelError> {
        self.update_inner(id, expected_revision, |r| {
            r.review.state = ErReviewState::Rejected;
            Ok(())
        })
    }

    /// 删除本地逻辑关系（§7 删除语义）：只删本地模型，不执行 DDL、不动数据库物理外键。
    /// 与 update/confirm 一致采用 expected_revision 修订校验（不能绕过并发保护）；
    /// load→remove→save 在 `lock` 内原子执行，写入失败返回错误、不假装删除成功。
    pub fn delete(&self, id: &str, expected_revision: u64) -> Result<(), ErModelError> {
        let _guard = self.lock.lock().unwrap();
        let mut all = self.load_all()?;
        let i = Self::index_of(&all, id).ok_or(ErModelError::NotFound)?;
        if all[i].revision != expected_revision {
            return Err(ErModelError::RevisionConflict);
        }
        all.remove(i);
        self.save_all(&all)
            .map_err(|_| ErModelError::InvalidDefinition("保存失败".into()))?;
        Ok(())
    }

    /// 计算某关系的 usage（§5.9）；`ctx` 由宿主从结构快照构造。
    pub fn usage(&self, id: &str, ctx: &ErUsageContext) -> Result<ErUsage, ErModelError> {
        let all = self.load_all()?;
        let i = Self::index_of(&all, id).ok_or(ErModelError::NotFound)?;
        Ok(all[i].usage(ctx))
    }

    /// 生成全部可用关系的 JOIN 计划（§6.6/查询地基）：仅 `join_candidate` 的关系入列，
    /// 每条完整保留复合配对与 required_filters。`ctx` 由宿主按关系从结构快照构造；
    /// coverage 不完整的关系不产出（宿主以 coverage 表达，不解释为“无关系”）。
    pub fn join_plans(
        &self,
        mut ctx: impl FnMut(&ErRelationship) -> ErUsageContext,
    ) -> Vec<fluxdb_core::ErJoinPlan> {
        self.load_all()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.join_plan(&ctx(r)))
            .collect()
    }

    /// 结构刷新后的重绑报告（§5.2）：按旧快照映射 `old_entities_by_id` + 新快照 `new_entities`
    /// 判定每条关系左右端实体/列是否可重绑；不自动改关系（unresolved 保留定义与证据，
    /// 由 UI 集中列出待处理项，用户手工重绑）。返回每条关系两端的判定。
    pub fn rebind_report(
        &self,
        old_entities_by_id: &std::collections::HashMap<String, fluxdb_core::ErRebindEntity>,
        new_entities: &[fluxdb_core::ErRebindEntity],
    ) -> Vec<(String, Vec<ErRebindEndpointStatus>)> {
        self.load_all()
            .unwrap_or_default()
            .iter()
            .map(|r| {
                let mut endpoints = Vec::new();
                // 左端。
                if let Some(old) = old_entities_by_id.get(&r.left_entity) {
                    let used = rel_columns_on(r, fluxdb_core::ErRelationSide::Left);
                    endpoints.push(rebase_endpoint(old, &used, new_entities));
                }
                // 右端。
                if let Some(old) = old_entities_by_id.get(&r.right_entity) {
                    let used = rel_columns_on(r, fluxdb_core::ErRelationSide::Right);
                    endpoints.push(rebase_endpoint(old, &used, new_entities));
                }
                (r.id.clone(), endpoints)
            })
            .collect()
    }
}

/// 单个端点重绑状态（供 UI「待处理项」列出）。
#[derive(Clone, Debug, PartialEq)]
pub struct ErRebindEndpointStatus {
    pub old_entity_id: String,
    pub matched_entity: Option<String>,
    /// 删除重建（stable 变但名同）→ 需确认。
    pub needs_review: bool,
    /// 实体未找到。
    pub entity_unresolved: bool,
    /// 缺失（unresolved）的列（记录具体 column_id，供手工重绑；绝不静默接相似列）。
    pub unresolved_columns: Vec<String>,
}

/// 取关系某侧用到的列（column_pairs 中该侧列 + required_filters 该侧列），去重保序。
fn rel_columns_on(r: &ErRelationship, side: fluxdb_core::ErRelationSide) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in &r.column_pairs {
        let col = match side {
            fluxdb_core::ErRelationSide::Left => &p.left_column,
            fluxdb_core::ErRelationSide::Right => &p.right_column,
        };
        if !out.contains(col) {
            out.push(col.clone());
        }
    }
    for f in &r.required_filters {
        if f.side == side && !out.contains(&f.column_id) {
            out.push(f.column_id.clone());
        }
    }
    out
}

/// 单个端点重绑（§5.2）→ 状态。
fn rebase_endpoint(
    old: &fluxdb_core::ErRebindEntity,
    used: &[String],
    new_entities: &[fluxdb_core::ErRebindEntity],
) -> ErRebindEndpointStatus {
    let out = fluxdb_core::rebind_entity(old, used, new_entities);
    ErRebindEndpointStatus {
        old_entity_id: old.entity_id.clone(),
        matched_entity: out.matched_entity,
        needs_review: out.entity_needs_review,
        entity_unresolved: out.entity_unresolved,
        unresolved_columns: out
            .columns
            .iter()
            .filter(|c| c.unresolved)
            .map(|c| c.old_column_id.clone())
            .collect(),
    }
}

/// 由结构化表身份生成实体稳定 ID（与 desktop `er_entity_id` 同规则，供快照/重绑身份对齐）。
fn er_snapshot_entity_id(reference: &fluxdb_core::ErTableRef) -> String {
    format!(
        "{}:{}:{}",
        reference.database,
        reference.schema.as_deref().unwrap_or_default(),
        reference.name
    )
}

/// 由列身份生成列 ID（`实体ID::列名`）。
fn er_snapshot_column_id(reference: &fluxdb_core::ErTableRef, column: &str) -> String {
    format!("{}::{column}", er_snapshot_entity_id(reference))
}

/// 由已加载的表节点（含字段，如字段按需加载后的 `ErTableNode`）构建结构快照
/// （§5.2/D1「结构快照解析」）：实体限定名 + 列名，无稳定对象标识（连接器未暴露时如实留空，
/// 不伪造稳定身份，重绑走 §5.2 第 2/3/4 步：限定名→列名→unresolved）。
/// 跨 schema 同名、含点标识符都按 `ErTableRef` 结构化身份生成稳定 entity_id/column_id。
pub fn er_snapshot_from_tables(tables: &[fluxdb_core::ErTableNode]) -> Vec<fluxdb_core::ErRebindEntity> {
    tables
        .iter()
        .filter(|t| t.status != fluxdb_core::ErLoadStatus::NotLoaded)
        .map(|t| fluxdb_core::ErRebindEntity {
            entity_id: er_snapshot_entity_id(&t.reference),
            qualified_name: t.reference.display(),
            stable_id: None,
            columns: t
                .columns
                .iter()
                .map(|c| fluxdb_core::ErRebindColumn {
                    column_id: er_snapshot_column_id(&t.reference, &c.name),
                    name: c.name.clone(),
                    stable_id: None,
                })
                .collect(),
        })
        .collect()
}


#[cfg(test)]
mod er_model_service_tests {
    use super::*;
    use std::sync::Mutex;

    /// 共享内部状态的内存 store：Clone 共享同一数据，便于持有引用切换 fail 标志。
    #[derive(Clone)]
    struct MemStore {
        data: std::sync::Arc<Mutex<std::collections::BTreeMap<String, Vec<ErRelationship>>>>,
        fail_save: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Default for MemStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MemStore {
        fn new() -> Self {
            Self {
                data: std::sync::Arc::new(Mutex::new(std::collections::BTreeMap::new())),
                fail_save: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
        fn set_fail_save(&self, fail: bool) {
            self.fail_save
                .store(fail, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl ErRelationshipStore for MemStore {
        fn load(&self, scope_key: &str) -> fluxdb_core::Result<Vec<ErRelationship>> {
            Ok(self
                .data
                .lock()
                .unwrap()
                .get(scope_key)
                .cloned()
                .unwrap_or_default())
        }
        fn save(&self, scope_key: &str, rels: &[ErRelationship]) -> fluxdb_core::Result<()> {
            if self.fail_save.load(std::sync::atomic::Ordering::SeqCst) {
                // 模拟存储写入失败（写入失败不能让 UI 假装删除/修改成功）。
                return Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Internal,
                    "模拟存储失败",
                ));
            }
            self.data.lock().unwrap().insert(scope_key.to_string(), rels.to_vec());
            Ok(())
        }
    }

    fn base_rel() -> ErRelationship {
        use fluxdb_core::{
            ErCardinality, ErCardinalityBasis, ErCardinalityBound, ErColumnPair, ErEnforcementKind,
            ErMatchCardinality, ErRelationshipEnforcement, ErRelationshipOrigin, ErValidity,
            ErValidityState,
        };
        ErRelationship {
            id: "r1".into(),
            revision: 1,
            left_entity: "e-orders".into(),
            right_entity: "e-customers".into(),
            role: "order_customer".into(),
            column_pairs: vec![ErColumnPair {
                left_column: "orders-customer_id".into(),
                right_column: "customers-id".into(),
            }],
            required_filters: Vec::new(),
            match_cardinality: ErMatchCardinality {
                left_to_right: ErCardinality {
                    min: ErCardinalityBound::Zero,
                    max: ErCardinalityBound::One,
                },
                right_to_left: ErCardinality {
                    min: ErCardinalityBound::Zero,
                    max: ErCardinalityBound::Many,
                },
                basis: ErCardinalityBasis::UserAssertion,
            },
            origin: ErRelationshipOrigin::User,
            review: fluxdb_core::ErRelationshipReview {
                state: ErReviewState::Proposed,
                confirmed_revision: None,
                confirmed_by: None,
            },
            enforcement: ErRelationshipEnforcement {
                kind: ErEnforcementKind::None,
                constraint_ref: None,
                enforced: None,
            },
            validity: ErValidity {
                state: ErValidityState::Current,
                reason: None,
            },
            description: None,
            evidence_refs: Vec::new(),
        }
    }

    #[test]
    fn create_and_confirm_flow() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        let rel = svc.confirm("r1", 1, "alice").unwrap();
        assert_eq!(rel.review.state, ErReviewState::Confirmed);
        assert_eq!(rel.review.confirmed_revision, Some(2));
        // usage 判定（端点存在上下文）。
        let mut ctx = ErUsageContext {
            left_entity_present: true,
            right_entity_present: true,
            pairs_resolvable: true,
            filters_resolvable: true,
            coverage_incomplete: false,
        };
        ctx.coverage_incomplete = false;
        let u = svc.usage("r1", &ctx).unwrap();
        assert_eq!(u.join_candidate, true);
    }

    #[test]
    fn duplicate_role_rejected() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        let mut other = base_rel();
        other.id = "r2".into();
        other.right_entity = "e-customers".into();
        other.role = "order_customer".into(); // 同 (left,right) 同 role
        assert_eq!(svc.create(other).unwrap_err(), ErModelError::DuplicateRole);
    }

    #[test]
    fn update_duplicate_role_rejected_without_mutating_relationship() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        let mut second = base_rel();
        second.id = "r2".into();
        second.role = "billing_customer".into();
        svc.create(second).unwrap();

        let error = svc
            .update("r1", 1, |relationship| {
                relationship.role = "billing_customer".into();
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error, ErModelError::DuplicateRole);
        let first = svc
            .list()
            .unwrap()
            .into_iter()
            .find(|relationship| relationship.id == "r1")
            .unwrap();
        assert_eq!(first.role, "order_customer");
        assert_eq!(first.revision, 1);
    }

    #[test]
    fn revision_conflict_rejected_and_wrong_rev() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        // 错误 expected_revision。
        assert_eq!(
            svc.confirm("r1", 99, "alice").unwrap_err(),
            ErModelError::RevisionConflict
        );
        // 正确 rev 后确认成功。
        svc.confirm("r1", 1, "alice").unwrap();
    }

    #[test]
    fn empty_pairs_invalid() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        let mut rel = base_rel();
        rel.column_pairs.clear();
        assert!(matches!(
            svc.create(rel).unwrap_err(),
            ErModelError::InvalidDefinition(_)
        ));
    }

    #[test]
    fn invalid_required_filter_is_rejected() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        let mut rel = base_rel();
        rel.required_filters.push(fluxdb_core::ErRequiredFilter {
            side: fluxdb_core::ErRelationSide::Left,
            column_id: String::new(),
            op: fluxdb_core::ErFilterOp::Eq,
            literal: fluxdb_core::ErLiteral::Text("active".into()),
        });
        assert!(matches!(
            svc.create(rel).unwrap_err(),
            ErModelError::InvalidDefinition(_)
        ));
    }

    #[test]
    fn definition_edit_invalidates_old_confirmation() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        svc.confirm("r1", 1, "alice").unwrap();
        // 编辑定义（改描述）→ revision 推进、旧确认失效（§5.2），usage 不再为候选。
        let updated = svc
            .update("r1", 2, |r| {
                r.description = Some("改描述".into());
                Ok(())
            })
            .unwrap();
        assert_eq!(updated.revision, 3);
        assert_eq!(updated.review.state, ErReviewState::Proposed);
        assert_eq!(updated.review.confirmed_revision, None);
        let mut ctx = ErUsageContext {
            left_entity_present: true,
            right_entity_present: true,
            pairs_resolvable: true,
            filters_resolvable: true,
            coverage_incomplete: false,
        };
        ctx.coverage_incomplete = false;
        let u = svc.usage("r1", &ctx).unwrap();
        assert!(!u.join_candidate, "定义编辑后旧确认失效，不自动作为 JOIN 候选");
    }

    #[test]
    fn join_plans_only_candidate_and_keeps_composite() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        let mut rel = base_rel();
        rel.column_pairs.push(fluxdb_core::ErColumnPair {
            left_column: "orders-tenant_id".into(),
            right_column: "customers-tenant_id".into(),
        });
        svc.create(rel).unwrap();
        svc.confirm("r1", 1, "alice").unwrap();
        // r9 (proposed, 不同 role) → 不入列；确认后 r1 复合 2 对完整入列。
        let mut p = base_rel();
        p.id = "r9".into();
        p.role = "billing_customer".into();
        p.column_pairs = vec![fluxdb_core::ErColumnPair {
            left_column: "orders-billing_id".into(),
            right_column: "customers-id".into(),
        }];
        svc.create(p).unwrap();

        let plans = svc.join_plans(|_| ErUsageContext {
            left_entity_present: true,
            right_entity_present: true,
            coverage_incomplete: false,
            pairs_resolvable: true,
            filters_resolvable: true,
        });
        assert_eq!(plans.len(), 1, "proposed 不入列");
        assert_eq!(plans[0].pairs.len(), 2, "复合 2 对完整保留");
    }

    fn temp_dir() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("fluxdb-er-test-{}-{}", std::process::id(), nanos))
    }

    #[test]
    fn service_persists_via_file_store_across_restart() {
        let dir = temp_dir();
        let storage = fluxdb_storage::FileStorage::new(&dir);
        let scope = "conn:db:";
        // 第一次“会话”：建 + 确认，经 FileErRelationshipStore 持久化。
        let svc = ErModelService::new(
            scope,
            Box::new(FileErRelationshipStore::new(storage.clone(), scope)),
        );
        svc.create(base_rel()).unwrap();
        svc.confirm("r1", 1, "alice").unwrap();
        // 第二次“会话”（新建 service，模拟重启）：读回已确认关系 + usage 候选。
        let svc2 = ErModelService::new(
            scope,
            Box::new(FileErRelationshipStore::new(storage, scope)),
        );
        let all = svc2.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].review.state, ErReviewState::Confirmed);
        let ctx = ErUsageContext {
            left_entity_present: true,
            right_entity_present: true,
            coverage_incomplete: false,
            pairs_resolvable: true,
            filters_resolvable: true,
        };
        assert!(svc2.usage("r1", &ctx).unwrap().join_candidate);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebind_report_flags_unresolved_and_needs_review() {
        use fluxdb_core::{ErRebindColumn, ErRebindEntity};
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        // 左端 orders(限定名 orders, stable 100)，用 customer_id；右端 customers(stable 200)。
        let mut rel = base_rel();
        rel.left_entity = "e-orders".into();
        rel.right_entity = "e-customers".into();
        rel.column_pairs = vec![fluxdb_core::ErColumnPair {
            left_column: "orders-customer_id".into(),
            right_column: "customers-id".into(),
        }];
        svc.create(rel).unwrap();
        // 旧快照。
        let old = std::collections::HashMap::from([
            (
                "e-orders".to_string(),
                ErRebindEntity {
                    entity_id: "e-orders".into(),
                    qualified_name: "orders".into(),
                    stable_id: Some(100),
                    columns: vec![ErRebindColumn {
                        column_id: "orders-customer_id".into(),
                        name: "customer_id".into(),
                        stable_id: Some(10),
                    }],
                },
            ),
            (
                "e-customers".to_string(),
                ErRebindEntity {
                    entity_id: "e-customers".into(),
                    qualified_name: "customers".into(),
                    stable_id: Some(200),
                    columns: vec![ErRebindColumn {
                        column_id: "customers-id".into(),
                        name: "id".into(),
                        stable_id: Some(20),
                    }],
                },
            ),
        ]);
        // 新快照：orders stable 变（重建）→ 左端 needs_review；customers 列改名 → 右端 unresolved 列。
        let new = vec![
            ErRebindEntity {
                entity_id: "ne-orders".into(),
                qualified_name: "orders".into(),
                stable_id: Some(999),
                columns: vec![ErRebindColumn {
                    column_id: "ne-orders-customer_id".into(),
                    name: "customer_id".into(),
                    stable_id: Some(10),
                }],
            },
            ErRebindEntity {
                entity_id: "ne-customers".into(),
                qualified_name: "customers".into(),
                stable_id: Some(200),
                columns: vec![ErRebindColumn {
                    column_id: "ne-customers-buyer_id".into(),
                    name: "buyer_id".into(),
                    stable_id: Some(21),
                }],
            },
        ];
        let report = svc.rebind_report(&old, &new);
        assert_eq!(report.len(), 1);
        let (_, endpoints) = &report[0];
        // 左端：stable 变但名同 → needs_review（不自动继承/丢弃）。
        let left = &endpoints[0];
        assert!(left.needs_review);
        assert_eq!(left.matched_entity, Some("ne-orders".into()));
        assert!(left.unresolved_columns.is_empty(), "列 stable 相同可重绑");
        // 右端：实体在(stable 200)但列 stable/名都不匹配 → unresolved 列，绝不静默接 buyer_id。
        let right = &endpoints[1];
        assert!(!right.entity_unresolved);
        assert_eq!(right.unresolved_columns, vec!["customers-id".to_string()]);
    }

    #[test]
    fn delete_removes_only_target_and_keeps_others() {
        let store = MemStore::new();
        let svc = ErModelService::new("s", Box::new(store));
        svc.create(base_rel()).unwrap(); // r1
        let mut r2 = base_rel();
        r2.id = "r2".into();
        r2.role = "billing_customer".into();
        r2.column_pairs = vec![fluxdb_core::ErColumnPair {
            left_column: "orders-billing_id".into(),
            right_column: "customers-id".into(),
        }];
        svc.create(r2).unwrap();
        // 删除 r1（rev=1 正确）→ 仅 r1 消失，r2 保留。
        svc.delete("r1", 1).unwrap();
        let all = svc.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "r2");
    }

    #[test]
    fn delete_not_found_and_revision_conflict() {
        let store = MemStore::new();
        let svc = ErModelService::new("s", Box::new(store));
        svc.create(base_rel()).unwrap();
        assert_eq!(svc.delete("nope", 1).unwrap_err(), ErModelError::NotFound);
        assert_eq!(svc.delete("r1", 99).unwrap_err(), ErModelError::RevisionConflict);
        assert_eq!(svc.list().unwrap().len(), 1, "失败删除不影响目录");
    }

    #[test]
    fn delete_storage_failure_does_not_pretend_success() {
        let store = MemStore::new();
        let svc = ErModelService::new("s", Box::new(store.clone()));
        svc.create(base_rel()).unwrap();
        store.set_fail_save(true);
        assert!(svc.delete("r1", 1).is_err(), "写入失败必须返回错误");
        store.set_fail_save(false);
        // 失败后列表不变（未假装删除成功）。
        assert_eq!(svc.list().unwrap().len(), 1);
    }

    #[test]
    fn confirm_and_delete_in_parallel_lose_no_update() {
        use std::sync::Arc;
        let store = MemStore::new();
        let svc = Arc::new(ErModelService::new("s", Box::new(store)));
        svc.create(base_rel()).unwrap(); // r1 rev=1
        let mut r2 = base_rel();
        r2.id = "r2".into();
        r2.role = "billing_customer".into();
        r2.column_pairs = vec![fluxdb_core::ErColumnPair {
            left_column: "orders-billing_id".into(),
            right_column: "customers-id".into(),
        }];
        svc.create(r2).unwrap(); // r2 rev=1
        // 两线程：确认 r1、删除 r2 —— Mutex 串行，两个效果都必须保留（不丢更新）。
        let s1 = svc.clone();
        let t1 = std::thread::spawn(move || s1.confirm("r1", 1, "alice").unwrap());
        let s2 = svc.clone();
        let t2 = std::thread::spawn(move || s2.delete("r2", 1).unwrap());
        t1.join().unwrap();
        t2.join().unwrap();
        let all = svc.list().unwrap();
        assert_eq!(all.len(), 1, "r2 被删、r1 保留");
        assert_eq!(all[0].review.state, ErReviewState::Confirmed, "r1 确认未丢");
    }

    #[test]
    fn list_includes_proposed_and_rejected() {
        let svc = ErModelService::new("s", Box::new(MemStore::default()));
        svc.create(base_rel()).unwrap();
        svc.reject("r1", 1).unwrap();
        let all = svc.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].review.state, ErReviewState::Rejected);
    }

    #[test]
    fn snapshot_from_tables_keeps_structured_identity_and_skips_not_loaded() {
        let mk = |schema: Option<&str>, name: &str, status: fluxdb_core::ErLoadStatus, cols: Vec<(&str, bool)>| {
            let reference = fluxdb_core::ErTableRef {
                database: "db".into(),
                schema: schema.map(str::to_string),
                name: name.into(),
            };
            fluxdb_core::ErTableNode {
                name: reference.display(),
                reference,
                comment: None,
                status,
                columns: cols
                    .into_iter()
                    .map(|(n, pk)| fluxdb_core::ErColumn {
                        name: n.into(),
                        type_name: Some("text".into()),
                        primary_key: pk,
                        nullable: false,
                    })
                    .collect(),
            }
        };
        let tables = vec![
            // 含点表名 + 跨 schema 同名，结构化身份不串。
            mk(Some("s"), "my.table", fluxdb_core::ErLoadStatus::Loaded, vec![("id", true)]),
            mk(None, "orders", fluxdb_core::ErLoadStatus::Loaded, vec![("id", true), ("customer_id", false)]),
            // 未加载（字段 NotLoaded）→ 不入快照（避免「未读」当「无字段」）。
            mk(None, "pending", fluxdb_core::ErLoadStatus::NotLoaded, vec![]),
        ];
        let snap = er_snapshot_from_tables(&tables);
        assert_eq!(snap.len(), 2, "未加载表不入快照");
        // 含点表名：entity_id 用完整身份（db:s:my.table），qualified_name 为 `s.my.table`。
        let dotted = snap
            .iter()
            .find(|e| e.entity_id == "db:s:my.table")
            .expect("含点表名快照存在");
        assert_eq!(dotted.qualified_name, "s.my.table");
        assert_eq!(dotted.columns[0].column_id, "db:s:my.table::id");
        assert_eq!(dotted.columns[0].name, "id");
        // orders 列齐全。
        let orders = snap.iter().find(|e| e.entity_id == "db::orders").unwrap();
        assert_eq!(orders.columns.len(), 2);
        // 无稳定标识时如实为空（不伪造）。
        assert!(snap.iter().all(|e| e.stable_id.is_none()));
    }
}
