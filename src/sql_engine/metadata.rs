//! MetadataIndex — central indexed store for database metadata.
//!
//! Replaces scattered tree walking in completion.rs and diagnostics.rs.
//! All methods are synchronous — the engine never does I/O.
//! Populated from AppState's sidebar tree and column_cache via the UI layer.

use std::collections::HashMap;

use crate::core::models::DatabaseType;
use crate::sql_engine::models::{ForeignKey, ResolvedColumn};

/// The kind of a database object in the metadata index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Procedure,
    Function,
    Package,
    Sequence,
    Index,
    Type,
    Trigger,
    Event,
}

/// Entry for a single database object in the index.
#[derive(Debug, Clone)]
pub struct ObjectEntry {
    /// Original-case name for display in completion popup.
    pub display_name: String,
    /// Original-case schema name for display.
    pub schema_display: String,
    /// What kind of object this is.
    pub kind: ObjectKind,
}

/// A callable member of a PL/SQL package (function or procedure). Cached so
/// completion can suggest `pkg.foo()` from anywhere in the editor.
#[derive(Debug, Clone)]
pub struct PackageMember {
    pub name: String,
    pub kind: PackageMemberKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageMemberKind {
    Function,
    Procedure,
}

/// Qualified key for object lookup. Both fields stored UPPERCASE for
/// case-insensitive matching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ObjectKey {
    schema: String,
    name: String,
}

/// Central metadata index. Populated incrementally as the sidebar tree loads.
///
/// All query methods are O(n) scans or HashMap lookups — fast enough for
/// interactive completion (typically <1000 objects per schema).
#[derive(Debug, Clone, Default)]
pub struct MetadataIndex {
    /// Known schemas: UPPERCASE key → display name.
    schemas: HashMap<String, String>,

    /// All known objects: (SCHEMA, NAME) → entry.
    objects: HashMap<ObjectKey, ObjectEntry>,

    /// Column cache: "SCHEMA.TABLE" (uppercase) → columns.
    columns: HashMap<String, Vec<ResolvedColumn>>,

    /// Package members: (SCHEMA, PACKAGE) uppercase → list of callable members.
    /// Populated on demand when the user opens a package — kept here so the
    /// completion engine can suggest `pkg.foo()` even from another tab.
    package_members: HashMap<(String, String), Vec<PackageMember>>,

    /// Pseudo-columns returned by a PL/SQL function used inside `TABLE(...)`.
    /// Key: (SCHEMA, PACKAGE, FUNCTION) uppercase — each component is empty
    /// string if the caller did not qualify it (e.g. top-level function uses
    /// ("", "", "FN")). Populated on demand from the Oracle adapter.
    function_return_columns: HashMap<(String, String, String), Vec<ResolvedColumn>>,

    /// Foreign key relationships (populated on demand).
    foreign_keys: Vec<ForeignKey>,

    /// Active database type.
    db_type: Option<DatabaseType>,

    /// The current/default schema for unqualified references.
    current_schema: Option<String>,
}

impl MetadataIndex {
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------------
    // Mutation methods — called by the UI layer to populate the index
    // -----------------------------------------------------------------------

    pub fn set_db_type(&mut self, db_type: DatabaseType) {
        self.db_type = Some(db_type);
    }

    #[allow(dead_code)]
    pub fn db_type(&self) -> Option<DatabaseType> {
        self.db_type
    }

    pub fn set_current_schema(&mut self, schema: &str) {
        self.current_schema = Some(schema.to_string());
    }

    pub fn current_schema(&self) -> Option<&str> {
        self.current_schema.as_deref()
    }

    pub fn add_schema(&mut self, name: &str) {
        self.schemas.insert(name.to_uppercase(), name.to_string());
    }

    pub fn add_object(&mut self, schema: &str, name: &str, kind: ObjectKind) {
        let key = ObjectKey {
            schema: schema.to_uppercase(),
            name: name.to_uppercase(),
        };
        self.objects.insert(
            key,
            ObjectEntry {
                display_name: name.to_string(),
                schema_display: schema.to_string(),
                kind,
            },
        );
    }

