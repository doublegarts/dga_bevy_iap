//! Platform backends. Each one forwards [`Command`]s to a store and answers with
//! [`NativeMessage`](crate::bridge::NativeMessage)s on the receiver it hands back.

use crate::bridge::Command;

#[cfg(target_os = "android")]
pub(crate) mod android;
#[cfg(target_os = "ios")]
pub(crate) mod ios;
pub mod mock;

/// A store the plugin can talk to.
pub(crate) trait Backend: Send + Sync + 'static {
    /// Sends one command. Results come back later, on the inbox the backend was created with.
    fn send(&self, command: Command);
}
