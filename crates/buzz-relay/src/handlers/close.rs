use std::sync::Arc;

use tracing::debug;

use crate::connection::ConnectionState;
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Handle a CLOSE command — remove the subscription and send CLOSED acknowledgement.
pub async fn handle_close(sub_id: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let conn_id = conn.conn_id;

    conn.subscriptions.lock().await.remove(&sub_id);

    // Deregister from the fan-out index before sending CLOSED so no new
    // messages are routed to this sub after the client's CLOSE is acknowledged.
    // Release the subscription's topic so the pubsub manager can drop the Redis
    // subscription once this pod has no remaining local interest in it.
    if let Some(topic) = state.sub_registry.remove_subscription(conn_id, &sub_id) {
        state.pubsub.release_topic(&conn.tenant, topic).await;
    }

    conn.send(RelayMessage::closed(&sub_id, ""));

    debug!(conn_id = %conn_id, sub_id = %sub_id, "Subscription closed");
}
