#[cfg(test)]
mod live_tests;
pub mod mssql;
pub mod mysql;
pub mod oracle;
pub mod postgres;
mod sink;
mod statement;

pub use mssql::MssqlAdapter;
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
            let adapter = PostgresAdapter::connect_with_config(config).await?;
            Ok(Box::new(adapter))
        }
        DatabaseType::MySQL => {
            let adapter = MysqlAdapter::connect_with_config(config).await?;
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
        DatabaseType::SqlServer => {
            let adapter = MssqlAdapter::connect(
                &config.host,
                config.port,
                config.database.as_deref(),
                &config.username,
                &config.password,
            )
            .await?;
            Ok(Box::new(adapter))
        }
    }
}
