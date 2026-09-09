const PLAINTEXT_PASSWORD_OPTION: &str = "password";
const URL_PARAMS_OPTION: &str = "url_params";

fn is_mysql_protocol_kind(kind: DatabaseKind) -> bool {
    matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb)
}
