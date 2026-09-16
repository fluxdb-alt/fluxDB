impl AppController {
    /// 经连接器列出 PG 角色（集群级主体）。复用 `role_operation_for_connection` 统一路由。
    fn list_pg_roles_for_connection(&self, connection_id: ConnectionId) -> fluxdb_core::Result<Vec<fluxdb_core::PgRole>> {
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| connector.list_roles(connection_id))
    }

    fn user_admin_state(&self, tab_id: TabId) -> Option<&UserAdminState> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::UserAdmin(admin) => Some(admin),
            _ => None,
        })
    }

    fn user_admin_state_mut(&mut self, tab_id: TabId) -> Option<&mut UserAdminState> {
        self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::UserAdmin(admin) => Some(admin),
            _ => None,
        })
    }

    fn user_admin_provider_for_tab(
        &self,
        tab_id: TabId,
    ) -> fluxdb_core::Result<fluxdb_core::DatabaseUserAdminProvider> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let kind = self
            .connection_kind(admin.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        database_user_admin_provider(kind)
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "暂不支持该连接的用户与权限管理"))
    }

    fn load_user_admin_users(&self, tab_id: TabId) -> fluxdb_core::Result<Vec<DatabaseUserIdentity>> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        // PG：角色是集群级身份，直接经连接器 list_roles 读取（不用 user@host SQL 文字），
        // 映射为 DatabaseUserIdentity[user=role, host="", plugin=None] 以复用现有用户列表承载。
        if self.connection_kind(admin.connection_id) == Some(DatabaseKind::Postgres) {
            let roles = self.list_pg_roles_for_connection(admin.connection_id)?;
            let role_identities: Vec<DatabaseUserIdentity> = roles
                .into_iter()
                .map(|role| DatabaseUserIdentity {
                    user: role.name,
                    host: String::new(),
                    plugin: None,
                })
                .collect();
            if !role_identities.is_empty() {
                return Ok(role_identities);
            }
        }
        let provider = self.user_admin_provider_for_tab(tab_id)?;
        let request = QueryRequest {
            connection_id: admin.connection_id,
            database: None,
            session_id: None,
            schema: None,
            text: provider.list_users_sql().to_string(),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        };

        let primary_result = self.execute_query_raw(&request);
        if let Ok(execution) = &primary_result {
            let users = execution
                .results
                .first()
                .map(|page| provider.parse_users(page))
                .unwrap_or_default();
            if !users.is_empty() {
                return Ok(users);
            }
        }

        let fallback_result = provider.fallback_list_users_sql().map(|fallback_sql| {
            let fallback_request = QueryRequest {
                text: fallback_sql.to_string(),
                ..request.clone()
            };
            self.execute_query_raw(&fallback_request)
        });
        if let Some(Ok(execution)) = &fallback_result {
            let users = execution
                .results
                .first()
                .map(|page| provider.parse_fallback_users(page))
                .unwrap_or_default();
            if !users.is_empty() {
                return Ok(users);
            }
        }

        let current_user_request = QueryRequest {
            text: provider.current_user_sql().to_string(),
            ..request
        };
        match self.execute_query_raw(&current_user_request) {
            Ok(execution) => {
                let users = execution
                    .results
                    .first()
                    .map(|page| provider.parse_users(page))
                    .unwrap_or_default();
                if !users.is_empty() {
                    return Ok(users);
                }
            }
            Err(current_user_error) => {
                return Err(primary_result
                    .err()
                    .or_else(|| fallback_result.and_then(|result| result.err()))
                    .unwrap_or(current_user_error));
            }
        }

        Err(primary_result
            .err()
            .or_else(|| fallback_result.and_then(|result| result.err()))
            .unwrap_or_else(|| Error::new(ErrorKind::Query, "未读取到当前数据库用户")))
    }

    fn load_user_admin_grants(
        &self,
        tab_id: TabId,
        user: &DatabaseUserIdentity,
    ) -> fluxdb_core::Result<Vec<String>> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        // PG：角色「授权」= 其所在的组角色（成员关系），经连接器读取，非 SHOW GRANTS 文本解析。
        // 空结果也是成功空态：无成员关系的角色不允许回退 MySQL provider（那是错误路径）。
        if self.connection_kind(admin.connection_id) == Some(DatabaseKind::Postgres) {
            let memberships = self.list_pg_memberships_for_connection(admin.connection_id)?;
            return Ok(pg_groups_for_member(&memberships, &user.user));
        }
        let provider = self.user_admin_provider_for_tab(tab_id)?;
        let request = QueryRequest {
            connection_id: admin.connection_id,
            database: None,
            session_id: None,
            schema: None,
            text: provider.show_grants_sql(user),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        };
        self.execute_query_raw(&request)
            .map(|execution| grants_from_query_result(&execution))
    }

    /// 经连接器读取 PG 成员关系（含 admin/inherit/set 选项，版本感知）。
    fn list_pg_memberships_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> fluxdb_core::Result<Vec<fluxdb_core::PgRoleMembership>> {
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| connector.list_role_membership(connection_id))
    }

    fn load_user_admin_member_grants(
        &self,
        tab_id: TabId,
        role: &DatabaseUserIdentity,
    ) -> fluxdb_core::Result<Vec<UserRoleMember>> {
        let users = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?
            .users
            .clone();
        let mut members = Vec::new();
        for user in users.into_iter().filter(|user| user != role) {
            let grants = self.load_user_admin_grants(tab_id, &user)?;
            let granted = role_memberships_from_grants(std::slice::from_ref(role), &grants, &user)
                .first()
                .is_some_and(|membership| membership.granted);
            members.push(UserRoleMember {
                member: user,
                granted,
            });
        }
        Ok(members)
    }

    fn apply_user_admin_sql(&self, tab_id: TabId, sql: &str) -> fluxdb_core::Result<()> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let request = QueryRequest {
            connection_id: admin.connection_id,
            database: None,
            session_id: None,
            schema: None,
            text: sql.to_string(),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        };
        let execution = self.execute_query(&request)?;
        // 连接器把单条语句错误放在 execution.summaries（success=false）而非抛 Err，
        // 以便查询编辑器批处理时不中断其他语句。这里需要「整批要么全成、要么全败」的
        // 语义，故显式检查是否有失败语句，避免 DROP USER 等被权限拒绝却误判成功。
        if let Some(failed) = execution.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(
                ErrorKind::Query,
                format!("数据库执行失败：{}", failed.message),
            ));
        }
        Ok(())
    }

    fn reset_user_admin_selection_after_users_load(
        admin: &mut UserAdminState,
        users: &[DatabaseUserIdentity],
    ) {
        if admin
            .selected_user
            .as_ref()
            .is_none_or(|selected| !users.iter().any(|user| user == selected))
        {
            admin.selected_user = users.first().cloned();
        }
        admin.creating_user = false;
        if let Some(selected) = &admin.selected_user {
            admin.create_user = selected.user.clone();
            admin.create_host = selected.host.clone();
            admin.auth_plugin = selected
                .plugin
                .clone()
                .unwrap_or_else(|| "caching_sha2_password".to_string());
        }
        admin.password_expiry_policy = "DEFAULT".to_string();
        admin.create_password.clear();
        admin.new_password.clear();
        admin.grants_loaded_user = None;
        admin.member_grants.clear();
        admin.member_grants_loaded_role = None;
        admin.member_grant_edits.clear();
        admin.role_membership_edits.clear();
        admin.reset_advanced_defaults();
    }
}

