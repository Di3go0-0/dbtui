pub mod mysql;
pub mod oracle;
pub mod postgres;

pub use mysql::MysqlAdapter;
pub use oracle::OracleAdapter;
pub use postgres::PostgresAdapter;

use crate::core::DatabaseAdapter;
use crate::core::error::DbError;
use crate::core::models::{ConnectionConfig, DatabaseType};

/// Factory: create the appropriate adapter from a connection config.
pub async fn create_adapter(
    config: &ConnectionConfig,
) -> Result<Box<dyn DatabaseAdapter>, DbError> {
    match config.db_type {
        DatabaseType::PostgreSQL => {
            let conn_str = format!(
                "postgres://{}:{}@{}:{}/{}",
                config.username,
                config.password,
                config.host,
                config.port,
                config.database.as_deref().unwrap_or("postgres")
            );
            let adapter = PostgresAdapter::connect(&conn_str).await?;
            Ok(Box::new(adapter))
        }
        DatabaseType::MySQL => {
            let conn_str = format!(
                "mysql://{}:{}@{}:{}/{}",
                config.username,
                config.password,
                config.host,
                config.port,
                config.database.as_deref().unwrap_or("")
            );
            let adapter = MysqlAdapter::connect(&conn_str).await?;
            Ok(Box::new(adapter))
        }
        DatabaseType::Oracle => {
            let connect_string = format!(
                "//{}:{}/{}",
                config.host,
                config.port,
                config.database.as_deref().unwrap_or("ORCL")
            );
            let adapter =
                OracleAdapter::connect(&config.username, &config.password, &connect_string).await?;
            Ok(Box::new(adapter))
        }
    }
}
