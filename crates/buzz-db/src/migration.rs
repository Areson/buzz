//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in SQL files under `migrations/`.
//! Existing pre-SQLx deployments are baselined when core Buzz tables already
//! exist but `_sqlx_migrations` does not, so startup will not try to replay the
//! initial schema over a live database.

use sqlx::PgPool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[cfg(test)]
static SCHEMA_SQL: &str = include_str!("../../../schema/schema.sql");

const BASELINE_MIGRATION_VERSIONS: &[i64] = &[1, 2];

/// Run all pending Buzz database migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    baseline_existing_database(pool).await?;
    MIGRATOR.run(pool).await?;
    Ok(())
}

async fn baseline_existing_database(pool: &PgPool) -> Result<()> {
    if migrations_table_exists(pool).await? || !pre_sqlx_schema_exists(pool).await? {
        return Ok(());
    }

    ensure_migrations_table(pool).await?;

    for version in BASELINE_MIGRATION_VERSIONS {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == *version)
            .expect("baseline migration version must exist in embedded migrator");

        sqlx::query(
            r#"
            INSERT INTO _sqlx_migrations
                (version, description, success, checksum, execution_time)
            VALUES ($1, $2, TRUE, $3, 0)
            ON CONFLICT (version) DO NOTHING
            "#,
        )
        .bind(migration.version)
        .bind(&*migration.description)
        .bind(&*migration.checksum)
        .execute(pool)
        .await?;
    }

    tracing::info!(
        versions = ?BASELINE_MIGRATION_VERSIONS,
        "Baselined existing Buzz database for SQLx migrations"
    );

    Ok(())
}

async fn migrations_table_exists(pool: &PgPool) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = '_sqlx_migrations'
        )
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

async fn pre_sqlx_schema_exists(pool: &PgPool) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = 'events'
        ) AND EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = 'channels'
        )
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

