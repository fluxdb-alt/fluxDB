#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveTime, TimeZone};
use fluxdb_app::{
    AppCommand, AppController, AppEvent, AppState, BackupListState, CellDetailMode,
    CellDetailPanelState, ConnectionState, CreateTableCheck, CreateTableCheckField,
    CreateTableColumn, CreateTableColumnField, CreateTableColumnFlag, CreateTableField,
    CreateTableForeignKey, CreateTableForeignKeyField, CreateTableIndex,
    CreateTableIndexColumnField, CreateTableIndexField, CreateTableMode, CreateTableOptionField,
    CreateTablePartitionField, CreateTableState, CreateTableTab, CreateTableTrigger,
    CreateTableTriggerEvent, CreateTableTriggerField, DataEditorState, ErDiagramState,
    ErLayoutResult, ForeignKeyCheckMode, LoadState, MongoCollectionState, MongoDisplayMode,
    MongoQueryDraft, MongoSection, ObjectListState, OracleGrantKind, OracleUserAdminState,
    OracleUserAdminTab, PgDraftAttrField, PgGrantEditOp, PgGrantObjectKind, PgRoleDraftUiExt,
    PgRoleSaveStatus, PgUserAdminTab, QueryEditorState, QueryHistoryEntry, QueryHistoryKind,
    QueryHistoryTransactionState, QueryOrigin, RedisAddKeyKind, RedisAddKeyRequest,
    RedisConnectionOverview, RedisListDirection, RedisWorkbenchState, SqlServerSecurityFormField,
    SqlServerSecurityFormFlag, SqlServerSecurityFormKind, SqlServerSecurityListFilter,
    SqlServerSecuritySaveStatus, SqlServerSecurityState, SqlServerUserAdminTab, TabId, TabKind,
    TabState, TabWorkspace, TableInfoState, TableInfoTab, UserAdminDetailTab, UserAdminMySqlTab,
    UserAdminState, compress_sql_text, copy_table_requires_source_ddl,
    copy_table_sql_preview_with_source_ddl, create_table_common_type_options,
    create_table_provider, create_table_special_type_options, drop_table_sql_preview,
    er_neighborhood_included_tables, er_place_avoiding_overlaps, er_relation_layout,
    format_sql_text_for_dialect, mongo_query_limit, rename_table_sql_preview,
    truncate_table_sql_preview,
};
use fluxdb_core::{
    CellValue, Column as GdbColumn, CommandBulk, CommandExecutionItem, CommandExecutionStatus,
    CommandReply, CommandWorkbenchExecution, ConnectionConfig, ConnectionDraft, ConnectionGroupId,
    ConnectionId, CreateDatabaseRequest, CreatePrincipalInput, DATA_TABLE_PAGE_SIZE_CHOICES,
    DataChangeSet, DataExportPreview, DataPage, DatabaseKind, DatabaseUserIdentity, Endpoint,
    ErGraphData, ErLoadStatus, FilterOp, FilterSpec, ForeignKeyInfo, IndexInfo, LogLevel,
    ObjectKind, ObjectPath, ObjectSummary, PgGrantTargetLists, PgObjectGrantScope, PgPasswordOp,
    PgRelationKind, PgRole, PgRoleChange, PgRoleDraft, PgRoleMembership, PgValidUntilOp,
    PostgresSslMode, QueryExecutionOptions, QueryExecutionSummary, RedisHashFieldTtl,
    RedisServerVersion, ResultsPlacement, Row, RowIdentity, SavedQuery, ScrollbarMode, Settings,
    SidebarOrderEntry, SortDirection, SortSpec, SqlDialect as _, SqlServerPrincipalKind,
    Theme as AppTheme, TriggerInfo, UiDensity, UserResourceLimits, WorkbenchHistoryItem,
    WorkbenchHistoryScope, WorkbenchHistoryStore, data_table_page_size_max,
    database_user_admin_provider, role_memberships_from_grants, sql_dialect,
    sqlite_attached_database_path, supports_database_user_admin,
};
use fluxdb_storage::{
    FileStorage, QueryHistoryRecord, RedisKeySearchHistoryRecord, RedisWorkbenchHistoryRecord,
    Storage,
};
use futures::FutureExt as _;
use gpui::prelude::*;
use gpui::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, AssetSource, Bounds, ClipboardItem,
    Context, Div, DragMoveEvent, Empty, Entity, FocusHandle, Focusable, Hsla, IntoElement,
    KeyBinding, KeyDownEvent, Keystroke, ListAlignment, ListState, Menu, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, PathPromptOptions, Pixels, Point,
    Render, Role, ScrollHandle, ScrollStrategy, SharedString, Size, Stateful,
    StatefulInteractiveElement, Subscription, Task, TitlebarOptions, Transformation,
    UniformListScrollHandle, WeakEntity, Window, WindowBounds, WindowControlArea, WindowOptions,
    actions, anchored, canvas, deferred, div, ease_in_out, hsla, img, list, opaque_grey,
    percentage, point, px, rgb, size, svg, uniform_list,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, IndexPath, Root, Sizable as _,
    Theme as ComponentTheme, ThemeConfig, ThemeMode, ThemeRegistry, VirtualListScrollHandle,
    WindowExt,
    alert::{Alert, AlertVariant},
    box_shadow,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::Dialog,
    form::{Field, field, h_form},
    group_box::{GroupBox, GroupBoxVariant, GroupBoxVariants as _},
    h_flex,
    highlighter::{LanguageConfig, LanguageRegistry},
    input::{Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState},
    popover::{Popover, PopoverState},
    progress::Progress,
    radio::Radio,
    scroll::{ScrollableElement, Scrollbar},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    sidebar::{Sidebar, SidebarGroup, SidebarMenuItem},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    tab::{Tab, TabBar},
    table::{Column as TableColumn, DataTable, TableDelegate, TableEvent, TableState},
    text::TextView,
    tooltip::Tooltip,
    v_virtual_list,
};
use gpui_fps::fps_monitor;
use lsp_types::Position;
use serde::{Deserialize, Serialize};

