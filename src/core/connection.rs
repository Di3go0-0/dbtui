use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::adapter::DatabaseAdapter;
use crate::core::models::{ConnectionId, DatabaseType};

#[allow(dead_code)]
pub struct Session {
    pub id: ConnectionId,
    pub db_type: DatabaseType,
    pub connection_name: String,
    pub current_schema: Option<String>,
    pub adapter: Box<dyn DatabaseAdapter>,
}

#[allow(dead_code)]
pub struct ConnectionEntry {
    pub id: ConnectionId,
    pub name: String,
    pub db_type: DatabaseType,
    pub adapter: Arc<dyn DatabaseAdapter>,
}

#[allow(dead_code)]
pub struct ConnectionManager {
    connections: Arc<RwLock<HashMap<ConnectionId, ConnectionEntry>>>,
    next_id: AtomicU64,
}

#[allow(dead_code)]
impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn add(
        &self,
        name: String,
        db_type: DatabaseType,
        adapter: Arc<dyn DatabaseAdapter>,
    ) -> ConnectionId {
        let id = ConnectionId(self.next_id.fetch_add(1, Ordering::SeqCst));
        let entry = ConnectionEntry {
            id,
            name,
            db_type,
            adapter,
        };
        self.connections.write().await.insert(id, entry);
        id
    }

    pub async fn get(&self, id: ConnectionId) -> Option<Arc<dyn DatabaseAdapter>> {
        self.connections
            .read()
            .await
            .get(&id)
            .map(|e| Arc::clone(&e.adapter))
    }

    pub async fn remove(&self, id: ConnectionId) {
        self.connections.write().await.remove(&id);
    }

    pub async fn list(&self) -> Vec<(ConnectionId, String, DatabaseType)> {
        self.connections
            .read()
            .await
            .values()
            .map(|e| (e.id, e.name.clone(), e.db_type.clone()))
            .collect()
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