async fn ensure_migrations_table(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
            success BOOLEAN NOT NULL,
            checksum BYTEA NOT NULL,
            execution_time BIGINT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    #[test]
    fn embedded_migrator_contains_all_schema_migrations() {
        let migrations: Vec<_> = MIGRATOR.iter().collect();

        assert_eq!(migrations.len(), 3);
        assert_eq!(migrations[0].version, 1);
        assert_eq!(&*migrations[0].description, "initial schema");
        assert!(
            migrations[0].sql.as_str().contains("CREATE TABLE channels"),
            "initial schema migration should include Buzz core tables"
        );
        assert!(
            migrations[0]
                .sql
                .as_str()
                .contains("CREATE TABLE IF NOT EXISTS relay_members"),
            "initial schema migration should include relay_members"
        );

        assert_eq!(migrations[1].version, 2);
        assert_eq!(&*migrations[1].description, "backfill d tag");
        assert!(
            migrations[1].sql.as_str().contains("UPDATE events"),
            "second migration should backfill existing event rows"
        );

        assert_eq!(migrations[2].version, 3);
        assert_eq!(&*migrations[2].description, "event reminders");
        assert!(
            migrations[2]
                .sql
                .as_str()
                .contains("ADD COLUMN not_before BIGINT")
                && migrations[2].sql.as_str().contains("idx_events_not_before"),
            "third migration should add the NIP-ER reminder columns and index"
        );
    }

    async fn connect_test_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn reset_public_schema(pool: &PgPool) {
        sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
            .execute(pool)
            .await
            .expect("drop public schema");
        sqlx::query("CREATE SCHEMA IF NOT EXISTS public")
            .execute(pool)
            .await
            .expect("create public schema");
    }

    async fn applied_versions(pool: &PgPool) -> Vec<i64> {
        sqlx::query_scalar::<_, i64>(
            "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .expect("read applied migrations")
    }

    /// Returns `schema/schema.sql` with the NIP-ER reminder DDL removed, so it
    /// models a pre-stack deployment whose `events` table lacks the reminder
    /// columns and index. The strip is asserted: if the snapshot text drifts so
    /// these fragments no longer match, the test fails loudly rather than
    /// silently loading a snapshot that already carries the reminder columns
    /// (which would make migration 0003 collide on re-add).
    fn pre_reminder_schema_snapshot() -> String {
        const REMINDER_COLUMNS: &str = "    not_before  BIGINT,\n    delivered_at BIGINT,\n";
        const REMINDER_INDEX: &str = "CREATE INDEX idx_events_not_before ON events (not_before)\n    WHERE not_before IS NOT NULL AND deleted_at IS NULL AND delivered_at IS NULL;\n";

        assert!(
            SCHEMA_SQL.contains(REMINDER_COLUMNS) && SCHEMA_SQL.contains(REMINDER_INDEX),
            "schema.sql reminder DDL drifted; update pre_reminder_schema_snapshot to match"
        );

        SCHEMA_SQL
            .replace(REMINDER_COLUMNS, "")
            .replace(REMINDER_INDEX, "")
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ConstraintKind {
        ForeignKey,
        PrimaryKey,
        Unique,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ConstraintLint {
        table: String,
        kind: ConstraintKind,
        description: String,
        columns: Vec<String>,
    }

    fn all_migration_sql() -> String {
        MIGRATOR
            .iter()
            .map(|migration| migration.sql.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn strip_sql_comments(sql: &str) -> String {
        sql.lines()
            .map(|line| line.split_once("--").map_or(line, |(before, _)| before))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn normalize_sql(sql: &str) -> String {
        strip_sql_comments(sql)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    fn split_sql_statements(sql: &str) -> Vec<String> {
        strip_sql_comments(sql)
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    fn find_matching_paren(sql: &str, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (offset, byte) in sql.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(open + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn split_top_level_csv(input: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut depth = 0usize;
        for (idx, byte) in input.bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    parts.push(input[start..idx].trim().to_owned());
                    start = idx + 1;
                }
                _ => {}
            }
        }
        let tail = input[start..].trim();
        if !tail.is_empty() {
            parts.push(tail.to_owned());
        }
        parts
    }

    fn identifier_after_keyword(statement: &str, keyword: &str) -> Option<String> {
        let lower = statement.to_ascii_lowercase();
        let keyword_pos = lower.find(keyword)?;
        let mut remainder = statement[keyword_pos + keyword.len()..].trim_start();
        for prefix in ["if not exists", "if exists", "only"] {
            if remainder.to_ascii_lowercase().starts_with(prefix) {
                remainder = remainder[prefix.len()..].trim_start();
            }
        }

        let identifier = remainder
            .split(|ch: char| ch.is_whitespace() || ch == '(')
            .next()?
            .trim_matches('"')
            .rsplit('.')
            .next()?
            .trim_matches('"')
            .to_ascii_lowercase();
        (!identifier.is_empty()).then_some(identifier)
    }

    fn first_parenthesized_columns(input: &str) -> Vec<String> {
        let Some(open) = input.find('(') else {
            return Vec::new();
        };
        let Some(close) = find_matching_paren(input, open) else {
            return Vec::new();
        };

        split_top_level_csv(&input[open + 1..close])
            .into_iter()
            .filter_map(|column| {
                let name = column
                    .trim()
                    .trim_matches('"')
                    .split_whitespace()
                    .next()?
                    .trim_matches('"')
                    .to_ascii_lowercase();
                (!name.is_empty()).then_some(name)
            })
            .collect()
    }

    fn column_definition_name(definition: &str) -> Option<String> {
        let trimmed = definition.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("constraint ")
            || lower.starts_with("primary key")
            || lower.starts_with("foreign key")
            || lower.starts_with("unique")
            || lower.starts_with("check ")
            || lower.starts_with("exclude ")
        {
            return None;
        }

        let name = trimmed
            .split_whitespace()
            .next()?
            .trim_matches('"')
            .to_ascii_lowercase();
        (!name.is_empty()).then_some(name)
    }

    fn create_table_body(statement: &str) -> Option<(String, Vec<String>)> {
        let table = identifier_after_keyword(statement, "create table")?;
        let open = statement.find('(')?;
        let close = find_matching_paren(statement, open)?;
        Some((table, split_top_level_csv(&statement[open + 1..close])))
    }

    fn create_table_definitions(sql: &str) -> Vec<(String, Vec<String>)> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                statement
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("create table")
                    .then(|| create_table_body(&statement))?
            })
            .collect()
    }

    fn scoped_tables(sql: &str) -> Vec<String> {
        let mut tables = create_table_definitions(sql)
            .into_iter()
            .filter_map(|(table, definitions)| {
                definitions
                    .iter()
                    .any(|definition| {
                        column_definition_name(definition).as_deref() == Some("community_id")
                    })
                    .then_some(table)
            })
            .collect::<Vec<_>>();

        for statement in split_sql_statements(sql) {
            let normalized = normalize_sql(&statement);
            if !normalized.starts_with("alter table")
                || !(normalized.contains("add column community_id")
                    || normalized.contains("add column if not exists community_id"))
            {
                continue;
            }
            if let Some(table) = identifier_after_keyword(&statement, "alter table") {
                tables.push(table);
            }
        }

        tables.sort();
        tables.dedup();
        tables
    }

    fn constraint_lint_for_definition(table: &str, definition: &str) -> Option<ConstraintLint> {
        let normalized = normalize_sql(definition);
        let definition_without_name = if normalized.starts_with("constraint ") {
            let after_constraint = definition
                .trim_start()
                .splitn(3, char::is_whitespace)
                .nth(2)
                .unwrap_or("");
            normalize_sql(after_constraint)
        } else {
            normalized.clone()
        };

        if definition_without_name.starts_with("primary key") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::PrimaryKey,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if definition_without_name.starts_with("unique") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::Unique,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if definition_without_name.starts_with("foreign key") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::ForeignKey,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if normalized.contains(" primary key") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::PrimaryKey,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else if normalized.contains(" references ") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::ForeignKey,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else if normalized.contains(" unique") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::Unique,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else {
            None
        }
    }

    fn table_constraints(sql: &str, scoped_tables: &[String]) -> Vec<ConstraintLint> {
        create_table_definitions(sql)
            .into_iter()
            .filter(|(table, _)| scoped_tables.contains(table))
            .flat_map(|(table, definitions)| {
                definitions.into_iter().filter_map(move |definition| {
                    constraint_lint_for_definition(&table, &definition)
                })
            })
            .collect()
    }

    fn alter_table_constraints(sql: &str, scoped_tables: &[String]) -> Vec<ConstraintLint> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                let normalized = normalize_sql(&statement);
                if !normalized.starts_with("alter table") {
                    return None;
                }

                let table = identifier_after_keyword(&statement, "alter table")?;
                if !scoped_tables.contains(&table) {
                    return None;
                }

                let add_pos = normalized.find(" add ")?;
                let definition = normalized[add_pos + " add ".len()..].trim();
                constraint_lint_for_definition(&table, definition)
            })
            .collect()
    }

    fn unique_indexes(sql: &str, scoped_tables: &[String]) -> Vec<ConstraintLint> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                let normalized = normalize_sql(&statement);
                if !normalized.starts_with("create unique index") {
                    return None;
                }

                let lower_statement = statement.to_ascii_lowercase();
                let on_pos = lower_statement.find(" on ")?;
                let table = statement[on_pos + " on ".len()..]
                    .trim_start()
                    .split(|ch: char| ch.is_whitespace() || ch == '(')
                    .next()?
                    .trim_matches('"')
                    .rsplit('.')
                    .next()?
                    .trim_matches('"')
                    .to_ascii_lowercase();

                if !scoped_tables.contains(&table) {
                    return None;
                }

                let columns = first_parenthesized_columns(&statement[on_pos + " on ".len()..]);
                Some(ConstraintLint {
                    table,
                    kind: ConstraintKind::Unique,
                    description: statement,
                    columns,
                })
            })
            .collect()
    }

    fn scoped_constraint_lints(sql: &str, scoped_tables: &[String]) -> Vec<ConstraintLint> {
        let mut constraints = table_constraints(sql, scoped_tables);
        constraints.extend(alter_table_constraints(sql, scoped_tables));
        constraints.extend(unique_indexes(sql, scoped_tables));
        constraints
    }

    fn channels_has_community_id(sql: &str) -> bool {
        scoped_tables(sql).iter().any(|table| table == "channels")
    }

    fn has_channels_community_id_immutability_guard(sql: &str) -> bool {
        let normalized = normalize_sql(sql);
        normalized.contains("create trigger")
            && normalized.contains("before update")
            && normalized.contains(" on channels")
            && normalized.contains("community_id")
            && normalized.contains("old.community_id")
            && normalized.contains("new.community_id")
            && normalized.contains("raise exception")
    }

    fn forbidden_channels_community_id_mutations(sql: &str) -> Vec<String> {
        split_sql_statements(sql)
            .into_iter()
            .filter(|statement| {
                let normalized = normalize_sql(statement);
                let updates_channels =
                    identifier_after_keyword(statement, "update").as_deref() == Some("channels");
                let mutates_with_update = updates_channels
                    && normalized.contains(" set ")
                    && normalized.contains("community_id");
                let alters_channels = identifier_after_keyword(statement, "alter table").as_deref()
                    == Some("channels");
                let drops_channels = identifier_after_keyword(statement, "drop table").as_deref()
                    == Some("channels");
                let drops_or_rewrites_column = alters_channels
                    && (normalized.contains("drop column community_id")
                        || normalized.contains("alter column community_id")
                        || normalized.contains("rename column community_id")
                        || normalized.contains("rename community_id")
                        || normalized.contains("drop trigger")
                        || normalized.contains("disable trigger"));
                let drops_table = drops_channels;

                mutates_with_update || drops_or_rewrites_column || drops_table
            })
            .collect()
    }

    fn scoped_constraint_violations(sql: &str) -> Vec<ConstraintLint> {
        let scoped_tables = scoped_tables(sql);
        scoped_constraint_lints(sql, &scoped_tables)
            .into_iter()
            .filter(|constraint| {
                !constraint
                    .columns
                    .iter()
                    .any(|column| column == "community_id")
            })
            .collect()
    }

    #[test]
    fn migration_lint_detects_scoped_key_constraints_missing_community_id() {
        let sql = r#"
            CREATE TABLE widgets (
                community_id UUID NOT NULL,
                id UUID PRIMARY KEY,
                channel_id UUID REFERENCES channels(id),
                slug TEXT,
                CONSTRAINT widgets_name_unique UNIQUE (slug),
                CONSTRAINT widgets_parent_fk FOREIGN KEY (channel_id) REFERENCES channels(id)
            );
            CREATE UNIQUE INDEX idx_widgets_slug ON widgets (slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_slug_unique UNIQUE (slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_parent_fk FOREIGN KEY (channel_id) REFERENCES channels(id);
        "#;

        let violations = scoped_constraint_violations(sql);

        assert!(violations
            .iter()
            .any(|violation| violation.kind == ConstraintKind::PrimaryKey));
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation.kind == ConstraintKind::ForeignKey)
                .count(),
            3
        );
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation.kind == ConstraintKind::Unique)
                .count(),
            3
        );
    }

    #[test]
    fn migration_lint_accepts_scoped_key_constraints_with_community_id() {
        let sql = r#"
            CREATE TABLE widgets (
                community_id UUID NOT NULL,
                id UUID NOT NULL,
                channel_id UUID NOT NULL,
                slug TEXT NOT NULL,
                PRIMARY KEY (community_id, id),
                UNIQUE (community_id, slug),
                FOREIGN KEY (community_id, channel_id) REFERENCES channels(community_id, id)
            );
            CREATE UNIQUE INDEX idx_widgets_slug ON widgets (community_id, slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_slug_unique UNIQUE (community_id, slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_parent_fk FOREIGN KEY (community_id, channel_id) REFERENCES channels(community_id, id);
        "#;

        assert!(scoped_constraint_violations(sql).is_empty());
    }

    #[test]
    fn migration_lint_requires_channels_community_id_update_guard() {
        let guarded = r#"
            CREATE TABLE channels (
                community_id UUID NOT NULL,
                id UUID NOT NULL,
                PRIMARY KEY (community_id, id)
            );
            CREATE FUNCTION prevent_channels_community_id_update() RETURNS trigger AS $$
            BEGIN
                IF OLD.community_id <> NEW.community_id THEN
                    RAISE EXCEPTION 'channels.community_id is immutable';
                END IF;
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql;
            CREATE TRIGGER channels_community_id_immutable
                BEFORE UPDATE ON channels
                FOR EACH ROW EXECUTE FUNCTION prevent_channels_community_id_update();
        "#;
        let unguarded = r#"
            CREATE TABLE channels (
                community_id UUID NOT NULL,
                id UUID NOT NULL,
                PRIMARY KEY (community_id, id)
            );
        "#;
        let forbidden = r#"
            UPDATE channels SET community_id = gen_random_uuid();
            ALTER TABLE channels DROP COLUMN community_id;
            DROP TABLE IF EXISTS channels;
        "#;

        assert!(channels_has_community_id(guarded));
        assert!(has_channels_community_id_immutability_guard(guarded));
        assert!(channels_has_community_id(unguarded));
        assert!(!has_channels_community_id_immutability_guard(unguarded));
        assert_eq!(
            forbidden_channels_community_id_mutations(forbidden).len(),
            3
        );
    }

    #[test]
    fn scoped_table_primary_key_unique_and_foreign_key_constraints_include_community_id() {
        // docs/multi-tenant-conformance.md §Migration gates and
        // docs/multi-tenant-relay.md C2.1 require every tenant-scoped key that
        // can otherwise become a cross-community existence oracle to carry
        // `community_id`.  The current main schema is still single-tenant, so
        // scope is detected structurally: a table becomes tenant-scoped in this
        // lint as soon as a migration gives it a `community_id` column.
        let sql = all_migration_sql();
        let violations = scoped_constraint_violations(&sql)
            .into_iter()
            .map(|constraint| {
                format!(
                    "{}. {:?} constraint missing community_id: {}",
                    constraint.table, constraint.kind, constraint.description
                )
            })
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "tenant-scoped tables are detected by a community_id column; every primary key, unique/FK constraint, or unique index on those tables must include community_id:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn channels_community_id_is_immutable_after_insert() {
        // docs/multi-tenant-relay.md P-RESOLVE/S2 makes channel tenancy a
        // load-bearing constant after resolution.  The lint is intentionally
        // green before `channels.community_id` exists, then requires a trigger
        // guard and rejects re-tenanting migration statements once it does.
        let sql = all_migration_sql();
        let forbidden_mutations = forbidden_channels_community_id_mutations(&sql);

        assert!(
            forbidden_mutations.is_empty(),
            "channels.community_id must not be re-tenanted after insert; forbidden migration statements:\n{}",
            forbidden_mutations.join("\n---\n")
        );

        if channels_has_community_id(&sql) {
            assert!(
                has_channels_community_id_immutability_guard(&sql),
                "migrations define channels.community_id but no BEFORE UPDATE trigger/function guard that rejects OLD.community_id <> NEW.community_id was found"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_migrations_applies_embedded_versions_on_fresh_database() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        run_migrations(&pool).await.expect("run migrations");

        assert_eq!(applied_versions(&pool).await, vec![1, 2, 3]);
        let events_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'events')",
        )
        .fetch_one(&pool)
        .await
        .expect("check events table");
        assert!(events_exists);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_migrations_baselines_existing_schema_and_preserves_allowlist_backfill_path() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        // Load a pre-stack snapshot (without the NIP-ER reminder DDL) so the
        // events table matches a real pre-SQLx deployment, which never had the
        // reminder columns. Migration 0003 must then add them — proving the
        // genuine prod-upgrade path, not a snapshot that already carries them.
        sqlx::raw_sql(sqlx::AssertSqlSafe(pre_reminder_schema_snapshot()))
            .execute(&pool)
            .await
            .expect("load pre-SQLx schema snapshot");
        sqlx::query(
            "INSERT INTO pubkey_allowlist (pubkey, added_at) VALUES (decode($1, 'hex'), now())",
        )
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .execute(&pool)
        .await
        .expect("seed legacy allowlist row");

        run_migrations(&pool).await.expect("baseline migrations");

        assert_eq!(applied_versions(&pool).await, vec![1, 2, 3]);
        let allowlist_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pubkey_allowlist")
            .fetch_one(&pool)
            .await
            .expect("count allowlist rows");
        assert_eq!(
            allowlist_count, 1,
            "baseline must not drop legacy allowlist rows before relay startup backfills them"
        );

        let inserted = crate::relay_members::backfill_from_allowlist(&pool)
            .await
            .expect("backfill legacy allowlist rows");
        assert_eq!(inserted, 1);
        let relay_member_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM relay_members WHERE pubkey = $1 AND role = 'member'",
        )
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .fetch_one(&pool)
        .await
        .expect("count backfilled relay member");
        assert_eq!(relay_member_count, 1);
    }
}