// 平台等宽字体仅指定通用首选项；具体 CJK/emoji 回退仍由系统字体栈处理。
#[cfg(target_os = "macos")]
pub(crate) const PLATFORM_MONOSPACE_FONT: &str = "Menlo";
#[cfg(target_os = "windows")]
pub(crate) const PLATFORM_MONOSPACE_FONT: &str = "Consolas";
#[cfg(target_os = "linux")]
pub(crate) const PLATFORM_MONOSPACE_FONT: &str = "monospace";

// First-pass source split: these files are included at crate-root scope to keep behavior unchanged.
include!("main_parts/i18n.rs");
include!("main_parts/backend_errors.rs");
include!("main_parts/backend_errors_external.rs");
include!("main_parts/backend_errors_connectors.rs");
include!("main_parts/backend_errors_storage.rs");
include!("main_parts/backend_errors_templates.rs");
include!("main_parts/foundation.rs");
include!("main_parts/theme_registry.rs");
include!("main_parts/shortcuts.rs");
include!("main_parts/editor_component.rs");
include!("main_parts/sql_editor_adapter.rs");
include!("main_parts/sql_preview.rs");
include!("main_parts/redis_editor_adapter.rs");
include!("main_parts/terminal_component.rs");
include!("main_parts/app_state.rs");
include!("main_parts/pubsub.rs");
include!("main_parts/table_delegates.rs");
include!("main_parts/data_editor_model.rs");
include!("main_parts/connection_state.rs");
include!("main_parts/navicat_main_impl.rs");
include!("main_parts/connection_dialog.rs");
include!("main_parts/connection_dialog_query_history.rs");
include!("main_parts/connection_dialog_fields.rs");
include!("main_parts/tabs_workspace.rs");
include!("main_parts/sidebar.rs");
include!("main_parts/menus_dialogs.rs");
// 「用户与权限」集中在本文件夹：公共壳（页签条、确认框、按钮）+ 四个库的专属面板。
include!("main_parts/user_admin/shared.rs");
include!("main_parts/user_admin/mysql/mod.rs");
include!("main_parts/user_admin/mysql/privileges.rs");
include!("main_parts/user_admin/sqlserver/mod.rs");
include!("main_parts/user_admin/oracle/mod.rs");
include!("main_parts/sqlserver_security_database.rs");
include!("main_parts/user_admin/postgres/mod.rs");
include!("main_parts/user_admin/postgres/general.rs");
include!("main_parts/user_admin/postgres/advanced.rs");
include!("main_parts/user_admin/postgres/membership.rs");
include!("main_parts/user_admin/postgres/privileges.rs");
include!("main_parts/user_admin/postgres/sql_preview.rs");
include!("main_parts/create_table.rs");
include!("main_parts/content_views.rs");
include!("main_parts/mongo_collection.rs");
include!("main_parts/mongo_query_completion.rs");
include!("main_parts/mongo_query_builder.rs");
include!("main_parts/mongo_documents.rs");
include!("main_parts/backup_tab.rs");
include!("main_parts/er/canvas.rs");
include!("main_parts/er/model.rs");
include!("main_parts/er/view_state.rs");
include!("main_parts/er/form.rs");
include!("main_parts/er/panel.rs");
include!("main_parts/er/loading.rs");
include!("main_parts/er/toolbar.rs");
include!("main_parts/er/scene.rs");
include!("main_parts/er/node.rs");
include!("main_parts/er/interaction.rs");
include!("main_parts/er/export.rs");
include!("main_parts/redis_detail.rs");
include!("main_parts/json_editor.rs");
include!("main_parts/json_component.rs");
include!("main_parts/cell_detail_table_info.rs");
include!("main_parts/filter_ui.rs");
include!("main_parts/data_table_ui.rs");
include!("main_parts/tree_helpers.rs");
include!("main_parts/sidebar_visible_rows.rs");
include!("main_parts/logging.rs");
include!("main_parts/single_instance.rs");
// Linux 首版不启用系统托盘（tray-icon 在 Linux 会无条件链接 GTK），托盘实现仅限 macOS/Windows。
#[cfg(not(target_os = "linux"))]
include!("main_parts/tray_icon.rs");
include!("main_parts/ai/stream.rs");
include!("main_parts/ai/state.rs");
include!("main_parts/ai/model_selection.rs");
include!("main_parts/ai/references.rs");
include!("main_parts/ai/message.rs");
include!("main_parts/ai/approval.rs");
include!("main_parts/ai/config_proposal.rs");
include!("main_parts/ai/relationship_review.rs");
include!("main_parts/ai/relationship_review_dialog.rs");
include!("main_parts/ai/timeline.rs");
include!("main_parts/ai/report.rs");
include!("main_parts/ai/diagnostics.rs");
include!("main_parts/ai/report_charts.rs");
include!("main_parts/ai/actions.rs");
include!("main_parts/ai/sql_editor_edit.rs");
include!("main_parts/ai/table_page.rs");
include!("main_parts/ai/current_page.rs");
include!("main_parts/ai/display.rs");
include!("main_parts/ai/stream_actions.rs");
include!("main_parts/ai/composer.rs");
include!("main_parts/ai/panel.rs");
include!("main_parts/app_boot.rs");
include!("main_parts/app_boot_helpers.rs");

#[cfg(test)]
include!("main_parts/i18n_tests.rs");
#[cfg(test)]
include!("main_parts/tabs_workspace_tests.rs");
#[cfg(test)]
include!("main_parts/sidebar_refresh_tests.rs");
#[cfg(test)]
include!("main_parts/mongo_sidebar_tests.rs");
#[cfg(test)]
include!("main_parts/theme_tests.rs");
