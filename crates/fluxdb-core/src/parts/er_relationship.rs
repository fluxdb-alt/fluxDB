// ER 逻辑关系数据模型（er-design.md §5 D1-D9）。
//
// 四层分离中的「逻辑关系目录」层：与图形视图（ErView，desktop 侧）分离；只存 FluxDB
// 本地逻辑关系，不执行数据库 DDL（§D3）。本文件定义稳定的关系类型与派生的 `usage`
// 判定（§5.9），均纯数据、可 serde、可单测，不依赖 UI / Connector。
//
// 约束身份（§6.2）：关系端点用本地持久化 opaque ID（ErEntityId/ErColumnId），引用用 ID，
// 展示与 SQL 用原始限定名。复合关系保存有序 column_pairs，不拆成多条独立单列关系。
// `required_filters` 为结构化字面量谓词，不接受表达式/函数/子查询/任意 SQL 字符串。

// 本文件被 include! 进 core lib.rs（crate root scope）；serde 的 Serialize/Deserialize 与
// 既有 include! 文件共享 crate-root 导入，不得在此重复 use（避免 E0252 重复定义）。

/// 本地持久化 opaque 实体/列 ID（与展示名分离；模型重载不重新随机生成）。
pub type ErEntityId = String;
pub type ErColumnId = String;
pub type ErRelationshipId = String;
pub type ErEvidenceId = String;

/// 关系两侧（用于 required_filters 指向的边）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErRelationSide {
    Left,
    Right,
}

/// 常驻谓词运算符（§5.3 required_filters）：只接受字面量比较。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErFilterOp {
    Eq,
    Ne,
    IsNull,
    IsNotNull,
    In,
}

/// 结构化字面量值（仅字面量常量；不支持表达式/函数/子查询）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ErLiteral {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// 用于 In 的空列表等无值场景（非 "未加载"）。
    Null,
}

/// 关系成立所需的常驻谓词（如 `right.customer.is_deleted = 0`）。
/// 与 column_pairs 一起进入 JOIN 的 ON 子句（不放入 WHERE，避免改变 LEFT JOIN 语义）。
/// 空列表是常态（表示无过滤），不代表未加载——由加载状态区分。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRequiredFilter {
    pub side: ErRelationSide,
    pub column_id: ErColumnId,
    pub op: ErFilterOp,
    pub literal: ErLiteral,
}

/// 有序字段配对（等值 AND；第一版只支持等值 AND）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErColumnPair {
    pub left_column: ErColumnId,
    pub right_column: ErColumnId,
}

/// 双向匹配基数的最小/最大范围（§5.4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErCardinalityBound {
    Zero,
    One,
    Many,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErCardinality {
    pub min: ErCardinalityBound,
    pub max: ErCardinalityBound,
}

/// 基数依据（§5.4）：未知不猜，缺元数据用 unknown。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErCardinalityBasis {
    DatabaseConstraint,
    UserAssertion,
    ProfileObservation,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErMatchCardinality {
    pub left_to_right: ErCardinality,
    pub right_to_left: ErCardinality,
    /// 完整基数对共用的依据。
    pub basis: ErCardinalityBasis,
}

/// 初始来源（§5.3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErRelationshipOrigin {
    /// 来自数据库真实外键/唯一约束。
    DatabaseConstraint,
    /// 用户手动建立（无物理外键）。
    User,
    /// 从授权 SQL JOIN 观察推断。
    SqlObservation,
    /// Agent 建议。
    Agent,
}

/// 人工确认状态（§5.3 review）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErReviewState {
    Proposed,
    Confirmed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRelationshipReview {
    pub state: ErReviewState,
    /// 确认时对应的关系 revision（确认过期检测，§5.9）。
    pub confirmed_revision: Option<u64>,
    pub confirmed_by: Option<String>,
}

/// 是否有数据库约束声明（§5.3 enforcement；由确认状态不能推断）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErEnforcementKind {
    None,
    /// 数据库物理外键约束（只读展示；不因连线自动改物理库）。
    DeclaredForeignKey,
    DeclaredUnique,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRelationshipEnforcement {
    pub kind: ErEnforcementKind,
    /// 对应数据库约束引用（identity），仅作匹配证据。
    pub constraint_ref: Option<String>,
    /// 数据库实际是否 validate/enable（未知为 None；未启用/未验证不能声称完整性成立）。
    pub enforced: Option<bool>,
}

/// 有效性状态（§5.2 重绑四步结果）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErValidityState {
    Current,
    Stale,
    Unresolved,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErValidity {
    pub state: ErValidityState,
    pub reason: Option<String>,
}