    pub fn cache_columns(&mut self, schema: &str, table: &str, columns: Vec<ResolvedColumn>) {
        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
        self.columns.insert(key, columns);
    }

    #[allow(dead_code)]
    pub fn add_foreign_key(&mut self, fk: ForeignKey) {
        self.foreign_keys.push(fk);
    }

    /// Clear all data (e.g., on connection change).
    pub fn clear(&mut self) {
        self.schemas.clear();
        self.objects.clear();
        self.columns.clear();
        self.package_members.clear();
        self.function_return_columns.clear();
        self.foreign_keys.clear();
        self.db_type = None;
        self.current_schema = None;
    }

    // -----------------------------------------------------------------------
    // Function return-type cache (for `TABLE(pkg.fn()) tb` completion)
    // -----------------------------------------------------------------------

    fn function_key(
        schema: Option<&str>,
        package: Option<&str>,
        function: &str,
    ) -> (String, String, String) {
        (
            schema.map(|s| s.to_uppercase()).unwrap_or_default(),
            package.map(|s| s.to_uppercase()).unwrap_or_default(),
            function.to_uppercase(),
        )
    }

    /// Cache the pseudo-columns of the type returned by a PL/SQL function.
    pub fn cache_function_return_columns(
        &mut self,
        schema: Option<&str>,
        package: Option<&str>,
        function: &str,
        columns: Vec<ResolvedColumn>,
    ) {
        self.function_return_columns
            .insert(Self::function_key(schema, package, function), columns);
    }

    /// Look up the cached return-type columns of a function.
    pub fn get_function_return_columns(
        &self,
        schema: Option<&str>,
        package: Option<&str>,
        function: &str,
    ) -> Option<&[ResolvedColumn]> {
        self.function_return_columns
            .get(&Self::function_key(schema, package, function))
            .map(|v| v.as_slice())
    }

    /// Whether the return columns for the given function have been loaded.
    pub fn has_function_return_columns_cached(
        &self,
        schema: Option<&str>,
        package: Option<&str>,
        function: &str,
    ) -> bool {
        self.function_return_columns
            .contains_key(&Self::function_key(schema, package, function))
    }

    // -----------------------------------------------------------------------
    // Query methods — called by the engine (analyzer, completion, diagnostics)
    // -----------------------------------------------------------------------

    /// Check if a name matches a known schema (case-insensitive).
    pub fn is_known_schema(&self, name: &str) -> bool {
        self.schemas.contains_key(&name.to_uppercase())
    }

    /// Check if any objects have been loaded for a schema.
    pub fn has_objects_loaded(&self, schema: &str) -> bool {
        let upper = schema.to_uppercase();
        self.objects.keys().any(|k| k.schema == upper)
    }

    /// Check if an object exists, optionally within a specific schema.
    #[allow(dead_code)]
    pub fn is_known_object(&self, schema: Option<&str>, name: &str) -> bool {
        let upper_name = name.to_uppercase();
        if let Some(s) = schema {
            self.objects.contains_key(&ObjectKey {
                schema: s.to_uppercase(),
                name: upper_name,
            })
        } else {
            self.objects.keys().any(|k| k.name == upper_name)
        }
    }

    /// Check if a package with the given name exists in any (or a specific)
    /// schema. Used by the analyzer to detect `pkg.<cursor>` and
    /// `schema.pkg.<cursor>` completion contexts.
    pub fn has_package(&self, schema: Option<&str>, package_name: &str) -> bool {
        let upper = package_name.to_uppercase();
        match schema {
            Some(s) => {
                let s_upper = s.to_uppercase();
                self.objects.iter().any(|(k, entry)| {
                    k.schema == s_upper && k.name == upper && entry.kind == ObjectKind::Package
                })
            }
            None => self
                .objects
                .iter()
                .any(|(k, entry)| k.name == upper && entry.kind == ObjectKind::Package),
        }
    }

