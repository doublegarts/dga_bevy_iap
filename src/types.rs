//! Store data as the game sees it: products, offerings and the player's entitlements.

use std::collections::HashMap;

use bevy_ecs::resource::Resource;
use serde::{Deserialize, Serialize};

/// Whether a product is bought once or renews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    /// Consumables and non-consumables (Play `INAPP`, App Store consumable / non-consumable).
    OneTime,
    /// Auto-renewable or non-renewing subscription.
    Subscription,
    /// The store reported a type this crate doesn't know.
    #[default]
    #[serde(other)]
    Unknown,
}

/// A product as configured in Google Play Console / App Store Connect, with localized pricing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Product {
    /// Store product identifier, for example `coins_100`.
    pub id: String,
    /// Localized name from the store listing.
    pub title: String,
    /// Localized description from the store listing.
    pub description: String,
    /// Localized price ready to show, for example `€4.99`.
    pub price_string: String,
    /// Price in millionths of the currency unit (4.99 is `4_990_000`).
    pub price_micros: i64,
    /// ISO 4217 currency code, for example `EUR`.
    pub currency_code: String,
    /// One-time purchase or subscription.
    pub kind: ProductKind,
    /// ISO 8601 billing period for subscriptions, for example `P1M`.
    #[serde(default)]
    pub subscription_period: Option<String>,
}

/// One purchasable item in an offering, as set up in the RevenueCat dashboard.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Package {
    /// Package identifier, for example `$rc_monthly` or a custom one.
    pub id: String,
    /// RevenueCat package type, for example `MONTHLY`, `LIFETIME` or `CUSTOM`.
    pub package_type: String,
    /// The store product this package sells.
    pub product: Product,
}

/// A set of packages shown together, typically one paywall or shop page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Offering {
    /// Offering identifier from the dashboard, for example `default`.
    pub id: String,
    /// Description entered in the dashboard.
    pub description: String,
    /// Packages in dashboard order.
    pub packages: Vec<Package>,
}

impl Offering {
    /// Finds a package by its identifier, for example `$rc_monthly`.
    pub fn package(&self, package_id: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.id == package_id)
    }
}

/// Offerings fetched from RevenueCat. Empty until the first [`IapEvent::OfferingsUpdated`](crate::IapEvent::OfferingsUpdated).
#[derive(Resource, Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Offerings {
    /// Identifier of the offering marked current in the dashboard.
    pub current: Option<String>,
    /// Every offering in the project.
    pub all: Vec<Offering>,
}

impl Offerings {
    /// The offering marked current in the dashboard, if there is one.
    pub fn current(&self) -> Option<&Offering> {
        self.current.as_deref().and_then(|id| self.get(id))
    }

    /// Finds an offering by its identifier.
    pub fn get(&self, offering_id: &str) -> Option<&Offering> {
        self.all.iter().find(|o| o.id == offering_id)
    }
}

/// Products fetched with [`IapRequest::FetchProducts`](crate::IapRequest::FetchProducts), by product id.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub struct Products(pub HashMap<String, Product>);

impl Products {
    /// Finds a product by its store id.
    pub fn get(&self, product_id: &str) -> Option<&Product> {
        self.0.get(product_id)
    }
}

/// An entitlement the player has unlocked, for example `premium`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Entitlement {
    /// Entitlement identifier from the dashboard.
    pub id: String,
    /// Product that granted it.
    pub product_id: String,
    /// Whether the entitlement currently grants access.
    pub is_active: bool,
    /// Whether the subscription behind it will renew; `false` once the player cancels.
    pub will_renew: bool,
    /// Expiry in Unix milliseconds; `None` for lifetime purchases.
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    /// Latest purchase or renewal in Unix milliseconds.
    #[serde(default)]
    pub latest_purchase_at_ms: Option<i64>,
    /// `true` for test purchases (Play license testers, App Store sandbox).
    pub is_sandbox: bool,
}

/// What the player owns, as reported by RevenueCat. RevenueCat validates every receipt on its servers
/// before it appears here.
#[derive(Resource, Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CustomerInfo {
    /// RevenueCat app user id (anonymous `$RCAnonymousID:...` unless you log in).
    pub app_user_id: String,
    /// Active entitlements by entitlement id.
    pub active_entitlements: HashMap<String, Entitlement>,
    /// Product ids of active subscriptions.
    pub active_subscriptions: Vec<String>,
    /// Every product id ever purchased, including consumables.
    pub all_purchased_product_ids: Vec<String>,
    /// Store page where the player manages subscriptions.
    #[serde(default)]
    pub management_url: Option<String>,
}

impl CustomerInfo {
    /// `true` when the entitlement is active, for example `customer.has_entitlement("premium")`.
    pub fn has_entitlement(&self, entitlement_id: &str) -> bool {
        self.active_entitlements
            .get(entitlement_id)
            .is_some_and(|e| e.is_active)
    }
}

/// An error reported by RevenueCat or the store.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IapError {
    /// RevenueCat error code name, for example `NETWORK_ERROR` or `STORE_PROBLEM_ERROR`.
    pub code: String,
    /// Human-readable detail from the SDK or store.
    pub message: String,
}

impl std::fmt::Display for IapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IapError {}

/// Whether the store is ready to take requests.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub enum IapStatus {
    /// The plugin hasn't started configuring RevenueCat yet.
    #[default]
    NotConfigured,
    /// RevenueCat is being configured.
    Configuring,
    /// Requests can be sent.
    Ready,
    /// The store can't be used, for example a missing API key or no Play services.
    Unavailable(String),
}

impl IapStatus {
    /// `true` when the store is [`IapStatus::Ready`].
    pub fn is_ready(&self) -> bool {
        matches!(self, IapStatus::Ready)
    }
}
