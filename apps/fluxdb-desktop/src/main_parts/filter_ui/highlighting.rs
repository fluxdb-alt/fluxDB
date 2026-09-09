fn mysql_ddl_highlights_query() -> &'static str {
    r#"
(identifier) @link_text
(literal) @link_text
(parameter) @link_text
(comment) @comment
(marginalia) @comment

[
  (keyword_add)
  (keyword_alter)
  (keyword_and)
  (keyword_auto_increment)
  (keyword_bigint)
  (keyword_binary)
  (keyword_bit)
  (keyword_boolean)
  (keyword_by)
  (keyword_char)
  (keyword_character)
  (keyword_collate)
  (keyword_comment)
  (keyword_constraint)
  (keyword_create)
  (keyword_delete)
  (keyword_desc)
  (keyword_distinct)
  (keyword_current_timestamp)
  (keyword_date)
  (keyword_datetime)
  (keyword_decimal)
  (keyword_default)
  (keyword_double)
  (keyword_drop)
  (keyword_engine)
  (keyword_enum)
  (keyword_exists)
  (keyword_float)
  (keyword_foreign)
  (keyword_from)
  (keyword_group)
  (keyword_having)
  (keyword_if)
  (keyword_in)
  (keyword_insert)
  (keyword_int)
  (keyword_into)
  (keyword_index)
  (keyword_join)
  (keyword_json)
  (keyword_key)
  (keyword_left)
  (keyword_like)
  (keyword_limit)
  (keyword_mediumint)
  (keyword_not)
  (keyword_null)
  (keyword_offset)
  (keyword_on)
  (keyword_or)
  (keyword_order)
  (keyword_primary)
  (keyword_references)
  (keyword_right)
  (keyword_select)
  (keyword_set)
  (keyword_smallint)
  (keyword_table)
  (keyword_text)
  (keyword_time)
  (keyword_timestamp)
  (keyword_tinyint)
  (keyword_unique)
  (keyword_unsigned)
  (keyword_update)
  (keyword_using)
  (keyword_values)
  (keyword_varbinary)
  (keyword_varchar)
  (keyword_where)
  (keyword_datetime2)
  (keyword_timestamptz)
  (keyword_zerofill)
] @variable.special
"#
}

fn json_highlights_query() -> &'static str {
    r#"
(pair
  key: (_) @string.special)

(string) @string
(number) @number

[
  (null)
  (true)
  (false)
] @boolean

(escape_sequence) @string.escape
(comment) @comment
"#
}

fn register_mysql_ddl_highlighter() {
    LanguageRegistry::singleton().register(
        MYSQL_DDL_HIGHLIGHT_LANGUAGE,
        &LanguageConfig::new(
            MYSQL_DDL_HIGHLIGHT_LANGUAGE,
            tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            vec![],
            mysql_ddl_highlights_query(),
            "",
            "",
        ),
    );
}

fn register_json_highlighter() {
    LanguageRegistry::singleton().register(
        JSON_HIGHLIGHT_LANGUAGE,
        &LanguageConfig::new(
            JSON_HIGHLIGHT_LANGUAGE,
            tree_sitter::Language::new(tree_sitter_json::LANGUAGE),
            vec![],
            json_highlights_query(),
            "",
            "",
        ),
    );
}

fn register_sql_highlighter() {
    LanguageRegistry::singleton().register(
        SQL_HIGHLIGHT_LANGUAGE,
        &LanguageConfig::new(
            SQL_HIGHLIGHT_LANGUAGE,
            tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            vec![],
            mysql_ddl_highlights_query(),
            "",
            "",
        ),
    );
}