/// PG 成员关系中某角色直接所在的组角色名列表：(grantee, member, admin_option) → grantee。
/// 供 PG 用户/权限 UI 展示「该 role 是哪些组角色的成员」（MemberOf 语义），纯函数便于测试。
pub fn pg_groups_for_member(memberships: &[fluxdb_core::PgRoleMembership], member: &str) -> Vec<String> {
    memberships
        .iter()
        .filter(|m| m.member == member)
        .map(|m| m.grantee.clone())
        .collect()
}

/// 由 PG 权限面板状态构造连接器所需的授权目标 scope（纯函数，便于测试）。
///
/// 表/视图/序列归 Relation（带 PgRelationKind），函数归 Routine（schema+name+签名，区分重载），
/// schema/数据库各自独立。空 schema 回退 public（PG 默认）。
pub fn pg_grant_scope_from_state(
    kind: PgGrantObjectKind,
    schema: &str,
    object: &str,
    signature: &str,
) -> PgObjectGrantScope {
    let schema = if schema.trim().is_empty() { "public" } else { schema.trim() };
    match kind {
        PgGrantObjectKind::Table => PgObjectGrantScope::Relation {
            schema: schema.to_string(),
            name: object.trim().to_string(),
            kind: PgRelationKind::Table,
        },
        PgGrantObjectKind::View => PgObjectGrantScope::Relation {
            schema: schema.to_string(),
            name: object.trim().to_string(),
            kind: PgRelationKind::View,
        },
        PgGrantObjectKind::Sequence => PgObjectGrantScope::Relation {
            schema: schema.to_string(),
            name: object.trim().to_string(),
            kind: PgRelationKind::Sequence,
        },
        PgGrantObjectKind::Schema => PgObjectGrantScope::Schema {
            schema: schema.to_string(),
        },
        PgGrantObjectKind::Database => PgObjectGrantScope::Database {
            database: object.trim().to_string(),
        },
        PgGrantObjectKind::Routine => PgObjectGrantScope::Routine {
            schema: schema.to_string(),
            name: object.trim().to_string(),
            signature: signature.trim().to_string(),
        },
    }
}

