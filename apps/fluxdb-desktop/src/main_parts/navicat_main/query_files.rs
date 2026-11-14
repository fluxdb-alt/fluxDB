#[derive(Default)]
struct QueryFilesUi {
    loading: bool,
    task: Option<Task<()>>,
}

impl NavicatMain {
    fn choose_local_sql_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.query_files.loading {
            return;
        }
        let Some(TabKind::QueryEditor(query)) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| &tab.kind)
        else {
            return;
        };
        // 固定用户点击时的执行作用域；选择文件期间切换页面不得改变新查询的目标。
        let connection_id = query.connection_id;
        let database = query.database.clone();
        let schema = query.schema.clone();
        self.query_files.loading = true;
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.query.picker.open_sql").into()),
        });
        self.query_files.task=Some(cx.spawn(async move |view,cx|{
            let selected=picker.await;
            let result=match selected {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => Some(cx.background_spawn(async move {
                        fluxdb_app::read_sql_file(&path).map(|text|(path,text))
                    }).await),
                    None=>None,
                },
                Ok(Ok(None))=>None,
                _=>Some(Err(fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal,tr("app.query.error.file_picker").to_string()))),
            };
            let _=view.update(cx,|this,cx|{
                this.query_files.loading=false;
                this.query_files.task=None;
                match result {
                    Some(Ok((path,text)))=>{
                        if let Some(existing)=this.controller.state().tabs.iter().find(|tab|
                            matches!(&tab.kind,TabKind::QueryEditor(q) if q.origin==Some(QueryOrigin::File{path:path.clone()}))
                        ).map(|tab|tab.id){
                            // 已打开文件只激活页签，不覆盖用户后续的未保存修改。
                            this.dispatch(AppCommand::ActivateTab(existing),cx);
                        } else {
                            let event=this.controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
                                connection_id,database,schema,
                            });
                            this.apply_app_event(&event,cx);
                            if let AppEvent::TabOpened(id)=event{
                                let key = this.query_pane_key(id);
                                this.dispatch(AppCommand::UpdateQueryText{tab_id:id,pane_id:key.pane_id,text:text.clone()},cx);
                                if let Some(editor)=this.query_editors.get(&key){
                                    editor.update(cx,|editor,cx|editor.sync_text_silent(&text,cx));
                                }
                                this.query_save_targets.insert(id,QuerySaveTarget::Local(path.clone()));
                                this.mark_query_saved(id,local_query_title(&path),QueryOrigin::File{path},cx);
                                this.show_message(tr("app.query.toast.opened_new_tab").as_ref(),AppMessageKind::Success,cx);
                            }
                        }
                    }
                    Some(Err(error))=>this.show_message(error.message,AppMessageKind::Error,cx),
                    None=>{},
                }
                cx.notify();
            });
        }));
    }
}
