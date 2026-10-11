//! Messages games send to the store and receive back.

use bevy_ecs::message::Message;

use crate::types::{CustomerInfo, IapError, Offerings, Product};

/// Ask the store to do something. Write it with `MessageWriter<IapRequest>`.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum IapRequest {
    /// Reload offerings from RevenueCat. Done automatically once at startup.
    FetchOfferings,
    /// Load products that are not part of an offering.
    FetchProducts(Vec<String>),
    /// Buy a package from an offering. `offering_id: None` uses the current offering.
    PurchasePackage {
        offering_id: Option<String>,
        package_id: String,
    },
    /// Buy a store product directly by id.
    PurchaseProduct(String),
    /// Restore purchases made with the player's store account (Apple requires a Restore button).
    Restore,
    /// Reload [`CustomerInfo`] from RevenueCat.
    RefreshCustomerInfo,
    /// Switch to your own user id, so purchases follow the player across devices.
    LogIn(String),
    /// Go back to an anonymous RevenueCat user.
    LogOut,
}

/// Something the store did. Read it with `MessageReader<IapEvent>`.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum IapEvent {
    /// The SDK is configured and requests can be sent.
    Ready,
    /// The store can't be used; see [`IapStatus::Unavailable`](crate::IapStatus::Unavailable).
    Unavailable(IapError),
    OfferingsUpdated(Offerings),
    ProductsFetched(Vec<Product>),
    /// Validated by RevenueCat. Grant the item here; RevenueCat has already consumed or acknowledged it.
    PurchaseCompleted {
        product_id: String,
        transaction_id: Option<String>,
        customer_info: CustomerInfo,
    },
    /// Waiting on the player or a payment (Ask to Buy, cash). A later
    /// [`IapEvent::CustomerInfoUpdated`] carries the result.
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
    Restored(CustomerInfo),
    /// Entitlements changed: a renewal, expiry, refund, or a pending purchase that went through.
    CustomerInfoUpdated(CustomerInfo),
    LoggedIn {
        created: bool,
        customer_info: CustomerInfo,
    },
    LoggedOut(CustomerInfo),
    /// A request other than a purchase failed.
    RequestFailed {
        request: String,
        error: IapError,
    },
}
