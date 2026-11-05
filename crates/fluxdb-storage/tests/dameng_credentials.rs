use fluxdb_core::{
    ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
    DamengConnectionProfile, DatabaseKind, Endpoint, SecretRef,
};
use fluxdb_storage::{FileStorage, Storage as _};
use std::collections::BTreeMap;

#[test]
fn dameng_profile_password_is_encrypted_and_restored() {
    let root = std::env::temp_dir().join(format!(
        "fluxdb-dameng-credentials-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let storage = FileStorage::new(&root);
    let password = "dm-e2e-secret";
    let config = ConnectionConfig {
        id: ConnectionId(7_001),
        name: "DM8 credentials".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 5236,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: "127.0.0.1".into(),
                port: 5236,
                database: "DAMENG".into(),
                schema: "SYSDBA".into(),
                username: "SYSDBA".into(),
                password: SecretRef::inline(password),
            },
            advanced: DamengAdvancedOptions::default(),
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };

    storage
        .save_connections(&[config])
        .expect("save DM8 connection");
    let sqlite = std::fs::read(root.join("fluxdb.sqlite")).expect("read connection database");
    assert!(
        !sqlite
            .windows(password.len())
            .any(|window| window == password.as_bytes()),
        "plaintext DM8 password must not be written to SQLite"
    );

    let loaded = storage.load_connections().expect("load DM8 connection");
    let profile = loaded
        .first()
        .and_then(|connection| connection.dameng_profile.as_ref())
        .expect("DM8 profile");
    assert_eq!(profile.password(), Some(password));
    assert_eq!(profile.basic.schema, "SYSDBA");

    std::fs::remove_dir_all(root).expect("remove test profile");
}
