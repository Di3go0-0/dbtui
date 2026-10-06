//! A minimal connection pool for tiberius.
//!
//! tiberius ships no pool of its own, and a single shared client would
//! serialize every metadata fetch behind whatever long query is streaming.
//! This keeps a small stack of idle clients, grows it on demand up to
//! `MAX_CONNECTIONS`, and hands each one back on drop.

use std::sync::{Arc, Mutex};

use tiberius::{Client, Config};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::core::error::{DbError, DbResult};

pub type MssqlClient = Client<Compat<TcpStream>>;

/// Upper bound on concurrent server connections. The UI issues at most a
/// streaming query plus a handful of metadata fetches at once.
const MAX_CONNECTIONS: usize = 4;

/// How many times to follow an Azure SQL routing redirect before giving up.
const MAX_REDIRECTS: usize = 3;

pub struct MssqlPool {
    config: Config,
    idle: Mutex<Vec<MssqlClient>>,
    permits: Arc<Semaphore>,
}

impl MssqlPool {
    /// Open one connection eagerly so `connect()` fails fast on bad
    /// credentials instead of at the first query.
    pub async fn connect(config: Config) -> DbResult<Arc<Self>> {
        let client = open(&config).await?;
        Ok(Arc::new(Self {
            config,
            idle: Mutex::new(vec![client]),
            permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        }))
    }

    /// Check out a client, opening a new connection if none are idle.
    pub async fn get(self: &Arc<Self>) -> DbResult<PooledClient> {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| DbError::QueryFailed(format!("connection pool closed: {e}")))?;

        let pooled = self.idle.lock().ok().and_then(|mut idle| idle.pop());
        let client = match pooled {
            Some(client) => client,
            None => open(&self.config).await?,
        };

        Ok(PooledClient {
            client: Some(client),
            pool: Arc::clone(self),
            _permit: permit,
        })
    }
}

/// A checked-out client that returns itself to the pool when dropped.
pub struct PooledClient {
    client: Option<MssqlClient>,
    pool: Arc<MssqlPool>,
    _permit: OwnedSemaphorePermit,
}

impl PooledClient {
    /// Borrow the underlying client.
    ///
    /// The `Option` is only empty after `discard()`, which consumes `self`, so
    /// in practice this always yields a client — it returns a `DbResult`
    /// rather than unwrapping to keep the driver panic-free.
    pub fn client(&mut self) -> DbResult<&mut MssqlClient> {
        self.client
            .as_mut()
            .ok_or_else(|| DbError::QueryFailed("connection already released".to_string()))
    }

    /// Give up on this connection instead of recycling it. Used after an error
    /// that may have left unread tokens in the TDS stream, which would
    /// desynchronize the next query to reuse it.
    pub fn discard(mut self) {
        self.client = None;
    }
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        if let Some(client) = self.client.take()
            && let Ok(mut idle) = self.pool.idle.lock()
            && idle.len() < MAX_CONNECTIONS
        {
            idle.push(client);
        }
    }
}

/// Open one connection, following Azure SQL routing redirects.
async fn open(config: &Config) -> DbResult<MssqlClient> {
    let mut config = config.clone();

    for _ in 0..MAX_REDIRECTS {
        let tcp = TcpStream::connect(config.get_addr())
            .await
            .map_err(|e| connect_error(&e.to_string()))?;
        tcp.set_nodelay(true)
            .map_err(|e| connect_error(&e.to_string()))?;

        match Client::connect(config.clone(), tcp.compat_write()).await {
            Ok(client) => return Ok(client),
            // Azure SQL answers the first connect with the address of the node
            // actually holding the database; reconnect there.
            Err(tiberius::error::Error::Routing { host, port }) => {
                config.host(&host);
                config.port(port);
            }
            Err(e) => return Err(connect_error(&e.to_string())),
        }
    }

    Err(connect_error(
        "server kept redirecting the connection to another address",
    ))
}

fn connect_error(raw: &str) -> DbError {
    DbError::ConnectionFailed(crate::core::error::friendly_connect_error(
        crate::core::models::DatabaseType::SqlServer,
        raw,
    ))
}
