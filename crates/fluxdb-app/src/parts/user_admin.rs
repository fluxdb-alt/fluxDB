impl AppController {
    /// 经连接器列出 PG 角色（集群级主体）。复用 `role_operation_for_connection` 统一路由。
    fn list_pg_roles_for_connection(&self, connection_id: ConnectionId) -> fluxdb_core::Result<Vec<fluxdb_core::PgRole>> {
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| connector.list_roles(connection_id))
    }

    /// 经连接器列出 PG 角色 → 名称 → 是否可登录（LOGIN），供角色选项切换展示。
    fn load_pg_role_login_map(
        &self,
        connection_id: ConnectionId,
    ) -> fluxdb_core::Result<BTreeMap<String, bool>> {
        let roles = self.list_pg_roles_for_connection(connection_id)?;
        Ok(roles.into_iter().map(|r| (r.name, r.can_login)).collect())
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
        if self.connection_kind(admin.connection_id) == Some(DatabaseKind::Postgres) {
            let memberships = self.list_pg_memberships_for_connection(admin.connection_id)?;
            let groups = pg_groups_for_member(&memberships, &user.user);
            if !groups.is_empty() || user.user.is_empty() {
                return Ok(groups);
            }
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
            .selected_user
            .as_ref()
            .map(|u| u.user.clone())
            .ok_or_else(|| Error::new(ErrorKind::Query, "请先选择角色"))?;
        let scope = pg_grant_scope_from_state(
            admin.pg_grant_kind,
            &admin.pg_grant_schema,
            &admin.pg_grant_object,
            &admin.pg_grant_signature,
        );
        let connection_id = admin.connection_id;
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| {
            let grants = connector.list_object_grants(connection_id, &scope)?;
            let effective = connector.role_effective_grants(connection_id, &scope, &role)?;
            Ok((grants, effective))
        })
    }

    /// 授予选中角色某权限（GRANT ... ON 目标 TO 角色）。仅对**直接授权**生效，不改继承/owner。
    fn apply_pg_grant(
        &self,
        tab_id: TabId,
        privilege: &str,
        grant_option: bool,
    ) -> fluxdb_core::Result<()> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let role = admin
            .selected_user
            .as_ref()
            .map(|u| u.user.clone())
            .ok_or_else(|| Error::new(ErrorKind::Query, "请先选择角色"))?;
        let scope = pg_grant_scope_from_state(
            admin.pg_grant_kind,
            &admin.pg_grant_schema,
            &admin.pg_grant_object,
            &admin.pg_grant_signature,
        );
        let connection_id = admin.connection_id;
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| {
            connector.grant_object_privilege(connection_id, privilege, &scope, &role, grant_option)
        })
    }

    /// 撤销选中角色某权限（REVOKE ... ON 目标 FROM 角色）。只撤销该角色的**直接**授权。
    fn apply_pg_revoke(&self, tab_id: TabId, privilege: &str) -> fluxdb_core::Result<()> {
        let admin = self
            .user_admin_state(tab_id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))?;
        let role = admin
            .selected_user
            .as_ref()
            .map(|u| u.user.clone())
            .ok_or_else(|| Error::new(ErrorKind::Query, "请先选择角色"))?;
        let scope = pg_grant_scope_from_state(
            admin.pg_grant_kind,
            &admin.pg_grant_schema,
            &admin.pg_grant_object,
            &admin.pg_grant_signature,
        );
        let connection_id = admin.connection_id;
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        role_operation_for_connection(&config, |connector| {
            connector.revoke_object_privilege(connection_id, privilege, &scope, &role)
        })
    }
}

/// 把授权目标渲染为 GRANT/REVOKE 的 `ON <object>` 片段（PG 双引号、函数带签名）。
///
/// 渲染与校验的事实来源在连接器（`pg_object_scope_sql`，设计 §12 要求由 connector 负责
/// 标识符引用与白名单）；此处仅转发，供预览/测试使用，避免两份不一致的渲染实现。
pub fn pg_grant_object_sql(scope: &PgObjectGrantScope) -> fluxdb_core::Result<String> {
    fluxdb_connectors::pg_object_scope_sql(scope)
}