impl AppController {
    /// 读取 PG 对象权限：对象读模型（owner/默认/显式条目）+ 选中角色生效权限（直接 vs 继承）。
    fn load_pg_object_grants(
        &self,
        tab_id: TabId,
    ) -> fluxdb_core::Result<(fluxdb_core::PgObjectGrants, Vec<fluxdb_core::PgEffectivePrivilege>)> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let role = admin
            .pg_effective_grantee_name();
        if role.is_empty() {
            return Err(Error::new(ErrorKind::Query, "请先选择角色"));
        }
        let scope = pg_grant_scope_from_state(
            admin.pg_grant_kind,
            &admin.pg_grant_schema,
            &admin.pg_grant_object,
            &admin.pg_grant_signature,
        );
        let connection_id = admin.connection_id;
        let mut config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?
            .clone();
        // 权限接口使用连接上下文的库；仅调整本次请求副本，不能把所选库对象查到维护库。
        if !admin.pg_grant_database.is_empty() {
            let profile = config.postgres_profile.as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;
            profile.basic.maintenance_database = admin.pg_grant_database.clone();
        }
        let result = role_operation_for_connection(&config, |connector| {
            let grants = connector.list_object_grants(connection_id, &scope)?;
            let effective = connector.role_effective_grants(connection_id, &scope, &role)?;
            Ok((grants, effective))
        });
        if let Err(error) = &result {
            tracing::warn!(target: "pg_user_admin", database = %admin.pg_grant_database,
                ?scope, error = %error, "读取 PostgreSQL 对象权限失败");
        }
        result
    }

    /// 由草稿 diff 出本次保存的结构化变更计划（预览与执行共用）。
    ///
    /// 顺序由连接器渲染保证：创建 → 改名 → 属性/密码 → 成员 → 对象授权；
    /// 改名后所有变更引用新身份。失败（如连接数限制非法）返回错误，不产生半份计划。
    pub fn build_pg_role_plan(&self, tab_id: TabId) -> fluxdb_core::Result<PgRoleSavePlan> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let Some(draft) = admin.pg_draft.clone() else {
            return Err(Error::new(ErrorKind::Query, "没有待保存的草稿"));
        };
        let baseline = admin.pg_baseline_role();
        let name = draft.name.trim().to_string();
        if name.is_empty() {
            return Err(Error::new(ErrorKind::Query, "角色名不能为空"));
        }
        // 连接数限制提前校验（diff 失败即视为非法文本）。
        if draft.parsed_connection_limit().is_none() {
            return Err(Error::new(ErrorKind::Query, "连接数限制必须是不限（-1）或非负整数"));
        }
        // 确认密码一致性由 UI 保证；此处校验密码操作合法性。
        if let PgPasswordOp::Set(password) = &draft.password {
            if draft.create && draft.can_login && password.is_empty() {
                return Err(Error::new(
                    ErrorKind::Query,
                    "新建可登录角色未设置密码（如需无密码请选择「不设置密码」）",
                ));
            }
        }
        // 预定义角色（pg_*）由系统管理：拒绝改名/属性/密码/有效期变更；
        // 成员关系与对象授权仍可按授权能力操作（服务端最终判定）。
        if let Some(base) = baseline
            && UserAdminState::pg_is_predefined_role(&base.name)
        {
            let identity_touched = base.name != name
                || fluxdb_core::pg_role_attributes_diff(Some(base), &draft).is_some()
                || draft.password != PgPasswordOp::Keep
                || draft.valid_until != PgValidUntilOp::Keep;
            if identity_touched {
                return Err(Error::new(
                    ErrorKind::Query,
                    format!("预定义角色「{}」由系统管理，不允许修改名称、属性或密码", base.name),
                ));
            }
        }
        let mut changes: Vec<PgRoleChange> = Vec::new();
        match baseline {
            None => {
                // 新建：CREATE + 全量属性 + 初始密码操作。
                let attributes =
                    fluxdb_core::pg_role_attributes_diff(None, &draft)
                        .unwrap_or_default();
                changes.push(PgRoleChange::Create {
                    name: name.clone(),
                    can_login: draft.can_login,
                    password: draft.password.clone(),
                    attributes,
                });
            }
            Some(base) => {
                if base.name != name {
                    changes.push(PgRoleChange::Rename {
                        from: base.name.clone(),
                        to: name.clone(),
                    });
                }
                if let Some(attributes) = fluxdb_core::pg_role_attributes_diff(Some(base), &draft)
                    && !attributes.is_empty()
                {
                    changes.push(PgRoleChange::AlterAttributes {
                        name: name.clone(),
                        attributes,
                    });
                }
                match &draft.password {
                    PgPasswordOp::Keep => {}
                    PgPasswordOp::Set(password) => changes.push(PgRoleChange::SetPassword {
                        name: name.clone(),
                        password: (!password.is_empty()).then_some(password.clone()),
                    }),
                    PgPasswordOp::Clear => changes.push(PgRoleChange::SetPassword {
                        name: name.clone(),
                        password: None,
                    }),
                }
            }
        }
        // 成员与授权草稿：受影响角色统一改为最终身份（改名场景）。
        // 草稿记录里的受影响角色是编辑时的旧名；重命名时全部替换为新名。
        let old_name = admin.pg_selected_role.clone().unwrap_or_default();
        let final_name = name;
        for edit in &admin.pg_membership_edits {
            changes.push(rewrite_pg_change_role(edit, &old_name, &final_name));
        }
        for edit in &admin.pg_grant_edits {
            changes.push(rewrite_pg_change_role(edit, &old_name, &final_name));
        }
        // 对象授权所在数据库（一期同批一个库）；无对象授权时为 None。
        let database = admin
            .pg_grant_edits
            .first()
            .map(|_| admin.pg_grant_database.clone())
            .filter(|db| !db.is_empty());
        Ok(PgRoleSavePlan {
            database,
            role_name: final_name,
            changes,
        })
    }
}

