use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// First-pass source split: included files remain in crate-root scope while domain modules are refined.
include!("parts/error.rs");
include!("parts/connection.rs");
include!("parts/redis_profile.rs");
include!("parts/mysql_profile.rs");
include!("parts/object_query.rs");
include!("parts/settings_filter.rs");
include!("parts/data_page.rs");
include!("parts/user_admin.rs");
include!("parts/changes.rs");
include!("parts/connector.rs");
include!("parts/command_workbench.rs");
include!("parts/workbench_history.rs");
include!("parts/terminal.rs");
include!("parts/tests.rs");
