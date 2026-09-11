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
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveTime, TimeZone};
use fluxdb_app::{
    AppCommand, AppController, AppEvent, AppState, BackupListState, CellDetailMode,
    CellDetailPanelState, ConnectionState, CreateTableCheck, CreateTableCheckField,
    CreateTableColumn, CreateTableColumnField, CreateTableColumnFlag, CreateTableField,
    CreateTableForeignKey, CreateTableForeignKeyField, CreateTableIndex,
    CreateTableIndexColumnField, CreateTableIndexField, CreateTableOptionField,
    CreateTablePartitionField, CreateTableState, CreateTableTab, CreateTableTrigger,
    CreateTableTriggerEvent, CreateTableTriggerField, DataEditorState, ForeignKeyCheckMode,
    LoadState, ObjectListState, QueryEditorState, QueryHistoryEntry, QueryHistoryKind, QueryOrigin,
    RedisAddKeyKind, RedisAddKeyRequest, RedisConnectionOverview, RedisListDirection,
    RedisWorkbenchState, TabId, TabKind, TabState, TabWorkspace, TableInfoState, TableInfoTab,
    UserAdminDetailTab, UserAdminState, compress_sql_text, copy_table_sql_preview_with_source_ddl,
    create_table_provider, drop_table_sql_preview, format_sql_text_for_dialect,
    rename_table_sql_preview, truncate_table_sql_preview,
};
use fluxdb_core::{
    BinaryUpdatePayload, CellValue, Column as GdbColumn, CommandBulk, CommandExecutionItem,
    CommandExecutionStatus, CommandReply, CommandWorkbenchExecution, ConnectionConfig,
    ConnectionDraft, ConnectionGroupId, ConnectionId, CreateDatabaseRequest, CreatePrincipalInput,
    DATA_TABLE_PAGE_SIZE_CHOICES, DataChangeSet, DataExportPreview, DataPage, DatabaseKind,
    DatabaseUserIdentity, Endpoint, FilterOp, FilterSpec, ForeignKeyInfo, IndexInfo, LogLevel,
    ObjectKind, ObjectPath, ObjectSummary, QueryExecutionOptions, QueryExecutionSummary,
    RedisHashFieldTtl, RedisServerVersion, ResultsPlacement, Row, RowIdentity, SavedQuery,
    ScrollbarMode, Settings, SidebarOrderEntry, SortDirection, SortSpec, Theme as AppTheme,
    TriggerInfo, UiDensity, UserResourceLimits, WorkbenchHistoryItem, WorkbenchHistoryScope,
    WorkbenchHistoryStore, data_table_page_size_max, database_user_admin_provider,
    role_memberships_from_grants, sqlite_attached_database_path, supports_database_user_admin,
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
    KeyBinding, KeyDownEvent, Keystroke, Menu, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, NoAction, PathPromptOptions, Pixels, Point, Render, ScrollHandle, ScrollStrategy,
    SharedString, Size, Stateful, StatefulInteractiveElement, Subscription, Task, TitlebarOptions,
    Transformation, UniformListScrollHandle, WeakEntity, Window, WindowBounds, WindowOptions,
    actions, anchored, canvas, deferred, div, ease_in_out, hsla, img, opaque_grey, percentage,
    point, px, rgb, size, svg, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, IndexPath, Root, Sizable as _,
    Theme as ComponentTheme, ThemeConfig, ThemeMode, ThemeRegistry, VirtualListScrollHandle,
    WindowExt,
    alert::{Alert, AlertVariant},
    box_shadow,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::Dialog,
    form::{Field, field, h_form},
    group_box::{GroupBox, GroupBoxVariant, GroupBoxVariants as _},
    h_flex,
    highlighter::{LanguageConfig, LanguageRegistry},
    input::{Input, InputEvent, InputState},
    popover::{Popover, PopoverState},
    progress::Progress,
    scroll::{ScrollableElement, Scrollbar},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    tab::{Tab, TabBar},
    table::{Column as TableColumn, DataTable, TableDelegate, TableEvent, TableState},
    tooltip::Tooltip,
    v_virtual_list,
};
use gpui_fps::fps_monitor;
use lsp_types::Position;
use serde::{Deserialize, Serialize};

// gpui-component 0.6 fixes input mode at the state type level. Keep the old
// builder calls source-compatible while the existing state fields migrate.
trait LegacyInputStateExt {
    fn code_editor(self, _: impl Into<SharedString>) -> Self;
    fn multi_line(self, _: bool) -> Self;
    fn line_number(self, _: bool) -> Self;
    fn rows(self, _: usize) -> Self;
    fn legacy_soft_wrap(self, _: bool) -> Self;
}

impl LegacyInputStateExt for InputState {
    fn code_editor(self, _: impl Into<SharedString>) -> Self {
        self
    }

    fn multi_line(self, _: bool) -> Self {
        self
    }

    fn line_number(self, _: bool) -> Self {
        self
    }

    fn rows(self, _: usize) -> Self {
        self
    }

    fn legacy_soft_wrap(self, _: bool) -> Self {
        self
    }
}

// First-pass source split: these files are included at crate-root scope to keep behavior unchanged.
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
include!("main_parts/tabs_workspace.rs");
include!("main_parts/sidebar.rs");
include!("main_parts/menus_dialogs.rs");
include!("main_parts/user_admin.rs");
include!("main_parts/user_admin_privileges.rs");
include!("main_parts/create_table.rs");
include!("main_parts/content_views.rs");
include!("main_parts/backup_tab.rs");
include!("main_parts/redis_detail.rs");
include!("main_parts/json_editor.rs");
include!("main_parts/json_component.rs");
include!("main_parts/cell_detail_table_info.rs");
include!("main_parts/filter_ui.rs");
include!("main_parts/data_table_ui.rs");
include!("main_parts/tree_helpers.rs");
include!("main_parts/logging.rs");
include!("main_parts/tray_icon.rs");
include!("main_parts/app_boot.rs");
include!("main_parts/tests.rs");
