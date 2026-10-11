//! Platform backends. Each one forwards [`Command`]s to a store and answers with
//! [`NativeMessage`](crate::bridge::NativeMessage)s on the receiver it hands back.

use crate::bridge::Command;

#[cfg(target_os = "android")]
pub(crate) mod android;
#[cfg(target_os = "ios")]
pub(crate) mod ios;
pub mod mock;

pub(crate) trait Backend: Send + Sync + 'static {
    fn send(&self, command: Command);
}