/// 一条关系（§5.3）：role 在同一对实体内唯一；column_pairs 非空、有序、不重复。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRelationship {
    pub id: ErRelationshipId,
    pub revision: u64,
    pub left_entity: ErEntityId,
    pub right_entity: ErEntityId,
    /// 在 (left_entity, right_entity) 内唯一；自关联两个方向靠 role 区分。
    pub role: String,
    pub column_pairs: Vec<ErColumnPair>,
    /// 空列表是常态（无过滤），不代表未加载。
    pub required_filters: Vec<ErRequiredFilter>,
    pub match_cardinality: ErMatchCardinality,
    pub origin: ErRelationshipOrigin,
    pub review: ErRelationshipReview,
    pub enforcement: ErRelationshipEnforcement,
    pub validity: ErValidity,
    pub description: Option<String>,
    pub evidence_refs: Vec<ErEvidenceId>,
}

/// usage 投影（§5.9）：每次读取按关系状态与结构有效性确定性计算，不是 Agent/用户可写属性。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErUsage {
    pub join_candidate: bool,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
}

/// usage 判定输入的最小结构上下文：端点/字段在当前结构快照中是否有效绑定。
/// 由宿主（fluxdb-app）从 CatalogSnapshot 计算传入；本函数只做确定性判定（§5.9 无主体维度）。
#[derive(Clone, Debug, Default)]
pub struct ErUsageContext {
    /// 左/右端点实体在当前快照中是否存在且已加载。
    pub left_entity_present: bool,
    pub right_entity_present: bool,
    /// 是否因端点/字段加载不完整而无法判定（区分「无关系」与「未加载到」）。
    pub coverage_incomplete: bool,
    /// column_pairs 引用的列是否都存在且可比较。
    pub pairs_resolvable: bool,
    /// required_filters 引用的列是否存在且字面量类型兼容。
    pub filters_resolvable: bool,
}

impl ErUsageContext {
    /// 结构快照缺实体/列且未加载时置 coverage_incomplete（不解释为“无关系”）。
    fn incomplete(&self) -> bool {
        self.coverage_incomplete
            || !self.left_entity_present
            || !self.right_entity_present
            || !self.pairs_resolvable
            || !self.filters_resolvable
    }
}

impl ErRelationship {
    /// 计算 usage（§5.9 确定性规则，先命中的拒绝条件优先）。
    ///
    /// 注意：直接的端点结构缺失/未加载由调用方用 `coverage_incomplete` 表达（§5.9 第一行
    /// “不返回该关系”），本函数返回 `join_candidate=false` + `reason_codes`，宿主读取相邻
    /// 时把 coverage 情况随 `ErGraphSlice.coverage` 返回，不解释成“没有关联”。
    pub fn usage(&self, ctx: &ErUsageContext) -> ErUsage {
        let mut reason_codes: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // 端点/字段在快照缺失或未加载 → 不完整，不判为可用（原因交给 coverage 表达）。
        if ctx.incomplete() {
            reason_codes.push("incomplete_definition".to_string());
            return ErUsage {
                join_candidate: false,
                reason_codes,
                warnings,
            };
        }
        if self.column_pairs.is_empty() {
            reason_codes.push("incomplete_definition".to_string());
            return ErUsage {
                join_candidate: false,
                reason_codes,
                warnings,
            };
        }
        // rejected / 确认过期。
        match self.review.state {
            ErReviewState::Rejected => {
                reason_codes.push("rejected".to_string());
                return ErUsage {
                    join_candidate: false,
                    reason_codes,
                    warnings,
                };
            }
            ErReviewState::Confirmed => {
                if self
                    .review
                    .confirmed_revision
                    .map(|r| r != self.revision)
                    .unwrap_or(false)
                {
                    reason_codes.push("confirmation_outdated".to_string());
                    return ErUsage {
                        join_candidate: false,
                        reason_codes,
                        warnings,
                    };
                }
            }
            ErReviewState::Proposed => {}
        }
        // validity：stale/unresolved/invalid。
        match self.validity.state {
            ErValidityState::Stale => {
                reason_codes.push("stale".to_string());
                return ErUsage {
                    join_candidate: false,
                    reason_codes,
                    warnings,
                };
            }
            ErValidityState::Unresolved | ErValidityState::Invalid => {
                reason_codes.push(
                    if self.validity.state == ErValidityState::Unresolved {
                        "invalid_binding".to_string()
                    } else {
                        "invalid_binding".to_string()
                    },
                );
                return ErUsage {
                    join_candidate: false,
                    reason_codes,
                    warnings,
                };
            }
            ErValidityState::Current => {}
        }
        // 字段类型不可比较由 pairs_resolvable=false 已覆盖；filter 同理。
        // 有效数据库约束声明 & 未被用户拒绝 → 候选；未启用/未验证加警告。
        let has_declared = matches!(
            self.enforcement.kind,
            ErEnforcementKind::DeclaredForeignKey | ErEnforcementKind::DeclaredUnique
        );
        if has_declared {
            match self.enforcement.enforced {
                Some(true) => {}
                Some(false) | None => {
                    warnings.push("constraint_not_enforced".to_string());
                }
            }
            return ErUsage {
                join_candidate: true,
                reason_codes,
                warnings,
            };
        }
        // 用户已确认 → 候选（无约束标记 logical_only）。
        if self.review.state == ErReviewState::Confirmed {
            warnings.push("logical_only".to_string());
            if self.match_cardinality.left_to_right.max == ErCardinalityBound::Unknown
                || self.match_cardinality.right_to_left.max == ErCardinalityBound::Unknown
            {
                warnings.push("cardinality_unknown".to_string());
            }
            // 多对多/可能放大行数风险（不能据此直接累加金额）。
            if self.match_cardinality.left_to_right.max == ErCardinalityBound::Many
                && self.match_cardinality.right_to_left.max == ErCardinalityBound::Many
            {
                warnings.push("fanout_risk".to_string());
            }
            return ErUsage {
                join_candidate: true,
                reason_codes,
                warnings,
            };
        }
        // 未确认（proposed / agent / sql_observation）→ 需确认。
        reason_codes.push("confirmation_required".to_string());
        ErUsage {
            join_candidate: false,
            reason_codes,
            warnings,
        }
    }

