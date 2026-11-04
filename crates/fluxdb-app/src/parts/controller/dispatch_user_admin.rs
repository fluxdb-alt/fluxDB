impl AppController {
    // 公共用户列表、授权与 MySQL 草稿操作独立成帧，避免权限草稿与计划占用无关命令的栈空间。
    fn dispatch_user_admin_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::LoadUserAdminUsers(tab_id) => match self.load_user_admin_users(tab_id) {
                Ok(users) => AppEvent::UserAdminUsersLoaded(tab_id, users),
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::StartUserAdminUsersLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.loading_users = true;
                    admin.users_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminUsersLoad { tab_id, result } => match result {
                Ok(users) => {
                    let default_plugin = self.user_admin_default_auth_plugin(tab_id);
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.loading_users = false;
                        admin.users = users.clone();
                        Self::reset_user_admin_selection_after_users_load(
                            admin,
                            &users,
                            default_plugin,
                        );
                        admin.users_error = None;
                        AppEvent::UserAdminUsersLoaded(tab_id, users)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.loading_users = false;
                        admin.users.clear();
                        admin.users_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::LoadUserAdminGrants { tab_id, user } => {
                match self.load_user_admin_grants(tab_id, &user) {
                    Ok(grants) => AppEvent::UserAdminGrantsLoaded(tab_id, grants),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::LoadUserAdminMemberGrants { tab_id, role } => {
                match self.load_user_admin_member_grants(tab_id, &role) {
                    Ok(members) => AppEvent::UserAdminMemberGrantsLoaded(tab_id, members),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::StartUserAdminGrantsLoad { tab_id, user } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(user);
                    admin.loading_grants = true;
                    admin.grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminGrantsLoad {
                tab_id,
                user,
                result,
            } => match result {
                Ok(grants) => {
                    let privilege_grants = self
                        .user_admin_provider_for_tab(tab_id)
                        .map(|provider| provider.privilege_grants_from_grants(&grants))
                        .unwrap_or_default();
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&user) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_grants = false;
                        admin.grants = grants.clone();
                        admin.grants_loaded_user = Some(user);
                        admin.set_privilege_grants(privilege_grants);
                        admin.role_membership_edits.clear();
                        admin.grants_error = None;
                        AppEvent::UserAdminGrantsLoaded(tab_id, grants)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&user) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_grants = false;
                        admin.grants.clear();
                        admin.grants_loaded_user = None;
                        admin.privilege_rows.clear();
                        admin.base_privilege_rows.clear();
                        admin.grants_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::StartUserAdminMemberGrantsLoad { tab_id, role } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(role);
                    admin.loading_member_grants = true;
                    admin.member_grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminMemberGrantsLoad {
                tab_id,
                role,
                result,
            } => match result {
                Ok(members) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&role) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_member_grants = false;
                        admin.member_grants = members.clone();
                        admin.member_grants_loaded_role = Some(role);
                        admin.member_grant_edits.clear();
                        admin.member_grants_error = None;
                        AppEvent::UserAdminMemberGrantsLoaded(tab_id, members)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&role) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_member_grants = false;
                        admin.member_grants.clear();
                        admin.member_grants_loaded_role = None;
                        admin.member_grants_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::SelectUserAdminUser { tab_id, user } => {
                let default_plugin = self.user_admin_default_auth_plugin(tab_id);
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(user);
                    admin.creating_user = false;
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    admin.member_grants.clear();
                    admin.member_grants_loaded_role = None;
                    admin.grants_error = None;
                    admin.member_grants_error = None;
                    admin.role_membership_edits.clear();
                    admin.member_grant_edits.clear();
                    admin.privilege_rows.clear();
                    admin.base_privilege_rows.clear();
                    if let Some(selected) = &admin.selected_user {
                        admin.create_user = selected.user.clone();
                        admin.create_host = selected.host.clone();
                        admin.auth_plugin = selected
                            .plugin
                            .clone()
                            .unwrap_or_else(|| default_plugin.to_string());
                    }
                    admin.password_expiry_policy = "DEFAULT".to_string();
                    admin.create_password.clear();
                    admin.new_password.clear();
                    admin.reset_advanced_defaults();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::BeginUserAdminCreateUser(tab_id) => {
                let default_plugin = self.user_admin_default_auth_plugin(tab_id);
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.creating_user = true;
                    admin.search.clear();
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    admin.member_grants.clear();
                    admin.member_grants_loaded_role = None;
                    admin.grants_error = None;
                    admin.member_grants_error = None;
                    admin.role_membership_edits.clear();
                    admin.member_grant_edits.clear();
                    admin.privilege_rows.clear();
                    admin.base_privilege_rows.clear();
                    // 用户名留空：与 PG / SQL Server / Oracle 的新建表单一致，不预填占位名。
                    admin.create_user.clear();
                    admin.create_host = "%".to_string();
                    admin.auth_plugin = default_plugin.to_string();
                    admin.password_expiry_policy = "DEFAULT".to_string();
                    admin.create_password.clear();
                    admin.new_password.clear();
                    admin.reset_advanced_defaults();
                    admin.selected_user = Some(admin.draft_user_identity());
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::EndUserAdminCreateUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.creating_user = false;
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::DiscardUserAdminEdits(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.discard_mysql_edits();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::BeginUserAdminDeleteUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    // 仅在选中了已有用户（非新建草稿态）时允许弹出删除确认。
                    if let Some(selected) =
                        admin.selected_user.clone().filter(|_| !admin.creating_user)
                    {
                        admin.pending_delete_user = Some(selected);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::CancelUserAdminDeleteUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_delete_user = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.active_detail_tab = detail_tab;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSearch { tab_id, search } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.search = search;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeDatabase { tab_id, database } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.privilege_database = database;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeTable { tab_id, table } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.privilege_table = table;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ToggleUserAdminPrivilege { tab_id, privilege } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if let Some(index) = admin
                        .selected_privileges
                        .iter()
                        .position(|item| item == &privilege)
                    {
                        admin.selected_privileges.remove(index);
                    } else {
                        admin.selected_privileges.push(privilege);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminGrantOption { tab_id, enabled } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.grant_option = enabled;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::AddUserAdminPrivilegeRow { tab_id, database } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.add_privilege_row(database);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeRowDatabase {
                tab_id,
                row_id,
                database,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_privilege_row_database(row_id, database);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ToggleUserAdminPrivilegeRowPrivilege {
                tab_id,
                row_id,
                privilege,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.toggle_privilege_row_privilege(row_id, privilege);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeRowGrantOption {
                tab_id,
                row_id,
                enabled,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_privilege_row_grant_option(row_id, enabled);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreateUser { tab_id, user } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_user = user;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreateHost { tab_id, host } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_host = host;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminAuthPlugin { tab_id, plugin } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.auth_plugin = plugin;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPasswordExpiryPolicy { tab_id, policy } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.password_expiry_policy = policy;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreatePassword { tab_id, password } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_password = password;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminNewPassword { tab_id, password } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.new_password = password;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxQueriesPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_queries_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxUpdatesPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_updates_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxConnectionsPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_connections_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxUserConnections { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_user_connections = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslType { tab_id, ssl_type } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_type = ssl_type;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslCipher { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_cipher = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslIssuer { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_issuer = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslSubject { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_subject = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMembershipGranted {
                tab_id,
                role,
                granted,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_membership_granted(role, granted);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMembershipDefault {
                tab_id,
                role,
                default_role,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_membership_default(role, default_role);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMemberGranted {
                tab_id,
                member,
                granted,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_member_granted(member, granted);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            // 「保存」第一步：只弹框展示，不碰数据库。真正执行要等用户点确认。
            AppCommand::PreviewUserAdminSql {
                tab_id,
                sql,
                danger,
                kind,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_sql = Some(UserAdminPendingSql { sql, danger, kind });
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::BeginUserAdminSqlConfirm(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_sql_awaits_preview = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ClearUserAdminPendingSql(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_sql = None;
                    admin.pending_sql_awaits_preview = false;
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::StartUserAdminSqlApply(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.applying = true;
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ApplyUserAdminSql { tab_id, sql } => {
                match self.apply_user_admin_sql(tab_id, &sql) {
                    Ok(()) => AppEvent::UserAdminSqlApplied(tab_id),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishUserAdminSqlApply { tab_id, result } => match result {
                Ok(()) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.applying = false;
                        admin.pending_sql = None;
                        admin.apply_error = None;
                        if admin.creating_user {
                            admin.pending_selected_user = Some(admin.draft_user_identity());
                        }
                        admin.creating_user = false;
                        admin.create_password.clear();
                        admin.new_password.clear();
                        admin.grants_loaded_user = None;
                        admin.member_grants.clear();
                        admin.member_grants_loaded_role = None;
                        admin.member_grants_error = None;
                        admin.role_membership_edits.clear();
                        admin.member_grant_edits.clear();
                        admin.base_privilege_rows = admin.privilege_rows.clone();
                        AppEvent::UserAdminSqlApplied(tab_id)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.applying = false;
                        admin.apply_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            // 私有入口仅由 dispatch 的对应变体分组调用。
            _ => unreachable!("仅公共用户列表、授权与 MySQL 草稿操作命令进入此分发器"),
        }
    }
}
