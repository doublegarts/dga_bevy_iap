//! Wire protocol between Rust and the native RevenueCat bridges.
//!
//! Rust sends a [`Command`] as one JSON object; the Kotlin and Swift bridges answer with
//! [`NativeMessage`] JSON objects, from any thread, into a global inbox that a Bevy system drains.

use std::sync::LazyLock;

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::{Deserialize, Serialize};

use crate::types::{CustomerInfo, IapError, Offerings, Product};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub(crate) enum Command {
    Configure {
        api_key: String,
        app_user_id: Option<String>,
        debug_logs: bool,
    },
    GetOfferings,
    GetProducts {
        product_ids: Vec<String>,
    },
    PurchasePackage {
        offering_id: Option<String>,
        package_id: String,
    },
    PurchaseProduct {
        product_id: String,
    },
    Restore,
    GetCustomerInfo,
    LogIn {
        app_user_id: String,
    },
    LogOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum NativeMessage {
    Configured,
    ConfigureFailed {
        error: IapError,
    },
    Offerings {
        offerings: Offerings,
    },
    Products {
        products: Vec<Product>,
    },
    PurchaseCompleted {
        product_id: String,
        #[serde(default)]
        transaction_id: Option<String>,
        customer_info: CustomerInfo,
    },
    PurchasePending {
        product_id: String,
    },
    PurchaseCancelled {
        product_id: String,
    },
    PurchaseFailed {
        product_id: String,
        error: IapError,
    },
    Restored {
        customer_info: CustomerInfo,
    },
    CustomerInfo {
        customer_info: CustomerInfo,
    },
    LoggedIn {
        created: bool,
        customer_info: CustomerInfo,
    },
    LoggedOut {
        customer_info: CustomerInfo,
    },
    Error {
        op: String,
        error: IapError,
    },
}

/// Inbox for the native bridges, which call back through C symbols and so need a global.
#[cfg_attr(not(any(target_os = "android", target_os = "ios", test)), allow(dead_code))]
static NATIVE_INBOX: LazyLock<(Sender<NativeMessage>, Receiver<NativeMessage>)> = LazyLock::new(unbounded);

#[cfg_attr(not(any(target_os = "android", target_os = "ios", test)), allow(dead_code))]
pub(crate) fn native_inbox() -> Receiver<NativeMessage> {
    NATIVE_INBOX.1.clone()
}

/// Parses a JSON message from a native bridge and queues it.
#[cfg_attr(not(any(target_os = "android", target_os = "ios", test)), allow(dead_code))]
pub(crate) fn post_native_json(json: &str) {
    match serde_json::from_str::<NativeMessage>(json) {
        // The receiver lives in a static, so the channel never disconnects.
        Ok(message) => _ = NATIVE_INBOX.0.send(message),
        Err(err) => {
            tracing::error!("dga_bevy_iap: unreadable message from native bridge ({err}): {json}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_json_matches_native_bridges() {
        let json = serde_json::to_value(Command::PurchasePackage {
            offering_id: None,
            package_id: "$rc_monthly".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"cmd": "purchase_package", "offering_id": null, "package_id": "$rc_monthly"})
        );
    }

    #[test]
    fn parses_native_purchase_message() {
        let json = r#"{"type":"purchase_completed","product_id":"coins_100","transaction_id":"GPA.1",
            "customer_info":{"app_user_id":"u1","active_entitlements":{},"active_subscriptions":[],
            "all_purchased_product_ids":["coins_100"],"management_url":null}}"#;
        let message: NativeMessage = serde_json::from_str(json).unwrap();
        assert!(
            matches!(message, NativeMessage::PurchaseCompleted { ref product_id, .. } if product_id == "coins_100")
        );
    }

    #[test]
    fn native_json_reaches_the_inbox() {
        post_native_json(r#"{"type":"purchase_cancelled","product_id":"coins_100"}"#);
        post_native_json("not json");
        assert_eq!(
            native_inbox().try_iter().collect::<Vec<_>>(),
            vec![NativeMessage::PurchaseCancelled {
                product_id: "coins_100".into()
            }]
        );
    }

    #[test]
    fn unknown_product_kind_is_tolerated() {
        let product: Product = serde_json::from_str(
            r#"{"id":"x","title":"","description":"","price_string":"","price_micros":0,"currency_code":"","kind":"prepaid"}"#,
        )
        .unwrap();
        assert_eq!(product.kind, crate::ProductKind::Unknown);
    }
}