/// 把成员/授权草稿变更的受影响角色统一替换为最终名（改名后引用新身份）。
///
/// `old_name` 是编辑时的身份：成员关系的 member、对象授权的 grantee 若等于旧名则替换。
fn rewrite_pg_change_role(edit: &PgRoleChange, old_name: &str, final_name: &str) -> PgRoleChange {
    let rewrite_member = |member: &str| {
        if member == old_name && !old_name.is_empty() {
            final_name.to_string()
        } else {
            member.to_string()
        }
    };
    match edit {
        PgRoleChange::GrantMembership { role, member, admin, inherit, set } => {
            PgRoleChange::GrantMembership {
                role: role.clone(),
                member: rewrite_member(member),
                admin: *admin,
                inherit: *inherit,
                set: *set,
            }
        }
        PgRoleChange::RevokeMembership { role, member } => PgRoleChange::RevokeMembership {
            role: role.clone(),
            member: rewrite_member(member),
        },
        PgRoleChange::GrantObject { privilege, scope, grantee, grant_option } => {
            PgRoleChange::GrantObject {
                privilege: privilege.clone(),
                scope: scope.clone(),
                grantee: rewrite_member(grantee),
                grant_option: *grant_option,
            }
        }
        PgRoleChange::RevokeObject { privilege, scope, grantee } => PgRoleChange::RevokeObject {
            privilege: privilege.clone(),
            scope: scope.clone(),
            grantee: rewrite_member(grantee),
        },
        PgRoleChange::RevokeGrantOption { privilege, scope, grantee } => {
            PgRoleChange::RevokeGrantOption {
                privilege: privilege.clone(),
                scope: scope.clone(),
                grantee: rewrite_member(grantee),
            }
        }
        other => other.clone(),
    }
}

/// 把授权目标渲染为 GRANT/REVOKE 的 `ON <object>` 片段（PG 双引号、函数带签名）。
///
/// 渲染与校验的事实来源在连接器（`pg_object_scope_sql`，设计 §12 要求由 connector 负责
/// 标识符引用与白名单）；此处仅转发，供预览/测试使用，避免两份不一致的渲染实现。
pub fn pg_grant_object_sql(scope: &PgObjectGrantScope) -> fluxdb_core::Result<String> {
    fluxdb_connectors::pg_object_scope_sql(scope)
}

/// 当前授权目标指纹（kind|schema|object|signature），用于判断已读权限是否过期。
pub fn pg_grant_target_fingerprint(admin: &UserAdminState) -> String {
    format!(
        "{}|{}|{}|{}",
        admin.pg_grant_kind.label(),
        admin.pg_grant_schema,
        admin.pg_grant_object,
        admin.pg_grant_signature
    )
}