    /// 生成可执行的 JOIN 计划（§5.5/§6.6 查询服务地基）：只有 usage 为 join_candidate 的关系
    /// 才产出；否则 None（宿主用 coverage 区分"未加载"与"不可用"，不得解释为"无关系"）。
    ///
    /// 返回的 `pairs` 与 `filters` 均为**完整**返回（§6.6：column_pairs 与 required_filters
    /// 都不可从中间截断——少一个配对丢失租户边界，少一个常驻过滤把软删数据算进结果）。
    /// ON 条件 = pairs（等值 AND）+ filters（常驻谓词）；调用方必须把二者一起放入 JOIN ON
    /// 子句（不放 WHERE，否则 LEFT JOIN 语义被改变，§5.3）。本函数只产出结构化条件，
    /// 不拼任意 SQL 字符串（字面量由 `ErLiteral` 类型约束，无表达式/函数/子查询）。
    pub fn join_plan(&self, ctx: &ErUsageContext) -> Option<ErJoinPlan> {
        let usage = self.usage(ctx);
        if !usage.join_candidate {
            return None;
        }
        Some(ErJoinPlan {
            relationship_id: self.id.clone(),
            rev: self.revision,
            role: self.role.clone(),
            left_entity: self.left_entity.clone(),
            right_entity: self.right_entity.clone(),
            pairs: self.column_pairs.clone(),
            filters: self.required_filters.clone(),
            warnings: usage.warnings,
        })
    }
}

