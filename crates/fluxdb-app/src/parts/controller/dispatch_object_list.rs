/// 概览只展示数据表：分组目录里可能混有视图，且连接器返回顺序不保证稳定。
fn table_overview_objects(mut objects: Vec<ObjectSummary>) -> Vec<ObjectSummary> {
    objects.retain(|object| object.path.kind == ObjectKind::Table);
    objects.sort_by(|a, b| a.path.name.cmp(&b.path.name));
    objects
}

impl AppController {
    // 表概览独立成帧：后台读取与本地过滤不占用公共分派器的小栈空间。
    fn dispatch_object_list_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::StartObjectListLoad(tab_id) => match self.object_list_state_mut(tab_id) {
                Some(list) => {
                    list.loading = true;
                    list.error = None;
                    AppEvent::TabActivated(tab_id)
                }
                None => self.fail(Error::new(ErrorKind::Internal, "表概览标签页不存在")),
            },
            // 只读连接器读取，不改 AppState；桌面端在后台线程执行本条命令。
            AppCommand::LoadObjectList(tab_id) => {
                let Some(list) = self.object_list_state(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "表概览标签页不存在"));
                };
                let Some(parent) = list.parent.clone() else {
                    return self.fail(Error::new(ErrorKind::Internal, "表概览缺少父对象"));
                };
                let Some(config) = self.connection_config(parent.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let result = list_objects_for_connection(&config, Some(&parent))
                    .map_err(UserFacingError::from);
                AppEvent::ObjectListLoaded { tab_id, result }
            }
            AppCommand::FinishObjectListLoad { tab_id, result } => {
                let Some(list) = self.object_list_state_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "表概览标签页不存在"));
                };
                list.loading = false;
                match result {
                    Ok(objects) => {
                        list.objects = table_overview_objects(objects);
                        list.error = None;
                    }
                    Err(error) => {
                        // 保留上次成功的数据，局部错误只落在页面内，不升级为全局失败。
                        list.error = Some(error);
                    }
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetObjectListSearch { tab_id, search } => {
                if let Some(list) = self.object_list_state_mut(tab_id) {
                    list.search = search;
                }
                AppEvent::TabActivated(tab_id)
            }
            _ => unreachable!("非表概览命令不应进入 dispatch_object_list_command"),
        }
    }

    fn object_list_state(&self, tab_id: TabId) -> Option<&ObjectListState> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::ObjectList(list) => Some(list),
            _ => None,
        })
    }

    fn object_list_state_mut(&mut self, tab_id: TabId) -> Option<&mut ObjectListState> {
        self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::ObjectList(list) => Some(list),
            _ => None,
        })
    }
}
