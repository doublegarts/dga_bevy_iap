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
        /// Offering to buy from; `None` for the current offering.
        offering_id: Option<String>,
        /// Package identifier, for example `$rc_monthly`.
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
    /// Offerings were loaded; the [`Offerings`] resource now holds them.
    OfferingsUpdated(Offerings),
    /// Answer to [`IapRequest::FetchProducts`]. Ids the store doesn't know are left out.
    ProductsFetched(Vec<Product>),
    /// Validated by RevenueCat. Grant the item here; RevenueCat has already consumed or acknowledged it.
    PurchaseCompleted {
        /// Store product id that was bought.
        product_id: String,
        /// Store transaction id (Play order id or App Store transaction id), when the store gives one.
        transaction_id: Option<String>,
        /// The player's entitlements after the purchase.
        customer_info: CustomerInfo,
    },
    /// Waiting on the player or a payment (Ask to Buy, cash). A later
    /// [`IapEvent::CustomerInfoUpdated`] carries the result.
    PurchasePending {
        /// Store product id that is waiting.
        product_id: String,
    },
    /// The player closed the store sheet without buying.
    PurchaseCancelled {
        /// Store product id that was not bought.
        product_id: String,
    },
    /// The purchase didn't go through.
    PurchaseFailed {
        /// Store product id; empty when the package couldn't be found.
        product_id: String,
        /// What went wrong.
        error: IapError,
    },
    /// Answer to [`IapRequest::Restore`], with the player's restored entitlements.
    Restored(CustomerInfo),
    /// Entitlements changed: a renewal, expiry, refund, or a pending purchase that went through.
    CustomerInfoUpdated(CustomerInfo),
    /// Answer to [`IapRequest::LogIn`].
    LoggedIn {
        /// `true` when RevenueCat had never seen this user id before.
        created: bool,
        /// The logged-in user's entitlements.
        customer_info: CustomerInfo,
    },
    /// Answer to [`IapRequest::LogOut`], with the new anonymous user's entitlements.
    LoggedOut(CustomerInfo),
    /// A request other than a purchase failed.
    RequestFailed {
        /// The failed request, for example `get_offerings` or `restore`.
        request: String,
        /// What went wrong.
        error: IapError,
    },
}