/// 可执行的 JOIN 条件（§5.5）：完整保留复合配对与常驻谓词，供 SQL 生成 / Agent 投影消费。
/// 不含任何任意 SQL 字符串；列/过滤用 opaque ID 表达，宿主绑定原始限定名。
#[derive(Clone, Debug, PartialEq)]
pub struct ErJoinPlan {
    pub relationship_id: ErRelationshipId,
    pub rev: u64,
    pub role: String,
    pub left_entity: ErEntityId,
    pub right_entity: ErEntityId,
    /// 有序等值 AND 字段配对（完整，不截断）。
    pub pairs: Vec<ErColumnPair>,
    /// 常驻谓词（完整，不截断；进入 ON 而非 WHERE）。
    pub filters: Vec<ErRequiredFilter>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod er_relationship_tests {
    use super::*;

    fn confirmed_rel(revision: u64, review: ErReviewState) -> ErRelationship {
        ErRelationship {
            id: "r1".into(),
            revision,
            left_entity: "e-orders".into(),
            right_entity: "e-customers".into(),
            role: "order_customer".into(),
            column_pairs: vec![ErColumnPair {
                left_column: "orders-customer_id".into(),
                right_column: "customers-id".into(),
            }],
            required_filters: vec![ErRequiredFilter {
                side: ErRelationSide::Right,
                column_id: "customers-is_deleted".into(),
                op: ErFilterOp::Eq,
                literal: ErLiteral::Int(0),
            }],
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
            review: ErRelationshipReview {
                state: review,
                confirmed_revision: Some(revision),
                confirmed_by: Some("alice".into()),
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
            description: Some("订单归属于客户".into()),
            evidence_refs: vec!["ev1".into()],
        }
    }

    fn ok_ctx() -> ErUsageContext {
        ErUsageContext {
            left_entity_present: true,
            right_entity_present: true,
            coverage_incomplete: false,
            pairs_resolvable: true,
            filters_resolvable: true,
        }
    }

    #[test]
    fn confirmed_user_rel_is_join_candidate_with_logical_only() {
        let u = confirmed_rel(3, ErReviewState::Confirmed).usage(&ok_ctx());
        assert!(u.join_candidate);
        assert!(u.warnings.contains(&"logical_only".to_string()));
    }

    #[test]
    fn proposed_rel_requires_confirmation() {
        let u = confirmed_rel(1, ErReviewState::Proposed).usage(&ok_ctx());
        assert!(!u.join_candidate);
        assert!(u.reason_codes.contains(&"confirmation_required".to_string()));
    }

    #[test]
    fn stale_and_rejected_not_candidates() {
        let mut r = confirmed_rel(2, ErReviewState::Confirmed);
        r.validity.state = ErValidityState::Stale;
        assert!(!r.usage(&ok_ctx()).join_candidate);

        let mut r2 = confirmed_rel(2, ErReviewState::Confirmed);
        r2.review.state = ErReviewState::Rejected;
        assert!(!r2.usage(&ok_ctx()).join_candidate);
    }

    #[test]
    fn outdated_confirmation_not_candidate() {
        // 确认 revision 早于当前 revision → 确认过期。
        let mut r = confirmed_rel(5, ErReviewState::Confirmed);
        r.review.confirmed_revision = Some(3);
        let u = r.usage(&ok_ctx());
        assert!(!u.join_candidate);
        assert!(u.reason_codes.contains(&"confirmation_outdated".to_string()));
    }

    #[test]
    fn incomplete_coverage_not_candidate_but_not_reasoned_as_none() {
        let mut ctx = ok_ctx();
        ctx.coverage_incomplete = true;
        let u = confirmed_rel(2, ErReviewState::Confirmed).usage(&ctx);
        assert!(!u.join_candidate);
        // 宿主应把 coverage 状态返回，不得把 false 解释成“无关系”。
        assert!(ctx.incomplete());
    }

    #[test]
    fn declared_constraint_candidate_with_warning_when_not_enforced() {
        let mut r = confirmed_rel(1, ErReviewState::Proposed);
        r.enforcement = ErRelationshipEnforcement {
            kind: ErEnforcementKind::DeclaredForeignKey,
            constraint_ref: Some("fk_x".into()),
            enforced: None,
        };
        let u = r.usage(&ok_ctx());
        // 有效数据库约束声明，即使未确认也可作候选（§5.9），但未启用/未验证需警告。
        assert!(u.join_candidate);
        assert!(u.warnings.contains(&"constraint_not_enforced".to_string()));
    }

    #[test]
    fn empty_pairs_not_candidate() {
        let mut r = confirmed_rel(1, ErReviewState::Confirmed);
        r.column_pairs = Vec::new();
        assert!(!r.usage(&ok_ctx()).join_candidate);
    }

    fn composite_confirmed() -> ErRelationship {
        use super::ErColumnPair as P;
        let mut r = confirmed_rel(4, ErReviewState::Confirmed);
        r.column_pairs = vec![
            P {
                left_column: "orders-tenant_id".into(),
                right_column: "customers-tenant_id".into(),
            },
            P {
                left_column: "orders-customer_id".into(),
                right_column: "customers-id".into(),
            },
        ];
        r
    }

    #[test]
    fn join_plan_keeps_composite_pairs_and_filters_intact() {
        let r = composite_confirmed();
        let plan = r.join_plan(&ok_ctx()).expect("confirmed 复合关系应有 plan");
        // 复合两对全部保留（§6.6 不截断）。
        assert_eq!(plan.pairs.len(), 2);
        assert_eq!(plan.pairs[0].left_column, "orders-tenant_id");
        assert_eq!(plan.pairs[1].right_column, "customers-id");
        // required_filters 完整返回（不做节选）。
        assert_eq!(plan.filters.len(), r.required_filters.len());
        assert_eq!(plan.filters, r.required_filters);
        // logical_only 警告随 plan 输出。
        assert!(plan.warnings.contains(&"logical_only".to_string()));
    }

    #[test]
    fn join_plan_none_when_not_candidate() {
        // proposed → 不可用 → 无 plan。
        let r = confirmed_rel(1, ErReviewState::Proposed);
        assert!(r.join_plan(&ok_ctx()).is_none());
        // 覆盖不完整 → 无 plan（宿主应由 coverage 表达，不解释为“无关系”）。
        let mut ctx = ok_ctx();
        ctx.coverage_incomplete = true;
        assert!(composite_confirmed().join_plan(&ctx).is_none());
    }
}