    /// Replace the cached members of a package. Called when the user opens
    /// a package — extract_names() in app/messages.rs gives us the lists.
    pub fn set_package_members(
        &mut self,
        schema: &str,
        package: &str,
        members: Vec<PackageMember>,
    ) {
        let key = (schema.to_uppercase(), package.to_uppercase());
        self.package_members.insert(key, members);
    }

    /// Get the cached callable members of a package. Returns an empty slice
    /// if the package hasn't been loaded yet.
    pub fn package_members(&self, schema: &str, package: &str) -> &[PackageMember] {
        let key = (schema.to_uppercase(), package.to_uppercase());
        self.package_members
            .get(&key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Resolve the schema that owns a package by name (any schema). Returns
    /// the first match.
    pub fn schema_for_package(&self, package_name: &str) -> Option<&str> {
        let upper = package_name.to_uppercase();
        self.objects
            .iter()
            .find(|(k, entry)| k.name == upper && entry.kind == ObjectKind::Package)
            .map(|(k, _)| k.schema.as_str())
    }

    /// Get cached columns for a table. Returns None if not yet loaded.
    pub fn get_columns(&self, schema: &str, table: &str) -> Option<&[ResolvedColumn]> {
        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
        self.columns.get(&key).map(|v| v.as_slice())
    }

    /// Check if columns are already cached for a table.
    pub fn has_columns_cached(&self, schema: &str, table: &str) -> bool {
        let key = format!("{}.{}", schema.to_uppercase(), table.to_uppercase());
        self.columns.contains_key(&key)
    }

    /// All schemas whose names start with `prefix` (case-insensitive).
    #[allow(dead_code)]
    pub fn schemas_matching(&self, prefix: &str) -> Vec<&str> {
        let upper = prefix.to_uppercase();
        self.schemas
            .iter()
            .filter(|(key, _)| key.starts_with(&upper))
            .map(|(_, display)| display.as_str())
            .collect()
    }

    /// All schema display names.
    pub fn all_schemas(&self) -> Vec<&str> {
        self.schemas.values().map(|s| s.as_str()).collect()
    }

    /// All objects of given kinds, optionally filtered by schema.
    pub fn objects_by_kind(&self, schema: Option<&str>, kinds: &[ObjectKind]) -> Vec<&ObjectEntry> {
        let schema_upper = schema.map(|s| s.to_uppercase());
        self.objects
            .iter()
            .filter(|(key, entry)| {
                kinds.contains(&entry.kind)
                    && schema_upper.as_ref().is_none_or(|s| key.schema == *s)
            })
            .map(|(_, entry)| entry)
            .collect()
    }

    /// Find tables with FK relationships to a given table (for JOIN suggestions).
    pub fn fk_related_tables(&self, schema: &str, table: &str) -> Vec<&ForeignKey> {
        let s = schema.to_uppercase();
        let t = table.to_uppercase();
        self.foreign_keys
            .iter()
            .filter(|fk| {
                (fk.from_schema.to_uppercase() == s && fk.from_table.to_uppercase() == t)
                    || (fk.to_schema.to_uppercase() == s && fk.to_table.to_uppercase() == t)
            })
            .collect()
    }

    /// Find the schema for an unqualified object name. Returns the first match.
    pub fn resolve_schema_for(&self, object_name: &str) -> Option<&str> {
        let upper = object_name.to_uppercase();
        self.objects
            .iter()
            .find(|(key, _)| key.name == upper)
            .map(|(_, entry)| entry.schema_display.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> MetadataIndex {
        let mut idx = MetadataIndex::new();
        idx.set_db_type(DatabaseType::Oracle);
        idx.set_current_schema("HR");
        idx.add_schema("HR");
        idx.add_schema("FINANCE");
        idx.add_object("HR", "EMPLOYEES", ObjectKind::Table);
        idx.add_object("HR", "DEPARTMENTS", ObjectKind::Table);
        idx.add_object("HR", "EMPLOYEE_SUMMARY", ObjectKind::View);
        idx.add_object("FINANCE", "INVOICES", ObjectKind::Table);
        idx
    }

    #[test]
    fn is_known_schema_case_insensitive() {
        let idx = sample_index();
        assert!(idx.is_known_schema("hr"));
        assert!(idx.is_known_schema("HR"));
        assert!(idx.is_known_schema("Hr"));
        assert!(!idx.is_known_schema("SALES"));
    }

    #[test]
    fn is_known_object_qualified() {
        let idx = sample_index();
        assert!(idx.is_known_object(Some("HR"), "employees"));
        assert!(idx.is_known_object(Some("hr"), "EMPLOYEES"));
        assert!(!idx.is_known_object(Some("HR"), "INVOICES"));
    }

    #[test]
    fn is_known_object_unqualified() {
        let idx = sample_index();
        assert!(idx.is_known_object(None, "employees"));
        assert!(idx.is_known_object(None, "INVOICES"));
        assert!(!idx.is_known_object(None, "NONEXISTENT"));
    }

    #[test]
    fn schemas_matching_prefix() {
        let idx = sample_index();
        let matches = idx.schemas_matching("F");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "FINANCE");

        let matches = idx.schemas_matching("");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn objects_by_kind_filtered() {
        let idx = sample_index();
        let tables = idx.objects_by_kind(Some("HR"), &[ObjectKind::Table]);
        assert_eq!(tables.len(), 2);

        let views = idx.objects_by_kind(Some("HR"), &[ObjectKind::View]);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].display_name, "EMPLOYEE_SUMMARY");
    }

    #[test]
    fn column_cache() {
        let mut idx = sample_index();
        assert!(!idx.has_columns_cached("HR", "EMPLOYEES"));
        assert!(idx.get_columns("HR", "EMPLOYEES").is_none());

        idx.cache_columns(
            "HR",
            "EMPLOYEES",
            vec![ResolvedColumn {
                name: "EMPLOYEE_ID".to_string(),
                data_type: "NUMBER".to_string(),
                nullable: false,
                is_primary_key: true,
                table_schema: "HR".to_string(),
                table_name: "EMPLOYEES".to_string(),
            }],
        );

        assert!(idx.has_columns_cached("HR", "EMPLOYEES"));
        assert!(idx.has_columns_cached("hr", "employees")); // case-insensitive
        let cols = idx.get_columns("HR", "EMPLOYEES").unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "EMPLOYEE_ID");
    }

    #[test]
    fn fk_related_tables() {
        let mut idx = sample_index();
        idx.add_foreign_key(ForeignKey {
            constraint_name: "FK_EMP_DEPT".to_string(),
            from_schema: "HR".to_string(),
            from_table: "EMPLOYEES".to_string(),
            from_columns: vec!["DEPARTMENT_ID".to_string()],
            to_schema: "HR".to_string(),
            to_table: "DEPARTMENTS".to_string(),
            to_columns: vec!["DEPARTMENT_ID".to_string()],
        });

        let related = idx.fk_related_tables("HR", "EMPLOYEES");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].to_table, "DEPARTMENTS");

        // Also found from the other side
        let related = idx.fk_related_tables("HR", "DEPARTMENTS");
        assert_eq!(related.len(), 1);
    }

    #[test]
    fn resolve_schema_for_object() {
        let idx = sample_index();
        assert_eq!(idx.resolve_schema_for("employees"), Some("HR"));
        assert_eq!(idx.resolve_schema_for("INVOICES"), Some("FINANCE"));
        assert_eq!(idx.resolve_schema_for("nonexistent"), None);
    }

    #[test]
    fn clear_resets_everything() {
        let mut idx = sample_index();
        idx.clear();
        assert!(!idx.is_known_schema("HR"));
        assert!(!idx.is_known_object(None, "EMPLOYEES"));
        assert!(idx.db_type().is_none());
        assert!(idx.current_schema().is_none());
    }
}
