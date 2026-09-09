impl AppController {
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
        let provider = self.user_admin_provider_for_tab(tab_id)?;
        let request = QueryRequest {
            connection_id: admin.connection_id,
            database: None,
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
        let provider = self.user_admin_provider_for_tab(tab_id)?;
        let request = QueryRequest {
            connection_id: admin.connection_id,
            database: None,
            text: provider.show_grants_sql(user),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        };
        self.execute_query_raw(&request)
            .map(|execution| grants_from_query_result(&execution))
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
            text: sql.to_string(),
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        };
        self.execute_query(&request).map(|_| ())
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
